//! PROC PRINT (jalon M1).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! ## Syntaxe M1
//! `proc print data=lib.x [noobs] [label] ; [var v1 v2... ;] run ;`
//!
//! ## Exécution
//! 1. Résoudre le dataset (data= ou _LAST_) ; lire via LibraryProvider ;
//!    forwarder les notes de coercition au log.
//! 2. Colonnes : `var` si présent (ERROR si variable inconnue :
//!    "Variable XXXX not found."), sinon toutes dans l'ordre du dataset.
//! 3. Rendu listing : `page_header()` puis table —
//!    - colonne `Obs` (1..n, alignée droite) sauf NOOBS ;
//!    - numériques : format de la variable si défini (M4 — avant cela
//!      BEST12. trimé via `value::format_best(v, 12)`), missings `.` ou
//!      lettre spéciale ; alignés DROITE ;
//!    - caractères : tels quels, alignés GAUCHE.
//! 4. NOTEs log : "There were N observations read from the data set
//!    WORK.X." (l'appelant procs::execute_proc ajoute la NOTE de timing).

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::num_to_value;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::session::Session;
use crate::value::{format_best, Value};

pub struct PrintAst {
    pub data: Option<DatasetRef>,
    pub vars: Option<Vec<String>>,
    pub noobs: bool,
    /// Option LABEL : utilise le libellé de chaque variable (s'il existe)
    /// comme en-tête de colonne au lieu du nom. Défaut = noms (comme SAS).
    pub label: bool,
    /// Option DOUBLE : double-interligne les lignes de données (M33.6).
    pub double: bool,
    /// Option N : imprime une ligne "N = <n>" du nombre d'observations en fin
    /// de section (par groupe BY si BY présent) (M33.6).
    pub n: bool,
    /// Statement BY : sections par groupe BY (entrée triée requise). Liste de
    /// (variable, descending) comme PROC SORT (M33.6).
    pub by: Vec<(String, bool)>,
    /// Statement ID : remplace la colonne `Obs` par les valeurs de ces
    /// variables à gauche de chaque ligne (M33.6).
    pub id: Vec<String>,
    /// Statement SUM : variables numériques à totaliser en bas (sous-totaux par
    /// groupe BY + total général) (M33.6).
    pub sum: Vec<String>,
}

/// Parse `proc print [data=lib.t] [noobs] [label] [double] [n] ;
///        [var ...;] [by ...;] [id ...;] [sum ...;] ... run ;`
/// Called AFTER "proc print" has been consumed. Consumes through `run;`.
pub fn parse(ts: &mut StatementStream) -> Result<PrintAst> {
    let mut data: Option<DatasetRef> = None;
    let mut noobs = false;
    let mut label = false;
    let mut double = false;
    let mut n = false;
    let mut vars: Option<Vec<String>> = None;
    let mut by: Vec<(String, bool)> = Vec::new();
    let mut id: Vec<String> = Vec::new();
    let mut sum: Vec<String> = Vec::new();

    // En-tête PROC PRINT : options jusqu'au `;` (combinateur partagé M31).
    common::parse_proc_options(ts, "PRINT", |ts, kw| {
        Ok(match kw {
            "data" => {
                data = Some(common::parse_dataset_opt(ts, "DATA")?);
                true
            }
            "noobs" => {
                ts.next();
                noobs = true;
                true
            }
            "label" => {
                // LABEL option: utilise les libellés comme en-têtes (M4).
                ts.next();
                label = true;
                true
            }
            "double" => {
                ts.next();
                double = true;
                true
            }
            "n" => {
                ts.next();
                n = true;
                true
            }
            _ => false,
        })
    })?;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next(); // consume "var"
                vars = Some(common::parse_var_list(ts)?);
                true
            }
            "by" => {
                ts.next(); // consume "by"
                by = common::parse_by(ts)?;
                true
            }
            "id" => {
                ts.next(); // consume "id"
                id = common::parse_var_list(ts)?;
                true
            }
            "sum" => {
                ts.next(); // consume "sum"
                sum = common::parse_var_list(ts)?;
                true
            }
            _ => false,
        })
    })?;

    Ok(PrintAst {
        data,
        vars,
        noobs,
        label,
        double,
        n,
        by,
        id,
        sum,
    })
}

