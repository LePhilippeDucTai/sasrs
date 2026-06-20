//! PROC IMPORT (jalon M14.3).
//!
//! Lit un fichier texte délimité (CSV/TAB/DLM) via le lecteur CSV de Polars
//! et l'importe dans une table SAS (parquet) via `SasDataset::from_dataframe`.
//!
//! # Syntaxe prise en charge
//!
//! ```sas
//! proc import datafile='chemin' out=lib.table dbms=CSV [replace];
//!     getnames=yes|no;
//!     delimiter='x';   /* ou dlm='x' */
//!     guessingrows=n;  /* ignoré — Polars infère toujours sur 100 lignes */
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
//! ## GETNAMES
//! - `YES` (défaut) : la première ligne donne les noms de colonnes.
//! - `NO` : noms automatiques `VAR1`, `VAR2`, … (style SAS ; Polars produit
//!   `column_1`… qui est renommé ici).
//!
//! ## REPLACE
//! Option flag : documenté mais non appliqué — on écrase toujours (comportement
//! documenté ; SAS 9.4 renvoie une erreur sans REPLACE si la table existe).
//!
//! ## Invariants
//! - `SasDataset::from_dataframe` est appelé systématiquement → coercition
//!   de types, i64 > 2^53 WARNING, dates → f64+format.
//! - NOTE de fin : "The data set LIB.TABLE has N observations and M variables."
//!   (pluriel invariable — fidèle à SAS, cf. PLAN.md §Checklist piège 7).

use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::procs::common;
use crate::session::Session;
use crate::token::TokenKind;
use polars::prelude::*;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

/// DBMS (système de fichier source) reconnu par PROC IMPORT.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportDbms {
    /// DBMS=CSV  → séparateur `,`
    Csv,
    /// DBMS=TAB  → séparateur `\t`
    Tab,
    /// DBMS=DLM  → séparateur fourni par `delimiter=` (défaut ` `)
    Dlm,
}

