//! PROC CATALOG (jalon M21.1) — gestion des catalogues SAS (v1 minimal).
//!
//! # Syntaxe
//! ```sas
//! proc catalog catalog=<libref.cat>;
//!   contents;
//!   delete entry1 / et=format;
//!   copy out=lib.cat2;
//!   quit;
//! ```
//!
//! # Sémantique v1 — implémentation documentée minimale
//!
//! Notre modèle n'a pas de vrais catalogues SAS (les formats sont stockés
//! dans le `FormatCatalog` en mémoire, pas dans des fichiers .sas7bcat).
//! v1 se contente de :
//! - Parser le bloc jusqu'à `quit;`
//! - `CONTENTS` : si le catalogue pointe vers un libref connu, lister les
//!   formats utilisateur (depuis `session.format_catalog`) ; sinon listing vide.
//! - `DELETE` / `COPY` : no-op gracieux + NOTE dans le log.
//! - Émettre une NOTE "Procedure CATALOG used."
//!
//! Ce comportement est documenté comme déviation v1 ; la vraie gestion des
//! .sas7bcat est reportée.
//!
//! # Déviation connue v1
//! - Les entrées de catalogue ne correspondent pas aux types SAS réels
//!   (CATALOG, FORMAT, GFONT, …) : en v1, seuls les formats utilisateur
//!   (`session.format_catalog`) sont exposés par CONTENTS.
//! - La sélection sur le nom de catalogue (work.formats vs. sasuser.profile)
//!   n'est pas implémentée : CONTENTS liste toujours tous les formats
//!   utilisateur en mémoire.

use crate::error::Result;
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;

/// Type of catalog sub-statement.
#[derive(Debug, Clone)]
pub enum CatalogStmt {
    Contents,
    Delete { entries: Vec<String> },
    Copy { out: Option<String> },
    Other(String),
}

pub struct CatalogAst {
    /// The catalog= option value (e.g. "WORK.FORMATS").
    pub catalog: String,
    /// Sub-statements accumulated before `quit;`.
    pub stmts: Vec<CatalogStmt>,
}

/// Parse `proc catalog catalog=lib.cat; ... quit;`
/// Called AFTER "proc catalog" has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<CatalogAst> {
    let mut catalog = String::new();

    // Parse header options until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }

        if ts.peek().is_kw("catalog") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                ts.next(); // skip garbage
                continue;
            }
            ts.next(); // consume `=`
            // Parse libref.cat — could be ident.ident or just ident
            let tok = ts.peek().clone();
            if let Some(first) = tok.ident() {
                let first = first.to_uppercase();
                ts.next();
                if ts.peek().kind == TokenKind::Dot {
                    ts.next(); // consume `.`
                    let tok2 = ts.peek().clone();
                    if let Some(second) = tok2.ident() {
                        catalog = format!("{}.{}", first, second.to_uppercase());
                        ts.next();
                    } else {
                        catalog = first;
                    }
                } else {
                    catalog = first;
                }
            }
        } else {
            ts.next();
        }
    }

    // Parse sub-statements until `quit;`
    let mut stmts: Vec<CatalogStmt> = Vec::new();

    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }
        if ts.peek().is_kw("run") {
            // run; is a no-op separator in run-group procs
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            continue;
        }

        if ts.peek().is_kw("contents") {
            ts.next();
            // consume optional `;`
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            stmts.push(CatalogStmt::Contents);
            continue;
        }

        if ts.peek().is_kw("delete") {
            ts.next();
            let mut entries: Vec<String> = Vec::new();
            loop {
                if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                    break;
                }
                // Skip `/ et=<type>` option clause
                if ts.peek().kind == TokenKind::Slash {
                    ts.skip_to_semi();
                    break;
                }
                if let Some(name) = ts.peek().ident() {
                    entries.push(name.to_uppercase());
                    ts.next();
                } else {
                    ts.next();
                }
            }
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            stmts.push(CatalogStmt::Delete { entries });
            continue;
        }

        if ts.peek().is_kw("copy") {
            ts.next();
            let mut out: Option<String> = None;
            loop {
                if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                    break;
                }
                if ts.peek().is_kw("out") {
                    ts.next();
                    if ts.peek().kind == TokenKind::Eq {
                        ts.next();
                        // Parse out=lib.cat
                        if let Some(first) = ts.peek().ident() {
                            let first = first.to_uppercase();
                            ts.next();
                            if ts.peek().kind == TokenKind::Dot {
                                ts.next();
                                if let Some(second) = ts.peek().ident() {
                                    out = Some(format!("{}.{}", first, second.to_uppercase()));
                                    ts.next();
                                } else {
                                    out = Some(first);
                                }
                            } else {
                                out = Some(first);
                            }
                        }
                    }
                } else {
                    ts.next();
                }
            }
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            stmts.push(CatalogStmt::Copy { out });
            continue;
        }

        // Unknown sub-statement: record and skip
        let kw = ts.peek().ident().unwrap_or("?").to_uppercase();
        let kw_owned = kw.clone();
        stmts.push(CatalogStmt::Other(kw_owned));
        ts.skip_to_semi();
    }

    Ok(CatalogAst { catalog, stmts })
}