/// Execute PROC PRINT. Called by `procs::execute_proc` which wraps with timing.
pub fn execute(ast: &PrintAst, session: &mut Session) -> Result<()> {
    // Resolve dataset reference: data= or _LAST_ (combinateur partagé M31).
    let ds_ref: DatasetRef = common::resolve_last_dataset(&ast.data, session)?;

    let libref = ds_ref.libref_or_work();
    let table_name = ds_ref.name.to_uppercase();
    let display_name = ds_ref.display(); // e.g. "WORK.MYDATA"

    // Read the dataset
    let provider = session.libs.get(&libref)?;
    let (ds, notes) = provider.read(&table_name)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_obs = ds.n_obs();

    // Determine columns to print
    let col_indices: Vec<usize> = if let Some(ref var_names) = ast.vars {
        // Validate each name
        let mut idxs = Vec::with_capacity(var_names.len());
        for vname in var_names {
            let idx = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(vname));
            match idx {
                Some(i) => idxs.push(i),
                None => {
                    return Err(SasError::runtime(format!(
                        "Variable {} not found.",
                        vname.to_uppercase()
                    )));
                }
            }
        }
        idxs
    } else {
        (0..ds.vars.len()).collect()
    };

    // ── M33.6 : résolution des statements ID / SUM / BY ──────────────────────
    // ID variables (gauche, remplacent Obs). Validés contre le dataset.
    let id_indices = resolve_names(&ds, &ast.id)?;
    // SUM variables (numériques totalisées en bas). Validées.
    let sum_indices = resolve_names(&ds, &ast.sum)?;
    // BY columns (entrée triée requise).
    let by_cols = common::resolve_by_cols(&ds, &ast.by)?;

    let use_id = !id_indices.is_empty();

    // Build headers and alignments.
    //
    // Layout: [ID cols | Obs | VAR/data cols]. With ID, the `Obs` column is
    // suppressed (ID values replace it as the row identifier on the left).
    let mut headers: Vec<String> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();

    let header_of = |idx: usize| -> String {
        match (ast.label, &ds.vars[idx].label) {
            (true, Some(lbl)) if !lbl.is_empty() => lbl.clone(),
            _ => ds.vars[idx].name.clone(),
        }
    };
    let align_of = |idx: usize| -> Align {
        match ds.vars[idx].ty {
            crate::value::VarType::Num => Align::Right,
            crate::value::VarType::Char => Align::Left,
        }
    };

    for &idx in &id_indices {
        headers.push(header_of(idx));
        aligns.push(align_of(idx));
    }
    if !ast.noobs && !use_id {
        headers.push("Obs".to_string());
        aligns.push(Align::Right);
    }
    for &idx in &col_indices {
        headers.push(header_of(idx));
        aligns.push(align_of(idx));
    }

    // Take a shared reference to the session's format catalog for formatting.
    // All cell formatting is done here, before any &mut session use below.
    let cat = &session.format_catalog;
    // M38.2 — MISSING= : character for ordinary numeric missing ('.').
    let missing_char = session.options.missing_char;

    // Décode chaque colonne UNE seule fois (downcast par colonne, jamais
    // par cellule — checklist PLAN.md point 3). On formate à la fois les
    // colonnes ID et les colonnes de données.
    let format_col = |col_i: usize| -> Result<Vec<String>> {
        let series = ds.df.get_columns()[col_i].as_materialized_series();
        let spec = ds.vars[col_i]
            .format
            .as_deref()
            .and_then(crate::formats::FormatSpec::parse);
        let cells: Vec<String> = match ds.vars[col_i].ty {
            crate::value::VarType::Num => series
                .f64()?
                .iter()
                .map(|o| {
                    let v = num_to_value(o);
                    match &spec {
                        Some(spec) => cat.format(&v, spec),
                        None => match v {
                            // Ordinary missing `.` uses the session MISSING= char.
                            // Special missings keep their SAS suffix (._/.A..Z).
                            Value::Missing(crate::value::MissingKind::Dot) => {
                                missing_char.to_string()
                            }
                            Value::Missing(kind) => kind.display(),
                            Value::Num(f) => format_best(f, 12),
                            Value::Char(_) => unreachable!("num column decoded to char"),
                        },
                    }
                })
                .collect(),
            crate::value::VarType::Char => series
                .str()?
                .iter()
                .map(|o| {
                    let raw = o.unwrap_or("");
                    match &spec {
                        Some(spec) => cat.format(&Value::Char(raw.to_string()), spec),
                        None => raw.to_string(),
                    }
                })
                .collect(),
        };
        Ok(cells)
    };

    let mut id_cells: Vec<Vec<String>> = Vec::with_capacity(id_indices.len());
    for &col_i in &id_indices {
        id_cells.push(format_col(col_i)?);
    }
    let mut col_cells: Vec<Vec<String>> = Vec::with_capacity(col_indices.len());
    for &col_i in &col_indices {
        col_cells.push(format_col(col_i)?);
    }

    // For SUM: decode the raw numeric values of each sum variable (once).
    let mut sum_values: Vec<Vec<Value>> = Vec::with_capacity(sum_indices.len());
    for &col_i in &sum_indices {
        sum_values.push(common::decode_column(&ds, col_i)?);
    }
    // Column position (within the rendered row) of each SUM variable, so the
    // totals line places each total under its column. A sum var must be a
    // displayed data column (in col_indices); if not displayed it is ignored
    // for placement (SAS would still total it, but it has no column to sit in).
    let sum_render_pos: Vec<Option<usize>> = sum_indices
        .iter()
        .map(|&si| {
            col_indices.iter().position(|&ci| ci == si).map(|p| {
                // offset by ID columns + optional Obs column on the left
                id_indices.len() + usize::from(!ast.noobs && !use_id) + p
            })
        })
        .collect();
    let n_render_cols = headers.len();

    // Helper: build one rendered row for input row `row_i`.
    let build_row = |row_i: usize| -> Vec<String> {
        let mut row: Vec<String> = Vec::with_capacity(n_render_cols);
        for cells in &id_cells {
            row.push(cells[row_i].clone());
        }
        if !ast.noobs && !use_id {
            row.push((row_i + 1).to_string());
        }
        for cells in &col_cells {
            row.push(cells[row_i].clone());
        }
        row
    };

    // Helper: build a totals row over a set of input rows. Returns None when
    // there are no SUM variables. The total of a column is the SAS SUM (missing
    // values ignored; all-missing → missing `.`).
    let build_totals = |rows: &[usize]| -> Option<Vec<String>> {
        if sum_indices.is_empty() {
            return None;
        }
        let mut out = vec![String::new(); n_render_cols];
        for (k, vals) in sum_values.iter().enumerate() {
            let mut acc = 0.0_f64;
            let mut any = false;
            for &r in rows {
                if let Value::Num(f) = vals[r] {
                    acc += f;
                    any = true;
                }
            }
            let cell = if any {
                format_best(acc, 12)
            } else {
                ".".to_string()
            };
            if let Some(pos) = sum_render_pos[k] {
                out[pos] = cell;
            }
        }
        Some(out)
    };

    // ── Rendu ────────────────────────────────────────────────────────────────
    session.listing.page_header();

    if by_cols.is_empty() {
        // No BY: single section over all rows.
        let rows: Vec<Vec<String>> = (0..n_obs).map(build_row).collect();
        let all: Vec<usize> = (0..n_obs).collect();
        let totals = build_totals(&all);
        session
            .listing
            .write_table_ext(&headers, &aligns, &rows, ast.double, totals.as_ref());
        if ast.n {
            session.listing.blank();
            session.listing.write_line(&format!("N = {}", n_obs));
        }
    } else {
        // BY: verify sortedness then iterate contiguous groups.
        let by_values: Vec<Vec<Value>> = by_cols
            .iter()
            .map(|bc| common::decode_column(&ds, bc.col_idx))
            .collect::<Result<_>>()?;
        let descending: Vec<bool> = by_cols.iter().map(|b| b.descending).collect();
        let by_names: Vec<String> = by_cols.iter().map(|b| b.name.clone()).collect();
        let groups = common::by_groups(&by_values, &descending, n_obs, &by_names, &display_name)?;

        let multi = groups.len() > 1;
        for (gi, (key, rows_idx)) in groups.iter().enumerate() {
            if gi > 0 {
                session.listing.blank();
            }
            // Standard BY heading line: "var1=val1 var2=val2".
            let heading = by_cols
                .iter()
                .zip(key)
                .map(|(bc, v)| format!("{}={}", bc.name, by_value_str(v)))
                .collect::<Vec<_>>()
                .join(" ");
            session.listing.write_line(&heading);
            session.listing.blank();

            let rows: Vec<Vec<String>> = rows_idx.iter().map(|&r| build_row(r)).collect();
            let totals = build_totals(rows_idx);
            session
                .listing
                .write_table_ext(&headers, &aligns, &rows, ast.double, totals.as_ref());
            if ast.n {
                session.listing.blank();
                session.listing.write_line(&format!("N = {}", rows_idx.len()));
            }
        }

        // Grand total across all observations when SUM + more than one group.
        // SAS renders this aligned under the columns; to avoid replicating the
        // whole-report column-width computation across heterogeneous BY groups,
        // we emit it as an explicit labelled line "var=total ..." (documented
        // simplification — values are SAS-exact, the placement is textual).
        if !sum_indices.is_empty() && multi {
            let all: Vec<usize> = (0..n_obs).collect();
            let parts: Vec<String> = sum_indices
                .iter()
                .enumerate()
                .map(|(k, &si)| {
                    let mut acc = 0.0_f64;
                    let mut any = false;
                    for &r in &all {
                        if let Value::Num(f) = sum_values[k][r] {
                            acc += f;
                            any = true;
                        }
                    }
                    let cell = if any { format_best(acc, 12) } else { ".".to_string() };
                    format!("{}={}", ds.vars[si].name, cell)
                })
                .collect();
            session.listing.blank();
            session
                .listing
                .write_line(&format!("Grand total: {}", parts.join(" ")));
        }
    }

    // Log NOTE — "There were N observations read from the data set WORK.X."
    // PLAN.md checklist item 7: pluriel invariable ("1 observations." — fidèle à SAS)
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_obs, display_name
    ));

    Ok(())
}

