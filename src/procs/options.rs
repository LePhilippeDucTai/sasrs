//! PROC OPTIONS (jalon M21.1) — afficher les options système SAS.
//!
//! # Syntaxe
//! ```sas
//! proc options [option1 option2 ...] [short|long];
//! run;
//! ```
//!
//! - Sans liste → affiche toutes les options connues.
//! - Avec liste → seulement les options demandées.
//! - SHORT / LONG : hints de format (v1 : ignorés, même format dans les deux cas).
//! - Options inconnues → WARNING dans le log.
//!
//! # Rendu
//! Les lignes sont émises au LOG (pas au listing), comme SAS le fait nativement.
//! Format : `OPTION_NAME=<value>` pour les options scalaires, ou `OPTION_NAME`
//! (sans `=`) pour les booléennes à activer (e.g. MPRINT) et `NOMPRINT` pour
//! les booléennes désactivées.
//!
//! # Sous-ensemble représentatif de SasOptions
//! OBS, FIRSTOBS, LINESIZE (LS=), PAGESIZE (PS=), CENTER/NOCENTER,
//! DATE/NODATE, NUMBER/NONUMBER, YEARCUTOFF=, MPRINT/NOMPRINT,
//! MLOGIC/NOMLOGIC, SYMBOLGEN/NOSYMBOLGEN.
//!
//! J02-P9 — toutes les valeurs viennent de l'ÉTAT RÉEL de la session
//! (`session.options`, `session.ods_options`) : PAGESIZE=60, NOCENTER et
//! NODATE n'y sont plus codés en dur. Le détail par option (rendu fidèle à
//! `proc options` SAS) reste J11-P5.

use crate::error::Result;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::session::Session;
use crate::token::TokenKind;

pub struct OptionsAst {
    /// Specific option names requested. Empty = all options.
    pub option_names: Vec<String>,
    /// SHORT format hint (v1: ignored).
    pub short: bool,
    /// LONG format hint (v1: ignored).
    pub long: bool,
}

/// Parse `proc options [name ...] [short|long]; run;`
/// Called AFTER "proc options" has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<OptionsAst> {
    let mut option_names: Vec<String> = Vec::new();
    let mut short = false;
    let mut long = false;

    // Parse header options until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }

        if ts.peek().is_kw("short") {
            ts.next();
            short = true;
        } else if ts.peek().is_kw("long") {
            ts.next();
            long = true;
        } else if ts.peek().is_kw("option") {
            // J02-P9 — `OPTION=nom` (et la forme parenthésée `OPTION(nom)`,
            // doc SAS 9.4 PROC OPTIONS statement) : l'option DÉMANDÉE est
            // `nom`, pas « OPTION ». L'ancien parse ne lisait que l'ident
            // `option` et produisait un WARNING trompeur sur OPTION.
            ts.next(); // consume `option`
            if ts.peek().kind == TokenKind::Eq {
                ts.next(); // consume `=`
            } else if ts.peek().kind == TokenKind::LParen {
                ts.next(); // consume `(`
            } else {
                // `proc options option;` nu : SAS exigerait une valeur ; sans
                // valeur, l'option demandée reste « OPTION » et produira le
                // WARNING habituel (comportement inchangé).
                option_names.push("OPTION".to_string());
                continue;
            }
            let Some(ident) = ts.peek().ident() else {
                option_names.push("OPTION".to_string());
                continue;
            };
            option_names.push(ident.to_uppercase());
            ts.next(); // consume the option name
            if ts.peek().kind == TokenKind::RParen {
                ts.next(); // consume `)` of OPTION(nom)
            }
        } else if let Some(ident) = ts.peek().ident() {
            option_names.push(ident.to_uppercase());
            ts.next();
        } else {
            ts.next();
        }
    }

    // Parse sub-statements until `run;` or `quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, "OPTIONS", |_ts, _kw| Ok(false))?;

    Ok(OptionsAst {
        option_names,
        short,
        long,
    })
}

/// A known SAS option with its canonical name and how to render it.
enum OptionValue<'a> {
    /// Numeric option: rendered as `NAME=<value>`
    Num(String),
    /// Nullable numeric (OBS=MAX when None): rendered as `NAME=<value>` or `NAME=MAX`
    NumOrMax(Option<usize>),
    /// Boolean: rendered as `NAME` (true) or `NONAME` (false)
    Bool { name: &'a str, value: bool },
}

