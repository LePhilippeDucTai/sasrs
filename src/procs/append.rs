//! PROC APPEND (jalon M7 + options M33.9).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc append base=a data=b [force] [nowarn] [appendver=Vn] ; run ;`
//!
//! - base inexistante → créée comme copie de data (NOTE SAS).
//! - Sans FORCE : toute variable de DATA absente de BASE, ou type
//!   différent, ou longueur char supérieure → ERROR "No appending done
//!   because of anomalies...". Avec FORCE : variables en trop ignorées
//!   (WARNING), longueurs tronquées à celle de BASE, variables de BASE
//!   absentes de DATA → missing.
//! - Alignement par NOM (pas par position) ; décoder en Vec<Value> et
//!   reconstruire le DataFrame.
//! - NOTEs : "Appending WORK.B to WORK.A." + obs lues / obs ajoutées.
//!
//! ## Options M33.9
//! - NOWARN : avec FORCE, supprime le WARNING sur les différences
//!   structurelles forcées. Sans FORCE, la présence de NOWARN est sans
//!   effet (aucun WARNING n'est émis dans ce chemin). L'append est
//!   toujours réalisé normalement — NOWARN ne change pas le résultat,
//!   seulement la verbosité du log.
//! - APPENDVER=Vn (ex. APPENDVER=V6) : hint de version SAS des métadonnées
//!   d'en-tête. Aucun effet sémantique sur la sortie. Accepté et ignoré.

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::session::Session;
use crate::value::{Value, VarType};
use polars::prelude::*;

pub struct AppendAst {
    pub base: DatasetRef,
    pub data: DatasetRef,
    pub force: bool,
    /// NOWARN : supprime le WARNING FORCE sur les différences structurelles.
    pub nowarn: bool,
    /// APPENDVER : hint de version (no-op), ex. "V6". None = non spécifié.
    pub appendver: Option<String>,
}

/// Parse `proc append base=<ref> data=<ref> [force] [nowarn] [appendver=Vn] ; run ;`.
/// Called AFTER `proc append` has been consumed. Consumes through
/// `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<AppendAst> {
    use crate::token::TokenKind;

    let mut base: Option<DatasetRef> = None;
    let mut data: Option<DatasetRef> = None;
    let mut force = false;
    let mut nowarn = false;
    let mut appendver: Option<String> = None;

    // --- PROC APPEND statement options, until `;` (combinateur M31) ---
    common::parse_proc_options(ts, "APPEND", |ts, kw| {
        Ok(match kw {
            "base" => {
                base = Some(common::parse_dataset_opt(ts, "BASE")?);
                true
            }
            "data" => {
                data = Some(common::parse_dataset_opt(ts, "DATA")?);
                true
            }
            "force" => {
                ts.next();
                force = true;
                true
            }
            "nowarn" => {
                // NOWARN : supprime le WARNING de FORCE sur différences structurelles.
                ts.next();
                nowarn = true;
                true
            }
            "appendver" => {
                // APPENDVER=Vn : hint de version, aucun effet sémantique.
                ts.next(); // consume "appendver"
                if ts.peek().kind != TokenKind::Eq {
                    return Err(SasError::parse(
                        "expected '=' after APPENDVER".to_string(),
                        ts.peek().span,
                    ));
                }
                ts.next(); // consume '='
                let ver_tok = ts.peek().clone();
                let ver_name = ver_tok
                    .ident()
                    .ok_or_else(|| {
                        SasError::parse(
                            "expected a version string after APPENDVER=".to_string(),
                            ver_tok.span,
                        )
                    })?
                    .to_uppercase();
                ts.next();
                appendver = Some(ver_name);
                true
            }
            _ => false,
        })
    })?;

    let base = base.ok_or_else(|| {
        SasError::runtime(
            "The BASE= option is required on the PROC APPEND statement.",
        )
    })?;
    let data = data.ok_or_else(|| {
        SasError::runtime(
            "The DATA= option is required on the PROC APPEND statement.",
        )
    })?;

    // Consume through run;/quit; (sub-statements loop) (combinateur M31).
    common::parse_proc_body(ts, |_ts, _kw| Ok(false))?;

    Ok(AppendAst { base, data, force, nowarn, appendver })
}

