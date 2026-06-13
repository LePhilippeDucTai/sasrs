//! PROC DATASETS (jalon M7) — run-group proc terminée par QUIT;.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc datasets lib=work [nolist] ; delete a b ; change old=new ;
//! [run ;] quit ;`
//!
//! - Run-group : les statements s'exécutent à chaque `run;` ET à
//!   `quit;` — M7 : exécution à quit; suffit, documenter l'écart.
//!   DEVIATION : sub-statements are accumulated and executed all at once at
//!   `quit;`. SAS would execute them at each `run;` too; we only execute at
//!   `quit;` for simplicity. A `run;` sub-statement is treated as a no-op
//!   separator.
//! - Sans NOLIST : afficher le répertoire de la librairie APRÈS modifications
//!   (table Name / Member Type DATA / nb obs).
//! - `delete` → `LibraryProvider::delete` (inexistant → WARNING comme
//!   SAS, pas ERROR) ; `change old=new` → rename.
//! - Order of operations: `delete`s first, then `change`s (all in declaration
//!   order within each group). Tables deleted first cannot be renamed.

use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;

pub struct DatasetsAst {
    pub lib: String,
    pub nolist: bool,
    pub deletes: Vec<String>,
    pub changes: Vec<(String, String)>,
}

/// Parse `proc datasets [lib=<ident>] [nolist] ; ... quit ;`
/// Called AFTER "proc datasets" has been consumed. Consumes through `quit;`.
/// Defaults to lib=WORK if the LIB= option is absent.
pub fn parse(ts: &mut StatementStream) -> Result<DatasetsAst> {
    let mut lib = "WORK".to_string();
    let mut nolist = false;
    let mut deletes: Vec<String> = Vec::new();
    let mut changes: Vec<(String, String)> = Vec::new();

    // ── Parse PROC DATASETS header options until `;` ─────────────────────────
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next(); // consume `;`
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("lib") || ts.peek().is_kw("library") {
            ts.next(); // consume "lib" / "library"
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after LIB",
                    ts.peek().span,
                ));
            }
            ts.next(); // consume `=`
            let ident_tok = ts.peek().clone();
            let Some(name) = ident_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a libref name after LIB=",
                    ident_tok.span,
                ));
            };
            ts.next();
            lib = name.to_uppercase();
        } else if ts.peek().is_kw("nolist") {
            ts.next();
            nolist = true;
        } else {
            // Unknown header option: skip to `;`
            ts.skip_to_semi();
            break;
        }
    }

    // ── Parse sub-statements until `quit;` ───────────────────────────────────
    loop {
        // Skip stray semicolons
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }

        if ts.peek().kind == TokenKind::Eof {
            break;
        }

        if ts.peek().is_kw("quit") {
            ts.next(); // consume "quit"
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }

        if ts.peek().is_kw("run") {
            // `run;` is a no-op separator in M7 (run-group deviation documented above)
            ts.next(); // consume "run"
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            continue;
        }

        if ts.peek().is_kw("delete") {
            ts.next(); // consume "delete"
            // Read one or more names until `;`
            loop {
                if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                    break;
                }
                let name_tok = ts.peek().clone();
                let Some(name) = name_tok.ident().map(str::to_string) else {
                    // non-ident token: skip to `;`
                    ts.skip_to_semi();
                    break;
                };
                ts.next();
                deletes.push(name.to_uppercase());
            }
            // consume trailing `;`
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            continue;
        }

        if ts.peek().is_kw("change") {
            ts.next(); // consume "change"
            // Read one or more `old=new` pairs until `;`
            loop {
                if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                    break;
                }
                let old_tok = ts.peek().clone();
                let Some(old_name) = old_tok.ident().map(str::to_string) else {
                    ts.skip_to_semi();
                    break;
                };
                ts.next(); // consume old name
                if ts.peek().kind != TokenKind::Eq {
                    return Err(SasError::parse(
                        "expected '=' in CHANGE statement old=new pair",
                        ts.peek().span,
                    ));
                }
                ts.next(); // consume `=`
                let new_tok = ts.peek().clone();
                let Some(new_name) = new_tok.ident().map(str::to_string) else {
                    return Err(SasError::parse(
                        "expected a new name after '=' in CHANGE statement",
                        new_tok.span,
                    ));
                };
                ts.next(); // consume new name
                changes.push((old_name.to_uppercase(), new_name.to_uppercase()));
            }
            // consume trailing `;`
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            continue;
        }

        // Unknown sub-statement: skip to `;`
        ts.skip_to_semi();
    }

    Ok(DatasetsAst {
        lib,
        nolist,
        deletes,
        changes,
    })
}

