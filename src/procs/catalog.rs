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
//! - `DELETE` / `COPY` : ERROR (mutations non implémentées).
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
        if crate::procs::common::parse_proc_inert_or_global(ts)? {
            continue;
        }
        if ts.peek().is_kw("data") || ts.peek().is_kw("proc") {
            break;
        }

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

        if ts.peek().is_kw("delete") || ts.peek().is_kw("copy") {
            return Err(crate::procs::common::unsupported_statement(
                "CATALOG",
                ts.peek().ident().unwrap(),
            ));
        }

        crate::procs::common::unhandled_proc_statement(ts, "CATALOG")?;
    }

    Ok(CatalogAst { catalog, stmts })
}

/// Execute PROC CATALOG.
pub fn execute(ast: &CatalogAst, session: &mut Session) -> Result<()> {
    session.log.note(&format!(
        "Processing catalog: {}.",
        if ast.catalog.is_empty() {
            "(none)"
        } else {
            &ast.catalog
        }
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
                session
                    .listing
                    .write_line(&format!("Catalog: {}", ast.catalog));
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

            CatalogStmt::Delete { .. } => {
                return Err(super::common::unsupported_statement("CATALOG", "DELETE"));
            }
            CatalogStmt::Copy { .. } => {
                return Err(super::common::unsupported_statement("CATALOG", "COPY"));
            }
            CatalogStmt::Other(kw) => {
                return Err(super::common::unsupported_statement("CATALOG", kw));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
