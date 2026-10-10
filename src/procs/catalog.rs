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
//! # Contrat (J02-P8)
//!
//! Base SAS 9.4 Procedures Guide, CATALOG Procedure, PROC CATALOG statement :
//! `PROC CATALOG CATALOG=<libref.>catalog <ENTRYTYPE=etype> <FORCE> <KILL>;`
//! - `CATALOG=` (alias `CAT=`, `C=`) est requis ; l'alias `CAT=` était sauté
//!   (« Processing catalog: (none) ») : il est honoré.
//! - `ENTRYTYPE=` (alias `ET=`) et `KILL` étaient sautés : ERROR (J11-P6 et
//!   J03-P5) ; option inconnue : ERROR « Unexpected option ».
//! - `FORCE` (« forces statements to execute on a catalog that is opened by
//!   another resource environment ») est accepté : aucun autre environnement
//!   ne tient de catalogue ouvert dans une session sasrs.
//! - Les instructions SAS valides non implémentées (CHANGE, EXCHANGE,
//!   EXCLUDE, MODIFY, SAVE, SELECT ; options OUT= et FILE= de CONTENTS)
//!   donnent le message « not supported … cannot be ignored » du contrat.
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

use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::{Span, TokenKind};

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
    let header = ts.peek().span;
    let catalog = parse_header_options(ts)?;

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
            parse_contents_options(ts)?;
            stmts.push(CatalogStmt::Contents);
            continue;
        }

        let kw = ts.peek().ident().unwrap_or("").to_ascii_lowercase();
        if let Some((_, unit)) = UNSUPPORTED_STATEMENTS.iter().find(|(s, _)| *s == kw) {
            return Err(unsupported(
                &format!("The {} statement", kw.to_ascii_uppercase()),
                *unit,
                ts.peek().span,
            ));
        }

        crate::procs::common::unhandled_proc_statement(ts, "CATALOG")?;
    }

    // SAS 9.4 : CATALOG= is a required argument. Checked once the body is
    // read, so that an invalid statement keeps its own diagnostic.
    let Some(catalog) = catalog else {
        return Err(SasError::parse(
            "The CATALOG= option is required on the PROC CATALOG statement.",
            header,
        ));
    };
    Ok(CatalogAst { catalog, stmts })
}

// ─────────────────────── Contract diagnostics (J02-P8) ───────────────────────

/// Valid SAS 9.4 PROC CATALOG statements that sasrs does not implement, with
/// the roadmap-avancee unit that will lift the ERROR (None: not planned).
/// They used to be reported « 180-322 … not valid » (DELETE and COPY were
/// already rejected with the shared message).
const UNSUPPORTED_STATEMENTS: &[(&str, Option<&str>)] = &[
    ("change", None),
    ("copy", Some("J03-P5")),
    ("delete", Some("J03-P5")),
    ("exchange", None),
    ("exclude", None),
    ("modify", None),
    ("save", None),
    ("select", None),
];

/// Suffix naming the roadmap-avancee unit that will lift a provisional ERROR.
fn planned(unit: Option<&str>) -> String {
    match unit {
        Some(u) => format!(" (planned: roadmap-avancee {u})"),
        None => String::new(),
    }
}

/// Contract ERROR for a request that sasrs cannot honor.
fn unsupported(what: &str, unit: Option<&str>, span: Span) -> SasError {
    SasError::parse(
        format!(
            "{what} is not supported in PROC CATALOG; it can affect results and cannot be \
             ignored{}.",
            planned(unit)
        ),
        span,
    )
}

/// PROC CATALOG statement options until `;` (consumed). Returns the
/// `CATALOG=` value (`LIB.CAT` or `CAT`, uppercased), if any.
fn parse_header_options(ts: &mut StatementStream) -> Result<Option<String>> {
    let mut catalog: Option<String> = None;
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        let span = ts.peek().span;
        let kw = ts.peek().ident().unwrap_or("").to_ascii_lowercase();
        match kw.as_str() {
            // SAS 9.4 : « CATALOG=<libref.>catalog … Alias: CAT=, C= ».
            "catalog" | "cat" | "c" => {
                crate::procs::common::consume_option_eq(ts, "CATALOG")?;
                catalog = Some(parse_catalog_name(ts)?);
            }
            "entrytype" | "et" => {
                return Err(unsupported("The ENTRYTYPE= option", Some("J11-P6"), span));
            }
            "kill" => return Err(unsupported("The KILL option", Some("J03-P5"), span)),
            // No other resource environment holds a catalog open in a sasrs
            // session: FORCE has nothing to force.
            "force" => {
                ts.next();
            }
            _ => {
                return Err(crate::procs::common::unknown_option_error(ts, "CATALOG"));
            }
        }
    }
    Ok(catalog)
}

/// `<libref.>catalog` after `CATALOG=` : `LIB.CAT` or `CAT` (uppercased).
fn parse_catalog_name(ts: &mut StatementStream) -> Result<String> {
    let tok = ts.peek().clone();
    let Some(first) = tok.ident().map(str::to_uppercase) else {
        return Err(SasError::parse(
            "expected a catalog name after CATALOG=",
            tok.span,
        ));
    };
    ts.next();
    if ts.peek().kind != TokenKind::Dot {
        return Ok(first);
    }
    ts.next(); // consume `.`
    let tok2 = ts.peek().clone();
    let Some(second) = tok2.ident().map(str::to_uppercase) else {
        return Err(SasError::parse(
            "expected a catalog name after the libref in CATALOG=",
            tok2.span,
        ));
    };
    ts.next();
    Ok(format!("{first}.{second}"))
}

/// Options of the CONTENTS statement until `;` (consumed). SAS 9.4 :
/// `CONTENTS <OUT=SAS-data-set> <FILE=fileref>;` — OUT= used to make the
/// rest of the statement read as a new statement (« 180-322: Statement
/// 'OUT' is not valid »).
fn parse_contents_options(ts: &mut StatementStream) -> Result<()> {
    match ts.peek().kind {
        TokenKind::Semi => {
            ts.next();
            return Ok(());
        }
        TokenKind::Eof => return Ok(()),
        _ => {}
    }
    let span = ts.peek().span;
    let kw = ts.peek().ident().unwrap_or("?").to_ascii_uppercase();
    Err(match kw.as_str() {
        "OUT" => unsupported(
            "The OUT= option of the CONTENTS statement",
            Some("J11-P6"),
            span,
        ),
        "FILE" => unsupported("The FILE= option of the CONTENTS statement", None, span),
        _ => SasError::parse(
            format!("Unexpected option '{kw}' on the CONTENTS statement of PROC CATALOG."),
            span,
        ),
    })
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

#[cfg(test)]
mod contract_tests;
