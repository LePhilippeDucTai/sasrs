//! PROC IMPORT (jalon M14.3).
//!
//! Lit un fichier externe (CSV, TSV, DLM, XLSX) et crée un dataset SAS.
//!
//! ## Syntaxe
//! ```sas
//! proc import datafile='path' out=lib.table dbms=csv [replace];
//!     getnames=yes;   /* par défaut YES */
//!     delimiter=',';  /* pour dbms=dlm  */
//!     datarow=2;      /* numéro de la première ligne de données */
//!     sheet='Sheet1'; /* pour dbms=xlsx */
//! run;
//! ```
//!
//! ## DBMS supportés dans ce build
//! - `CSV`  : CsvReader Polars (séparateur `,`).
//! - `TAB`  : CsvReader Polars (séparateur `\t`).
//! - `DLM`  : CsvReader Polars (séparateur configurable via `DELIMITER=`/`DLM=`, défaut ` `).
//! - `XLSX` : non disponible dans ce build (dépendance `calamine` absente) →
//!            `SasError` "PROC IMPORT with DBMS=XLSX is not yet implemented in this build."
//!
//! ## Points d'attache
//! - Lecture CSV → `DataFrame` Polars → `SasDataset::from_dataframe` (coercition SAS).
//! - `GETNAMES=NO` : noms générés (`VAR1`, `VAR2`, …).
//! - Chemin résolu vs `session.base_dir`.
//! - `REPLACE` : le dataset de sortie est écrasé s'il existe (sans REPLACE, erreur si présent).
//! - `DATAROW=n` : saute `n-1` lignes après l'en-tête (ou après la ligne 1 si GETNAMES=NO).

use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;
use polars::prelude::*;
use std::path::PathBuf;

// ──────────────────────────────────────────────────────────────────────────────
// AST
// ──────────────────────────────────────────────────────────────────────────────

/// DBMS = format du fichier d'entrée.
#[derive(Debug, Clone, PartialEq)]
pub enum Dbms {
    Csv,
    Tab,
    Dlm,
    Xlsx,
    /// Autre valeur passée par l'utilisateur : stockée telle quelle (majuscules)
    /// pour le message d'erreur.
    Other(String),
}

impl Dbms {
    fn from_str(s: &str) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "CSV" => Dbms::Csv,
            "TAB" => Dbms::Tab,
            "DLM" | "DLMSTR" => Dbms::Dlm,
            "XLSX" | "EXCEL" | "XLS" | "XLSM" => Dbms::Xlsx,
            other => Dbms::Other(other.to_string()),
        }
    }
}

