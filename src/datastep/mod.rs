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
    /// Valeurs initiales (slot, valeur) issues de RETAIN avec init et des
    /// sum statements (0) : l'exécuteur les applique via `pdv.set` AVANT la
    /// première itération (la troncature char s'applique donc normalement).
    /// Appliquées dans l'ordre — une entrée ultérieure pour le même slot
    /// gagne (cas `n + 1; retain n 100;` : le RETAIN l'emporte).
    pub initial_values: Vec<(usize, Value)>,
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
        retain_all: false,
        retain_pending: Vec::new(),
        retained_slots: HashSet::new(),
        initial_values: Vec::new(),
    };
    for stmt in &ast.stmts {
        c.walk_stmt(stmt)?;
    }

    // RETAIN sans valeur initiale — SIMPLIFICATION M2 ASSUMÉE : en vrai
    // SAS, `retain x;` ne fige PAS le type — la variable le prend à sa
    // prochaine référence. Pour approcher ça sans bouleverser la passe
    // unique, le statement n'a fait que mémoriser le nom ; ICI (fin de
    // compilation) on applique le flag `retained` à la variable, qui doit
    // alors exister (créée par une autre référence). Sinon on la crée Num
    // + uninitialized — elle arrive donc en FIN d'ordre PDV (divergence
    // mineure assumée par rapport à l'ordre de première référence SAS).
    let pending = std::mem::take(&mut c.retain_pending);
    for name in &pending {
        let slot = match c.pdv.slot(name) {
            Some(slot) => slot,
            None => c.add_var(name, VarType::Num, 8),
        };
        c.retained_slots.insert(slot);
    }
    // `retain;` seul — SIMPLIFICATION M2 : retient TOUT le PDV (en vrai
    // SAS, seulement ce qui est connu au point du statement).
    if c.retain_all {
        c.retained_slots.extend(0..c.pdv.vars().len());
    }
    // Le PDV ne permet pas de modifier une variable existante (première
    // référence fige tout, et `pdv.rs` n'expose pas de mutateur) : on le
    // reconstruit à l'identique en appliquant les flags `retained`. Les
    // slots sont préservés (même ordre d'insertion) et aucune valeur n'a
    // encore été posée à la compilation.
    if !c.retained_slots.is_empty() {
        c.pdv = rebuild_with_retained(&c.pdv, &c.retained_slots);
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
        initial_values: c.initial_values,
    })
}

