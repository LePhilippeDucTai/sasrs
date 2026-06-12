//! Compilation de l'étape DATA : AST → `StepProgram` exécutable.
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : ÉLEVÉE — sémantique SAS)
//!
//! SAS exécute une étape DATA en deux phases ; ce module est la phase de
//! COMPILATION. Une passe sur l'AST construit :
//!
//! ## 1. Le PDV (Program Data Vector)
//! - Variables dans l'ORDRE DE PREMIÈRE RÉFÉRENCE textuelle (cet ordre
//!   définit l'ordre des colonnes en sortie !).
//! - `set lib.a` : lire le dataset (via `session.libs`, libref par défaut
//!   WORK) — ses variables entrent dans le PDV avec type/longueur/format
//!   de `VarMeta`, marquées `from_input=true` (elles ne sont PAS remises
//!   à missing à chaque itération). Logger les notes de coercition
//!   parquet (`LogWriter::forward`).
//! - Cible d'assignation : si absente du PDV, créer avec le type INFÉRÉ
//!   de l'expression (voir §3). Variable seulement RÉFÉRENCÉE (jamais
//!   assignée ni lue d'un input) : créer numérique + NOTE
//!   "Variable x is uninitialized." à l'exécution de la 1re itération.
//!
//! ## 2. Les sorties
//! Pour chaque dataset de `ast.outputs` : appliquer KEEP/DROP (statements
//! M1 ; options de dataset M2) → `kept_slots` = indices PDV dans l'ordre
//! PDV. KEEP et DROP simultanés : DROP gagne sur l'intersection (SAS
//! émet WARNING). Variables KEEP inexistantes → ERROR à la compilation
//! ("The variable x in the DROP, KEEP, or RENAME list has never been
//! referenced.").
//!
//! ## 3. Inférence de type d'une expression (compile-time, comme SAS)
//! - littéral num / opération arithmétique / comparaison / logique → Num
//! - littéral chaîne → Char(longueur du littéral)
//! - `Var` → type/longueur de la variable au PDV
//! - `||` → Char(somme des longueurs des opérandes)
//! - `Call` : table par fonction (upcase/lowcase/trim/strip/left →
//!   Char(longueur arg), substr → Char(longueur arg1), cat* → 200,
//!   put → largeur du format ; défaut → Num)
//! - La PREMIÈRE assignation fige type et longueur (redéfinir → la
//!   longueur d'origine reste, SAS tronque silencieusement).
//!
//! ## 4. has_explicit_output
//! Si AU MOINS UN `output;` apparaît dans l'étape, l'output implicite de
//! fin d'itération est désactivé (règle SAS).
//!
//! Erreur de compilation → l'exécuteur loggue ERROR + NOTE "The SAS
//! System stopped processing this step because of errors." et n'exécute
//! pas (mais la session continue).
//!
//! ## Choix d'implémentation
//! - Ordre de première référence d'une assignation : la CIBLE entre au PDV
//!   avant les variables de son expression (ordre textuel gauche→droite).
//! - Opérande numérique de `||` : contribue 12 à la longueur inférée
//!   (conversion implicite BEST12. comme SAS).
//! - `put(x, fmt)` : largeur = chiffres finaux du nom de format si
//!   disponibles, sinon 200 (le parser M1 ne sait pas encore produire un
//!   littéral de format ; best-effort documenté).
//! - Un seul SET par étape en M1 ; le second → erreur "not yet implemented".

pub mod eval;
pub mod exec;
pub mod functions;
pub mod pdv;

use crate::ast::{BinaryOp, DataStepAst, DatasetRef, DsStmt, Expr};
use crate::error::{Result, SasError};
use crate::missing::num_to_value;
use crate::session::Session;
use crate::value::{Value, VarType};
use pdv::{Pdv, PdvVar};
use std::collections::HashSet;

/// Données d'entrée matérialisées (M1 : un seul SET).
pub struct InputData {
    /// "WORK.B" pour les NOTEs du log.
    pub display: String,
    /// Colonnes décodées en `Value` (via `missing::num_to_value`), une
    /// seule passe de downcast par colonne — jamais de get_row.
    pub columns: Vec<Vec<Value>>,
    /// Slot PDV de chaque colonne (parallèle à `columns`).
    pub var_slots: Vec<usize>,
    pub n_rows: usize,
}