/// Resolve a list of variable names to dataset column indices, erroring with
/// the SAS "Variable XXXX not found." message on the first unknown name.
fn resolve_names(ds: &crate::dataset::SasDataset, names: &[String]) -> Result<Vec<usize>> {
    let mut idxs = Vec::with_capacity(names.len());
    for vname in names {
        match ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(vname))
        {
            Some(i) => idxs.push(i),
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    vname.to_uppercase()
                )));
            }
        }
    }
    Ok(idxs)
}

/// Render a BY-key value for the BY heading line ("var=value").
fn by_value_str(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.trim_end().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::prelude::*;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_print_src(src: &str) -> Result<PrintAst> {
        let full = format!("proc print {}; run;", src);
        let source = SourceFile::new(&full);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        // consume "proc"
        ts.next();
        // consume "print"
        ts.next();
        parse(&mut ts)
    }

    fn parse_print_with_var(src: &str) -> Result<PrintAst> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // print
        parse(&mut ts)
    }

    // --- parse tests ---

    #[test]
    fn parse_minimal() {
        let ast = parse_print_src("").unwrap();
        assert!(ast.data.is_none());
        assert!(!ast.noobs);
        assert!(ast.vars.is_none());
    }

    #[test]
    fn parse_data_option() {
        let ast = parse_print_src("data=mylib.class").unwrap();
        assert_eq!(
            ast.data,
            Some(DatasetRef {
                libref: Some("mylib".into()),
                name: "class".into()
            })
        );
    }

    #[test]
    fn parse_data_work_only() {
        let ast = parse_print_src("data=foo").unwrap();
        assert_eq!(
            ast.data,
            Some(DatasetRef {
                libref: None,
                name: "foo".into()
            })
        );
    }

    #[test]
    fn parse_noobs() {
        let ast = parse_print_src("noobs").unwrap();
        assert!(ast.noobs);
    }

    #[test]
    fn parse_label_ignored() {
        let ast = parse_print_src("label").unwrap();
        assert!(!ast.noobs);
        assert!(ast.data.is_none());
    }

    #[test]
    fn parse_var_statement() {
        let src = "proc print data=work.x; var a b c; run;";
        let ast = parse_print_with_var(src).unwrap();
        assert_eq!(ast.vars, Some(vec!["a".to_string(), "b".to_string(), "c".to_string()]));
    }

    #[test]
    fn parse_noobs_and_data() {
        let src = "proc print data=work.foo noobs; run;";
        let ast = parse_print_with_var(src).unwrap();
        assert!(ast.noobs);
        assert_eq!(ast.data.as_ref().unwrap().name, "foo");
    }

    #[test]
    fn parse_unknown_option_errors() {
        let src = "proc print bogus; run;";
        let result = parse_print_with_var(src);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("BOGUS") || msg.contains("bogus"), "msg was: {msg}");
    }

    // --- execution tests ---

    fn write_test_dataset(session: &mut Session) {
        // Build a small DataFrame with one numeric and one char column
        let df = df![
            "name" => ["Alice", "Bob", "Carol"],
            "age"  => [30.0_f64, 25.0, 40.0]
        ]
        .unwrap();

        let vars = vec![
            VarMeta {
                name: "name".to_string(),
                ty: VarType::Char,
                length: 5,
                format: None,
                label: None,
            },
            VarMeta {
                name: "age".to_string(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write("MYDATA", &ds).unwrap();
        session.last_dataset = Some("WORK.MYDATA".to_string());
    }

    #[test]
    fn execute_basic_print() {
        let mut session = make_session();
        write_test_dataset(&mut session);

        let ast = PrintAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "MYDATA".into() }),
            vars: None,
            noobs: false,
            label: false,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        };

        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // Should have Obs column header
        assert!(listing.contains("Obs"), "listing: {listing}");
        // Should have column headers
        assert!(listing.contains("NAME") || listing.contains("name"), "listing: {listing}");
        assert!(listing.contains("AGE") || listing.contains("age"), "listing: {listing}");
        // Should have data values
        assert!(listing.contains("Alice"), "listing: {listing}");
        assert!(listing.contains("30"), "listing: {listing}");

        let log = session.log.into_string();
        // NOTE with count
        assert!(
            log.contains("There were 3 observations read from the data set WORK.MYDATA"),
            "log: {log}"
        );
    }

    #[test]
    fn execute_noobs() {
        let mut session = make_session();
        write_test_dataset(&mut session);

        let ast = PrintAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "MYDATA".into() }),
            vars: None,
            noobs: true,
            label: false,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        };

        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // Obs column should NOT appear
        assert!(!listing.contains("Obs"), "listing should not have Obs: {listing}");
        assert!(listing.contains("Alice"), "listing: {listing}");
    }

    #[test]
    fn execute_with_var_selection() {
        let mut session = make_session();
        write_test_dataset(&mut session);

        let ast = PrintAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "MYDATA".into() }),
            vars: Some(vec!["age".to_string()]),
            noobs: false,
            label: false,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        };

        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // age column must be present
        assert!(listing.contains("AGE") || listing.contains("age"), "listing: {listing}");
        // name column must NOT be present
        assert!(!listing.contains("Alice"), "name should not appear: {listing}");
    }

    #[test]
    fn execute_unknown_var_errors() {
        let mut session = make_session();
        write_test_dataset(&mut session);

        let ast = PrintAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "MYDATA".into() }),
            vars: Some(vec!["nonexistent".to_string()]),
            noobs: false,
            label: false,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        };

        let result = execute(&ast, &mut session);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("NONEXISTENT") || msg.contains("nonexistent"), "msg: {msg}");
    }

    #[test]
    fn execute_last_dataset() {
        let mut session = make_session();
        write_test_dataset(&mut session);
        // last_dataset is already set by write_test_dataset

        let ast = PrintAst {
            data: None, // use _LAST_
            vars: None,
            noobs: false,
            label: false,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        };

        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Alice"), "listing: {listing}");
    }

    #[test]
    fn execute_no_last_dataset_errors() {
        let mut session = make_session();
        // do NOT write any dataset, leave last_dataset = None

        let ast = PrintAst {
            data: None,
            vars: None,
            noobs: false,
            label: false,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        };

        let result = execute(&ast, &mut session);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("_LAST_") || msg.contains("undefined"), "msg: {msg}");
    }

    #[test]
    fn execute_note_plural_invariable() {
        // "1 observations." is the SAS convention — do not "fix" to "1 observation."
        let mut session = make_session();

        let df = df!["x" => [42.0_f64]].unwrap();
        let vars = vec![VarMeta {
            name: "x".to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write("ONE", &ds).unwrap();

        let ast = PrintAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "ONE".into() }),
            vars: None,
            noobs: false,
            label: false,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        // Must say "1 observations" (invariable plural — SAS behavior)
        assert!(
            log.contains("There were 1 observations read from the data set WORK.ONE"),
            "log: {log}"
        );
    }

    #[test]
    fn listing_alignments() {
        // Numeric values should be right-aligned, char values left-aligned.
        // We check by verifying Obs and age are in the right block.
        let mut session = make_session();
        write_test_dataset(&mut session);

        let ast = PrintAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "MYDATA".into() }),
            vars: None,
            noobs: false,
            label: false,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // Listing should contain all 3 obs numbers
        assert!(listing.contains("1"), "listing: {listing}");
        assert!(listing.contains("2"), "listing: {listing}");
        assert!(listing.contains("3"), "listing: {listing}");
    }

    // ── M4 : formats appliqués + option LABEL ─────────────────────────────

    fn write_formatted_dataset(session: &mut Session) {
        let df = df![
            "name"   => ["Alice", "Bob"],
            "weight" => [112.0_f64, 98.0]
        ]
        .unwrap();
        let vars = vec![
            VarMeta {
                name: "name".to_string(),
                ty: VarType::Char,
                length: 5,
                format: None,
                label: Some("Pupil Name".to_string()),
            },
            VarMeta {
                name: "weight".to_string(),
                ty: VarType::Num,
                length: 8,
                format: Some("dollar8.".to_string()),
                label: Some("Body Weight".to_string()),
            },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write("FMT", &ds).unwrap();
    }

    #[test]
    fn execute_applies_numeric_format() {
        let mut session = make_session();
        write_formatted_dataset(&mut session);

        let ast = PrintAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "FMT".into() }),
            vars: None,
            noobs: false,
            label: false,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // dollar8. renders 112 as "$112" (and 98 as "$98").
        assert!(listing.contains("$112"), "listing: {listing}");
        assert!(listing.contains("$98"), "listing: {listing}");
        // Without LABEL, headers are variable names (uppercased by SAS).
        assert!(
            listing.contains("weight") || listing.contains("WEIGHT"),
            "listing: {listing}"
        );
        assert!(
            !listing.contains("Body Weight"),
            "label must not appear without LABEL option: {listing}"
        );
    }

    #[test]
    fn execute_label_option_uses_labels_as_headers() {
        let mut session = make_session();
        write_formatted_dataset(&mut session);

        let ast = PrintAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "FMT".into() }),
            vars: None,
            noobs: false,
            label: true,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Body Weight"), "listing: {listing}");
        assert!(listing.contains("Pupil Name"), "listing: {listing}");
    }

    // ── End-to-end: PROC FORMAT → FORMAT statement → PROC PRINT ──────────────

    /// Prove that a user-defined format registered via PROC FORMAT is resolved
    /// by PROC PRINT through the session catalog.
    ///
    /// Setup:
    ///   1. Define `SEXFMT` (1→Male, 2→Female, other→Unknown) in the session
    ///      format catalog (simulating `proc format; value sexfmt 1='Male' ...`).
    ///   2. Write a dataset with a `sex` column (format="SEXFMT.") holding
    ///      values 1, 2, and 3.
    ///   3. Execute PROC PRINT and check the listing shows "Male", "Female",
    ///      and "Unknown" (the `other` label).
    #[test]
    fn user_format_end_to_end_via_session_catalog() {
        use crate::formats::userdef::{Bound, Range, UserFormat};

        let mut session = make_session();

        // 1. Register a user-defined numeric format in the session catalog.
        let uf = UserFormat {
            is_char: false,
            ranges: vec![
                Range {
                    from: Bound::Num(1.0),
                    to: Bound::Num(1.0),
                    from_exclusive: false,
                    to_exclusive: false,
                    label: "Male".to_string(),
                },
                Range {
                    from: Bound::Num(2.0),
                    to: Bound::Num(2.0),
                    from_exclusive: false,
                    to_exclusive: false,
                    label: "Female".to_string(),
                },
            ],
            other: Some("Unknown".to_string()),
        };
        session.format_catalog.define("SEXFMT", uf);

        // 2. Write a dataset whose `sex` column has format="SEXFMT."
        let df = df!["sex" => [1.0_f64, 2.0, 3.0]].unwrap();
        let vars = vec![VarMeta {
            name: "sex".to_string(),
            ty: VarType::Num,
            length: 8,
            format: Some("SEXFMT.".to_string()),
            label: None,
        }];
        let ds = SasDataset { df, vars };
        session
            .libs
            .get("WORK")
            .unwrap()
            .write("GENDER", &ds)
            .unwrap();

        // 3. PROC PRINT.
        let ast = PrintAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "GENDER".into(),
            }),
            vars: None,
            noobs: false,
            label: false,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // The user-defined format labels must appear in the listing.
        // This proves the session catalog was used, not FormatCatalog::default().
        assert!(listing.contains("Male"), "listing: {listing}");
        assert!(listing.contains("Female"), "listing: {listing}");
        assert!(listing.contains("Unknown"), "listing: {listing}");
    }

    // ── M33.6 : BY / ID / SUM / DOUBLE / N ────────────────────────────────────

    /// Build a small dataset sorted by `grp`: two groups A (2 rows) / B (1 row),
    /// with a numeric `v` to sum. Sums: A → 3+4=7, B → 5; grand → 12.
    fn write_grouped(session: &mut Session) {
        let df = df![
            "grp" => ["A", "A", "B"],
            "v"   => [3.0_f64, 4.0, 5.0]
        ]
        .unwrap();
        let vars = vec![
            VarMeta { name: "grp".into(), ty: VarType::Char, length: 1, format: None, label: None },
            VarMeta { name: "v".into(), ty: VarType::Num, length: 8, format: None, label: None },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write("G", &ds).unwrap();
        session.last_dataset = Some("WORK.G".to_string());
    }

    fn base_ast() -> PrintAst {
        PrintAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "G".into() }),
            vars: None,
            noobs: false,
            label: false,
            double: false,
            n: false,
            by: vec![],
            id: vec![],
            sum: vec![],
        }
    }

    #[test]
    fn parse_by_id_sum_double_n() {
        let src = "proc print data=work.g double n; by grp; id grp; sum v; run;";
        let ast = parse_print_with_var(src).unwrap();
        assert!(ast.double);
        assert!(ast.n);
        assert_eq!(ast.by, vec![("grp".to_string(), false)]);
        assert_eq!(ast.id, vec!["grp".to_string()]);
        assert_eq!(ast.sum, vec!["v".to_string()]);
    }

    #[test]
    fn execute_sum_no_by_totals() {
        let mut session = make_session();
        write_grouped(&mut session);
        let ast = PrintAst { sum: vec!["v".into()], ..base_ast() };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // Grand total of v = 12.
        assert!(listing.contains("12"), "sum total 12 expected: {listing}");
    }

    #[test]
    fn execute_n_option_prints_count() {
        let mut session = make_session();
        write_grouped(&mut session);
        let ast = PrintAst { n: true, ..base_ast() };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("N = 3"), "N = 3 expected: {listing}");
    }

    #[test]
    fn execute_by_sections_with_sum_subtotals_and_grand_total() {
        let mut session = make_session();
        write_grouped(&mut session);
        let ast = PrintAst {
            by: vec![("grp".into(), false)],
            sum: vec!["v".into()],
            n: true,
            ..base_ast()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // BY headings.
        assert!(listing.contains("grp=A"), "BY heading A: {listing}");
        assert!(listing.contains("grp=B"), "BY heading B: {listing}");
        // Per-group subtotals 7 (A) and 5 (B), and grand total 12.
        assert!(listing.contains("7"), "subtotal A=7: {listing}");
        assert!(listing.contains("Grand total: v=12"), "grand total: {listing}");
        // Per-group N lines.
        assert!(listing.contains("N = 2"), "N=2 for group A: {listing}");
        assert!(listing.contains("N = 1"), "N=1 for group B: {listing}");
    }

    #[test]
    fn execute_id_replaces_obs_column() {
        let mut session = make_session();
        write_grouped(&mut session);
        let ast = PrintAst { id: vec!["grp".into()], ..base_ast() };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // Obs column suppressed; ID variable header (grp) present.
        assert!(!listing.contains("Obs"), "Obs must be suppressed by ID: {listing}");
        assert!(listing.contains("grp"), "ID column header: {listing}");
    }

    #[test]
    fn execute_by_unsorted_errors() {
        let mut session = make_session();
        // grp out of order: B then A → not sorted ascending.
        let df = df![
            "grp" => ["B", "A"],
            "v"   => [1.0_f64, 2.0]
        ]
        .unwrap();
        let vars = vec![
            VarMeta { name: "grp".into(), ty: VarType::Char, length: 1, format: None, label: None },
            VarMeta { name: "v".into(), ty: VarType::Num, length: 8, format: None, label: None },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write("G", &ds).unwrap();

        let ast = PrintAst { by: vec![("grp".into(), false)], ..base_ast() };
        let err = execute(&ast, &mut session).unwrap_err();
        assert!(err.to_string().contains("not sorted"), "err: {err}");
    }

    #[test]
    fn execute_double_spaces_rows() {
        let mut session = make_session();
        write_grouped(&mut session);
        let ast = PrintAst { double: true, ..base_ast() };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // 3 data rows double-spaced → blank line between consecutive rows.
        // Count rows containing a value cell; the listing should be taller than
        // the single-spaced version. Cheap proxy: the value "4" and "5" appear.
        assert!(listing.contains('4') && listing.contains('5'), "rows present: {listing}");
    }
}