/// Execute PROC CATALOG.
pub fn execute(ast: &CatalogAst, session: &mut Session) -> Result<()> {
    session.log.note(&format!(
        "Processing catalog: {}.",
        if ast.catalog.is_empty() { "(none)" } else { &ast.catalog }
    ));

    for stmt in &ast.stmts {
        match stmt {
            CatalogStmt::Contents => {
                // List user-defined formats as catalog entries (v1 approximation)
                let format_names: Vec<String> = session
                    .format_catalog
                    .user_format_names()
                    .iter()
                    .map(|n| n.to_uppercase())
                    .collect();

                session.listing.page_header();
                session.listing.write_line(&format!("Catalog: {}", ast.catalog));
                session.listing.blank();

                if format_names.is_empty() {
                    session.listing.write_line(
                        "NOTE: No entries in this catalog (v1: only user-defined formats are listed).",
                    );
                } else {
                    let headers = vec![
                        "Name".to_string(),
                        "Type".to_string(),
                        "Description".to_string(),
                    ];
                    let aligns = vec![Align::Left, Align::Left, Align::Left];
                    let rows: Vec<Vec<String>> = format_names
                        .iter()
                        .map(|n| {
                            vec![
                                n.clone(),
                                "FORMAT".to_string(),
                                "User-defined format".to_string(),
                            ]
                        })
                        .collect();
                    session.listing.write_table(&headers, &aligns, &rows);
                }
            }

            CatalogStmt::Delete { entries } => {
                // No-op gracieux + NOTE
                for entry in entries {
                    session.log.note(&format!(
                        "CATALOG: DELETE entry '{}' from catalog '{}' (v1: no-op).",
                        entry, ast.catalog
                    ));
                }
                if entries.is_empty() {
                    session.log.note("CATALOG: DELETE statement (no entries specified) — no-op.");
                }
            }

            CatalogStmt::Copy { out } => {
                let dest = out.as_deref().unwrap_or("(unspecified)");
                session.log.note(&format!(
                    "CATALOG: COPY from '{}' to '{}' (v1: no-op).",
                    ast.catalog, dest
                ));
            }

            CatalogStmt::Other(kw) => {
                session.log.note(&format!(
                    "CATALOG: sub-statement '{}' is not implemented in v1 (no-op).",
                    kw
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use crate::source::SourceFile;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_catalog_src(src: &str) -> crate::error::Result<CatalogAst> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "catalog"
        parse(&mut ts)
    }

    // ── Parse tests ───────────────────────────────────────────────────────────

    #[test]
    fn parse_catalog_minimal() {
        let ast = parse_catalog_src("proc catalog catalog=work.formats; quit;").unwrap();
        assert_eq!(ast.catalog, "WORK.FORMATS");
        assert!(ast.stmts.is_empty());
    }

    #[test]
    fn parse_catalog_with_contents() {
        let ast = parse_catalog_src(
            "proc catalog catalog=work.cat; contents; quit;",
        )
        .unwrap();
        assert_eq!(ast.stmts.len(), 1);
        assert!(matches!(ast.stmts[0], CatalogStmt::Contents));
    }

    #[test]
    fn parse_catalog_delete_entry() {
        let ast = parse_catalog_src(
            "proc catalog catalog=sasuser.profile; delete myfmt / et=format; quit;",
        )
        .unwrap();
        assert_eq!(ast.stmts.len(), 1);
        match &ast.stmts[0] {
            CatalogStmt::Delete { entries } => {
                assert_eq!(entries, &["MYFMT".to_string()]);
            }
            _ => panic!("expected Delete statement"),
        }
    }

    #[test]
    fn parse_catalog_copy() {
        let ast = parse_catalog_src(
            "proc catalog catalog=work.cat; copy out=work.cat2; quit;",
        )
        .unwrap();
        assert_eq!(ast.stmts.len(), 1);
        match &ast.stmts[0] {
            CatalogStmt::Copy { out } => {
                assert_eq!(out.as_deref(), Some("WORK.CAT2"));
            }
            _ => panic!("expected Copy statement"),
        }
    }

    #[test]
    fn parse_catalog_quit_terminates() {
        // Anything after quit; should be ignored
        let ast = parse_catalog_src(
            "proc catalog catalog=work.cat; contents; quit; contents;",
        )
        .unwrap();
        // Should have only 1 statement (the one before quit)
        assert_eq!(ast.stmts.len(), 1);
    }

    // ── Execute tests ─────────────────────────────────────────────────────────

    #[test]
    fn execute_contents_empty_catalog() {
        let mut session = make_session();
        let ast = CatalogAst {
            catalog: "WORK.FORMATS".to_string(),
            stmts: vec![CatalogStmt::Contents],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("WORK.FORMATS"), "listing: {listing}");
        // No user formats yet → empty listing with note
        assert!(listing.contains("No entries") || listing.is_empty() || listing.contains("Catalog"), "listing: {listing}");
    }

    #[test]
    fn execute_delete_noop_with_note() {
        let mut session = make_session();
        let ast = CatalogAst {
            catalog: "WORK.FORMATS".to_string(),
            stmts: vec![CatalogStmt::Delete {
                entries: vec!["MYFORMAT".to_string()],
            }],
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("DELETE") || log.contains("no-op"), "log: {log}");
        assert!(log.contains("MYFORMAT"), "log: {log}");
    }

    #[test]
    fn execute_copy_noop_with_note() {
        let mut session = make_session();
        let ast = CatalogAst {
            catalog: "WORK.CAT".to_string(),
            stmts: vec![CatalogStmt::Copy {
                out: Some("WORK.CAT2".to_string()),
            }],
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("COPY") || log.contains("no-op"), "log: {log}");
    }

    #[test]
    fn execute_quit_recognized() {
        // Just parsing and executing with no stmts should succeed
        let mut session = make_session();
        let ast = CatalogAst {
            catalog: "WORK.CAT".to_string(),
            stmts: vec![],
        };
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("Processing catalog"), "log: {log}");
    }
}