/// AST de PROC IMPORT.
pub struct ImportAst {
    /// Chemin du fichier source (`DATAFILE=`).
    pub datafile: String,
    /// Dataset de sortie (`OUT=`).
    pub out: DatasetRef,
    /// Moteur de lecture.
    pub dbms: ImportDbms,
    /// `REPLACE` présent ? (documenté : on écrase toujours).
    pub replace: bool,
    /// `GETNAMES=YES` (défaut) : la 1re ligne donne les noms.
    pub getnames: bool,
    /// Séparateur explicite (DELIMITER=/DLM= dans le corps).
    pub delimiter: Option<u8>,
    /// `GUESSINGROWS=` (ignoré ; Polars infère sur ses propres heuristiques).
    pub guessingrows: Option<usize>,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse `proc import ...` jusqu'à `run;`/`quit;`. Appelé APRÈS que
/// `proc import` a été consommé par le dispatcher.
pub fn parse(ts: &mut StatementStream) -> Result<ImportAst> {
    let mut datafile: Option<String> = None;
    let mut out: Option<DatasetRef> = None;
    let mut dbms: Option<ImportDbms> = None;
    let mut replace = false;

    // --- Options sur le statement PROC IMPORT (jusqu'au `;`) ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("datafile") || ts.peek().is_kw("filename") {
            common::expect_eq(ts, "DATAFILE")?;
            datafile = Some(parse_string_or_ident(ts, "DATAFILE")?);
        } else if ts.peek().is_kw("out") {
            common::expect_eq(ts, "OUT")?;
            out = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("dbms") {
            common::expect_eq(ts, "DBMS")?;
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
    let mut getnames = true;
    let mut delimiter: Option<u8> = None;
    let mut guessingrows: Option<usize> = None;

    loop {
        // Sauter les `;` isolés
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
        // Détecter `name = value ;`
        let kw_tok = ts.peek().clone();
        let kw = match kw_tok.ident() {
            Some(s) => s.to_ascii_lowercase(),
            None => {
                ts.skip_to_semi();
                continue;
            }
        };
        ts.next(); // consommer le nom du sous-statement

        match kw.as_str() {
            "getnames" => {
                expect_eq(ts, "GETNAMES")?;
                let val_tok = ts.peek().clone();
                let val = val_tok
                    .ident()
                    .ok_or_else(|| {
                        SasError::parse("expected YES or NO after GETNAMES=", val_tok.span)
                    })?
                    .to_ascii_uppercase();
                ts.next();
                getnames = val != "NO";
                ts.skip_to_semi();
            }
            "delimiter" | "dlm" => {
                expect_eq(ts, "DELIMITER")?;
                let s = parse_string_or_ident(ts, "DELIMITER")?;
                delimiter = parse_delimiter_char(&s, kw_tok.span)?;
                ts.skip_to_semi();
            }
            "guessingrows" => {
                expect_eq(ts, "GUESSINGROWS")?;
                // Valeur numérique : lire et ignorer
                if let TokenKind::Num(n) = ts.peek().kind {
                    guessingrows = Some(n as usize);
                    ts.next();
                }
                ts.skip_to_semi();
            }
            _ => {
                // sous-statement inconnu → ignorer
                ts.skip_to_semi();
            }
        }
    }

    let datafile = datafile.ok_or_else(|| {
        SasError::runtime("PROC IMPORT: DATAFILE= is required.")
    })?;
    let out = out.ok_or_else(|| {
        SasError::runtime("PROC IMPORT: OUT= is required.")
    })?;
    let dbms = dbms.unwrap_or(ImportDbms::Csv);

    Ok(ImportAst {
        datafile,
        out,
        dbms,
        replace,
        getnames,
        delimiter,
        guessingrows,
    })
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Execute PROC IMPORT. Appelé par `procs::execute_proc`.
pub fn execute(ast: &ImportAst, session: &mut Session) -> Result<()> {
    // --- Résoudre le séparateur ---
    let sep = resolve_separator(ast)?;

    // --- Lire le DataFrame avec Polars (chemin relatif résolu sous base_dir) ---
    let path = session.resolve_path(&ast.datafile);
    let df = CsvReadOptions::default()
        .with_has_header(ast.getnames)
        .with_parse_options(
            CsvParseOptions::default().with_separator(sep),
        )
        .try_into_reader_with_file_path(Some(path))
        .map_err(|e| SasError::runtime(format!("PROC IMPORT: cannot open '{}': {}", ast.datafile, e)))?
        .finish()
        .map_err(|e| SasError::runtime(format!("PROC IMPORT: error reading '{}': {}", ast.datafile, e)))?;

    // --- Renommer les colonnes si GETNAMES=NO (Polars → VAR1, VAR2, …) ---
    let df = if !ast.getnames {
        rename_to_var_n(df)?
    } else {
        df
    };

    // --- Coercition vers le modèle de types SAS ---
    let (ds, notes) = SasDataset::from_dataframe(df)?;
    for note in &notes {
        session.log.forward(note);
    }

    let n_obs = ds.n_obs();
    let n_vars = ds.n_vars();

    // --- Écrire dans la bibliothèque cible ---
    let out_libref = ast.out.libref_or_work();
    let out_table = ast.out.name.to_uppercase();
    let display = ast.out.display();

    let provider = session.libs.get(&out_libref)?;
    provider.write(&out_table, &ds)?;

    // --- Mettre à jour _LAST_ et émettre la NOTE ---
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_obs, n_vars
    ));

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers internes
// ---------------------------------------------------------------------------

/// Résout le séparateur en octet selon DBMS + DELIMITER éventuel.
fn resolve_separator(ast: &ImportAst) -> Result<u8> {
    match &ast.dbms {
        ImportDbms::Csv => Ok(b','),
        ImportDbms::Tab => Ok(b'\t'),
        ImportDbms::Dlm => {
            // DELIMITER= fourni → l'utiliser ; sinon espace (défaut SAS DLM)
            Ok(ast.delimiter.unwrap_or(b' '))
        }
    }
}

/// Renomme les colonnes Polars `column_1`…`column_N` en `VAR1`…`VARN`
/// lorsque `GETNAMES=NO`.
fn rename_to_var_n(mut df: DataFrame) -> Result<DataFrame> {
    let n = df.width();
    let old_names: Vec<String> = df
        .get_column_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    for (i, old) in old_names.iter().enumerate() {
        let new_name = format!("VAR{}", i + 1);
        df.rename(old, new_name.as_str().into())
            .map_err(|e| SasError::runtime(format!("PROC IMPORT: rename column: {e}")))?;
    }
    let _ = n; // silence unused warning
    Ok(df)
}

/// Parse un DBMS par son nom en majuscules ; renvoie une erreur propre pour
/// les DBMS différés (XLSX/EXCEL).
fn parse_dbms(name: &str, span: crate::token::Span) -> Result<ImportDbms> {
    match name {
        "CSV" => Ok(ImportDbms::Csv),
        "TAB" => Ok(ImportDbms::Tab),
        "DLM" | "DLMSTR" => Ok(ImportDbms::Dlm),
        "XLSX" | "EXCEL" | "XLS" => Err(SasError::runtime(format!(
            "PROC IMPORT with DBMS={name} is not yet implemented in this build \
             (the calamine/rust_xlsxwriter crates are not available)."
        ))),
        other => Err(SasError::parse(
            format!("Unknown DBMS '{other}' for PROC IMPORT."),
            span,
        )),
    }
}

/// Parse un caractère délimiteur depuis une chaîne (potentiellement de
/// longueur 1 pour une casse simple, ou représentation mnémonique courante).
/// Renvoie une erreur si la chaîne est vide ou contient plus d'un octet ASCII.
fn parse_delimiter_char(s: &str, span: crate::token::Span) -> Result<Option<u8>> {
    // Mnémoniques courants
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

/// Parse un littéral de chaîne ou un identifiant sans guillemets (pour les
/// chemins et valeurs courtes). Retourne le contenu sans guillemets.
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
    use crate::session::Session;
    use crate::source::SourceFile;
    use std::io::Write;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_import_src(src: &str) -> Result<ImportAst> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "import"
        parse(&mut ts)
    }

    // --- Tests du parser ---

    #[test]
    fn parse_import_csv_minimal() {
        let ast = parse_import_src(
            "proc import datafile='/tmp/x.csv' out=work.myds dbms=csv; run;",
        )
        .unwrap();
        assert_eq!(ast.datafile, "/tmp/x.csv");
        assert_eq!(ast.out.name.to_uppercase(), "MYDS");
        assert_eq!(ast.dbms, ImportDbms::Csv);
        assert!(ast.getnames);
        assert!(!ast.replace);
    }

    #[test]
    fn parse_import_tab_with_replace() {
        let ast = parse_import_src(
            "proc import datafile='data.txt' out=work.t dbms=TAB replace; run;",
        )
        .unwrap();
        assert_eq!(ast.dbms, ImportDbms::Tab);
        assert!(ast.replace);
    }

    #[test]
    fn parse_import_getnames_no() {
        let ast = parse_import_src(
            "proc import datafile='x.csv' out=work.t dbms=csv; getnames=no; run;",
        )
        .unwrap();
        assert!(!ast.getnames);
    }

    #[test]
    fn parse_import_delimiter_in_body() {
        let ast = parse_import_src(
            "proc import datafile='x.txt' out=work.t dbms=dlm; delimiter='|'; run;",
        )
        .unwrap();
        assert_eq!(ast.dbms, ImportDbms::Dlm);
        assert_eq!(ast.delimiter, Some(b'|'));
    }

    #[test]
    fn parse_import_guessingrows_ignored() {
        let ast = parse_import_src(
            "proc import datafile='x.csv' out=work.t dbms=csv; guessingrows=200; run;",
        )
        .unwrap();
        assert_eq!(ast.guessingrows, Some(200));
    }

    #[test]
    fn parse_import_missing_datafile_errors() {
        let result = parse_import_src("proc import out=work.t dbms=csv; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("DATAFILE="), "msg: {msg}");
    }

    #[test]
    fn parse_import_missing_out_errors() {
        let result = parse_import_src("proc import datafile='x.csv' dbms=csv; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("OUT="), "msg: {msg}");
    }

    #[test]
    fn parse_import_xlsx_deferred_error() {
        let result = parse_import_src(
            "proc import datafile='x.xlsx' out=work.t dbms=xlsx; run;",
        );
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("not yet implemented"),
            "expected deferral message, got: {msg}"
        );
        assert!(msg.contains("XLSX"), "msg: {msg}");
    }

    #[test]
    fn parse_import_excel_deferred_error() {
        let result = parse_import_src(
            "proc import datafile='x.xlsx' out=work.t dbms=excel; run;",
        );
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("not yet implemented"), "msg: {msg}");
    }

    // --- Tests d'exécution ---

    fn write_csv(path: &std::path::Path, content: &str) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn execute_import_csv_basic() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("test.csv");
        write_csv(
            &csv_path,
            "name,age,score\nAlice,30,95.5\nBob,25,88.0\nCarol,35,72.3\n",
        );

        let mut session = make_session();
        let ast = ImportAst {
            datafile: csv_path.to_string_lossy().into_owned(),
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "mydata".into(),
            },
            dbms: ImportDbms::Csv,
            replace: false,
            getnames: true,
            delimiter: None,
            guessingrows: None,
        };
        execute(&ast, &mut session).unwrap();