/// Une sortie : où écrire et quels slots du PDV.
pub struct OutputSpec {
    pub libref: String,
    pub table: String,
    /// "WORK.A" pour les NOTEs.
    pub display: String,
    /// Slots PDV conservés, dans l'ordre PDV (= ordre des colonnes).
    pub kept_slots: Vec<usize>,
}

pub struct StepProgram {
    pub pdv: Pdv,
    pub stmts: Vec<crate::ast::DsStmt>,
    pub input: Option<InputData>,
    pub outputs: Vec<OutputSpec>,
    pub has_explicit_output: bool,
    /// Noms (casse de première référence, ordre PDV) des variables jamais
    /// assignées ni lues d'un input : l'exécuteur émet la NOTE
    /// "Variable x is uninitialized." à la première itération.
    pub uninitialized: Vec<String>,
}

pub fn compile(ast: &DataStepAst, session: &mut Session) -> Result<StepProgram> {
    let mut c = Compiler {
        pdv: Pdv::new(),
        session,
        input: None,
        keeps: Vec::new(),
        drops: Vec::new(),
        assigned: HashSet::new(),
        has_explicit_output: false,
    };
    for stmt in &ast.stmts {
        c.walk_stmt(stmt)?;
    }
    let outputs = c.resolve_outputs(&ast.outputs)?;
    let uninitialized = c
        .pdv
        .vars()
        .iter()
        .filter(|v| !v.from_input && !c.assigned.contains(&v.name.to_uppercase()))
        .map(|v| v.name.clone())
        .collect();
    Ok(StepProgram {
        pdv: c.pdv,
        stmts: ast.stmts.clone(),
        input: c.input,
        outputs,
        has_explicit_output: c.has_explicit_output,
        uninitialized,
    })
}

struct Compiler<'a> {
    pdv: Pdv,
    session: &'a mut Session,
    input: Option<InputData>,
    keeps: Vec<String>,
    drops: Vec<String>,
    /// Noms (uppercase) ayant au moins une assignation dans l'étape.
    assigned: HashSet<String>,
    has_explicit_output: bool,
}

