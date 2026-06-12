//! Exécution de l'étape DATA : la boucle implicite.
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : ÉLEVÉE — cœur du langage)
//!
//! ## Boucle implicite (M1, un seul SET)
//! ```text
//! boucle :
//!   pdv.n_ += 1
//!   pdv.reset_non_retained()
//!   exécuter les statements dans l'ordre ; le statement SET, QUAND IL
//!     S'EXÉCUTE, lit la ligne suivante de l'input dans le PDV ; s'il n'y
//!     a plus de ligne, l'ÉTAPE SE TERMINE IMMÉDIATEMENT (au milieu de
//!     l'itération — c'est la règle SAS, pas un test en tête de boucle)
//!   en fin d'itération : output implicite (si !has_explicit_output)
//! ```
//! Étape SANS instruction de lecture (ni SET) : UNE seule itération puis
//! stop (sinon boucle infinie — règle SAS).
//!
//! ## Flux de contrôle intra-itération
//! `enum Flow { Normal, NextIter, EndStep }` :
//! - `SubsettingIf` faux → NextIter (pas d'output implicite)
//! - `Output` → pousser les valeurs des `kept_slots` dans les builders
//! - `Stop` → EndStep
//! - `If/Block` propagent le Flow de leurs branches.
//!
//! ## Erreurs d'exécution (style SAS : on continue !)
//! Division par zéro, argument invalide, conversion char→num ratée :
//! résultat missing `.`, `pdv.error_ = true`, NOTE dans le log
//! ("Division by zero detected...", "Invalid numeric data..."),
//! compteur "Missing values were generated" pour la NOTE de fin d'étape.
//! Implémenté via `EvalCtx` (eval.rs) qui collecte notes + compteurs.
//!
//! ## Builders de sortie
//! Par output et par slot conservé : `Vec<Option<f64>>` (missing spéciaux
//! ré-encodés NaN-payload via `missing::value_to_num`) ou
//! `Vec<Option<String>>`. À la fin : construire les `Column` Polars dans
//! l'ordre PDV, créer `SasDataset` (VarMeta depuis le PDV), écrire via
//! `session.libs.get(libref)?.write(table, ds)`, et mettre à jour
//! `session.last_dataset`.
//!
//! ## NOTEs de fin d'étape (ordre SAS)
//! 1. "There were N observations read from the data set WORK.B."
//! 2. par output : "The data set WORK.A has N observations and M
//!    variables."  (M = nb de slots conservés ; SAS ne met jamais le
//!    singulier — garder "variables" même pour 1 !)
//! L'appelant (executor) ajoute ensuite la NOTE de timing.
//!
//! ## Choix d'implémentation
//! - Les NOTEs de conversion/erreur n'incluent pas les positions
//!   (Line):(Column) de SAS — divergence assumée, cf. PLAN.md (log sans
//!   numéros de page/date).
//! - La coercition à l'ASSIGNATION (expression num vers variable char et
//!   inversement) vit ici : num→char via BEST12. justifié à droite sur
//!   12, char→num via trim+parse (mêmes règles que dans eval).
//! - Garde-fou anti-boucle infinie (SET jamais exécuté alors qu'un input
//!   existe) : n_ > n_rows + 10_000 → erreur d'exécution. SAS bouclerait
//!   sans fin ; divergence assumée.

use super::eval::{eval, EvalCtx};
use super::pdv::Pdv;
use super::{InputData, OutputSpec, StepProgram};
use crate::ast::DsStmt;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::missing::value_to_num;
use crate::session::Session;
use crate::value::{format_best, Value, VarType};
use polars::prelude::{Column, DataFrame, NamedFrom, Series};

pub struct StepStats {
    /// (display, lignes lues) par input.
    pub read: Vec<(String, usize)>,
    /// (display, obs, vars) par output écrit.
    pub written: Vec<(String, usize, usize)>,
}

#[derive(PartialEq)]
enum Flow {
    Normal,
    NextIter,
    EndStep,
}

enum ColBuilder {
    Num(Vec<Option<f64>>),
    Char(Vec<String>),
}

