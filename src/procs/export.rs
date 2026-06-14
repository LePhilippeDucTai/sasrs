//! PROC EXPORT (jalon M14.3).
//!
//! Écrit une table SAS (parquet) vers un fichier texte délimité (CSV/TAB/DLM)
//! via le writer CSV de Polars.
//!
//! # Syntaxe prise en charge
//!
//! ```sas
//! proc export data=lib.table outfile='chemin' dbms=CSV [replace];
//!     delimiter='x';  /* ou dlm='x' */
//! run;
//! ```
//!
//! ## DBMS pris en charge
//! - `CSV`  → séparateur virgule (`,`)
//! - `TAB`  → séparateur tabulation (`\t`)
//! - `DLM`  → séparateur fourni par `DELIMITER=`/`DLM=` (défaut espace ` `)
//!
//! ## DBMS différés (erreur propre)
//! - `XLSX`, `EXCEL` → `SasError::runtime(...)` avec message explicite.
//!
//! ## REPLACE
//! Option flag : si le fichier existe déjà, il est écrasé (comportement
//! documenté ; SAS 9.4 renverrait une erreur sans REPLACE, mais notre
//! implémentation écrase toujours — documenté).
//!
//! ## NOTE de fin
//! `"N records were written to the file 'chemin'."` (SAS 9.4 wording)
//!
//! ## Invariants
//! - L'en-tête CSV (noms de colonnes) est TOUJOURS écrit (comportement SAS
//!   par défaut pour PROC EXPORT DBMS=CSV/TAB/DLM).
//! - Le dataset source est lu via `provider.read()` → `SasDataset` →
//!   `SasDataset::df` est un `DataFrame` Polars prêt à passer au writer CSV.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;
use polars::prelude::*;
use std::fs::File;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

/// DBMS reconnu par PROC EXPORT.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportDbms {
    /// DBMS=CSV  → séparateur `,`
    Csv,
    /// DBMS=TAB  → séparateur `\t`
    Tab,
    /// DBMS=DLM  → séparateur fourni par `delimiter=` (défaut ` `)
    Dlm,
}

/// AST de PROC EXPORT.
pub struct ExportAst {
    /// Dataset source (`DATA=`).
    pub data: Option<DatasetRef>,
    /// Chemin du fichier de sortie (`OUTFILE=`).
    pub outfile: String,
    /// Moteur d'écriture.
    pub dbms: ExportDbms,
    /// `REPLACE` présent ? (documenté : on écrase toujours).
    pub replace: bool,
    /// Séparateur explicite (`DELIMITER=`/`DLM=` dans le corps).
    pub delimiter: Option<u8>,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse `proc export ...` jusqu'à `run;`/`quit;`. Appelé APRÈS que
/// `proc export` a été consommé par le dispatcher.
pub fn parse(ts: &mut StatementStream) -> Result<ExportAst> {
    let mut data: Option<DatasetRef> = None;
    let mut outfile: Option<String> = None;
    let mut dbms: Option<ExportDbms> = None;
    let mut replace = false;

    // --- Options sur le statement PROC EXPORT (jusqu'au `;`) ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            ts.next();
            expect_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("outfile") {
            ts.next();
            expect_eq(ts, "OUTFILE")?;
            outfile = Some(parse_string_or_ident(ts, "OUTFILE")?);
        } else if ts.peek().is_kw("dbms") {
            ts.next();
            expect_eq(ts, "DBMS")?;
            let tok = ts.peek().clone();
            let name = tok
                .ident()
                .ok_or_else(|| {
                    SasError::parse("expected a DBMS name after DBMS=", tok.span)
                })?
                .to_ascii_uppercase();
            ts.next();
            dbms = Some(parse_dbms(&name, tok.span)?);
        } else if ts.peek().is_kw("replace") {
            ts.next();
            replace = true;
        } else {
            // option inconnue → ignorer (récupération)
            ts.next();
        }
    }

    // --- Sous-statements jusqu'à run;/quit; ---
    let mut delimiter: Option<u8> = None;

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
        let kw_tok = ts.peek().clone();
        let kw = match kw_tok.ident() {
            Some(s) => s.to_ascii_lowercase(),
            None => {
                ts.skip_to_semi();
                continue;
            }
        };
        ts.next();