impl Compiler<'_> {
    fn walk_stmt(&mut self, stmt: &DsStmt) -> Result<()> {
        match stmt {
            DsStmt::Set(r) => self.compile_set(r),
            DsStmt::Assign { var, expr } => {
                // La cible entre au PDV en premier (ordre textuel), avec le
                // type inféré AVANT création des variables de l'expression
                // (les inconnues comptent comme Num, cohérent avec SAS).
                let (ty, length) = self.infer(expr);
                self.add_var(var, ty, length);
                self.assigned.insert(var.to_uppercase());
                self.walk_expr(expr);
                Ok(())
            }
            DsStmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.walk_expr(cond);
                self.walk_stmt(then_branch)?;
                if let Some(e) = else_branch {
                    self.walk_stmt(e)?;
                }
                Ok(())
            }
            DsStmt::SubsettingIf(cond) => {
                self.walk_expr(cond);
                Ok(())
            }
            DsStmt::Block(stmts) => {
                for s in stmts {
                    self.walk_stmt(s)?;
                }
                Ok(())
            }
            DsStmt::Output => {
                self.has_explicit_output = true;
                Ok(())
            }
            DsStmt::Keep(names) => {
                self.keeps.extend(names.iter().cloned());
                Ok(())
            }
            DsStmt::Drop(names) => {
                self.drops.extend(names.iter().cloned());
                Ok(())
            }
            DsStmt::Stop => Ok(()),
        }
    }

    /// Crée les variables simplement référencées (Num par défaut), en ordre
    /// textuel gauche→droite.
    fn walk_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Num(_) | Expr::Str(_) | Expr::Missing(_) => {}
            Expr::Var(name) => {
                self.add_var(name, VarType::Num, 8);
            }
            Expr::Unary { expr, .. } => self.walk_expr(expr),
            Expr::Binary { left, right, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::In { expr, list } => {
                self.walk_expr(expr);
                for e in list {
                    self.walk_expr(e);
                }
            }
            Expr::Call { args, .. } => {
                for a in args {
                    self.walk_expr(a);
                }
            }
        }
    }

    /// `add_var` PDV : la première référence fige tout (le PDV ignore les
    /// ajouts suivants du même nom).
    fn add_var(&mut self, name: &str, ty: VarType, length: usize) -> usize {
        self.pdv.add_var(PdvVar {
            name: name.to_string(),
            ty,
            length,
            retained: false,
            from_input: false,
            format: None,
        })
    }

    fn compile_set(&mut self, r: &DatasetRef) -> Result<()> {
        if self.input.is_some() {
            return Err(SasError::runtime(
                "Multiple SET statements are not yet implemented.",
            ));
        }
        let libref = r.libref_or_work();
        let provider = self.session.libs.get(&libref)?;
        if !provider.exists(&r.name) {
            return Err(SasError::runtime(format!(
                "File {}.DATA does not exist.",
                r.display()
            )));
        }
        let (ds, notes) = provider.read(&r.name)?;
        for note in &notes {
            self.session.log.forward(note);
        }

        let mut columns = Vec::with_capacity(ds.vars.len());
        let mut var_slots = Vec::with_capacity(ds.vars.len());
        for (col, meta) in ds.df.get_columns().iter().zip(&ds.vars) {
            let slot = self.pdv.add_var(PdvVar {
                name: meta.name.clone(),
                ty: meta.ty,
                length: meta.length,
                retained: false,
                from_input: true,
                format: meta.format.clone(),
            });
            // Si la variable existait déjà (référence textuelle antérieure
            // au SET), la marquer issue de l'input malgré tout.
            self.pdv.mark_from_input(slot);
            var_slots.push(slot);

            // Downcast UNE FOIS par colonne — jamais de get_row.
            let s = col.as_materialized_series();
            let values: Vec<Value> = match meta.ty {
                VarType::Num => s.f64()?.iter().map(num_to_value).collect(),
                VarType::Char => s
                    .str()?
                    .iter()
                    .map(|o| Value::Char(o.unwrap_or("").to_string()))
                    .collect(),
            };
            columns.push(values);
        }

        self.input = Some(InputData {
            display: r.display(),
            columns,
            var_slots,
            n_rows: ds.n_obs(),
        });
        Ok(())
    }

    /// Type et longueur inférés d'une expression (compile-time, comme SAS).
    fn infer(&self, expr: &Expr) -> (VarType, usize) {
        match expr {
            Expr::Num(_) | Expr::Missing(_) => (VarType::Num, 8),
            Expr::Str(s) => (VarType::Char, s.chars().count().max(1)),
            Expr::Var(name) => match self.pdv.slot(name) {
                Some(slot) => {
                    let v = &self.pdv.vars()[slot];
                    (v.ty, v.length)
                }
                // Inconnue au moment de l'inférence : numérique.
                None => (VarType::Num, 8),
            },
            Expr::Unary { .. } => (VarType::Num, 8),
            Expr::Binary {
                op: BinaryOp::Concat,
                left,
                right,
            } => (VarType::Char, self.char_len(left) + self.char_len(right)),
            Expr::Binary { .. } | Expr::In { .. } => (VarType::Num, 8),
            Expr::Call { name, args } => {
                let lower = name.to_ascii_lowercase();
                match lower.as_str() {
                    "upcase" | "lowcase" | "trim" | "strip" | "left" | "right" => {
                        let len = args.first().map_or(200, |a| self.char_len(a));
                        (VarType::Char, len)
                    }
                    "substr" => {
                        let len = args.first().map_or(200, |a| self.char_len(a));
                        (VarType::Char, len)
                    }
                    _ if lower.starts_with("cat") => (VarType::Char, 200),
                    "put" => (VarType::Char, put_width(args)),
                    _ => (VarType::Num, 8),
                }
            }
        }
    }

    /// Longueur d'un opérande en contexte caractère : un opérande numérique
    /// contribue 12 (conversion implicite BEST12., comme SAS).
    fn char_len(&self, expr: &Expr) -> usize {
        match self.infer(expr) {
            (VarType::Char, l) => l,
            (VarType::Num, _) => 12,
        }
    }

    fn resolve_outputs(&mut self, refs: &[DatasetRef]) -> Result<Vec<OutputSpec>> {
        // Toute variable de KEEP/DROP doit exister au PDV.
        for name in self.keeps.iter().chain(self.drops.iter()) {
            if self.pdv.slot(name).is_none() {
                return Err(SasError::runtime(format!(
                    "The variable {} in the DROP, KEEP, or RENAME list has never been referenced.",
                    name
                )));
            }
        }
        let keep_set: Option<HashSet<String>> = if self.keeps.is_empty() {
            None
        } else {
            Some(self.keeps.iter().map(|n| n.to_uppercase()).collect())
        };
        let drop_set: HashSet<String> = self.drops.iter().map(|n| n.to_uppercase()).collect();

        // KEEP ∩ DROP : DROP gagne, avec WARNING.
        if let Some(ref ks) = keep_set {
            for d in &drop_set {
                if ks.contains(d) {
                    self.session.log.warning(&format!(
                        "Variable {d} is in both the KEEP and DROP lists; it will be dropped."
                    ));
                }
            }
        }

        let kept_slots: Vec<usize> = self
            .pdv
            .vars()
            .iter()
            .enumerate()
            .filter(|(_, v)| {
                let u = v.name.to_uppercase();
                keep_set.as_ref().is_none_or(|k| k.contains(&u)) && !drop_set.contains(&u)
            })
            .map(|(i, _)| i)
            .collect();

        Ok(refs
            .iter()
            .map(|r| OutputSpec {
                libref: r.libref_or_work(),
                table: r.name.clone(),
                display: r.display(),
                kept_slots: kept_slots.clone(),
            })
            .collect())
    }
}

