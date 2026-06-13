//! PROC EXPORT (jalon M14.3).
//!
//! Écrit un dataset SAS vers un fichier externe (CSV, TSV, DLM, XLSX).
//!
//! ## Syntaxe
//! ```sas
//! proc export data=lib.table outfile='path' dbms=csv [replace];
//!     delimiter=',';  /* pour dbms=dlm */
//!     sheet='Sheet1'; /* pour dbms=xlsx */
//! run;
//! ```
//!
//! ## DBMS supportés dans ce build
//! - `CSV`  : CsvWriter Polars (séparateur `,`).
//! - `TAB`  : CsvWriter Polars (séparateur `\t`).
//! - `DLM`  : CsvWriter Polars (séparateur configurable via `DELIMITER=`).
//! - `XLSX` : non disponible dans ce build (dépendance `rust_xlsxwriter` absente) →
//!            `SasError` "PROC EXPORT with DBMS=XLSX is not yet implemented in this build."
//!
//! ## Notes d'implémentation
//! - Les valeurs numériques sont exportées telles quelles (f64 ou null → vide).
//!   Les colonnes numériques portant un format DATE/DATETIME ne sont PAS
//!   reconverties en dates lisibles dans cette v1 : la valeur numérique SAS
//!   (jours depuis 1960) est exportée telle quelle. Ce choix est documenté.
//! - `REPLACE` : écraser le fichier s'il existe déjà.
//! - Chemin résolu vs `session.base_dir`.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;
use polars::prelude::*;
use std::fs::File;
use std::path::PathBuf;

// ──────────────────────────────────────────────────────────────────────────────
// AST
// ──────────────────────────────────────────────────────────────────────────────

/// DBMS = format du fichier de sortie.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportDbms {
    Csv,
    Tab,
    Dlm,
    Xlsx,
    Other(String),
}

impl ExportDbms {
    fn from_str(s: &str) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "CSV" => ExportDbms::Csv,
            "TAB" => ExportDbms::Tab,
            "DLM" | "DLMSTR" => ExportDbms::Dlm,
            "XLSX" | "EXCEL" | "XLS" | "XLSM" => ExportDbms::Xlsx,
            other => ExportDbms::Other(other.to_string()),
        }
    }
}

pub struct ExportAst {
    /// Dataset SAS source (`DATA=`).
    pub data: Option<DatasetRef>,
    /// Chemin du fichier de sortie (`OUTFILE=`).
    pub outfile: String,
    /// Format du fichier.
    pub dbms: ExportDbms,
    /// `REPLACE` : écraser le fichier s'il existe.
    pub replace: bool,
    /// `DELIMITER=` / `DLM=` pour DBMS=DLM.
    pub delimiter: Option<String>,
    /// `SHEET=` pour DBMS=XLSX.
    pub sheet: Option<String>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Parsing
// ──────────────────────────────────────────────────────────────────────────────

/// Parse `proc export … ; [sub-statements ;] run ;`
/// Appelé APRÈS que `proc export` a été consommé.
pub fn parse(ts: &mut StatementStream) -> Result<ExportAst> {
    let mut data: Option<DatasetRef> = None;
    let mut outfile: Option<String> = None;
    let mut dbms: Option<ExportDbms> = None;
    let mut replace = false;

    // Options sur le statement PROC EXPORT (jusqu'au `;`)
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        let tok = ts.peek().clone();
        if let Some(kw) = tok.ident() {
            match kw.to_ascii_lowercase().as_str() {
                "data" => {
                    ts.next();
                    expect_eq(ts, "DATA")?;
                    data = Some(ts.parse_dataset_ref()?);
                }
                "outfile" | "file" => {
                    ts.next();
                    expect_eq(ts, "OUTFILE")?;
                    outfile = Some(parse_string_value(ts, "OUTFILE")?);
                }
                "dbms" => {
                    ts.next();
                    expect_eq(ts, "DBMS")?;
                    let val = parse_string_value(ts, "DBMS")?;
                    dbms = Some(ExportDbms::from_str(&val));
                }
                "replace" => {
                    ts.next();
                    replace = true;
                }
                other => {
                    let bad = other.to_uppercase();
                    let span = tok.span;
                    ts.skip_to_semi();
                    return Err(SasError::parse(
                        format!("Unexpected option '{bad}' on PROC EXPORT statement."),
                        span,
                    ));
                }
            }
        } else {
            let span = tok.span;
            ts.skip_to_semi();
            return Err(SasError::parse(
                "Unexpected token on PROC EXPORT statement.",
                span,
            ));
        }
    }

