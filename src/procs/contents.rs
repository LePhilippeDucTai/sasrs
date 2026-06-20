//! PROC CONTENTS (jalon M4).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc contents data=lib.x [varnum] ; run ;` — affiche les métadonnées :
//! en-tête (nom, observations, variables), puis table des variables
//! (# / Variable / Type Num|Char / Len / Format / Label), triée par nom
//! (défaut) ou par position (VARNUM). Lit uniquement les métadonnées
//! (VarMeta + hauteur) — pas besoin de matérialiser les données :
//! prévoir plus tard un `LibraryProvider::read_meta` si les fichiers
//! deviennent gros.
//! `data=lib._all_` : liste les tables de la librairie (via `list()`).
//!
//! ## Header block layout
//!
//! Two-column layout: left column (~25 chars) holds the label, right column
//! holds the value.  Three lines:
//!
//!   Data Set Name: WORK.CLASS       Observations:  10
//!   Member Type:   DATA             Variables:      3
//!   Engine:        PARQUET
//!
//! Variable table columns (in order): #, Variable, Type, Len, Format, Label.
//! `#` and `Len` are right-aligned; all others left-aligned.

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::Result;
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::session::Session;
use crate::value::VarType;
use polars::prelude::*;

pub struct ContentsAst {
    pub data: Option<DatasetRef>,
    pub varnum: bool,
    /// data=lib._all_
    pub all: bool,
    /// OUT=<ds> : écrit un dataset (une ligne par variable) au lieu du listing
    /// normal de la table des variables (M33.7).
    pub out: Option<DatasetRef>,
    /// SHORT : n'imprime qu'une liste à plat des noms de variables (M33.7).
    pub short: bool,
    /// DETAILS : ajoute des infos d'observations/taille au bloc d'en-tête
    /// (M33.7).
    pub details: bool,
}

/// Parse `proc contents [data=lib.x] [varnum] [out=ds] [short] [details]
///        [nodetails] ; run ;`
/// Called AFTER "proc contents" has been consumed. Consumes through `run;`.
pub fn parse(ts: &mut StatementStream) -> Result<ContentsAst> {
    let mut data: Option<DatasetRef> = None;
    let mut varnum = false;
    let mut all = false;
    let mut out: Option<DatasetRef> = None;
    let mut short = false;
    let mut details = false;

    // Parse PROC CONTENTS header options until `;` (combinateur partagé M31).
    common::parse_proc_options(ts, "CONTENTS", |ts, kw| {
        Ok(match kw {
            "data" => {
                let ds_ref = common::parse_dataset_opt(ts, "DATA")?;
                // Detect data=lib._all_ or data=_all_
                if ds_ref.name.to_uppercase() == "_ALL_" {
                    all = true;
                }
                data = Some(ds_ref);
                true
            }
            "varnum" => {
                ts.next();
                varnum = true;
                true
            }
            "out" => {
                out = Some(common::parse_out_opt(ts)?);
                true
            }
            "short" => {
                ts.next();
                short = true;
                true
            }
            "details" => {
                ts.next();
                details = true;
                true
            }
            "nodetails" => {
                ts.next();
                details = false;
                true
            }
            _ => false,
        })
    })?;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |_ts, _kw| Ok(false))?;

    Ok(ContentsAst {
        data,
        varnum,
        all,
        out,
        short,
        details,
    })
}