/// Largeur du format d'un `put(x, fmt)` : chiffres finaux du nom du format
/// (`best12` → 12), sinon 200. Le parser M1 ne produit pas encore de
/// littéral de format complet — best-effort.
fn put_width(args: &[Expr]) -> usize {
    let Some(fmt) = args.get(1) else { return 200 };
    let name = match fmt {
        Expr::Var(n) => n.as_str(),
        Expr::Str(s) => s.as_str(),
        _ => return 200,
    };
    let trimmed = name.trim_end_matches('.');
    let digits: String = trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().unwrap_or(200)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::parser::StatementStream;
    use crate::source::SourceFile;
    use polars::df;
    use std::path::PathBuf;

    fn session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    /// Écrit un petit dataset (age num avec un missing, name char) dans WORK.
    fn write_class(session: &Session, table: &str) {
        let df = df!(
            "Age" => [Some(14.0), None, Some(13.0)],
            "Name" => ["Alfred", "Alice", "Barbara"],
        )
        .unwrap();
        let vars = vec![
            VarMeta {
                name: "Age".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
            VarMeta {
                name: "Name".into(),
                ty: VarType::Char,
                length: 7,
                format: None,
                label: None,
            },
        ];
        session
            .libs
            .get("WORK")
            .unwrap()
            .write(table, &SasDataset { df, vars })
            .unwrap();
    }

    fn parse_step(src: &str) -> DataStepAst {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        crate::parser::datastep::parse_data_step(&mut ts).unwrap()
    }

    fn compile_src(src: &str, session: &mut Session) -> Result<StepProgram> {
        compile(&parse_step(src), session)
    }

    #[test]
    fn set_brings_input_vars_in_dataset_order() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data out; set inp; x = age + 1; run;", &mut s).unwrap();

        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["Age", "Name", "x"]);
        assert!(prog.pdv.vars()[0].from_input);
        assert!(prog.pdv.vars()[1].from_input);
        assert!(!prog.pdv.vars()[2].from_input);
        assert_eq!(prog.pdv.vars()[1].ty, VarType::Char);
        assert_eq!(prog.pdv.vars()[1].length, 7);

        let input = prog.input.as_ref().unwrap();
        assert_eq!(input.n_rows, 3);
        assert_eq!(input.display, "WORK.INP");
        assert_eq!(input.var_slots, vec![0, 1]);
        // Colonnes décodées en Value, missing `.` inclus.
        assert_eq!(input.columns[0][0], Value::Num(14.0));
        assert_eq!(input.columns[0][1], Value::missing());
        assert_eq!(input.columns[1][2], Value::Char("Barbara".into()));

        // Implicit output : pas de OUTPUT explicite.
        assert!(!prog.has_explicit_output);
        assert!(prog.uninitialized.is_empty());
        assert_eq!(prog.outputs.len(), 1);
        assert_eq!(prog.outputs[0].display, "WORK.OUT");
        assert_eq!(prog.outputs[0].kept_slots, vec![0, 1, 2]);
    }

    #[test]
    fn first_reference_order_without_set() {
        let mut s = session();
        let prog = compile_src("data o; x = y; z = 'abc'; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        // Cible avant les variables de l'expression, ordre textuel.
        assert_eq!(names, vec!["x", "y", "z"]);
        // x inféré Num (y inconnue au moment de l'inférence).
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
        // z : Char(3) du littéral.
        assert_eq!(prog.pdv.vars()[2].ty, VarType::Char);
        assert_eq!(prog.pdv.vars()[2].length, 3);
        // y : référencée jamais assignée → uninitialized.
        assert_eq!(prog.uninitialized, vec!["y".to_string()]);
    }

    #[test]
    fn first_assignment_freezes_type_and_length() {
        let mut s = session();
        let prog = compile_src("data o; s = 'ab'; s = 'abcdef'; run;", &mut s).unwrap();
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Char);
        // La première assignation fige la longueur à 2.
        assert_eq!(prog.pdv.vars()[0].length, 2);
    }

    #[test]
    fn concat_length_is_sum_with_num_as_12() {
        let mut s = session();
        let prog = compile_src("data o; c = 'ab' || 'cde'; d = c || x; run;", &mut s).unwrap();
        // c = 2 + 3.
        assert_eq!(prog.pdv.vars()[0].length, 5);
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Char);
        // d = len(c) + 12 (x numérique).
        let d = &prog.pdv.vars()[1];
        assert_eq!(d.name, "d");
        assert_eq!(d.length, 5 + 12);
    }

    #[test]
    fn call_inference_table() {
        let mut s = session();
        let prog = compile_src(
            "data o; a = 'xyz'; u = upcase(a); t = cats(a, a); n = sum(1, 2); run;",
            &mut s,
        )
        .unwrap();
        let var = |n: &str| {
            let slot = prog.pdv.slot(n).unwrap();
            &prog.pdv.vars()[slot]
        };
        assert_eq!((var("u").ty, var("u").length), (VarType::Char, 3));
        assert_eq!((var("t").ty, var("t").length), (VarType::Char, 200));
        assert_eq!(var("n").ty, VarType::Num);
    }

    #[test]
    fn keep_drop_interaction() {
        let mut s = session();
        let prog = compile_src(
            "data o; x = 1; y = 2; z = 3; keep x y; drop y; run;",
            &mut s,
        )
        .unwrap();
        // keep {x,y} puis drop y → x seul.
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
        // Le WARNING de l'intersection est dans le log.
        let log = s.log.into_string();
        assert!(log.contains("WARNING"), "log was: {log}");
        assert!(log.contains("KEEP and DROP"), "log was: {log}");
    }

    #[test]
    fn keep_unknown_variable_errors() {
        let mut s = session();
        let err = compile_src("data o; x = 1; keep nosuch; run;", &mut s).err().unwrap();
        assert!(
            err.to_string()
                .contains("in the DROP, KEEP, or RENAME list has never been referenced"),
            "got: {err}"
        );
    }

    #[test]
    fn data_null_has_no_outputs() {
        let mut s = session();
        let prog = compile_src("data _null_; x = 1; run;", &mut s).unwrap();
        assert!(prog.outputs.is_empty());
    }

    #[test]
    fn set_missing_table_errors() {
        let mut s = session();
        let err = compile_src("data o; set nosuch; run;", &mut s).err().unwrap();
        assert_eq!(err.to_string(), "File WORK.NOSUCH.DATA does not exist.");
    }

    #[test]
    fn second_set_errors_m1() {
        let mut s = session();
        write_class(&s, "a");
        write_class(&s, "b");
        let err = compile_src("data o; set a; set b; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("not yet implemented"));
    }

    #[test]
    fn explicit_output_detected_inside_if() {
        let mut s = session();
        let prog = compile_src(
            "data o; x = 1; if x then do; output; end; run;",
            &mut s,
        )
        .unwrap();
        assert!(prog.has_explicit_output);
    }

    #[test]
    fn multiple_outputs_share_kept_slots() {
        let mut s = session();
        let prog = compile_src("data a b; x = 1; y = 2; drop y; run;", &mut s).unwrap();
        assert_eq!(prog.outputs.len(), 2);
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
        assert_eq!(prog.outputs[1].kept_slots, vec![0]);
        assert_eq!(prog.outputs[0].libref, "WORK");
        assert_eq!(prog.outputs[1].display, "WORK.B");
    }

    #[test]
    fn assign_before_set_still_marks_from_input() {
        let mut s = session();
        write_class(&s, "inp");
        // `age` référencée avant le SET : elle doit malgré tout être
        // marquée from_input (pas de reset à chaque itération).
        let prog = compile_src("data o; age = 0; set inp; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("age").unwrap();
        assert!(prog.pdv.vars()[slot].from_input);
        // Et l'ordre de première référence place age en tête.
        assert_eq!(prog.pdv.vars()[0].name, "age");
    }

    #[test]
    fn put_width_parsing() {
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Var("best12".into())]), 12);
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Str("date9.".into())]), 9);
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Var("words".into())]), 200);
        assert_eq!(put_width(&[Expr::Num(1.0)]), 200);
    }
}