/// Reconstruit le PDV en marquant `retained` les slots donnés (les autres
/// attributs sont copiés tels quels ; les indices de slots sont stables).
fn rebuild_with_retained(pdv: &Pdv, retained: &HashSet<usize>) -> Pdv {
    let mut rebuilt = Pdv::new();
    for (i, v) in pdv.vars().iter().enumerate() {
        let slot = rebuilt.add_var(PdvVar {
            name: v.name.clone(),
            ty: v.ty,
            length: v.length,
            retained: v.retained || retained.contains(&i),
            from_input: v.from_input,
            format: v.format.clone(),
        });
        debug_assert_eq!(slot, i, "rebuild must preserve slot indices");
    }
    rebuilt
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
    /// `retain;` sans liste rencontré : tout le PDV sera retenu.
    retain_all: bool,
    /// Noms d'un RETAIN SANS init : flag appliqué en fin de compilation.
    retain_pending: Vec<String>,
    /// Slots à marquer `retained` en fin de compilation (RETAIN avec init,
    /// sum statements, RETAIN sans init résolus).
    retained_slots: HashSet<usize>,
    /// Valeurs initiales (slot, valeur) appliquées avant la 1re itération.
    initial_values: Vec<(usize, Value)>,
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
            DsStmt::Retain(items) => {
                if items.is_empty() {
                    // `retain;` seul : tout le PDV (cf. fin de compile()).
                    self.retain_all = true;
                    return Ok(());
                }
                for (name, init) in items {
                    match init {
                        // AVEC init : la variable entre au PDV ICI (ordre de
                        // première référence), type/longueur du littéral, et
                        // sa valeur initiale part dans `initial_values`. Elle
                        // compte comme initialisée (pas de NOTE
                        // "uninitialized" — comme SAS).
                        Some(expr) => {
                            let (ty, length, value) = retain_literal(expr)?;
                            let slot = self.add_var(name, ty, length);
                            self.retained_slots.insert(slot);
                            self.assigned.insert(name.to_uppercase());
                            self.initial_values.push((slot, value));
                        }
                        // SANS init : ne crée PAS la variable (le type sera
                        // figé par sa prochaine référence) — voir compile().
                        None => self.retain_pending.push(name.clone()),
                    }
                }
                Ok(())
            }
            DsStmt::Sum { var, expr } => {
                // `var + expr;` : var entre au PDV (Num, 8), retenue, valeur
                // initiale 0 — SAUF si un RETAIN avec init a déjà posé une
                // valeur pour ce slot (le RETAIN gagne, comme SAS). La cible
                // entre avant les variables de l'expression (ordre textuel).
                let slot = self.add_var(var, VarType::Num, 8);
                self.retained_slots.insert(slot);
                self.assigned.insert(var.to_uppercase());
                if !self.initial_values.iter().any(|(s, _)| *s == slot) {
                    self.initial_values.push((slot, Value::Num(0.0)));
                }
                self.walk_expr(expr);
                Ok(())
            }
            DsStmt::Length(items) => {
                for (name, spec) in items {
                    // Plages SAS : char 1..=32767, num 3..=8.
                    let (lo, hi) = if spec.char { (1, 32767) } else { (3, 8) };
                    if spec.len < lo || spec.len > hi {
                        return Err(SasError::runtime(format!(
                            "The length {} specified for the variable {} is out of range ({}-{}).",
                            spec.len, name, lo, hi
                        )));
                    }
                    match self.pdv.slot(name) {
                        // LENGTH précède la première référence : crée la
                        // variable avec cette longueur. Pour une numérique,
                        // la longueur (3..=8) est une simple MÉTADONNÉE en
                        // M2 — le stockage reste f64 sur 8 octets.
                        None => {
                            let ty = if spec.char { VarType::Char } else { VarType::Num };
                            self.add_var(name, ty, spec.len);
                        }
                        // Déjà au PDV : la longueur est figée. SAS n'émet le
                        // WARNING que pour les variables CHAR dont la
                        // longueur demandée diffère ; num : silencieux.
                        Some(slot) => {
                            let v = &self.pdv.vars()[slot];
                            if v.ty == VarType::Char && spec.char && v.length != spec.len {
                                let name = v.name.clone();
                                self.session.log.warning(&format!(
                                    "Length of character variable {name} has already been set."
                                ));
                            }
                        }
                    }
                }
                Ok(())
            }
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

/// Type, longueur et valeur d'un littéral d'init RETAIN. Le parser ne
/// produit que `Num` (le `-` unaire y est replié), `Str` ou `Missing` ;
/// tout autre nœud est un garde-fou.
fn retain_literal(expr: &Expr) -> Result<(VarType, usize, Value)> {
    match expr {
        Expr::Num(n) => Ok((VarType::Num, 8, Value::Num(*n))),
        Expr::Missing(k) => Ok((VarType::Num, 8, Value::Missing(*k))),
        Expr::Str(s) => Ok((
            VarType::Char,
            s.chars().count().max(1),
            Value::Char(s.clone()),
        )),
        _ => Err(SasError::runtime(
            "RETAIN initial values must be literals.",
        )),
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

    // ── RETAIN (M2) ──────────────────────────────────────────────────────

    #[test]
    fn retain_with_init_creates_retained_var_with_initial_value() {
        let mut s = session();
        let prog = compile_src("data o; retain x 5 s 'ab'; y = 1; run;", &mut s).unwrap();
        // Ordre de première référence : x et s entrent au RETAIN, avant y.
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["x", "s", "y"]);
        assert!(prog.pdv.vars()[0].retained);
        assert!(prog.pdv.vars()[1].retained);
        assert!(!prog.pdv.vars()[2].retained);
        // Types/longueurs des littéraux.
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
        assert_eq!(prog.pdv.vars()[1].ty, VarType::Char);
        assert_eq!(prog.pdv.vars()[1].length, 2);
        // Valeurs initiales.
        assert_eq!(
            prog.initial_values,
            vec![(0, Value::Num(5.0)), (1, Value::Char("ab".into()))]
        );
        // RETAIN avec init = initialisée : pas de NOTE uninitialized.
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn retain_without_init_flags_later_reference_or_creates_num() {
        let mut s = session();
        // k : retenue sans init, type figé par l'assignation ultérieure.
        // j : retenue sans init, jamais référencée → Num + uninitialized,
        // créée en FIN d'ordre PDV (simplification M2 documentée).
        let prog = compile_src("data o; retain k j; x = 1; k = 'ab'; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["x", "k", "j"]);
        let var = |n: &str| &prog.pdv.vars()[prog.pdv.slot(n).unwrap()];
        assert!(var("k").retained);
        assert_eq!(var("k").ty, VarType::Char);
        assert_eq!(var("k").length, 2);
        assert!(var("j").retained);
        assert_eq!(var("j").ty, VarType::Num);
        assert!(!var("x").retained);
        assert_eq!(prog.uninitialized, vec!["j".to_string()]);
        // Pas de valeur initiale sans init.
        assert!(prog.initial_values.is_empty());
    }

    #[test]
    fn retain_bare_retains_whole_pdv() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp; retain; x = 1; run;", &mut s).unwrap();
        assert!(prog.pdv.vars().iter().all(|v| v.retained));
    }

    // ── Sum statement (M2) ───────────────────────────────────────────────

    #[test]
    fn sum_statement_compiles_retained_num_with_initial_zero() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp; total + age; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("total").unwrap();
        let v = &prog.pdv.vars()[slot];
        assert_eq!(v.ty, VarType::Num);
        assert_eq!(v.length, 8);
        assert!(v.retained);
        assert_eq!(prog.initial_values, vec![(slot, Value::Num(0.0))]);
        // La cible d'un sum statement compte comme initialisée.
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn retain_init_wins_over_sum_zero_in_both_orders() {
        let mut s = session();
        // RETAIN d'abord : le sum statement ne pousse pas son 0.
        let prog = compile_src("data o; retain n 100; n + 1; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("n").unwrap();
        assert_eq!(prog.initial_values, vec![(slot, Value::Num(100.0))]);
        // Sum d'abord : les deux entrées coexistent, le RETAIN (appliqué en
        // dernier par l'exécuteur) gagne.
        let prog = compile_src("data o; n + 1; retain n 100; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("n").unwrap();
        assert_eq!(
            prog.initial_values,
            vec![(slot, Value::Num(0.0)), (slot, Value::Num(100.0))]
        );
    }

    // ── LENGTH (M2) ──────────────────────────────────────────────────────

    #[test]
    fn length_before_first_reference_fixes_type_and_length() {
        let mut s = session();
        let prog = compile_src(
            "data o; length c $ 3 n 4; c = 'abcdef'; n = 1; run;",
            &mut s,
        )
        .unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["c", "n"]);
        let c = &prog.pdv.vars()[0];
        assert_eq!(c.ty, VarType::Char);
        // La longueur du LENGTH gagne sur celle du littéral assigné.
        assert_eq!(c.length, 3);
        let n = &prog.pdv.vars()[1];
        assert_eq!(n.ty, VarType::Num);
        // Pour une numérique, la longueur est une métadonnée (stockage f64).
        assert_eq!(n.length, 4);
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn length_after_reference_warns_for_differing_char() {
        let mut s = session();
        let prog = compile_src("data o; c = 'ab'; length c $ 10; run;", &mut s).unwrap();
        // La longueur reste figée par la première référence.
        assert_eq!(prog.pdv.vars()[0].length, 2);
        let log = s.log.into_string();
        assert!(
            log.contains("WARNING: Length of character variable c has already been set."),
            "log was: {log}"
        );
    }

    #[test]
    fn length_after_reference_is_silent_for_num_and_same_char_length() {
        let mut s = session();
        let prog = compile_src(
            "data o; x = 1; length x 5; c = 'ab'; length c $ 2; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(prog.pdv.vars()[0].length, 8);
        let log = s.log.into_string();
        assert!(!log.contains("WARNING"), "log was: {log}");
    }

    #[test]
    fn length_out_of_range_errors() {
        let mut s = session();
        let err = compile_src("data o; length n 9; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("out of range (3-8)"), "got: {err}");
        let err = compile_src("data o; length c $ 40000; run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("out of range (1-32767)"),
            "got: {err}"
        );
        let err = compile_src("data o; length n 2; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("out of range"), "got: {err}");
    }

    #[test]
    fn put_width_parsing() {
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Var("best12".into())]), 12);
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Str("date9.".into())]), 9);
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Var("words".into())]), 200);
        assert_eq!(put_width(&[Expr::Num(1.0)]), 200);
    }
}