/// Execute PROC CONTENTS. Called by `procs::execute_proc`.
pub fn execute(ast: &ContentsAst, session: &mut Session) -> Result<()> {
    session.listing.page_header();

    if ast.all {
        // data=lib._all_  — list all tables in the library
        let libref = match &ast.data {
            Some(r) => r.libref_or_work(),
            None => "WORK".to_string(),
        };
        let provider = session.libs.get(&libref)?;
        let mut tables = provider.list()?;
        tables.sort();

        let headers = vec!["Member Name".to_string()];
        let aligns = vec![Align::Left];
        let rows: Vec<Vec<String>> = tables
            .into_iter()
            .map(|t| vec![t.to_uppercase()])
            .collect();
        session.listing.write_table(&headers, &aligns, &rows);
        return Ok(());
    }

    // Resolve the dataset reference (data= or _LAST_) (combinateur partagé M31).
    let ds_ref: DatasetRef = common::resolve_last_dataset(&ast.data, session)?;

    let libref = ds_ref.libref_or_work();
    let table_name = ds_ref.name.to_uppercase();
    let display_name = ds_ref.display(); // e.g. "WORK.CLASS"

    // Read the dataset (metadata only — we only look at VarMeta + n_obs)
    let provider = session.libs.get(&libref)?;
    let (ds, notes) = provider.read(&table_name)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_obs = ds.n_obs();
    let n_vars = ds.vars.len();

    // ── Header block ─────────────────────────────────────────────────────────
    //
    // Two-column layout.  Each line: left label (fixed 25 chars wide),
    // left value.  Two items per line where a right-hand item exists.
    //
    //   Data Set Name: WORK.CLASS       Observations:  10
    //   Member Type:   DATA             Variables:       3
    //   Engine:        PARQUET
    //
    // Label column width = 16 chars (enough for "Data Set Name: ").
    // We use simple string formatting; no table renderer needed.

    let left_label_width = 16usize;
    let left_value_width = 20usize; // pad left value to this width for alignment

    // Line 1: Data Set Name / Observations
    session.listing.write_line(&format!(
        "{:<lw$}{:<vw$}  {:<lw$}{}",
        "Data Set Name:",
        display_name,
        "Observations:",
        n_obs,
        lw = left_label_width,
        vw = left_value_width,
    ));
    // Line 2: Member Type / Variables
    session.listing.write_line(&format!(
        "{:<lw$}{:<vw$}  {:<lw$}{}",
        "Member Type:",
        "DATA",
        "Variables:",
        n_vars,
        lw = left_label_width,
        vw = left_value_width,
    ));
    // Line 3: Engine (no right-hand item)
    session.listing.write_line(&format!(
        "{:<lw$}{}",
        "Engine:",
        "PARQUET",
        lw = left_label_width,
    ));
    // DETAILS (M33.7) : extra observation/size info. We report the observation
    // count again as "# Observations" plus a derived "Obs in Buffer" proxy
    // (number of observations, the only size figure available without reading
    // the parquet page layout). Documented simplification: SAS reports physical
    // file size / page size, which the parquet engine does not surface here.
    if ast.details {
        session.listing.write_line(&format!(
            "{:<lw$}{}",
            "# Observations:",
            n_obs,
            lw = left_label_width,
        ));
        session.listing.write_line(&format!(
            "{:<lw$}{}",
            "# Variables:",
            n_vars,
            lw = left_label_width,
        ));
    }
    session.listing.blank();

    // ── OUT= dataset (M33.7) ──────────────────────────────────────────────────
    //
    // One row per variable. Column set (documented subset of SAS's CONTENTS
    // OUT= dataset, in creation order):
    //   NAME    (char)  variable name
    //   TYPE    (num)   SAS convention: 1 = numeric, 2 = character
    //   LENGTH  (num)   storage length in bytes
    //   VARNUM  (num)   1-based creation-order position
    //   LABEL   (char)  variable label ("" if none)
    //   FORMAT  (char)  format name ("" if none)
    // Rows are ordered by VARNUM (creation order), matching SAS's default
    // OUT= ordering (the VARNUM column makes any later re-sort lossless).
    if let Some(out_ref) = &ast.out {
        write_out_dataset(&ds, out_ref, session)?;
    }

    // SHORT (M33.7) : just a space-separated list of variable names (in display
    // order: alphabetical by default, creation order under VARNUM). No header
    // table, no per-variable detail.
    if ast.short {
        let mut idxs: Vec<usize> = (0..n_vars).collect();
        if !ast.varnum {
            idxs.sort_by(|&a, &b| {
                ds.vars[a]
                    .name
                    .to_ascii_lowercase()
                    .cmp(&ds.vars[b].name.to_ascii_lowercase())
            });
        }
        let names: Vec<String> = idxs.iter().map(|&i| ds.vars[i].name.clone()).collect();
        session.listing.write_line(&names.join(" "));
        return Ok(());
    }

    // ── Variable table ────────────────────────────────────────────────────────
    //
    // Columns: #, Variable, Type, Len, Format, Label
    // `#` and `Len` are right-aligned; others left-aligned.
    //
    // Sort order:
    //   - default: alphabetical by variable name (case-insensitive)
    //   - varnum:  creation order (original position in ds.vars)
    //
    // In all cases `#` shows the CREATION-ORDER position (1-based).

    let headers: Vec<String> = vec![
        "#".to_string(),
        "Variable".to_string(),
        "Type".to_string(),
        "Len".to_string(),
        "Format".to_string(),
        "Label".to_string(),
    ];
    let aligns: Vec<Align> = vec![
        Align::Right, // #
        Align::Left,  // Variable
        Align::Left,  // Type
        Align::Right, // Len
        Align::Left,  // Format
        Align::Left,  // Label
    ];

    // Build index array, then sort it
    let mut indices: Vec<usize> = (0..n_vars).collect();
    if !ast.varnum {
        // Sort alphabetically by name, case-insensitive
        indices.sort_by(|&a, &b| {
            ds.vars[a]
                .name
                .to_ascii_lowercase()
                .cmp(&ds.vars[b].name.to_ascii_lowercase())
        });
    }
    // If varnum=true, leave in creation order (already 0..n_vars)

    let rows: Vec<Vec<String>> = indices
        .into_iter()
        .map(|i| {
            let v = &ds.vars[i];
            let type_str = match v.ty {
                VarType::Num => "Num",
                VarType::Char => "Char",
            };
            vec![
                (i + 1).to_string(),                              // creation-order #
                v.name.clone(),
                type_str.to_string(),
                v.length.to_string(),
                v.format.as_deref().unwrap_or("").to_string(),
                v.label.as_deref().unwrap_or("").to_string(),
            ]
        })
        .collect();

    session.listing.write_table(&headers, &aligns, &rows);

    Ok(())
}