struct Runner {
    pdv: Pdv,
    input: Option<InputData>,
    /// Prochaine ligne d'input à lire (== lignes déjà lues).
    row_idx: usize,
    ctx: EvalCtx,
    outputs: Vec<OutputSpec>,
    /// builders[output][colonne], parallèle à outputs[o].kept_slots.
    builders: Vec<Vec<ColBuilder>>,
    /// Nombre d'observations poussées (identique pour toutes les sorties).
    out_rows: usize,
}

pub fn execute(prog: StepProgram, session: &mut Session) -> Result<StepStats> {
    let StepProgram {
        pdv,
        stmts,
        input,
        outputs,
        has_explicit_output,
        uninitialized,
        initial_values,
    } = prog;

    for name in &uninitialized {
        session.log.note(&format!("Variable {name} is uninitialized."));
    }

    let builders = outputs
        .iter()
        .map(|o| {
            o.kept_slots
                .iter()
                .map(|s| match pdv.vars()[*s].ty {
                    VarType::Num => ColBuilder::Num(Vec::new()),
                    VarType::Char => ColBuilder::Char(Vec::new()),
                })
                .collect()
        })
        .collect();

    let single_iteration = input.is_none();
    let n_rows = input.as_ref().map_or(0, |i| i.n_rows);
    let mut r = Runner {
        pdv,
        input,
        row_idx: 0,
        ctx: EvalCtx::default(),
        outputs,
        builders,
        out_rows: 0,
    };

    // Valeurs initiales (RETAIN avec init, sum statements) : posées AVANT
    // la première itération via `pdv.set` (la troncature char des inits
    // trop longues s'applique donc normalement). Ces slots sont retenus,
    // `reset_non_retained` ne les touchera jamais. Une entrée ultérieure
    // pour le même slot écrase la précédente (le RETAIN gagne sur le 0
    // implicite d'un sum statement).
    for (slot, v) in initial_values {
        r.pdv.set(slot, v);
    }

    loop {
        r.pdv.n_ += 1;
        r.pdv.error_ = false;
        r.pdv.reset_non_retained();

        let mut flow = Flow::Normal;
        for stmt in &stmts {
            flow = r.exec_stmt(stmt)?;
            if flow != Flow::Normal {
                break;
            }
        }
        if flow == Flow::EndStep {
            break;
        }
        if flow != Flow::NextIter && !has_explicit_output {
            r.push_outputs();
        }
        if single_iteration {
            break;
        }
        // Garde-fou anti-boucle infinie (cf. en-tête).
        if r.pdv.n_ as usize > n_rows + 10_000 {
            return Err(SasError::runtime(
                "DATA step appears to loop infinitely (no input rows consumed); stopping.",
            ));
        }
    }

    // NOTEs d'erreurs/conversions collectées par l'évaluateur.
    if r.ctx.note_num_to_char {
        session
            .log
            .note("Numeric values have been converted to character values.");
    }
    if r.ctx.note_char_to_num {
        session
            .log
            .note("Character values have been converted to numeric values.");
    }
    if r.ctx.division_by_zero > 0 {
        session.log.note("Division by zero detected.");
    }
    if r.ctx.invalid_data > 0 {
        session.log.note("Invalid numeric data.");
    }
    if r.ctx.missing_generated > 0 {
        session.log.note(
            "Missing values were generated as a result of performing an operation on missing values.",
        );
    }

    let mut stats = StepStats {
        read: Vec::new(),
        written: Vec::new(),
    };
    if let Some(input) = &r.input {
        session.log.note(&format!(
            "There were {} observations read from the data set {}.",
            r.row_idx, input.display
        ));
        stats.read.push((input.display.clone(), r.row_idx));
    }

    // Écriture des sorties (ordre du statement DATA ; _LAST_ = la dernière).
    for (spec, bset) in r.outputs.iter().zip(r.builders) {
        let mut columns: Vec<Column> = Vec::with_capacity(spec.kept_slots.len());
        let mut vars: Vec<VarMeta> = Vec::with_capacity(spec.kept_slots.len());
        for (slot, b) in spec.kept_slots.iter().zip(bset) {
            let v = &r.pdv.vars()[*slot];
            let series = match b {
                ColBuilder::Num(vals) => Series::new(v.name.as_str().into(), vals),
                ColBuilder::Char(vals) => Series::new(v.name.as_str().into(), vals),
            };
            columns.push(series.into());
            vars.push(VarMeta {
                name: v.name.clone(),
                ty: v.ty,
                length: v.length,
                format: v.format.clone(),
                label: None,
            });
        }
        let df = DataFrame::new(columns)?;
        let ds = SasDataset { df, vars };
        session.libs.get(&spec.libref)?.write(&spec.table, &ds)?;
        session.last_dataset = Some(spec.display.clone());
        session.log.note(&format!(
            "The data set {} has {} observations and {} variables.",
            spec.display,
            r.out_rows,
            spec.kept_slots.len()
        ));
        stats
            .written
            .push((spec.display.clone(), r.out_rows, spec.kept_slots.len()));
    }

    Ok(stats)
}