/// Execute PROC DATASETS.
pub fn execute(ast: &DatasetsAst, session: &mut Session) -> Result<()> {
    let lib = ast.lib.to_uppercase();
    let provider = session.libs.get(&lib).map_err(|_| {
        SasError::runtime(format!("Libref {} is not assigned.", lib))
    })?;

    // ── Apply deletes ─────────────────────────────────────────────────────────
    for name in &ast.deletes {
        let name_upper = name.to_uppercase();
        if provider.exists(&name_upper) {
            provider.delete(&name_upper)?;
            session.log.note(&format!(
                "Deleting {}.{} (memtype=DATA).",
                lib, name_upper
            ));
        } else {
            session.log.warning(&format!(
                "Table {}.{} does not exist and was not deleted.",
                lib, name_upper
            ));
        }
    }

    // ── Apply renames (CHANGE old=new) ────────────────────────────────────────
    for (old, new) in &ast.changes {
        let old_upper = old.to_uppercase();
        let new_upper = new.to_uppercase();
        provider.rename(&old_upper, &new_upper)?;
        session.log.note(&format!(
            "Changing the name {}.{} to {}.{} (memtype=DATA).",
            lib, old_upper, lib, new_upper
        ));
    }

    // ── Directory listing (unless NOLIST) ─────────────────────────────────────
    if !ast.nolist {
        let mut tables = provider.list()?;
        tables.sort();

        session.listing.page_header();

        let headers = vec![
            "#".to_string(),
            "Name".to_string(),
            "Member Type".to_string(),
        ];
        let aligns = vec![Align::Right, Align::Left, Align::Left];

        let rows: Vec<Vec<String>> = tables
            .iter()
            .enumerate()
            .map(|(i, t)| {
                vec![
                    (i + 1).to_string(),
                    t.to_uppercase(),
                    "DATA".to_string(),
                ]
            })
            .collect();

        session.listing.write_table(&headers, &aligns, &rows);
    }

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

    fn parse_datasets_src(src: &str) -> Result<DatasetsAst> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "datasets"
        parse(&mut ts)
    }

    /// Write a simple numeric dataset into WORK.
    fn write_simple_dataset(session: &mut Session, name: &str) {
        let df = df!["x" => [1.0_f64, 2.0]].unwrap();
        let vars = vec![VarMeta {
            name: "x".to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }];
        let ds = SasDataset { df, vars };
        session
            .libs
            .get("WORK")
            .unwrap()
            .write(name, &ds)
            .unwrap();
    }

    /// Write a dataset with a format and label so a sidecar file is created.
    fn write_dataset_with_meta(session: &mut Session, name: &str) {
        let df = df!["age" => [30.0_f64, 25.0]].unwrap();
        let vars = vec![VarMeta {
            name: "age".to_string(),
            ty: VarType::Num,
            length: 8,
            format: Some("best12.".to_string()),
            label: Some("Age".to_string()),
        }];
        let ds = SasDataset { df, vars };
        session
            .libs
            .get("WORK")
            .unwrap()
            .write(name, &ds)
            .unwrap();
    }

    // ── Parse tests ───────────────────────────────────────────────────────────

    #[test]
    fn parse_full_example() {
        let src = "proc datasets lib=work nolist; delete a b; change c=d; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(ast.lib, "WORK");
        assert!(ast.nolist);
        assert_eq!(ast.deletes, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(
            ast.changes,
            vec![("C".to_string(), "D".to_string())]
        );
    }

    #[test]
    fn parse_defaults_to_work() {
        let src = "proc datasets nolist; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(ast.lib, "WORK");
        assert!(ast.nolist);
        assert!(ast.deletes.is_empty());
        assert!(ast.changes.is_empty());
    }

    #[test]
    fn parse_library_alias() {
        let src = "proc datasets library=mylib nolist; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(ast.lib, "MYLIB");
        assert!(ast.nolist);
    }

    #[test]
    fn parse_no_nolist_defaults_false() {
        let src = "proc datasets lib=work; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert!(!ast.nolist);
    }

    #[test]
    fn parse_multiple_changes() {
        let src = "proc datasets lib=work nolist; change a=b c=d; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(
            ast.changes,
            vec![
                ("A".to_string(), "B".to_string()),
                ("C".to_string(), "D".to_string()),
            ]
        );
    }

    #[test]
    fn parse_run_is_noop_separator() {
        // run; between statements should not stop accumulation
        let src = "proc datasets lib=work nolist; delete a; run; change b=c; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(ast.deletes, vec!["A".to_string()]);
        assert_eq!(ast.changes, vec![("B".to_string(), "C".to_string())]);
    }

    // ── Execute tests ─────────────────────────────────────────────────────────

    #[test]
    fn execute_delete_removes_table() {
        let mut session = make_session();
        write_simple_dataset(&mut session, "ALPHA");

        assert!(session.libs.get("WORK").unwrap().exists("ALPHA"));

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: true,
            deletes: vec!["ALPHA".to_string()],
            changes: vec![],
        };
        execute(&ast, &mut session).unwrap();

        assert!(!session.libs.get("WORK").unwrap().exists("ALPHA"));
        let log = session.log.into_string();
        assert!(log.contains("Deleting"), "log: {log}");
        assert!(log.contains("ALPHA"), "log: {log}");
    }

    #[test]
    fn execute_delete_missing_is_warning_not_error() {
        let mut session = make_session();

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: true,
            deletes: vec!["NONEXISTENT".to_string()],
            changes: vec![],
        };
        // Must not return Err
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(
            log.contains("WARNING") || log.contains("does not exist"),
            "log: {log}"
        );
    }

    #[test]
    fn execute_change_renames_table() {
        let mut session = make_session();
        write_simple_dataset(&mut session, "OLDNAME");

        assert!(session.libs.get("WORK").unwrap().exists("OLDNAME"));
        assert!(!session.libs.get("WORK").unwrap().exists("NEWNAME"));

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: true,
            deletes: vec![],
            changes: vec![("OLDNAME".to_string(), "NEWNAME".to_string())],
        };
        execute(&ast, &mut session).unwrap();

        assert!(!session.libs.get("WORK").unwrap().exists("OLDNAME"));
        assert!(session.libs.get("WORK").unwrap().exists("NEWNAME"));

        // Verify data is intact
        let (ds, _) = session
            .libs
            .get("WORK")
            .unwrap()
            .read("NEWNAME")
            .unwrap();
        assert_eq!(ds.n_obs(), 2);

        let log = session.log.into_string();
        assert!(log.contains("Changing"), "log: {log}");
        assert!(log.contains("OLDNAME"), "log: {log}");
        assert!(log.contains("NEWNAME"), "log: {log}");
    }

    #[test]
    fn execute_nolist_suppresses_listing() {
        let mut session = make_session();
        write_simple_dataset(&mut session, "T1");

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: true,
            deletes: vec![],
            changes: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.is_empty(), "listing should be empty with nolist: {listing}");
    }

    #[test]
    fn execute_without_nolist_emits_directory() {
        let mut session = make_session();
        write_simple_dataset(&mut session, "T1");
        write_simple_dataset(&mut session, "T2");

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: false,
            deletes: vec![],
            changes: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("T1"), "listing: {listing}");
        assert!(listing.contains("T2"), "listing: {listing}");
        assert!(listing.contains("DATA"), "Member Type column: {listing}");
        assert!(listing.contains("Name"), "Name header: {listing}");
    }

    #[test]
    fn execute_rename_moves_sidecar() {
        let mut session = make_session();
        write_dataset_with_meta(&mut session, "WITHFORMAT");

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: true,
            deletes: vec![],
            changes: vec![("WITHFORMAT".to_string(), "RENAMED".to_string())],
        };
        execute(&ast, &mut session).unwrap();

        assert!(!session.libs.get("WORK").unwrap().exists("WITHFORMAT"));
        assert!(session.libs.get("WORK").unwrap().exists("RENAMED"));

        // Read back and verify format survived the rename (sidecar was moved)
        let (ds, _) = session
            .libs
            .get("WORK")
            .unwrap()
            .read("RENAMED")
            .unwrap();
        let age_var = ds.vars.iter().find(|v| v.name.eq_ignore_ascii_case("age")).unwrap();
        assert_eq!(
            age_var.format.as_deref(),
            Some("best12."),
            "format should survive rename via sidecar move"
        );
        assert_eq!(
            age_var.label.as_deref(),
            Some("Age"),
            "label should survive rename via sidecar move"
        );
    }
}