/// Truncate a string to at most `max_chars` Unicode characters.
fn truncate_to_length(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    // SAS character length is in bytes, but we approximate with char count.
    // We truncate by chars to avoid splitting a multi-byte sequence.
    s.chars().take(max_chars).collect()
}

/// Execute PROC APPEND. Called by `procs::execute_proc`.
pub fn execute(ast: &AppendAst, session: &mut Session) -> Result<()> {
    let base_libref = ast.base.libref_or_work();
    let base_table = ast.base.name.to_uppercase();
    let data_libref = ast.data.libref_or_work();
    let data_table = ast.data.name.to_uppercase();

    let base_disp = ast.base.display();
    let data_disp = ast.data.display();

    // Read the DATA dataset.
    let data_provider = session.libs.get(&data_libref)?;
    let (data_ds, data_notes) = data_provider.read(&data_table)?;
    for note in data_notes {
        session.log.forward(&note);
    }
    let n_data = data_ds.n_obs();

    // Check whether BASE exists.
    let base_provider = session.libs.get(&base_libref)?;
    let base_exists = base_provider.exists(&base_table);

    session.log.note(&format!(
        "Appending {data_disp} to {base_disp}."
    ));

    if !base_exists {
        // BASE does not exist: copy DATA to BASE verbatim.
        session.log.note(
            "BASE data set does not exist. DATA file is being copied to BASE file.",
        );
        base_provider.write(&base_table, &data_ds)?;
        session.last_dataset = Some(format!("{base_libref}.{base_table}"));
        let n = data_ds.n_obs();
        session.log.note(&format!(
            "The data set {base_disp} has {n} observations and {} variables.",
            data_ds.n_vars(),
        ));
        return Ok(());
    }

    // BASE exists: read it.
    let (base_ds, base_notes) = base_provider.read(&base_table)?;
    for note in base_notes {
        session.log.forward(&note);
    }
    let n_base = base_ds.n_obs();

    // Build lookup: uppercase name → (index, VarMeta) for BASE and DATA.
    let base_idx: std::collections::HashMap<String, usize> = base_ds
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.to_uppercase(), i))
        .collect();
    let data_idx: std::collections::HashMap<String, usize> = data_ds
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.to_uppercase(), i))
        .collect();

    // --- Anomaly checks (without FORCE) ---
    if !ast.force {
        let mut anomalies = false;
        for dv in &data_ds.vars {
            let key = dv.name.to_uppercase();
            match base_idx.get(&key) {
                None => {
                    // Variable in DATA not in BASE.
                    session.log.error(&format!(
                        "Variable {key} was not found on the BASE file."
                    ));
                    anomalies = true;
                }
                Some(&bi) => {
                    let bv = &base_ds.vars[bi];
                    if bv.ty != dv.ty {
                        session.log.error(&format!(
                            "Variable {key} has a different type in the DATA and BASE data sets."
                        ));
                        anomalies = true;
                    } else if bv.ty == VarType::Char && dv.length > bv.length {
                        session.log.error(&format!(
                            "Variable {key} has a length of {} in the DATA file but {} in the BASE file.",
                            dv.length, bv.length
                        ));
                        anomalies = true;
                    }
                }
            }
        }
        if anomalies {
            return Err(SasError::runtime(
                "No appending done because of anomalies in the DATA file. \
                 Use the FORCE option to force the append.",
            ));
        }
    }

    // --- With FORCE: warn about DATA variables not in BASE (they'll be dropped). ---
    // NOWARN suppresses these warnings when FORCE is active.
    if ast.force && !ast.nowarn {
        for dv in &data_ds.vars {
            let key = dv.name.to_uppercase();
            if !base_idx.contains_key(&key) {
                session.log.warning(&format!(
                    "Variable {key} was not found on BASE file. \
                     The variable will not be appended."
                ));
            }
        }
    }

    // Decode all DATA columns once.
    let data_cols: Vec<Vec<Value>> = (0..data_ds.vars.len())
        .map(|i| decode_column(&data_ds, i))
        .collect::<Result<_>>()?;

    // Decode all BASE columns once.
    let base_cols: Vec<Vec<Value>> = (0..base_ds.vars.len())
        .map(|i| decode_column(&base_ds, i))
        .collect::<Result<_>>()?;

    // Build result: one column per BASE variable, BASE rows then DATA rows.
    let mut new_columns: Vec<Column> = Vec::with_capacity(base_ds.vars.len());
    let new_vars: Vec<VarMeta> = base_ds.vars.clone();

    for (bi, bv) in base_ds.vars.iter().enumerate() {
        let key = bv.name.to_uppercase();
        // Get the DATA column if present (by name, case-insensitive).
        let data_col_opt: Option<&Vec<Value>> = data_idx
            .get(&key)
            .map(|&di| &data_cols[di]);

        match bv.ty {
            VarType::Num => {
                // BASE rows first, then DATA rows.
                let mut vals: Vec<Option<f64>> = Vec::with_capacity(n_base + n_data);
                for v in &base_cols[bi] {
                    vals.push(value_to_num(v));
                }
                match data_col_opt {
                    Some(dc) => {
                        for v in dc {
                            vals.push(value_to_num(v));
                        }
                    }
                    None => {
                        // BASE-only variable: fill DATA rows with missing.
                        for _ in 0..n_data {
                            vals.push(None);
                        }
                    }
                }
                let ca = Float64Chunked::from_iter_options(bv.name.as_str().into(), vals.into_iter());
                new_columns.push(ca.into_series().into());
            }
            VarType::Char => {
                let base_len = bv.length;
                let mut vals: Vec<Option<String>> = Vec::with_capacity(n_base + n_data);
                for v in &base_cols[bi] {
                    match v {
                        Value::Char(s) => vals.push(Some(s.clone())),
                        _ => vals.push(Some(String::new())),
                    }
                }
                match data_col_opt {
                    Some(dc) => {
                        for v in dc {
                            match v {
                                Value::Char(s) => {
                                    // Truncate to BASE length.
                                    let truncated = truncate_to_length(s, base_len);
                                    vals.push(Some(truncated));
                                }
                                _ => vals.push(Some(String::new())),
                            }
                        }
                    }
                    None => {
                        // BASE-only variable: fill DATA rows with blank (missing char).
                        for _ in 0..n_data {
                            vals.push(Some(String::new()));
                        }
                    }
                }
                let ca: StringChunked = vals
                    .into_iter()
                    .map(|o| o.as_deref().map(str::to_string))
                    .collect();
                let ca = ca.with_name(bv.name.as_str().into());
                new_columns.push(ca.into_series().into());
            }
        }
    }

    let new_df = DataFrame::new(new_columns)?;
    let result = SasDataset {
        df: new_df,
        vars: new_vars,
    };

    base_provider.write(&base_table, &result)?;
    session.last_dataset = Some(format!("{base_libref}.{base_table}"));

    let n_total = result.n_obs();
    let n_vars = result.n_vars();

    // NOTEs SAS au pluriel invariable (checklist #7) : toujours
    // "observations"/"variables", même pour 1.
    session.log.note(&format!(
        "There were {n_data} observations read from the data set {data_disp}."
    ));
    session.log.note(&format!("{n_data} observations added."));
    session.log.note(&format!(
        "The data set {base_disp} has {n_total} observations and {n_vars} variables."
    ));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::df;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_append(src: &str) -> Result<AppendAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "append"
        parse(&mut ts)
    }

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    }

    fn read_dataset(session: &Session, table: &str) -> SasDataset {
        let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
        ds
    }

    fn num_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }
    }

    fn char_meta(name: &str, length: usize) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Char,
            length,
            format: None,
            label: None,
        }
    }

    // --- Parse tests ---

    #[test]
    fn parse_basic_fields() {
        let ast = parse_append("proc append base=work.a data=work.b force; run;").unwrap();
        assert_eq!(ast.base.libref.as_deref(), Some("work"));
        assert_eq!(ast.base.name, "a");
        assert_eq!(ast.data.libref.as_deref(), Some("work"));
        assert_eq!(ast.data.name, "b");
        assert!(ast.force);
    }

    #[test]
    fn parse_without_force() {
        let ast = parse_append("proc append base=a data=b; run;").unwrap();
        assert!(!ast.force);
        assert_eq!(ast.base.name, "a");
        assert_eq!(ast.data.name, "b");
    }

    #[test]
    fn parse_missing_base_errors() {
        let result = parse_append("proc append data=work.b; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("BASE="), "msg: {msg}");
    }

    #[test]
    fn parse_missing_data_errors() {
        let result = parse_append("proc append base=work.a; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("DATA="), "msg: {msg}");
    }

    #[test]
    fn parse_unknown_option_errors() {
        let result = parse_append("proc append base=a data=b bogus; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("BOGUS"), "msg: {msg}");
    }

    // --- Execute tests ---

    #[test]
    fn execute_base_missing_creates_copy() {
        let mut session = make_session();

        // Write DATA dataset only (no BASE).
        let df = df!["x" => [1.0_f64, 2.0, 3.0]].unwrap();
        let data_ds = SasDataset {
            df,
            vars: vec![num_meta("x")],
        };
        write_dataset(&mut session, "DATA_DS", data_ds);

        let ast = AppendAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE_DS".into() },
            data: DatasetRef { libref: Some("WORK".into()), name: "DATA_DS".into() },
            force: false,
            nowarn: false,
            appendver: None,
        };
        execute(&ast, &mut session).unwrap();

        // BASE_DS should now exist as a copy of DATA_DS.
        let result = read_dataset(&session, "BASE_DS");
        assert_eq!(result.n_obs(), 3);
        let col = decode_column(&result, 0).unwrap();
        assert_eq!(col, vec![Value::Num(1.0), Value::Num(2.0), Value::Num(3.0)]);

        // last_dataset should point to BASE.
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.BASE_DS"));

        // Log should mention copying.
        let log = session.log.into_string();
        assert!(log.contains("DATA file is being copied to BASE file"), "log: {log}");
    }

    #[test]
    fn execute_compatible_append_grows_base() {
        let mut session = make_session();

        let base_df = df!["x" => [1.0_f64, 2.0]].unwrap();
        let base_ds = SasDataset { df: base_df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "BASE", base_ds);

        let data_df = df!["x" => [3.0_f64, 4.0, 5.0]].unwrap();
        let data_ds = SasDataset { df: data_df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "DATA", data_ds);

        let ast = AppendAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE".into() },
            data: DatasetRef { libref: Some("WORK".into()), name: "DATA".into() },
            force: false,
            nowarn: false,
            appendver: None,
        };
        execute(&ast, &mut session).unwrap();

        let result = read_dataset(&session, "BASE");
        assert_eq!(result.n_obs(), 5, "base should have 2+3=5 rows");
        let col = decode_column(&result, 0).unwrap();
        assert_eq!(
            col,
            vec![
                Value::Num(1.0),
                Value::Num(2.0),
                Value::Num(3.0),
                Value::Num(4.0),
                Value::Num(5.0),
            ]
        );

        let log = session.log.into_string();
        assert!(log.contains("3 observations read from"), "log: {log}");
        assert!(log.contains("3 observations added"), "log: {log}");
        assert!(log.contains("5 observations and 1 variable"), "log: {log}");
    }

    #[test]
    fn execute_without_force_extra_data_var_anomaly_errors() {
        let mut session = make_session();

        // BASE has only x; DATA has x and y (y is extra — anomaly without FORCE).
        let base_df = df!["x" => [1.0_f64]].unwrap();
        let base_ds = SasDataset { df: base_df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "BASE", base_ds);

        let data_df = df!["x" => [2.0_f64], "y" => [99.0_f64]].unwrap();
        let data_ds = SasDataset {
            df: data_df,
            vars: vec![num_meta("x"), num_meta("y")],
        };
        write_dataset(&mut session, "DATA", data_ds);

        let ast = AppendAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE".into() },
            data: DatasetRef { libref: Some("WORK".into()), name: "DATA".into() },
            force: false,
            nowarn: false,
            appendver: None,
        };
        let result = execute(&ast, &mut session);
        assert!(result.is_err(), "expected anomaly error");
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("anomalies"), "msg: {msg}");

        // BASE should NOT have grown.
        let base_after = read_dataset(&session, "BASE");
        assert_eq!(base_after.n_obs(), 1, "BASE should still have 1 row");
    }

    #[test]
    fn execute_with_force_extra_data_var_dropped_base_only_var_missing() {
        let mut session = make_session();

        // BASE has x and z; DATA has x and y (y is extra, z is base-only).
        let base_df = df!["x" => [1.0_f64], "z" => [10.0_f64]].unwrap();
        let base_ds = SasDataset {
            df: base_df,
            vars: vec![num_meta("x"), num_meta("z")],
        };
        write_dataset(&mut session, "BASE", base_ds);

        let data_df = df!["x" => [2.0_f64], "y" => [99.0_f64]].unwrap();
        let data_ds = SasDataset {
            df: data_df,
            vars: vec![num_meta("x"), num_meta("y")],
        };
        write_dataset(&mut session, "DATA", data_ds);

        let ast = AppendAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE".into() },
            data: DatasetRef { libref: Some("WORK".into()), name: "DATA".into() },
            force: true,
            nowarn: false,
            appendver: None,
        };
        execute(&ast, &mut session).unwrap();

        let result = read_dataset(&session, "BASE");
        assert_eq!(result.n_obs(), 2, "base should have 1+1=2 rows");
        assert_eq!(result.n_vars(), 2, "base should still have 2 vars (x, z)");

        // x should be [1.0, 2.0].
        let xi = result.vars.iter().position(|v| v.name == "x").unwrap();
        let x_col = decode_column(&result, xi).unwrap();
        assert_eq!(x_col, vec![Value::Num(1.0), Value::Num(2.0)]);

        // z for the appended DATA row should be missing.
        let zi = result.vars.iter().position(|v| v.name == "z").unwrap();
        let z_col = decode_column(&result, zi).unwrap();
        assert_eq!(z_col[0], Value::Num(10.0));
        assert_eq!(z_col[1], Value::Missing(crate::value::MissingKind::Dot));

        // Log should contain a warning about y being dropped.
        let log = session.log.into_string();
        assert!(log.contains("Y") || log.contains("y"), "log: {log}");
        assert!(log.contains("not found on BASE"), "log: {log}");
    }

    #[test]
    fn execute_with_force_char_truncation() {
        let mut session = make_session();

        // BASE has char var name with length 3.
        let base_df = df!["name" => ["abc"]].unwrap();
        let base_ds = SasDataset {
            df: base_df,
            vars: vec![char_meta("name", 3)],
        };
        write_dataset(&mut session, "BASE", base_ds);

        // DATA has char var name with length 8 (longer than BASE).
        let data_df = df!["name" => ["hello!"]].unwrap();
        let data_ds = SasDataset {
            df: data_df,
            vars: vec![char_meta("name", 6)],
        };
        write_dataset(&mut session, "DATA", data_ds);

        let ast = AppendAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE".into() },
            data: DatasetRef { libref: Some("WORK".into()), name: "DATA".into() },
            force: true,
            nowarn: false,
            appendver: None,
        };
        execute(&ast, &mut session).unwrap();

        let result = read_dataset(&session, "BASE");
        assert_eq!(result.n_obs(), 2);
        let col = decode_column(&result, 0).unwrap();
        assert_eq!(col[0], Value::Char("abc".to_string()));
        // "hello!" truncated to 3 chars => "hel"
        assert_eq!(col[1], Value::Char("hel".to_string()));
    }

    #[test]
    fn execute_without_force_char_truncation_is_anomaly() {
        let mut session = make_session();

        // BASE has char var name with length 3; DATA has length 6 > 3 → anomaly.
        let base_df = df!["name" => ["abc"]].unwrap();
        let base_ds = SasDataset {
            df: base_df,
            vars: vec![char_meta("name", 3)],
        };
        write_dataset(&mut session, "BASE", base_ds);

        let data_df = df!["name" => ["hello!"]].unwrap();
        let data_ds = SasDataset {
            df: data_df,
            vars: vec![char_meta("name", 6)],
        };
        write_dataset(&mut session, "DATA", data_ds);

        let ast = AppendAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE".into() },
            data: DatasetRef { libref: Some("WORK".into()), name: "DATA".into() },
            force: false,
            nowarn: false,
            appendver: None,
        };
        let result = execute(&ast, &mut session);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("anomalies"), "msg: {msg}");
    }

    // --- M33.9 new option tests ---

    #[test]
    fn parse_nowarn_accepted() {
        let ast = parse_append("proc append base=a data=b force nowarn; run;").unwrap();
        assert!(ast.force);
        assert!(ast.nowarn);
        assert!(ast.appendver.is_none());
    }

    #[test]
    fn parse_appendver_accepted() {
        let ast = parse_append("proc append base=a data=b appendver=v6; run;").unwrap();
        assert_eq!(ast.appendver.as_deref(), Some("V6"));
        assert!(!ast.nowarn);
    }

    #[test]
    fn parse_appendver_v9_accepted() {
        let ast = parse_append("proc append base=a data=b appendver=v9; run;").unwrap();
        assert_eq!(ast.appendver.as_deref(), Some("V9"));
    }

    #[test]
    fn execute_nowarn_suppresses_force_warning() {
        // With FORCE + NOWARN, the "Variable ... not found on BASE file" WARNING
        // should NOT appear in the log.
        let mut session = make_session();

        // BASE has x only; DATA has x and y (y is extra).
        let base_df = df!["x" => [1.0_f64]].unwrap();
        let base_ds = SasDataset { df: base_df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "BASE", base_ds);

        let data_df = df!["x" => [2.0_f64], "y" => [99.0_f64]].unwrap();
        let data_ds = SasDataset {
            df: data_df,
            vars: vec![num_meta("x"), num_meta("y")],
        };
        write_dataset(&mut session, "DATA", data_ds);

        let ast = AppendAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE".into() },
            data: DatasetRef { libref: Some("WORK".into()), name: "DATA".into() },
            force: true,
            nowarn: true,
            appendver: None,
        };
        execute(&ast, &mut session).unwrap();

        // Append must have succeeded.
        let result = read_dataset(&session, "BASE");
        assert_eq!(result.n_obs(), 2, "NOWARN still appends");

        // No WARNING in log.
        let log = session.log.into_string();
        assert!(
            !log.to_uppercase().contains("WARNING"),
            "NOWARN should suppress FORCE warnings, log: {log}"
        );
    }

    #[test]
    fn execute_force_without_nowarn_emits_warning() {
        // Sanity: without NOWARN, the warning IS present.
        let mut session = make_session();

        let base_df = df!["x" => [1.0_f64]].unwrap();
        let base_ds = SasDataset { df: base_df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "BASE", base_ds);

        let data_df = df!["x" => [2.0_f64], "y" => [99.0_f64]].unwrap();
        let data_ds = SasDataset {
            df: data_df,
            vars: vec![num_meta("x"), num_meta("y")],
        };
        write_dataset(&mut session, "DATA", data_ds);

        let ast = AppendAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE".into() },
            data: DatasetRef { libref: Some("WORK".into()), name: "DATA".into() },
            force: true,
            nowarn: false,
            appendver: None,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(
            log.contains("not found on BASE"),
            "Without NOWARN, warning must appear, log: {log}"
        );
    }

    #[test]
    fn execute_appendver_no_effect_on_output() {
        // APPENDVER= is a no-op; result identical to without it.
        let mut s1 = make_session();
        let mut s2 = make_session();

        for s in [&mut s1, &mut s2] {
            let base_df = df!["x" => [1.0_f64]].unwrap();
            let base_ds = SasDataset { df: base_df, vars: vec![num_meta("x")] };
            write_dataset(s, "BASE", base_ds);

            let data_df = df!["x" => [2.0_f64]].unwrap();
            let data_ds = SasDataset { df: data_df, vars: vec![num_meta("x")] };
            write_dataset(s, "DATA", data_ds);
        }

        // Without APPENDVER.
        let ast_plain = AppendAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE".into() },
            data: DatasetRef { libref: Some("WORK".into()), name: "DATA".into() },
            force: false,
            nowarn: false,
            appendver: None,
        };
        execute(&ast_plain, &mut s1).unwrap();

        // With APPENDVER=V6.
        let ast_ver = AppendAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE".into() },
            data: DatasetRef { libref: Some("WORK".into()), name: "DATA".into() },
            force: false,
            nowarn: false,
            appendver: Some("V6".to_string()),
        };
        execute(&ast_ver, &mut s2).unwrap();

        // Both outputs must be identical (2 rows, x=[1.0, 2.0]).
        let r1 = read_dataset(&s1, "BASE");
        let r2 = read_dataset(&s2, "BASE");
        assert_eq!(r1.n_obs(), r2.n_obs());
        assert_eq!(
            decode_column(&r1, 0).unwrap(),
            decode_column(&r2, 0).unwrap(),
            "APPENDVER= must not affect output"
        );
    }
}