/// Build and write the CONTENTS OUT= dataset (one row per variable). See the
/// column documentation at the OUT= call site. Emits the standard
/// "The data set X has N observations and M variables." NOTE and updates
/// `_LAST_`.
fn write_out_dataset(ds: &SasDataset, out_ref: &DatasetRef, session: &mut Session) -> Result<()> {
    let names: Vec<Option<String>> = ds.vars.iter().map(|v| Some(v.name.clone())).collect();
    let types: Vec<Option<f64>> = ds
        .vars
        .iter()
        .map(|v| Some(match v.ty {
            VarType::Num => 1.0,
            VarType::Char => 2.0,
        }))
        .collect();
    let lengths: Vec<Option<f64>> = ds.vars.iter().map(|v| Some(v.length as f64)).collect();
    let varnums: Vec<Option<f64>> = (0..ds.vars.len()).map(|i| Some((i + 1) as f64)).collect();
    let labels: Vec<Option<String>> = ds
        .vars
        .iter()
        .map(|v| Some(v.label.clone().unwrap_or_default()))
        .collect();
    let formats: Vec<Option<String>> = ds
        .vars
        .iter()
        .map(|v| Some(v.format.clone().unwrap_or_default()))
        .collect();

    let columns: Vec<Column> = vec![
        Series::new("NAME".into(), names).into(),
        Series::new("TYPE".into(), types).into(),
        Series::new("LENGTH".into(), lengths).into(),
        Series::new("VARNUM".into(), varnums).into(),
        Series::new("LABEL".into(), labels).into(),
        Series::new("FORMAT".into(), formats).into(),
    ];
    let out_vars = vec![
        char_meta("NAME", 32),
        num_meta("TYPE"),
        num_meta("LENGTH"),
        num_meta("VARNUM"),
        char_meta("LABEL", 256),
        char_meta("FORMAT", 49),
    ];
    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars: out_vars };

    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = ds.vars.len();
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn parse_contents_src(src: &str) -> Result<ContentsAst> {
        let full = format!("proc contents {}; run;", src);
        let source = SourceFile::new(&full);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "contents"
        parse(&mut ts)
    }

    fn parse_contents_full(src: &str) -> Result<ContentsAst> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "contents"
        parse(&mut ts)
    }

    /// Write a small dataset with one Num and one Char variable.
    /// `age` has a format and label; `name` has neither.
    fn write_test_dataset(session: &mut Session) {
        let df = df![
            "name" => ["Alice", "Bob"],
            "age"  => [30.0_f64, 25.0]
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
                format: Some("best12.".to_string()),
                label: Some("Age of subject".to_string()),
            },
        ];
        let ds = SasDataset { df, vars };
        session
            .libs
            .get("WORK")
            .unwrap()
            .write("CLASS", &ds)
            .unwrap();
        session.last_dataset = Some("WORK.CLASS".to_string());
    }

    // ── Parse tests ───────────────────────────────────────────────────────────

    #[test]
    fn parse_minimal() {
        let ast = parse_contents_src("").unwrap();
        assert!(ast.data.is_none());
        assert!(!ast.varnum);
        assert!(!ast.all);
    }

    #[test]
    fn parse_data_option() {
        let ast = parse_contents_src("data=work.x").unwrap();
        assert_eq!(
            ast.data,
            Some(DatasetRef {
                libref: Some("work".into()),
                name: "x".into()
            })
        );
        assert!(!ast.varnum);
        assert!(!ast.all);
    }

    #[test]
    fn parse_varnum_option() {
        let ast = parse_contents_src("data=work.x varnum").unwrap();
        assert!(ast.varnum);
        assert!(!ast.all);
    }

    #[test]
    fn parse_all_option() {
        let ast = parse_contents_full("proc contents data=work._all_; run;").unwrap();
        assert!(ast.all);
        assert_eq!(ast.data.as_ref().unwrap().libref, Some("work".into()));
    }

    #[test]
    fn parse_all_uppercase() {
        let ast = parse_contents_full("proc contents data=MYLIB._ALL_; run;").unwrap();
        assert!(ast.all);
    }

    #[test]
    fn parse_unknown_option_errors() {
        let result = parse_contents_src("bogus");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("BOGUS") || msg.contains("bogus"), "msg: {msg}");
    }

    // ── Execute tests ─────────────────────────────────────────────────────────

    #[test]
    fn execute_basic_contents() {
        let mut session = make_session();
        write_test_dataset(&mut session);

        let ast = ContentsAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "CLASS".into(),
            }),
            varnum: false,
            all: false,
            out: None,
            short: false,
            details: false,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();

        // Header block should contain dataset name and observation count
        assert!(listing.contains("WORK.CLASS"), "listing: {listing}");
        assert!(listing.contains('2'), "obs count: {listing}");

        // Variable names must appear
        assert!(listing.contains("name") || listing.contains("NAME"), "listing: {listing}");
        assert!(listing.contains("age") || listing.contains("AGE"), "listing: {listing}");

        // Type column
        assert!(listing.contains("Num"), "Num type: {listing}");
        assert!(listing.contains("Char"), "Char type: {listing}");
    }

    #[test]
    fn execute_shows_format_and_label() {
        let mut session = make_session();
        write_test_dataset(&mut session);

        let ast = ContentsAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "CLASS".into(),
            }),
            varnum: false,
            all: false,
            out: None,
            short: false,
            details: false,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();

        // Format should appear in the variable table
        assert!(listing.contains("best12.") || listing.contains("BEST12."), "format: {listing}");

        // Label should appear in the variable table
        assert!(listing.contains("Age of subject"), "label: {listing}");
    }

    #[test]
    fn execute_varnum_ordering() {
        // With varnum, variables should appear in creation order (name then age).
        // Without varnum (default), they appear alphabetically (age then name).
        let mut session = make_session();
        write_test_dataset(&mut session);

        // Default: alphabetical → age before name
        let ast_alpha = ContentsAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "CLASS".into(),
            }),
            varnum: false,
            all: false,
            out: None,
            short: false,
            details: false,
        };
        execute(&ast_alpha, &mut session).unwrap();
        let listing = session.listing.into_string();

        // Find positions of "age" and "name" in the variable table section.
        // Use rfind so the header "Data Set Name:" (which also contains "name")
        // does not confuse the position check — the last occurrence of each
        // token is in the variable table, where alphabetical order must hold.
        let lower = listing.to_lowercase();
        let pos_age = lower.rfind("age");
        let pos_name = lower.rfind("name");
        assert!(pos_age.is_some() && pos_name.is_some(), "listing: {listing}");
        assert!(
            pos_age.unwrap() < pos_name.unwrap(),
            "alphabetical: age before name; listing:\n{listing}"
        );

        // With varnum: creation order → name (index 0) before age (index 1)
        let mut session2 = make_session();
        write_test_dataset(&mut session2);
        let ast_varnum = ContentsAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "CLASS".into(),
            }),
            varnum: true,
            all: false,
            out: None,
            short: false,
            details: false,
        };
        execute(&ast_varnum, &mut session2).unwrap();
        let listing2 = session2.listing.into_string();
        let lower2 = listing2.to_lowercase();
        // Skip the header "Variables: 2" which also contains text before variable table
        // Find the variable table section after the blank line following the header
        let pos_age2 = lower2.rfind("age");
        let pos_name2 = lower2.rfind("name");
        assert!(pos_age2.is_some() && pos_name2.is_some(), "listing2: {listing2}");
        assert!(
            pos_name2.unwrap() < pos_age2.unwrap(),
            "varnum: name (index 0) before age (index 1); listing:\n{listing2}"
        );
    }

    #[test]
    fn execute_all_lists_tables() {
        let mut session = make_session();
        write_test_dataset(&mut session);

        // Write a second dataset so there are 2 tables
        let df2 = df!["x" => [1.0_f64]].unwrap();
        let vars2 = vec![VarMeta {
            name: "x".to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }];
        let ds2 = SasDataset { df: df2, vars: vars2 };
        session
            .libs
            .get("WORK")
            .unwrap()
            .write("SCORES", &ds2)
            .unwrap();

        let ast = ContentsAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "_ALL_".into(),
            }),
            varnum: false,
            all: true,
            out: None,
            short: false,
            details: false,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("CLASS"), "listing: {listing}");
        assert!(listing.contains("SCORES"), "listing: {listing}");
        assert!(listing.contains("Member Name"), "listing: {listing}");
    }

    #[test]
    fn execute_uses_last_dataset_when_no_data() {
        let mut session = make_session();
        write_test_dataset(&mut session);

        let ast = ContentsAst {
            data: None,
            varnum: false,
            all: false,
            out: None,
            short: false,
            details: false,
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("WORK.CLASS"), "listing: {listing}");
    }

    #[test]
    fn execute_no_last_dataset_errors() {
        let mut session = make_session();

        let ast = ContentsAst {
            data: None,
            varnum: false,
            all: false,
            out: None,
            short: false,
            details: false,
        };
        let result = execute(&ast, &mut session);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("_LAST_") || msg.contains("undefined"), "msg: {msg}");
    }

    // ── M33.7 : OUT= / SHORT / DETAILS ────────────────────────────────────────

    #[test]
    fn parse_out_short_details() {
        let ast = parse_contents_full(
            "proc contents data=work.x out=work.meta short details; run;",
        )
        .unwrap();
        assert!(ast.short);
        assert!(ast.details);
        assert_eq!(
            ast.out,
            Some(DatasetRef { libref: Some("work".into()), name: "meta".into() })
        );
    }

    #[test]
    fn execute_out_dataset_shape_and_values() {
        let mut session = make_session();
        write_test_dataset(&mut session); // name (Char,5), age (Num,8, best12., "Age of subject")

        let ast = ContentsAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "CLASS".into() }),
            varnum: false,
            all: false,
            out: Some(DatasetRef { libref: Some("WORK".into()), name: "META".into() }),
            short: false,
            details: false,
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("META").unwrap();
        // 6 columns, 2 rows (one per variable in creation order: name, age).
        assert_eq!(out.n_obs(), 2, "one row per variable");
        let cols: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
        assert_eq!(cols, vec!["NAME", "TYPE", "LENGTH", "VARNUM", "LABEL", "FORMAT"]);

        // Decode rows. Row 0 = name (Char → TYPE 2, LENGTH 5, VARNUM 1).
        let name = out.df.column("NAME").unwrap().str().unwrap();
        assert_eq!(name.get(0), Some("name"));
        assert_eq!(name.get(1), Some("age"));
        let ty = out.df.column("TYPE").unwrap().f64().unwrap();
        assert_eq!(ty.get(0), Some(2.0)); // char
        assert_eq!(ty.get(1), Some(1.0)); // num
        let len = out.df.column("LENGTH").unwrap().f64().unwrap();
        assert_eq!(len.get(0), Some(5.0));
        assert_eq!(len.get(1), Some(8.0));
        let vn = out.df.column("VARNUM").unwrap().f64().unwrap();
        assert_eq!(vn.get(0), Some(1.0));
        assert_eq!(vn.get(1), Some(2.0));
        let label = out.df.column("LABEL").unwrap().str().unwrap();
        assert_eq!(label.get(1), Some("Age of subject"));
        let fmt = out.df.column("FORMAT").unwrap().str().unwrap();
        assert_eq!(fmt.get(1), Some("best12."));

        // NOTE about the OUT= dataset.
        let log = session.log.into_string();
        assert!(
            log.contains("The data set WORK.META has 2 observations and 6 variables."),
            "log: {log}"
        );
    }

    #[test]
    fn execute_short_lists_variable_names_only() {
        let mut session = make_session();
        write_test_dataset(&mut session);
        let ast = ContentsAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "CLASS".into() }),
            varnum: false,
            all: false,
            out: None,
            short: true,
            details: false,
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // Alphabetical name list "age name" (default sort).
        assert!(listing.contains("age name"), "short var list: {listing}");
        // SHORT suppresses the per-variable detail table: the "Num"/"Char" type
        // cells of the detail table must not appear.
        assert!(!listing.contains("Char"), "no detail table under SHORT: {listing}");
        assert!(!listing.contains("Num"), "no detail table under SHORT: {listing}");
    }

    #[test]
    fn execute_details_adds_header_lines() {
        let mut session = make_session();
        write_test_dataset(&mut session);
        let ast = ContentsAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "CLASS".into() }),
            varnum: false,
            all: false,
            out: None,
            short: false,
            details: true,
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("# Observations:"), "details obs line: {listing}");
        assert!(listing.contains("# Variables:"), "details var line: {listing}");
    }
}