    // Valider les options obligatoires
    let outfile = outfile.ok_or_else(|| {
        SasError::runtime("The OUTFILE= option is required on PROC EXPORT.")
    })?;
    let dbms = dbms.ok_or_else(|| {
        SasError::runtime("The DBMS= option is required on PROC EXPORT.")
    })?;

    // Sub-statements (DELIMITER=, SHEET=, …)
    let mut delimiter: Option<String> = None;
    let mut sheet: Option<String> = None;

    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }
        let tok = ts.peek().clone();
        let Some(kw) = tok.ident() else {
            ts.skip_to_semi();
            continue;
        };
        match kw.to_ascii_lowercase().as_str() {
            "delimiter" | "dlm" => {
                ts.next();
                expect_eq(ts, "DELIMITER")?;
                delimiter = Some(parse_string_value(ts, "DELIMITER")?);
                ts.skip_to_semi();
            }
            "sheet" => {
                ts.next();
                expect_eq(ts, "SHEET")?;
                sheet = Some(parse_string_value(ts, "SHEET")?);
                ts.skip_to_semi();
            }
            _ => {
                ts.skip_to_semi();
            }
        }
    }

    Ok(ExportAst {
        data,
        outfile,
        dbms,
        replace,
        delimiter,
        sheet,
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// Exécution
// ──────────────────────────────────────────────────────────────────────────────

pub fn execute(ast: &ExportAst, session: &mut Session) -> Result<()> {
    // Résoudre le dataset (data= ou _LAST_)
    let ds_ref = match &ast.data {
        Some(r) => r.clone(),
        None => {
            let last = session.last_dataset.as_deref().ok_or_else(|| {
                SasError::runtime(
                    "No data set has been created yet. The DATA= option is required.",
                )
            })?;
            // Décomposer "LIB.TABLE"
            let mut parts = last.splitn(2, '.');
            let lib = parts.next().unwrap_or("WORK").to_string();
            let tbl = parts.next().unwrap_or(last).to_string();
            DatasetRef { libref: Some(lib), name: tbl }
        }
    };

    let libref = ds_ref.libref_or_work();
    let table = ds_ref.name.to_uppercase();
    let disp = format!("{libref}.{table}");

    // Lire le dataset
    let provider = session.libs.get(&libref)?;
    let (ds, notes) = provider.read(&table)?;
    for note in &notes {
        session.log.forward(note);
    }

    // Résoudre le chemin de sortie
    let path = resolve_path(&ast.outfile, &session.base_dir);

    // Vérifier REPLACE
    if path.exists() && !ast.replace {
        return Err(SasError::runtime(format!(
            "The file '{}' already exists. \
             Use the REPLACE option if you want to replace it.",
            path.display()
        )));
    }

    // Dispatcher selon DBMS
    match &ast.dbms {
        ExportDbms::Csv => write_csv(&ds.df.clone(), &path, b',')?,
        ExportDbms::Tab => write_csv(&ds.df.clone(), &path, b'\t')?,
        ExportDbms::Dlm => {
            let sep = delimiter_byte(ast.delimiter.as_deref())?;
            write_csv(&ds.df.clone(), &path, sep)?;
        }
        ExportDbms::Xlsx => {
            return Err(SasError::runtime(
                "PROC EXPORT with DBMS=XLSX is not yet implemented in this build.",
            ));
        }
        ExportDbms::Other(name) => {
            return Err(SasError::runtime(format!(
                "PROC EXPORT with DBMS={name} is not yet implemented in this build.",
            )));
        }
    }

    let n_obs = ds.n_obs();
    let n_vars = ds.n_vars();
    session.log.note(&format!(
        "There were {n_obs} observations read from the data set {disp}."
    ));
    session.log.note(&format!(
        "The file '{}' has been created with {n_obs} observations and {n_vars} variables.",
        path.display()
    ));

    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers d'écriture
// ──────────────────────────────────────────────────────────────────────────────

fn write_csv(df: &DataFrame, path: &PathBuf, separator: u8) -> Result<()> {
    let mut df = df.clone();
    let file = File::create(path)?;
    CsvWriter::new(file)
        .with_separator(separator)
        .include_header(true)
        .finish(&mut df)?;
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Utilitaires
// ──────────────────────────────────────────────────────────────────────────────

fn resolve_path(file: &str, base_dir: &std::path::Path) -> PathBuf {
    let p = PathBuf::from(file);
    if p.is_absolute() {
        p
    } else {
        base_dir.join(p)
    }
}

fn delimiter_byte(dlm: Option<&str>) -> Result<u8> {
    match dlm {
        None | Some("") | Some(" ") => Ok(b' '),
        Some(s) => {
            let trimmed = s.trim();
            match trimmed {
                "," => Ok(b','),
                "|" => Ok(b'|'),
                ";" => Ok(b';'),
                "\t" => Ok(b'\t'),
                c if c.len() == 1 => Ok(c.as_bytes()[0]),
                _ => Err(SasError::runtime(format!(
                    "DELIMITER= value '{s}' is not a single-byte ASCII character."
                ))),
            }
        }
    }
}

fn parse_string_value(ts: &mut StatementStream, opt: &str) -> Result<String> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Str { value, .. } => {
            let v = value.clone();
            ts.next();
            Ok(v)
        }
        TokenKind::Ident(s) => {
            let v = s.clone();
            ts.next();
            Ok(v)
        }
        _ => Err(SasError::parse(
            format!("expected a string or identifier after {opt}="),
            tok.span,
        )),
    }
}

fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("expected '=' after {opt}"),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::df;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn make_session_with_base(base: PathBuf) -> Session {
        Session::new(None, base, true).unwrap()
    }

    fn parse_export(src: &str) -> Result<ExportAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "export"
        parse(&mut ts)
    }

    fn write_test_dataset(session: &mut Session) {
        let df = df![
            "name" => ["Alice", "Bob"],
            "age"  => [30.0_f64, 25.0],
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![
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
            ],
        };
        session.libs.get("WORK").unwrap().write("PEOPLE", &ds).unwrap();
        session.last_dataset = Some("WORK.PEOPLE".to_string());
    }

    // ── Parse tests ──────────────────────────────────────────────────────────

    #[test]
    fn parse_basic_export() {
        let ast = parse_export(
            "proc export data=work.t outfile='out.csv' dbms=csv replace; run;",
        )
        .unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "t");
        assert_eq!(ast.outfile, "out.csv");
        assert_eq!(ast.dbms, ExportDbms::Csv);
        assert!(ast.replace);
    }

    #[test]
    fn parse_no_data_option() {
        // data= is optional (uses _LAST_)
        let ast = parse_export(
            "proc export outfile='out.csv' dbms=csv; run;",
        )
        .unwrap();
        assert!(ast.data.is_none());
    }

    #[test]
    fn parse_tab_dbms() {
        let ast = parse_export(
            "proc export data=work.t outfile='out.tsv' dbms=tab; run;",
        )
        .unwrap();
        assert_eq!(ast.dbms, ExportDbms::Tab);
    }

    #[test]
    fn parse_dlm_with_delimiter() {
        let ast = parse_export(
            "proc export data=work.t outfile='out.txt' dbms=dlm; delimiter='|'; run;",
        )
        .unwrap();
        assert_eq!(ast.dbms, ExportDbms::Dlm);
        assert_eq!(ast.delimiter.as_deref(), Some("|"));
    }

    #[test]
    fn parse_xlsx() {
        let ast = parse_export(
            "proc export data=work.t outfile='out.xlsx' dbms=xlsx; sheet='Data'; run;",
        )
        .unwrap();
        assert_eq!(ast.dbms, ExportDbms::Xlsx);
        assert_eq!(ast.sheet.as_deref(), Some("Data"));
    }

    #[test]
    fn parse_missing_outfile_errors() {
        let result = parse_export("proc export data=work.t dbms=csv; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("OUTFILE="), "msg: {msg}");
    }

    #[test]
    fn parse_missing_dbms_errors() {
        let result = parse_export("proc export data=work.t outfile='f.csv'; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("DBMS="), "msg: {msg}");
    }

    // ── Execute tests ─────────────────────────────────────────────────────────

    #[test]
    fn execute_csv_creates_file() {
        let dir = tempdir().unwrap();
        let out_path = dir.path().join("output.csv");

        let mut session = make_session_with_base(dir.path().to_path_buf());
        write_test_dataset(&mut session);

        let ast = ExportAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "PEOPLE".into() }),
            outfile: out_path.to_str().unwrap().to_string(),
            dbms: ExportDbms::Csv,
            replace: false,
            delimiter: None,
            sheet: None,
        };
        execute(&ast, &mut session).unwrap();

        assert!(out_path.exists(), "output CSV should have been created");
        let content = std::fs::read_to_string(&out_path).unwrap();
        assert!(content.contains("name"), "csv should have header: {content}");
        assert!(content.contains("Alice"), "csv should have data: {content}");
        assert!(content.contains("Bob"), "csv should have data: {content}");
    }

    #[test]
    fn execute_replace_false_errors_if_file_exists() {
        let dir = tempdir().unwrap();
        let out_path = dir.path().join("output.csv");
        std::fs::write(&out_path, "existing content").unwrap();

        let mut session = make_session_with_base(dir.path().to_path_buf());
        write_test_dataset(&mut session);

        let ast = ExportAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "PEOPLE".into() }),
            outfile: out_path.to_str().unwrap().to_string(),
            dbms: ExportDbms::Csv,
            replace: false,
            delimiter: None,
            sheet: None,
        };
        let result = execute(&ast, &mut session);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("already exists"), "msg: {msg}");
    }

    #[test]
    fn execute_replace_true_overwrites_file() {
        let dir = tempdir().unwrap();
        let out_path = dir.path().join("output.csv");
        std::fs::write(&out_path, "old content").unwrap();

        let mut session = make_session_with_base(dir.path().to_path_buf());
        write_test_dataset(&mut session);

        let ast = ExportAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "PEOPLE".into() }),
            outfile: out_path.to_str().unwrap().to_string(),
            dbms: ExportDbms::Csv,
            replace: true,
            delimiter: None,
            sheet: None,
        };
        execute(&ast, &mut session).unwrap();

        let content = std::fs::read_to_string(&out_path).unwrap();
        assert!(content.contains("name"), "csv should have header: {content}");
    }

    #[test]
    fn execute_xlsx_errors_not_implemented() {
        let dir = tempdir().unwrap();
        let out_path = dir.path().join("output.xlsx");

        let mut session = make_session_with_base(dir.path().to_path_buf());
        write_test_dataset(&mut session);

        let ast = ExportAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "PEOPLE".into() }),
            outfile: out_path.to_str().unwrap().to_string(),
            dbms: ExportDbms::Xlsx,
            replace: false,
            delimiter: None,
            sheet: None,
        };
        let result = execute(&ast, &mut session);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("not yet implemented"), "msg: {msg}");
        assert!(msg.contains("XLSX"), "msg: {msg}");
    }

    #[test]
    fn execute_uses_last_dataset_when_data_absent() {
        let dir = tempdir().unwrap();
        let out_path = dir.path().join("output.csv");

        let mut session = make_session_with_base(dir.path().to_path_buf());
        write_test_dataset(&mut session); // sets last_dataset = WORK.PEOPLE

        let ast = ExportAst {
            data: None, // uses _LAST_
            outfile: out_path.to_str().unwrap().to_string(),
            dbms: ExportDbms::Csv,
            replace: false,
            delimiter: None,
            sheet: None,
        };
        execute(&ast, &mut session).unwrap();

        let content = std::fs::read_to_string(&out_path).unwrap();
        assert!(content.contains("Alice"), "should have used WORK.PEOPLE: {content}");
    }
}