impl Runner {
    fn exec_stmt(&mut self, stmt: &DsStmt) -> Result<Flow> {
        match stmt {
            DsStmt::Set(_) => {
                let Some(input) = &self.input else {
                    // Impossible après compile() ; garde-fou.
                    return Err(SasError::runtime("SET statement without input data."));
                };
                if self.row_idx >= input.n_rows {
                    // Fin de l'input : l'étape se termine IMMÉDIATEMENT.
                    return Ok(Flow::EndStep);
                }
                for (col, slot) in input.columns.iter().zip(&input.var_slots) {
                    self.pdv.set(*slot, col[self.row_idx].clone());
                }
                self.row_idx += 1;
                Ok(Flow::Normal)
            }
            DsStmt::Assign { var, expr } => {
                let value = self.eval_checked(expr)?;
                let Some(slot) = self.pdv.slot(var) else {
                    return Err(SasError::runtime(format!(
                        "Variable {var} is not addressable."
                    )));
                };
                let coerced = self.coerce_assign(value, self.pdv.vars()[slot].ty);
                self.pdv.set(slot, coerced);
                Ok(Flow::Normal)
            }
            DsStmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let c = self.eval_checked(cond)?;
                if c.truthy() {
                    self.exec_stmt(then_branch)
                } else if let Some(e) = else_branch {
                    self.exec_stmt(e)
                } else {
                    Ok(Flow::Normal)
                }
            }
            DsStmt::SubsettingIf(cond) => {
                let c = self.eval_checked(cond)?;
                if c.truthy() {
                    Ok(Flow::Normal)
                } else {
                    Ok(Flow::NextIter)
                }
            }
            DsStmt::Block(stmts) => {
                for s in stmts {
                    let f = self.exec_stmt(s)?;
                    if f != Flow::Normal {
                        return Ok(f);
                    }
                }
                Ok(Flow::Normal)
            }
            DsStmt::Output => {
                self.push_outputs();
                Ok(Flow::Normal)
            }
            DsStmt::Stop => Ok(Flow::EndStep),
            DsStmt::Sum { var, expr } => {
                // Sum statement `var + expr;` — sémantique SUM de SAS : les
                // missings sont IGNORÉS, jamais propagés. Un incrément
                // missing ajoute 0 (sans `missing_generated`), et un
                // accumulateur missing (l'utilisateur a pu assigner `.`)
                // est traité comme 0 : `total=.; total+x;` donne x.
                let value = self.eval_checked(expr)?;
                let incr = self.coerce_sum_operand(value);
                let Some(slot) = self.pdv.slot(var) else {
                    return Err(SasError::runtime(format!(
                        "Variable {var} is not addressable."
                    )));
                };
                let acc = match self.pdv.get(slot) {
                    Value::Num(f) => *f,
                    // Missing (ou char dégénéré) : repart de 0.
                    _ => 0.0,
                };
                self.pdv.set(slot, Value::Num(acc + incr));
                Ok(Flow::Normal)
            }
            // Directives de compilation : rien à exécuter.
            DsStmt::Keep(_) | DsStmt::Drop(_) | DsStmt::Retain(_) | DsStmt::Length(_) => {
                Ok(Flow::Normal)
            }
        }
    }

    /// Coercition numérique d'un opérande de sum statement. Mêmes règles de
    /// conversion char→num que l'évaluateur (note + invalid data + _ERROR_
    /// sur une chaîne invalide), MAIS un résultat missing contribue 0 sans
    /// incrémenter `missing_generated` (le SUM ignore les missings).
    fn coerce_sum_operand(&mut self, value: Value) -> f64 {
        match value {
            Value::Num(f) => f,
            Value::Missing(_) => 0.0,
            Value::Char(s) => {
                self.ctx.note_char_to_num = true;
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    0.0
                } else {
                    match trimmed.parse::<f64>() {
                        Ok(f) => f,
                        Err(_) => {
                            self.ctx.invalid_data += 1;
                            self.pdv.error_ = true;
                            0.0
                        }
                    }
                }
            }
        }
    }

    /// Évalue, propage les fatals, reporte `_ERROR_` au PDV.
    fn eval_checked(&mut self, expr: &crate::ast::Expr) -> Result<Value> {
        let v = eval(expr, &self.pdv, &mut self.ctx);
        if let Some(msg) = self.ctx.fatal.take() {
            return Err(SasError::runtime(msg));
        }
        if self.ctx.error_flag {
            self.pdv.error_ = true;
            self.ctx.error_flag = false;
        }
        Ok(v)
    }

    /// Coercition à l'assignation : expression d'un type vers une variable
    /// de l'autre type (mêmes règles que dans les expressions).
    fn coerce_assign(&mut self, value: Value, target: VarType) -> Value {
        match (value, target) {
            (v @ (Value::Num(_) | Value::Missing(_)), VarType::Num) => v,
            (v @ Value::Char(_), VarType::Char) => v,
            (Value::Char(s), VarType::Num) => {
                self.ctx.note_char_to_num = true;
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    self.ctx.missing_generated += 1;
                    Value::missing()
                } else {
                    match trimmed.parse::<f64>() {
                        Ok(f) => Value::Num(f),
                        Err(_) => {
                            self.ctx.invalid_data += 1;
                            self.pdv.error_ = true;
                            Value::missing()
                        }
                    }
                }
            }
            (Value::Num(f), VarType::Char) => {
                self.ctx.note_num_to_char = true;
                Value::Char(format!("{:>12}", format_best(f, 12)))
            }
            (Value::Missing(k), VarType::Char) => {
                self.ctx.note_num_to_char = true;
                Value::Char(format!("{:>12}", k.display()))
            }
        }
    }

    fn push_outputs(&mut self) {
        for (spec, bset) in self.outputs.iter().zip(self.builders.iter_mut()) {
            for (slot, b) in spec.kept_slots.iter().zip(bset.iter_mut()) {
                let v = self.pdv.get(*slot);
                match b {
                    ColBuilder::Num(vals) => vals.push(value_to_num(v)),
                    ColBuilder::Char(vals) => vals.push(match v {
                        Value::Char(s) => s.clone(),
                        // Une variable char ne contient jamais autre chose
                        // après pdv.set ; blanc par sûreté.
                        _ => String::new(),
                    }),
                }
            }
        }
        self.out_rows += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datastep::compile;
    use crate::parser::StatementStream;
    use crate::source::SourceFile;
    use crate::value::MissingKind;
    use polars::df;
    use std::path::PathBuf;

    fn session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

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

    fn run(src: &str, session: &mut Session) -> Result<StepStats> {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        let ast = crate::parser::datastep::parse_data_step(&mut ts).unwrap();
        let prog = compile(&ast, session)?;
        execute(prog, session)
    }

    fn read_work(session: &Session, table: &str) -> SasDataset {
        session.libs.get("WORK").unwrap().read(table).unwrap().0
    }

    #[test]
    fn set_assign_implicit_output() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data out; set inp; x = age * 2; run;", &mut s).unwrap();
        assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 3, 3)]);

        let ds = read_work(&s, "out");
        assert_eq!(ds.n_obs(), 3);
        let x = ds.df.column("x").unwrap().f64().unwrap();
        assert_eq!(x.get(0), Some(28.0));
        // age missing → x missing (propagation) + note.
        assert_eq!(x.get(1), None);
        assert_eq!(x.get(2), Some(26.0));

        let log = s.log.into_string();
        assert!(log.contains("There were 3 observations read from the data set WORK.INP."));
        assert!(log.contains("The data set WORK.OUT has 3 observations and 3 variables."));
        assert!(log.contains("Missing values were generated"));
        assert_eq!(s.last_dataset.as_deref(), Some("WORK.OUT"));
    }

    #[test]
    fn subsetting_if_filters() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data out; set inp; if age > 13; run;", &mut s).unwrap();
        // age > 13 : 14 vrai, missing faux (. < 14), 13 faux.
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
        let ds = read_work(&s, "out");
        let name = ds.df.column("Name").unwrap().str().unwrap();
        assert_eq!(name.get(0), Some("Alfred"));
    }

    #[test]
    fn explicit_output_disables_implicit() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data out; set inp; output; output; run;", &mut s).unwrap();
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 6, 2)]);
    }

    #[test]
    fn stop_ends_step_without_output() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data out; set inp; output; stop; run;", &mut s).unwrap();
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
        // STOP au milieu : une seule ligne lue.
        assert_eq!(stats.read, vec![("WORK.INP".to_string(), 1)]);
    }

    #[test]
    fn no_input_runs_single_iteration() {
        let mut s = session();
        let stats = run("data out; x = 1; y = 'ab'; run;", &mut s).unwrap();
        assert_eq!(stats.read, vec![]);
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
        let ds = read_work(&s, "out");
        assert_eq!(ds.n_obs(), 1);
        assert_eq!(
            ds.df.column("y").unwrap().str().unwrap().get(0),
            Some("ab")
        );
    }

    #[test]
    fn data_null_writes_nothing() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data _null_; set inp; run;", &mut s).unwrap();
        assert!(stats.written.is_empty());
        assert!(!s.libs.get("WORK").unwrap().exists("_null_"));
        // _LAST_ inchangé.
        assert_eq!(s.last_dataset, None);
    }

    #[test]
    fn if_then_else_branches() {
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data out; set inp; if age >= 14 then grp = 'old'; else grp = 'yng'; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let grp = ds.df.column("grp").unwrap().str().unwrap();
        assert_eq!(grp.get(0), Some("old"));
        // age missing : . >= 14 faux → else.
        assert_eq!(grp.get(1), Some("yng"));
        assert_eq!(grp.get(2), Some("yng"));
    }

    #[test]
    fn uninitialized_note_and_missing_column() {
        let mut s = session();
        let stats = run("data out; y = x; run;", &mut s).unwrap();
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("x").unwrap().f64().unwrap().get(0), None);
        let log = s.log.into_string();
        assert!(log.contains("Variable x is uninitialized."));
    }

    #[test]
    fn assign_coercion_num_to_char_best12_right_justified() {
        let mut s = session();
        run("data out; c = 'init'; c = 7; run;", &mut s).unwrap();
        // c figée Char(4) par la 1re assignation ; 7 → BEST12 justifié
        // droite sur 12 ('           7') puis tronqué à 4 ('    ') → trim
        // stockage → "".
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("c").unwrap().str().unwrap().get(0), Some(""));
        let log = s.log.into_string();
        assert!(log.contains("Numeric values have been converted to character values."));
    }

    #[test]
    fn special_missing_roundtrip_through_output() {
        let mut s = session();
        write_class(&s, "inp");
        // .a : missing spécial assigné puis écrit ; doit survivre au parquet.
        run("data out; set inp; m = .a; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        // Relu : NaN payload → décodé par num_to_value au prochain SET ;
        // au niveau parquet brut c'est un NaN (pas un null).
        let m = ds.df.column("m").unwrap().f64().unwrap();
        let raw = m.get(0);
        assert!(raw.is_some_and(f64::is_nan));
        assert_eq!(
            crate::missing::num_to_value(raw),
            Value::Missing(MissingKind::Letter(0))
        );
    }

    // ── RETAIN / sum statement / LENGTH (M2) ─────────────────────────────

    #[test]
    fn sum_statement_counter_increments_per_obs() {
        let mut s = session();
        write_class(&s, "inp");
        run("data out; set inp; n + 1; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        let n = ds.df.column("n").unwrap().f64().unwrap();
        assert_eq!(n.get(0), Some(1.0));
        assert_eq!(n.get(1), Some(2.0));
        assert_eq!(n.get(2), Some(3.0));
    }

    #[test]
    fn sum_statement_ignores_missing_increment() {
        let mut s = session();
        write_class(&s, "inp");
        // age = 14, ., 13 : le missing du milieu ajoute 0 (PAS propagé).
        run("data out; set inp; total + age; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        let total = ds.df.column("total").unwrap().f64().unwrap();
        assert_eq!(total.get(0), Some(14.0));
        assert_eq!(total.get(1), Some(14.0));
        assert_eq!(total.get(2), Some(27.0));
        // Aucun missing généré par le sum statement.
        let log = s.log.into_string();
        assert!(
            !log.contains("Missing values were generated"),
            "log was: {log}"
        );
    }

    #[test]
    fn sum_statement_missing_accumulator_restarts_from_zero() {
        let mut s = session();
        write_class(&s, "inp");
        // total remis à `.` à chaque itération : total + age repart de 0.
        run("data out; set inp; total = .; total + age; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        let total = ds.df.column("total").unwrap().f64().unwrap();
        assert_eq!(total.get(0), Some(14.0));
        assert_eq!(total.get(1), Some(0.0));
        assert_eq!(total.get(2), Some(13.0));
    }

    #[test]
    fn retain_with_init_accumulates_max() {
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data out; set inp; retain maxage 0; if age > maxage then maxage = age; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let maxage = ds.df.column("maxage").unwrap().f64().unwrap();
        // 14 dès la 1re obs, retenu ensuite (. > 14 faux, 13 > 14 faux).
        assert_eq!(maxage.get(0), Some(14.0));
        assert_eq!(maxage.get(1), Some(14.0));
        assert_eq!(maxage.get(2), Some(14.0));
    }

    #[test]
    fn retain_initial_value_wins_over_sum_zero() {
        let mut s = session();
        write_class(&s, "inp");
        run("data out; set inp; n + 1; retain n 100; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        let n = ds.df.column("n").unwrap().f64().unwrap();
        assert_eq!(n.get(0), Some(101.0));
        assert_eq!(n.get(2), Some(103.0));
    }

    #[test]
    fn retain_without_init_keeps_value_across_iterations() {
        let mut s = session();
        write_class(&s, "inp");
        // prev : Name de l'observation précédente ('' à la 1re itération).
        run(
            "data out; set inp; retain prev; output; prev = name; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let prev = ds.df.column("prev").unwrap().str().unwrap();
        assert_eq!(prev.get(0), Some(""));
        assert_eq!(prev.get(1), Some("Alfred"));
        assert_eq!(prev.get(2), Some("Alice"));
    }

    #[test]
    fn length_truncates_longer_assignment() {
        let mut s = session();
        let stats = run("data out; length c $ 3; c = 'abcdef'; run;", &mut s).unwrap();
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 1)]);
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("c").unwrap().str().unwrap().get(0), Some("abc"));
        assert_eq!(ds.vars[0].length, 3);
    }

    #[test]
    fn retain_char_init_truncated_to_fixed_length() {
        let mut s = session();
        // c figée Char(3) par LENGTH ; l'init RETAIN 'abcdef' est tronquée
        // par le pdv.set normal au moment de poser les valeurs initiales.
        run("data out; length c $ 3; retain c 'abcdef'; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("c").unwrap().str().unwrap().get(0), Some("abc"));
    }

    #[test]
    fn multiple_outputs_written() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data a b; set inp; run;", &mut s).unwrap();
        assert_eq!(stats.written.len(), 2);
        assert!(s.libs.get("WORK").unwrap().exists("a"));
        assert!(s.libs.get("WORK").unwrap().exists("b"));
        // _LAST_ = dernière sortie.
        assert_eq!(s.last_dataset.as_deref(), Some("WORK.B"));
    }
}
