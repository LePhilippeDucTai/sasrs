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
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value};

pub struct PrintAst {
    pub data: Option<DatasetRef>,
    pub vars: Option<Vec<String>>,
    pub noobs: bool,
    /// Option LABEL : utilise le libellé de chaque variable (s'il existe)
    /// comme en-tête de colonne au lieu du nom. Défaut = noms (comme SAS).
    pub label: bool,
}

/// Parse `proc print [data=lib.t] [noobs] [label] ; [var v1 v2... ;] ... run ;`
/// Called AFTER "proc print" has been consumed. Consumes through `run;`.
pub fn parse(ts: &mut StatementStream) -> Result<PrintAst> {
    let mut data: Option<DatasetRef> = None;
    let mut noobs = false;
    let mut label = false;
    let mut vars: Option<Vec<String>> = None;

    // Parse PROC PRINT header options until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next(); // consume `;`
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            ts.next(); // consume "data"
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after DATA",
                    ts.peek().span,
                ));
            }
            ts.next(); // consume `=`
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("noobs") {
            ts.next();
            noobs = true;
        } else if ts.peek().is_kw("label") {
            // LABEL option: utilise les libellés comme en-têtes (M4).
            ts.next();
            label = true;
        } else {
            // Unknown option on PROC PRINT statement — skip to end of statement
            let span = ts.peek().span;
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unexpected option '{bad}' on PROC PRINT statement."),
                span,
            ));
        }
    }

    // Parse sub-statements until `run;` or `quit;` or step boundary
    loop {
        // Skip stray semicolons
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }

        if ts.peek().kind == TokenKind::Eof {
            break;
        }

        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next(); // consume run/quit
            // consume the `;`
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }

        if ts.peek().is_kw("var") {
            ts.next(); // consume "var"
            vars = Some(ts.parse_name_list()?);
            ts.expect_semi()?;
        } else {
            // Unknown sub-statement: skip it
            ts.skip_to_semi();
        }
    }

    Ok(PrintAst { data, vars, noobs, label })
}

/// Execute PROC PRINT. Called by `procs::execute_proc` which wraps with timing.
pub fn execute(ast: &PrintAst, session: &mut Session) -> Result<()> {
    // Resolve dataset reference: data= or _LAST_
    let ds_ref: DatasetRef = match &ast.data {
        Some(r) => r.clone(),
        None => {
            // Use _LAST_
            let last = session.last_dataset.clone().ok_or_else(|| {
                SasError::runtime(
                    "There is no default input data set (_LAST_ is undefined).",
                )
            })?;
            // last_dataset is "LIBREF.NAME"
            let parts: Vec<&str> = last.splitn(2, '.').collect();
            if parts.len() == 2 {
                DatasetRef {
                    libref: Some(parts[0].to_string()),
                    name: parts[1].to_string(),
                }
            } else {
                DatasetRef {
                    libref: None,
                    name: last,
                }
            }
        }
    };

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

    // Build headers and alignments
    let mut headers: Vec<String> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();

    if !ast.noobs {
        headers.push("Obs".to_string());
        aligns.push(Align::Right);
    }

    for &idx in &col_indices {
        // Option LABEL : libellé en en-tête s'il existe, sinon le nom.
        // Sans l'option, toujours le nom (casse d'origine — SAS affiche le
        // nom tel que déclaré).
        let header = match (ast.label, &ds.vars[idx].label) {
            (true, Some(lbl)) if !lbl.is_empty() => lbl.clone(),
            _ => ds.vars[idx].name.clone(),
        };
        headers.push(header);
        aligns.push(match ds.vars[idx].ty {
            crate::value::VarType::Num => Align::Right,
            crate::value::VarType::Char => Align::Left,
        });
    }

    // Décode chaque colonne UNE seule fois (downcast par colonne, jamais
    // par cellule — checklist PLAN.md point 3).
    let mut col_cells: Vec<Vec<String>> = Vec::with_capacity(col_indices.len());
    for &col_i in &col_indices {
        let series = ds.df.get_columns()[col_i].as_materialized_series();
        // M4 : si la variable porte un format VALIDE, on rend chaque valeur
        // via le moteur de formats. SANS format (ou format invalide), on
        // garde EXACTEMENT le chemin historique (stabilité des snapshots).
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
                        Some(spec) => {
                            crate::formats::FormatCatalog::default().format(&v, spec)
                        }
                        None => match v {
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
                        Some(spec) => crate::formats::FormatCatalog::default()
                            .format(&Value::Char(raw.to_string()), spec),
                        None => raw.to_string(),
                    }
                })
                .collect(),
        };
        col_cells.push(cells);
    }

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(n_obs);
    for row_i in 0..n_obs {
        let mut row: Vec<String> = Vec::with_capacity(headers.len());
        if !ast.noobs {
            row.push((row_i + 1).to_string());
        }
        for cells in &col_cells {
            row.push(cells[row_i].clone());
        }
        rows.push(row);
    }

    // Write listing
    session.listing.page_header();
    session.listing.write_table(&headers, &aligns, &rows);

    // Log NOTE — "There were N observations read from the data set WORK.X."
    // PLAN.md checklist item 7: pluriel invariable ("1 observations." — fidèle à SAS)
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_obs, display_name
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
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Body Weight"), "listing: {listing}");
        assert!(listing.contains("Pupil Name"), "listing: {listing}");
    }
}