        // Vérifier la NOTE dans le log
        let log = session.log.into_string();
        assert!(
            log.contains("The data set WORK.MYDATA has 3 observations and 3 variables."),
            "log: {log}"
        );

        // Vérifier _LAST_
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.MYDATA"));

        // Re-lire le dataset et vérifier les colonnes
        // On vérifie juste que _LAST_ et la NOTE sont corrects.
    }

    #[test]
    fn execute_import_csv_values_correct() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("vals.csv");
        write_csv(&csv_path, "x,y\n1.0,a\n2.0,b\n3.0,c\n");

        // Créer une session pointant le WORK vers un répertoire connu.
        let work_dir = dir.path().join("work");
        std::fs::create_dir(&work_dir).unwrap();
        let mut session =
            Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

        let ast = ImportAst {
            datafile: csv_path.to_string_lossy().into_owned(),
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            },
            dbms: ImportDbms::Csv,
            replace: false,
            getnames: true,
            delimiter: None,
            guessingrows: None,
        };
        execute(&ast, &mut session).unwrap();

        // Re-lire le dataset depuis le même WORK
        let provider = session.libs.get("WORK").unwrap();
        let (ds, _) = provider.read("T").unwrap();
        assert_eq!(ds.n_obs(), 3);
        assert_eq!(ds.n_vars(), 2);

        let x_col = ds.df.column("x").unwrap();
        let x = x_col.f64().unwrap();
        assert_eq!(x.get(0), Some(1.0));
        assert_eq!(x.get(1), Some(2.0));
        assert_eq!(x.get(2), Some(3.0));

        let y_col = ds.df.column("y").unwrap();
        let y = y_col.str().unwrap();
        assert_eq!(y.get(0), Some("a"));
        assert_eq!(y.get(1), Some("b"));
        assert_eq!(y.get(2), Some("c"));
    }

    #[test]
    fn execute_import_tab_separated() {
        let dir = tempfile::tempdir().unwrap();
        let tsv_path = dir.path().join("test.tsv");
        write_csv(&tsv_path, "a\tb\n10\t20\n30\t40\n");

        let work_dir = dir.path().join("work");
        std::fs::create_dir(&work_dir).unwrap();
        let mut session =
            Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

        let ast = ImportAst {
            datafile: tsv_path.to_string_lossy().into_owned(),
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "TAB".into(),
            },
            dbms: ImportDbms::Tab,
            replace: false,
            getnames: true,
            delimiter: None,
            guessingrows: None,
        };
        execute(&ast, &mut session).unwrap();

        let provider = session.libs.get("WORK").unwrap();
        let (ds, _) = provider.read("TAB").unwrap();
        assert_eq!(ds.n_obs(), 2);
        assert_eq!(ds.n_vars(), 2);

        let a = ds.df.column("a").unwrap().f64().unwrap();
        assert_eq!(a.get(0), Some(10.0));
        assert_eq!(a.get(1), Some(30.0));
    }

    #[test]
    fn execute_import_dlm_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        write_csv(&path, "x|y\n1|hello\n2|world\n");

        let work_dir = dir.path().join("work");
        std::fs::create_dir(&work_dir).unwrap();
        let mut session =
            Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

        let ast = ImportAst {
            datafile: path.to_string_lossy().into_owned(),
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "PIPE".into(),
            },
            dbms: ImportDbms::Dlm,
            replace: false,
            getnames: true,
            delimiter: Some(b'|'),
            guessingrows: None,
        };
        execute(&ast, &mut session).unwrap();

        let provider = session.libs.get("WORK").unwrap();
        let (ds, _) = provider.read("PIPE").unwrap();
        assert_eq!(ds.n_obs(), 2);
        assert_eq!(ds.n_vars(), 2);

        let y = ds.df.column("y").unwrap().str().unwrap();
        assert_eq!(y.get(0), Some("hello"));
        assert_eq!(y.get(1), Some("world"));
    }

    #[test]
    fn execute_import_getnames_no_produces_var_n() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("noheader.csv");
        write_csv(&csv_path, "Alice,30\nBob,25\n");

        let work_dir = dir.path().join("work");
        std::fs::create_dir(&work_dir).unwrap();
        let mut session =
            Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

        let ast = ImportAst {
            datafile: csv_path.to_string_lossy().into_owned(),
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "NOHEAD".into(),
            },
            dbms: ImportDbms::Csv,
            replace: false,
            getnames: false,
            delimiter: None,
            guessingrows: None,
        };
        execute(&ast, &mut session).unwrap();

        let provider = session.libs.get("WORK").unwrap();
        let (ds, _) = provider.read("NOHEAD").unwrap();
        assert_eq!(ds.n_vars(), 2);
        // Les noms doivent être VAR1, VAR2
        let names: Vec<&str> = ds.df.get_column_names().into_iter().map(|s| s.as_str()).collect();
        assert_eq!(names, vec!["VAR1", "VAR2"], "column names: {names:?}");
    }

    #[test]
    fn execute_import_sets_last_dataset() {
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("a.csv");
        write_csv(&csv_path, "x\n1\n2\n");

        let work_dir = dir.path().join("work");
        std::fs::create_dir(&work_dir).unwrap();
        let mut session =
            Session::new(Some(work_dir), PathBuf::from("."), true).unwrap();

        let ast = ImportAst {
            datafile: csv_path.to_string_lossy().into_owned(),
            out: DatasetRef {
                libref: None,
                name: "LAST".into(),
            },
            dbms: ImportDbms::Csv,
            replace: false,
            getnames: true,
            delimiter: None,
            guessingrows: None,
        };
        execute(&ast, &mut session).unwrap();
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.LAST"));
    }

    #[test]
    fn execute_import_nonexistent_file_errors() {
        let mut session = make_session();
        let ast = ImportAst {
            datafile: "/nonexistent/path/missing.csv".into(),
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            },
            dbms: ImportDbms::Csv,
            replace: false,
            getnames: true,
            delimiter: None,
            guessingrows: None,
        };
        let result = execute(&ast, &mut session);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("PROC IMPORT"), "msg: {msg}");
    }

    #[test]
    fn resolve_separator_csv() {
        let ast = ImportAst {
            datafile: String::new(),
            out: DatasetRef { libref: None, name: "t".into() },
            dbms: ImportDbms::Csv,
            replace: false,
            getnames: true,
            delimiter: None,
            guessingrows: None,
        };
        assert_eq!(resolve_separator(&ast).unwrap(), b',');
    }

    #[test]
    fn resolve_separator_tab() {
        let ast = ImportAst {
            datafile: String::new(),
            out: DatasetRef { libref: None, name: "t".into() },
            dbms: ImportDbms::Tab,
            replace: false,
            getnames: true,
            delimiter: None,
            guessingrows: None,
        };
        assert_eq!(resolve_separator(&ast).unwrap(), b'\t');
    }

    #[test]
    fn resolve_separator_dlm_default_space() {
        let ast = ImportAst {
            datafile: String::new(),
            out: DatasetRef { libref: None, name: "t".into() },
            dbms: ImportDbms::Dlm,
            replace: false,
            getnames: true,
            delimiter: None,
            guessingrows: None,
        };
        assert_eq!(resolve_separator(&ast).unwrap(), b' ');
    }

    #[test]
    fn resolve_separator_dlm_with_delimiter() {
        let ast = ImportAst {
            datafile: String::new(),
            out: DatasetRef { libref: None, name: "t".into() },
            dbms: ImportDbms::Dlm,
            replace: false,
            getnames: true,
            delimiter: Some(b';'),
            guessingrows: None,
        };
        assert_eq!(resolve_separator(&ast).unwrap(), b';');
    }

    #[test]
    fn parse_delimiter_char_single() {
        let span = crate::token::Span::default();
        assert_eq!(parse_delimiter_char(",", span).unwrap(), Some(b','));
        assert_eq!(parse_delimiter_char("|", span).unwrap(), Some(b'|'));
        assert_eq!(parse_delimiter_char(";", span).unwrap(), Some(b';'));
    }

    #[test]
    fn parse_delimiter_char_mnemonic_tab() {
        let span = crate::token::Span::default();
        assert_eq!(parse_delimiter_char("TAB", span).unwrap(), Some(b'\t'));
        assert_eq!(parse_delimiter_char("tab", span).unwrap(), Some(b'\t'));
    }

    #[test]
    fn parse_delimiter_char_mnemonic_space() {
        let span = crate::token::Span::default();
        assert_eq!(parse_delimiter_char("SPACE", span).unwrap(), Some(b' '));
    }
}