pub struct ImportAst {
    /// Chemin du fichier source (`DATAFILE=`).
    pub datafile: String,
    /// Dataset SAS de sortie (`OUT=`).
    pub out: DatasetRef,
    /// Format du fichier.
    pub dbms: Dbms,
    /// `REPLACE` : écraser le dataset s'il existe.
    pub replace: bool,
    /// `GETNAMES=YES|NO` — YES par défaut.
    pub getnames: bool,
    /// `DELIMITER=` / `DLM=` pour DBMS=DLM.
    pub delimiter: Option<String>,
    /// `DATAROW=` : numéro de la première ligne de données (1-indexé).
    pub datarow: Option<usize>,
    /// `SHEET=` pour DBMS=XLSX.
    pub sheet: Option<String>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Parsing
// ──────────────────────────────────────────────────────────────────────────────

/// Parse `proc import … ; [sub-statements ;] run ;`
/// Appelé APRÈS que `proc import` a été consommé.
pub fn parse(ts: &mut StatementStream) -> Result<ImportAst> {
    let mut datafile: Option<String> = None;
    let mut out: Option<DatasetRef> = None;
    let mut dbms: Option<Dbms> = None;
    let mut replace = false;

    // Options sur le statement PROC IMPORT (jusqu'au `;`)
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
                "datafile" | "file" => {
                    ts.next();
                    expect_eq(ts, "DATAFILE")?;
                    datafile = Some(parse_string_value(ts, "DATAFILE")?);
                }
                "out" => {
                    ts.next();
                    expect_eq(ts, "OUT")?;
                    out = Some(ts.parse_dataset_ref()?);
                }
                "dbms" => {
                    ts.next();
                    expect_eq(ts, "DBMS")?;
                    let val = parse_ident_or_string(ts, "DBMS")?;
                    dbms = Some(Dbms::from_str(&val));
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
                        format!("Unexpected option '{bad}' on PROC IMPORT statement."),
                        span,
                    ));
                }
            }
        } else {
            let span = tok.span;
            ts.skip_to_semi();
            return Err(SasError::parse(
                "Unexpected token on PROC IMPORT statement.",
                span,
            ));
        }
    }

    // Valider les options obligatoires
    let datafile = datafile.ok_or_else(|| {
        SasError::runtime("The DATAFILE= option is required on PROC IMPORT.")
    })?;
    let out = out.ok_or_else(|| {
        SasError::runtime("The OUT= option is required on PROC IMPORT.")
    })?;
    let dbms = dbms.ok_or_else(|| {
        SasError::runtime("The DBMS= option is required on PROC IMPORT.")
    })?;

    // Sub-statements (GETNAMES=, DELIMITER=, DATAROW=, SHEET=, …)
    let mut getnames = true;
    let mut delimiter: Option<String> = None;
    let mut datarow: Option<usize> = None;
    let mut sheet: Option<String> = None;

    loop {
        // Sauter les `;` en trop
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
        // Sub-statement : un mot-clé suivi de `=` valeur `;`
        let tok = ts.peek().clone();
        let Some(kw) = tok.ident() else {
            ts.skip_to_semi();
            continue;
        };
        match kw.to_ascii_lowercase().as_str() {
            "getnames" => {
                ts.next();
                expect_eq(ts, "GETNAMES")?;
                let val = parse_ident_or_string(ts, "GETNAMES")?;
                getnames = !val.eq_ignore_ascii_case("no");
                ts.skip_to_semi();
            }
            "delimiter" | "dlm" => {
                ts.next();
                expect_eq(ts, "DELIMITER")?;
                delimiter = Some(parse_string_value(ts, "DELIMITER")?);
                ts.skip_to_semi();
            }
            "datarow" => {
                ts.next();
                expect_eq(ts, "DATAROW")?;
                // Numéro de ligne : attendu comme un entier
                let num_tok = ts.peek().clone();
                match &num_tok.kind {
                    TokenKind::Num(n) => {
                        datarow = Some(*n as usize);
                        ts.next();
                    }
                    _ => {
                        return Err(SasError::parse(
                            "expected a number after DATAROW=",
                            num_tok.span,
                        ));
                    }
                }
                ts.skip_to_semi();
            }
            "sheet" => {
                ts.next();
                expect_eq(ts, "SHEET")?;
                sheet = Some(parse_string_value(ts, "SHEET")?);
                ts.skip_to_semi();
            }
            _ => {
                // Option sub-statement inconnue : ignorer (SAS 9.4 est très
                // permissif ici selon le DBMS).
                ts.skip_to_semi();
            }
        }
    }

    Ok(ImportAst {
        datafile,
        out,
        dbms,
        replace,
        getnames,
        delimiter,
        datarow,
        sheet,
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// Exécution
// ──────────────────────────────────────────────────────────────────────────────

pub fn execute(ast: &ImportAst, session: &mut Session) -> Result<()> {
    let out_libref = ast.out.libref_or_work();
    let out_table = ast.out.name.to_uppercase();

    // Résoudre le chemin du fichier
    let path = resolve_path(&ast.datafile, &session.base_dir);

    // Vérifier REPLACE
    let provider = session.libs.get(&out_libref)?;
    if provider.exists(&out_table) && !ast.replace {
        return Err(SasError::runtime(format!(
            "The data set {}.{} already exists. \
             Use the REPLACE option if you want to replace it.",
            out_libref, out_table
        )));
    }

    // Dispatcher selon DBMS
    let (ds, notes) = match &ast.dbms {
        Dbms::Csv => read_csv(&path, b',', ast)?,
        Dbms::Tab => read_csv(&path, b'\t', ast)?,
        Dbms::Dlm => {
            let sep = delimiter_byte(ast.delimiter.as_deref())?;
            read_csv(&path, sep, ast)?
        }
        Dbms::Xlsx => {
            return Err(SasError::runtime(
                "PROC IMPORT with DBMS=XLSX is not yet implemented in this build.",
            ));
        }
        Dbms::Other(name) => {
            return Err(SasError::runtime(format!(
                "PROC IMPORT with DBMS={name} is not yet implemented in this build.",
            )));
        }
    };

    // Forwarder les notes de coercition
    for note in &notes {
        session.log.forward(note);
    }

    // Écrire le dataset
    provider.write(&out_table, &ds)?;

    let out_disp = format!("{out_libref}.{out_table}");
    session.last_dataset = Some(out_disp.clone());

    let n_obs = ds.n_obs();
    let n_vars = ds.n_vars();
    session.log.note(&format!(
        "The data set {out_disp} has {n_obs} observations and {n_vars} variables."
    ));

    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers de lecture
// ──────────────────────────────────────────────────────────────────────────────

/// Lit un fichier CSV/DLM avec le séparateur indiqué et retourne un SasDataset.
fn read_csv(
    path: &PathBuf,
    separator: u8,
    ast: &ImportAst,
) -> Result<(SasDataset, Vec<String>)> {
    // DATAROW : skip_rows_after_header = datarow - 2 (car la ligne 1 est
    // l'en-tête si GETNAMES=YES, et datarow est 1-indexé depuis le début).
    // Si GETNAMES=NO, skip_rows = datarow - 1 (pas d'en-tête).
    let skip_after_header: usize = match ast.datarow {
        Some(dr) if dr >= 2 && ast.getnames => dr.saturating_sub(2),
        Some(dr) if !ast.getnames => dr.saturating_sub(1),
        _ => 0,
    };

    let parse_options = CsvParseOptions::default().with_separator(separator);

    let df = CsvReadOptions::default()
        .with_has_header(ast.getnames)
        .with_parse_options(parse_options)
        .with_skip_rows_after_header(skip_after_header)
        .try_into_reader_with_file_path(Some(path.clone()))?
        .finish()?;

    // Si GETNAMES=NO, renommer les colonnes en VAR1, VAR2, …
    let df = if !ast.getnames {
        rename_no_header_cols(df)?
    } else {
        df
    };

    SasDataset::from_dataframe(df)
}

/// Si GETNAMES=NO, Polars génère des noms `column_0`, `column_1`, … ;
/// SAS utilise `VAR1`, `VAR2`, … — on renomme ici.
fn rename_no_header_cols(df: DataFrame) -> Result<DataFrame> {
    let names: Vec<String> = (1..=df.width())
        .map(|i| format!("VAR{}", i))
        .collect();
    let mut df = df;
    for (col, new_name) in df.get_column_names_owned().iter().zip(names.iter()) {
        df.rename(col, new_name.as_str().into())?;
    }
    Ok(df)
}

// ──────────────────────────────────────────────────────────────────────────────
// Utilitaires
// ──────────────────────────────────────────────────────────────────────────────

/// Résoudre un chemin relatif vis-à-vis de `base_dir`.
fn resolve_path(file: &str, base_dir: &std::path::Path) -> PathBuf {
    let p = PathBuf::from(file);
    if p.is_absolute() {
        p
    } else {
        base_dir.join(p)
    }
}

/// Extraire le séparateur (un seul octet ASCII) depuis une chaîne DELIMITER=.
fn delimiter_byte(dlm: Option<&str>) -> Result<u8> {
    match dlm {
        None | Some("") | Some(" ") => Ok(b' '),
        Some(s) => {
            // Accepter soit un seul caractère, soit `'09'x` (tab hex), soit
            // des valeurs courantes.
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

/// Lire une valeur qui peut être une chaîne littérale `'...'` ou un identificateur.
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

/// Lire une valeur qui peut être un identificateur ou une chaîne (pour les
/// options comme DBMS=CSV ou GETNAMES=YES).
fn parse_ident_or_string(ts: &mut StatementStream, opt: &str) -> Result<String> {
    parse_string_value(ts, opt)
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

    fn parse_import(src: &str) -> Result<ImportAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "import"
        parse(&mut ts)
    }

    // ── Parse tests ──────────────────────────────────────────────────────────

    #[test]
    fn parse_basic_csv() {
        let ast = parse_import(
            "proc import datafile='data.csv' out=work.myds dbms=csv replace; run;",
        )
        .unwrap();
        assert_eq!(ast.datafile, "data.csv");
        assert_eq!(ast.out.name, "myds");
        assert_eq!(ast.dbms, Dbms::Csv);
        assert!(ast.replace);
        assert!(ast.getnames); // default YES
    }

    #[test]
    fn parse_getnames_no() {
        let ast = parse_import(
            "proc import datafile='f.csv' out=work.t dbms=csv; getnames=no; run;",
        )
        .unwrap();
        assert!(!ast.getnames);
    }

    #[test]
    fn parse_getnames_yes_explicit() {
        let ast = parse_import(
            "proc import datafile='f.csv' out=work.t dbms=csv; getnames=yes; run;",
        )
        .unwrap();
        assert!(ast.getnames);
    }

    #[test]
    fn parse_tab_dbms() {
        let ast = parse_import(
            "proc import datafile='f.tsv' out=work.t dbms=tab; run;",
        )
        .unwrap();
        assert_eq!(ast.dbms, Dbms::Tab);
    }

    #[test]
    fn parse_dlm_with_delimiter() {
        let ast = parse_import(
            "proc import datafile='f.txt' out=work.t dbms=dlm; delimiter='|'; run;",
        )
        .unwrap();
        assert_eq!(ast.dbms, Dbms::Dlm);
        assert_eq!(ast.delimiter.as_deref(), Some("|"));
    }

    #[test]
    fn parse_xlsx_dbms() {
        let ast = parse_import(
            "proc import datafile='f.xlsx' out=work.t dbms=xlsx; sheet='Sheet1'; run;",
        )
        .unwrap();
        assert_eq!(ast.dbms, Dbms::Xlsx);
        assert_eq!(ast.sheet.as_deref(), Some("Sheet1"));
    }

    #[test]
    fn parse_datarow_option() {
        let ast = parse_import(
            "proc import datafile='f.csv' out=work.t dbms=csv; datarow=3; run;",
        )
        .unwrap();
        assert_eq!(ast.datarow, Some(3));
    }

    #[test]
    fn parse_missing_datafile_errors() {
        let result = parse_import("proc import out=work.t dbms=csv; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("DATAFILE="), "msg: {msg}");
    }

    #[test]
    fn parse_missing_out_errors() {
        let result = parse_import("proc import datafile='f.csv' dbms=csv; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("OUT="), "msg: {msg}");
    }

    #[test]
    fn parse_missing_dbms_errors() {
        let result = parse_import("proc import datafile='f.csv' out=work.t; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("DBMS="), "msg: {msg}");
    }

    // ── Execute tests ─────────────────────────────────────────────────────────

    #[test]
    fn execute_csv_roundtrip() {
        // Écrire un CSV dans un tmpdir, l'importer, vérifier les données.
        let dir = tempdir().unwrap();
        let csv_path = dir.path().join("input.csv");
        std::fs::write(
            &csv_path,
            "name,age,score\nAlice,30,95.5\nBob,25,87.0\n",
        )
        .unwrap();

        let mut session = make_session_with_base(dir.path().to_path_buf());
        let ast = ImportAst {
            datafile: csv_path.to_str().unwrap().to_string(),
            out: DatasetRef { libref: Some("WORK".into()), name: "MYDS".into() },
            dbms: Dbms::Csv,
            replace: false,
            getnames: true,
            delimiter: None,
            datarow: None,
            sheet: None,
        };
        execute(&ast, &mut session).unwrap();

        let (ds, _) = session.libs.get("WORK").unwrap().read("MYDS").unwrap();
        assert_eq!(ds.n_obs(), 2);
        // "name" est une colonne char, "age" et "score" sont numériques
        let name_var = ds.vars.iter().find(|v| v.name.to_ascii_uppercase() == "NAME");
        assert!(name_var.is_some());
        assert_eq!(name_var.unwrap().ty, VarType::Char);

        let age_var = ds.vars.iter().find(|v| v.name.to_ascii_uppercase() == "AGE");
        assert!(age_var.is_some());
        assert_eq!(age_var.unwrap().ty, VarType::Num);

        // Vérifier session.last_dataset
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.MYDS"));

        // Vérifier la note dans le log
        let log = session.log.into_string();
        assert!(log.contains("2 observations and 3 variables"), "log: {log}");
    }

    #[test]
    fn execute_replace_false_errors_if_exists() {
        let dir = tempdir().unwrap();
        let csv_path = dir.path().join("input.csv");
        std::fs::write(&csv_path, "x\n1\n2\n").unwrap();

        let mut session = make_session_with_base(dir.path().to_path_buf());

        // Créer le dataset d'abord
        let df = df!["x" => [1.0_f64]].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![VarMeta {
                name: "x".to_string(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            }],
        };
        session.libs.get("WORK").unwrap().write("MYDS", &ds).unwrap();

        let ast = ImportAst {
            datafile: csv_path.to_str().unwrap().to_string(),
            out: DatasetRef { libref: Some("WORK".into()), name: "MYDS".into() },
            dbms: Dbms::Csv,
            replace: false,
            getnames: true,
            delimiter: None,
            datarow: None,
            sheet: None,
        };
        let result = execute(&ast, &mut session);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("already exists"), "msg: {msg}");
    }

    #[test]
    fn execute_replace_true_overwrites() {
        let dir = tempdir().unwrap();
        let csv_path = dir.path().join("input.csv");
        std::fs::write(&csv_path, "x\n42\n").unwrap();

        let mut session = make_session_with_base(dir.path().to_path_buf());

        // Créer un dataset existant avec des données différentes
        let df = df!["x" => [1.0_f64]].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![VarMeta {
                name: "x".to_string(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            }],
        };
        session.libs.get("WORK").unwrap().write("MYDS", &ds).unwrap();

        let ast = ImportAst {
            datafile: csv_path.to_str().unwrap().to_string(),
            out: DatasetRef { libref: Some("WORK".into()), name: "MYDS".into() },
            dbms: Dbms::Csv,
            replace: true,
            getnames: true,
            delimiter: None,
            datarow: None,
            sheet: None,
        };
        execute(&ast, &mut session).unwrap();

        let (ds, _) = session.libs.get("WORK").unwrap().read("MYDS").unwrap();
        assert_eq!(ds.n_obs(), 1);
    }

    #[test]
    fn execute_xlsx_errors_with_not_implemented() {
        let dir = tempdir().unwrap();
        let mut session = make_session_with_base(dir.path().to_path_buf());

        let ast = ImportAst {
            datafile: "data.xlsx".to_string(),
            out: DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            dbms: Dbms::Xlsx,
            replace: false,
            getnames: true,
            delimiter: None,
            datarow: None,
            sheet: None,
        };
        let result = execute(&ast, &mut session);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("not yet implemented"),
            "msg: {msg}"
        );
        assert!(msg.contains("XLSX"), "msg: {msg}");
    }

    #[test]
    fn execute_getnames_no_renames_columns() {
        let dir = tempdir().unwrap();
        let csv_path = dir.path().join("noheader.csv");
        std::fs::write(&csv_path, "Alice,30\nBob,25\n").unwrap();

        let mut session = make_session_with_base(dir.path().to_path_buf());

        let ast = ImportAst {
            datafile: csv_path.to_str().unwrap().to_string(),
            out: DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            dbms: Dbms::Csv,
            replace: false,
            getnames: false,
            delimiter: None,
            datarow: None,
            sheet: None,
        };
        execute(&ast, &mut session).unwrap();

        let (ds, _) = session.libs.get("WORK").unwrap().read("T").unwrap();
        assert_eq!(ds.n_obs(), 2);
        assert!(ds.vars.iter().any(|v| v.name == "VAR1"));
        assert!(ds.vars.iter().any(|v| v.name == "VAR2"));
    }
}