        match kw.as_str() {
            "delimiter" | "dlm" => {
                expect_eq(ts, "DELIMITER")?;
                let s = parse_string_or_ident(ts, "DELIMITER")?;
                delimiter = parse_delimiter_char(&s, kw_tok.span)?;
                ts.skip_to_semi();
            }
            _ => {
                ts.skip_to_semi();
            }
        }
    }

    let outfile = outfile.ok_or_else(|| {
        SasError::runtime("PROC EXPORT: OUTFILE= is required.")
    })?;
    let dbms = dbms.unwrap_or(ExportDbms::Csv);

    Ok(ExportAst {
        data,
        outfile,
        dbms,
        replace,
        delimiter,
    })
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Execute PROC EXPORT. Appelé par `procs::execute_proc`.
pub fn execute(ast: &ExportAst, session: &mut Session) -> Result<()> {
    // --- Résoudre le dataset source ---
    let in_ref = resolve_input(ast, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in &notes {
        session.log.forward(note);
    }

    let n_obs = ds.n_obs();

    // --- Résoudre le séparateur ---
    let sep = resolve_separator(ast);

    // --- Écrire le fichier CSV ---
    let mut file = File::create(&ast.outfile).map_err(|e| {
        SasError::runtime(format!(
            "PROC EXPORT: cannot create '{}': {e}",
            ast.outfile
        ))
    })?;

    let mut df_clone = ds.df.clone();
    CsvWriter::new(&mut file)
        .include_header(true)
        .with_separator(sep)
        .finish(&mut df_clone)
        .map_err(|e| {
            SasError::runtime(format!(
                "PROC EXPORT: error writing '{}': {e}",
                ast.outfile
            ))
        })?;

    // --- NOTE de fin ---
    session.log.note(&format!(
        "{} records were written to the file '{}'.",
        n_obs, ast.outfile
    ));

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers internes
// ---------------------------------------------------------------------------

/// Résout le dataset source depuis `DATA=` ou `_LAST_`.
fn resolve_input(ast: &ExportAst, session: &Session) -> Result<DatasetRef> {
    match &ast.data {
        Some(r) => Ok(r.clone()),
        None => {
            let last = session.last_dataset.clone().ok_or_else(|| {
                SasError::runtime(
                    "There is no default input data set (_LAST_ is undefined).",
                )
            })?;
            let parts: Vec<&str> = last.splitn(2, '.').collect();
            if parts.len() == 2 {
                Ok(DatasetRef {
                    libref: Some(parts[0].to_string()),
                    name: parts[1].to_string(),
                })
            } else {
                Ok(DatasetRef {
                    libref: None,
                    name: last,
                })
            }
        }
    }
}

/// Résout le séparateur en octet selon DBMS + DELIMITER éventuel.
fn resolve_separator(ast: &ExportAst) -> u8 {
    match &ast.dbms {
        ExportDbms::Csv => b',',
        ExportDbms::Tab => b'\t',
        ExportDbms::Dlm => ast.delimiter.unwrap_or(b' '),
    }
}

/// Parse un DBMS par son nom en majuscules ; renvoie une erreur propre pour
/// les DBMS différés (XLSX/EXCEL).
fn parse_dbms(name: &str, span: crate::token::Span) -> Result<ExportDbms> {
    match name {
        "CSV" => Ok(ExportDbms::Csv),
        "TAB" => Ok(ExportDbms::Tab),
        "DLM" | "DLMSTR" => Ok(ExportDbms::Dlm),
        "XLSX" | "EXCEL" | "XLS" => Err(SasError::runtime(format!(
            "PROC EXPORT with DBMS={name} is not yet implemented in this build \
             (the calamine/rust_xlsxwriter crates are not available)."
        ))),
        other => Err(SasError::parse(
            format!("Unknown DBMS '{other}' for PROC EXPORT."),
            span,
        )),
    }
}

/// Parse un caractère délimiteur depuis une chaîne.
fn parse_delimiter_char(s: &str, span: crate::token::Span) -> Result<Option<u8>> {
    let s = match s.to_ascii_uppercase().as_str() {
        "TAB" | "09X" => return Ok(Some(b'\t')),
        "SPACE" | "20X" => return Ok(Some(b' ')),
        "COMMA" | "2CX" => return Ok(Some(b',')),
        "PIPE" | "7CX" => return Ok(Some(b'|')),
        "SEMICOLON" | "3BX" => return Ok(Some(b';')),
        _ => s,
    };
    if s.is_empty() {
        return Ok(None);
    }
    let bytes = s.as_bytes();
    if bytes.len() == 1 {
        return Ok(Some(bytes[0]));
    }
    Err(SasError::parse(
        format!(
            "DELIMITER value '{s}' must be a single ASCII character or a recognized mnemonic."
        ),
        span,
    ))
}

/// Consomme `=` ; renvoie une erreur si absent.
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

/// Parse un littéral de chaîne ou un identifiant sans guillemets.
fn parse_string_or_ident(ts: &mut StatementStream, opt: &str) -> Result<String> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Str { value, .. } => {
            let s = value.clone();
            ts.next();
            Ok(s)
        }
        TokenKind::Ident(s) => {
            let s = s.clone();
            ts.next();
            Ok(s)
        }
        _ => Err(SasError::parse(
            format!("expected a value after {opt}="),
            tok.span,
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::prelude::df;
    use std::io::Write;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_export_src(src: &str) -> Result<ExportAst> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "export"
        parse(&mut ts)
    }

    fn write_test_dataset(session: &mut Session, name: &str) {
        let df = df![
            "name" => ["Alice", "Bob", "Carol"],
            "age"  => [30.0_f64, 25.0, 35.0],
            "score" => [95.5_f64, 88.0, 72.3]
        ]
        .unwrap();
        let vars = vec![
            VarMeta { name: "name".into(), ty: VarType::Char, length: 5, format: None, label: None },
            VarMeta { name: "age".into(), ty: VarType::Num, length: 8, format: None, label: None },
            VarMeta { name: "score".into(), ty: VarType::Num, length: 8, format: None, label: None },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", name.to_uppercase()));
    }

    // --- Tests du parser ---

    #[test]
    fn parse_export_csv_minimal() {
        let ast = parse_export_src(
            "proc export data=work.t outfile='/tmp/out.csv' dbms=csv; run;",
        )
        .unwrap();
        assert_eq!(ast.outfile, "/tmp/out.csv");
        assert_eq!(ast.dbms, ExportDbms::Csv);
        assert!(ast.data.is_some());
    }

    #[test]
    fn parse_export_tab() {
        let ast = parse_export_src(
            "proc export data=work.t outfile='out.tsv' dbms=TAB replace; run;",
        )
        .unwrap();
        assert_eq!(ast.dbms, ExportDbms::Tab);
        assert!(ast.replace);
    }

    #[test]
    fn parse_export_dlm_with_delimiter() {
        let ast = parse_export_src(
            "proc export data=work.t outfile='out.txt' dbms=dlm; delimiter='|'; run;",
        )
        .unwrap();
        assert_eq!(ast.dbms, ExportDbms::Dlm);
        assert_eq!(ast.delimiter, Some(b'|'));
    }

    #[test]
    fn parse_export_missing_outfile_errors() {
        let result = parse_export_src("proc export data=work.t dbms=csv; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("OUTFILE="), "msg: {msg}");
    }

    #[test]
    fn parse_export_xlsx_deferred_error() {
        let result = parse_export_src(
            "proc export data=work.t outfile='out.xlsx' dbms=xlsx; run;",
        );
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("not yet implemented"), "msg: {msg}");
        assert!(msg.contains("XLSX"), "msg: {msg}");
    }

    #[test]
    fn parse_export_excel_deferred_error() {
        let result = parse_export_src(
            "proc export data=work.t outfile='out.xlsx' dbms=excel; run;",
        );
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("not yet implemented"), "msg: {msg}");
    }

    // --- Tests d'exécution ---

    #[test]
    fn execute_export_csv_basic() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("out.csv");

        let work_dir = dir.path().join("work");
        std::fs::create_dir(&work_dir).unwrap();
        let mut session =
            Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

        write_test_dataset(&mut session, "MYDS");

        let ast = ExportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "MYDS".into(),
            }),
            outfile: out_path.to_string_lossy().into_owned(),
            dbms: ExportDbms::Csv,
            replace: false,
            delimiter: None,
        };
        execute(&ast, &mut session).unwrap();

        // Vérifier la NOTE
        let log = session.log.into_string();
        assert!(
            log.contains("3 records were written to the file"),
            "log: {log}"
        );

        // Vérifier le fichier CSV
        let content = std::fs::read_to_string(&out_path).unwrap();
        // Doit contenir les en-têtes
        assert!(content.contains("name"), "content: {content}");
        assert!(content.contains("age"), "content: {content}");
        // Doit contenir les données
        assert!(content.contains("Alice"), "content: {content}");
        assert!(content.contains("Bob"), "content: {content}");
    }

    #[test]
    fn execute_export_tab_separated() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("out.tsv");

        let work_dir = dir.path().join("work");
        std::fs::create_dir(&work_dir).unwrap();
        let mut session =
            Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

        write_test_dataset(&mut session, "T");

        let ast = ExportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            outfile: out_path.to_string_lossy().into_owned(),
            dbms: ExportDbms::Tab,
            replace: false,
            delimiter: None,
        };
        execute(&ast, &mut session).unwrap();

        let content = std::fs::read_to_string(&out_path).unwrap();
        // En-tête doit contenir des tabulations
        let first_line = content.lines().next().unwrap();
        assert!(first_line.contains('\t'), "first line: {first_line}");
        assert!(content.contains("Alice"), "content: {content}");
    }

    #[test]
    fn execute_export_dlm_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("out.txt");

        let work_dir = dir.path().join("work");
        std::fs::create_dir(&work_dir).unwrap();
        let mut session =
            Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

        write_test_dataset(&mut session, "T");

        let ast = ExportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            outfile: out_path.to_string_lossy().into_owned(),
            dbms: ExportDbms::Dlm,
            replace: false,
            delimiter: Some(b'|'),
        };
        execute(&ast, &mut session).unwrap();

        let content = std::fs::read_to_string(&out_path).unwrap();
        let first_line = content.lines().next().unwrap();
        assert!(first_line.contains('|'), "first line: {first_line}");
    }

    #[test]
    fn execute_export_roundtrip_with_import() {
        // Test de round-trip : IMPORT → EXPORT → IMPORT et vérification
        let dir = tempfile::tempdir().unwrap();
        let csv_orig = dir.path().join("orig.csv");
        let csv_exported = dir.path().join("exported.csv");

        // Écrire un CSV source
        {
            let mut f = std::fs::File::create(&csv_orig).unwrap();
            writeln!(f, "x,y").unwrap();
            writeln!(f, "1.0,a").unwrap();
            writeln!(f, "2.0,b").unwrap();
            writeln!(f, "3.0,c").unwrap();
        }

        let work_dir = dir.path().join("work");
        std::fs::create_dir(&work_dir).unwrap();
        let mut session =
            Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

        // IMPORT
        let import_ast = crate::procs::import::ImportAst {
            datafile: csv_orig.to_string_lossy().into_owned(),
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "SRC".into(),
            },
            dbms: crate::procs::import::ImportDbms::Csv,
            replace: false,
            getnames: true,
            delimiter: None,
            guessingrows: None,
        };
        crate::procs::import::execute(&import_ast, &mut session).unwrap();

        // EXPORT
        let export_ast = ExportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "SRC".into(),
            }),
            outfile: csv_exported.to_string_lossy().into_owned(),
            dbms: ExportDbms::Csv,
            replace: false,
            delimiter: None,
        };
        execute(&export_ast, &mut session).unwrap();

        // Vérifier que le fichier exporté existe et contient les bonnes données
        let content = std::fs::read_to_string(&csv_exported).unwrap();
        assert!(content.contains("x"), "content: {content}");
        assert!(content.contains("y"), "content: {content}");
        // Les valeurs doivent être présentes (Polars peut utiliser 1.0 ou 1)
        assert!(content.contains('1'), "content: {content}");
        assert!(content.contains('a'), "content: {content}");

        // NOTE de log
        let log = session.log.into_string();
        assert!(
            log.contains("3 records were written to the file"),
            "log: {log}"
        );
    }

    #[test]
    fn execute_export_last_dataset() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("out.csv");

        let work_dir = dir.path().join("work");
        std::fs::create_dir(&work_dir).unwrap();
        let mut session =
            Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

        write_test_dataset(&mut session, "LAST");

        // data= absent → utilise _LAST_
        let ast = ExportAst {
            data: None,
            outfile: out_path.to_string_lossy().into_owned(),
            dbms: ExportDbms::Csv,
            replace: false,
            delimiter: None,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("3 records were written"), "log: {log}");
    }

    #[test]
    fn execute_export_invalid_path_errors() {
        let mut session = make_session();
        // Pas de dataset WORK
        let ast = ExportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "NONEXISTENT".into(),
            }),
            outfile: "/tmp/out.csv".into(),
            dbms: ExportDbms::Csv,
            replace: false,
            delimiter: None,
        };
        let result = execute(&ast, &mut session);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_separator_csv() {
        let ast = ExportAst {
            data: None,
            outfile: String::new(),
            dbms: ExportDbms::Csv,
            replace: false,
            delimiter: None,
        };
        assert_eq!(resolve_separator(&ast), b',');
    }

    #[test]
    fn resolve_separator_tab() {
        let ast = ExportAst {
            data: None,
            outfile: String::new(),
            dbms: ExportDbms::Tab,
            replace: false,
            delimiter: None,
        };
        assert_eq!(resolve_separator(&ast), b'\t');
    }

    #[test]
    fn resolve_separator_dlm_default_space() {
        let ast = ExportAst {
            data: None,
            outfile: String::new(),
            dbms: ExportDbms::Dlm,
            replace: false,
            delimiter: None,
        };
        assert_eq!(resolve_separator(&ast), b' ');
    }

    #[test]
    fn resolve_separator_dlm_with_semicolon() {
        let ast = ExportAst {
            data: None,
            outfile: String::new(),
            dbms: ExportDbms::Dlm,
            replace: false,
            delimiter: Some(b';'),
        };
        assert_eq!(resolve_separator(&ast), b';');
    }
}