/// Build the list of all known options and their current values from the session.
///
/// J02-P9 — PAGESIZE, CENTER/NOCENTER et DATE/NODATE sont lus de l'ÉTAT RÉEL
/// de la session (`session.options.pagesize`, `session.ods_options`), plus
/// des valeurs codées en dur PAGESIZE=60 / NOCENTER / NODATE (audit d0b4d90 :
/// le listing imprimait des valeurs de session fausses en silence).
/// NUMBER/NONUMBER est exposé aussi (dans `session.ods_options`) ; le détail
/// de rendu de chaque option reste J11-P5.
fn all_known_options(session: &Session) -> Vec<(&'static str, OptionValue<'_>)> {
    vec![
        ("OBS", OptionValue::NumOrMax(session.options.obs)),
        (
            "FIRSTOBS",
            OptionValue::Num(session.options.firstobs.to_string()),
        ),
        ("LINESIZE", OptionValue::Num(session.options.ls.to_string())),
        ("PAGESIZE", OptionValue::Num(session.options.pagesize.to_string())),
        (
            "CENTER",
            OptionValue::Bool {
                name: "CENTER",
                value: !session.ods_options.nocenter,
            },
        ),
        (
            "DATE",
            OptionValue::Bool {
                name: "DATE",
                value: session.ods_options.date,
            },
        ),
        (
            "NUMBER",
            OptionValue::Bool {
                name: "NUMBER",
                value: session.ods_options.number,
            },
        ),
        (
            "YEARCUTOFF",
            OptionValue::Num(session.options.yearcutoff.to_string()),
        ),
        (
            "MPRINT",
            OptionValue::Bool {
                name: "MPRINT",
                value: session.options.mprint,
            },
        ),
        (
            "MLOGIC",
            OptionValue::Bool {
                name: "MLOGIC",
                value: session.options.mlogic,
            },
        ),
        (
            "SYMBOLGEN",
            OptionValue::Bool {
                name: "SYMBOLGEN",
                value: session.options.symbolgen,
            },
        ),
    ]
}

/// Format one option value as a string.
fn render_option(name: &str, val: &OptionValue<'_>) -> String {
    match val {
        OptionValue::Num(v) => format!("{name}={v}"),
        OptionValue::NumOrMax(None) => format!("{name}=MAX"),
        OptionValue::NumOrMax(Some(n)) => format!("{name}={n}"),
        OptionValue::Bool {
            name: opt_name,
            value: true,
        } => opt_name.to_string(),
        OptionValue::Bool {
            name: opt_name,
            value: false,
        } => format!("NO{opt_name}"),
    }
}

/// Execute PROC OPTIONS.
pub fn execute(ast: &OptionsAst, session: &mut Session) -> Result<()> {
    // Gather all data from session.options BEFORE touching session.log,
    // to avoid simultaneous mutable/immutable borrows.
    let (lines, warnings) = {
        let known = all_known_options(session);
        let known_map: std::collections::HashMap<&str, &OptionValue<'_>> =
            known.iter().map(|(n, v)| (*n, v)).collect();

        if ast.option_names.is_empty() {
            // All options
            let lines: Vec<String> = known.iter().map(|(n, v)| render_option(n, v)).collect();
            (lines, vec![])
        } else {
            // Requested options only
            let mut out_lines = Vec::new();
            let mut out_warns: Vec<String> = Vec::new();
            for req in &ast.option_names {
                let canon = req.as_str();
                let found = known_map.get(canon).or_else(|| {
                    canon
                        .strip_prefix("NO")
                        .and_then(|stripped| known_map.get(stripped))
                });
                if let Some(val) = found {
                    let actual_name = known
                        .iter()
                        .find(|(n, _)| {
                            n.eq_ignore_ascii_case(canon)
                                || canon
                                    .strip_prefix("NO")
                                    .map(|s| n.eq_ignore_ascii_case(s))
                                    .unwrap_or(false)
                        })
                        .map(|(n, _)| *n)
                        .unwrap_or(canon);
                    out_lines.push(render_option(actual_name, val));
                } else {
                    out_warns.push(format!("Option {req} is not a recognized SAS option."));
                }
            }
            (out_lines, out_warns)
        }
    };

    for warn in warnings {
        session.log.warning(&warn);
    }
    for line in lines {
        session.log.note(&line);
    }

    Ok(())
}

#[cfg(test)]
mod tests;
