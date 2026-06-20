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
//! # v1 : sous-ensemble représentatif de SasOptions
//! OBS, FIRSTOBS, LINESIZE (LS=), PAGESIZE (PS=, valeur par défaut=60),
//! CENTER/NOCENTER (défaut NOCENTER), DATE/NODATE (défaut NODATE),
//! MPRINT/NOMPRINT, MLOGIC/NOMLOGIC, SYMBOLGEN/NOSYMBOLGEN.

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
        } else if let Some(ident) = ts.peek().ident() {
            option_names.push(ident.to_uppercase());
            ts.next();
        } else {
            ts.next();
        }
    }

    // Parse sub-statements until `run;` or `quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |_ts, _kw| Ok(false))?;

    Ok(OptionsAst { option_names, short, long })
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
fn all_known_options(session: &Session) -> Vec<(&'static str, OptionValue<'_>)> {
    vec![
        ("OBS", OptionValue::NumOrMax(session.options.obs)),
        ("FIRSTOBS", OptionValue::Num(session.options.firstobs.to_string())),
        ("LINESIZE", OptionValue::Num(session.options.ls.to_string())),
        // PAGESIZE: not in SasOptions yet; use a reasonable default
        ("PAGESIZE", OptionValue::Num("60".to_string())),
        // CENTER/NOCENTER: not in SasOptions yet; default NOCENTER (like SAS listing mode)
        ("CENTER", OptionValue::Bool { name: "CENTER", value: false }),
        // DATE/NODATE: not in SasOptions yet; default NODATE
        ("DATE", OptionValue::Bool { name: "DATE", value: false }),
        ("MPRINT", OptionValue::Bool { name: "MPRINT", value: session.options.mprint }),
        ("MLOGIC", OptionValue::Bool { name: "MLOGIC", value: session.options.mlogic }),
        ("SYMBOLGEN", OptionValue::Bool { name: "SYMBOLGEN", value: session.options.symbolgen }),
    ]
}

/// Format one option value as a string.
fn render_option(name: &str, val: &OptionValue<'_>) -> String {
    match val {
        OptionValue::Num(v) => format!("{name}={v}"),
        OptionValue::NumOrMax(None) => format!("{name}=MAX"),
        OptionValue::NumOrMax(Some(n)) => format!("{name}={n}"),
        OptionValue::Bool { name: opt_name, value: true } => opt_name.to_string(),
        OptionValue::Bool { name: opt_name, value: false } => format!("NO{opt_name}"),
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
                    canon.strip_prefix("NO")
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
mod tests {
    use super::*;
    use crate::session::Session;
    use crate::source::SourceFile;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_options_src(src: &str) -> crate::error::Result<OptionsAst> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "options"
        parse(&mut ts)
    }

    // ── Parse tests ───────────────────────────────────────────────────────────

    #[test]
    fn parse_no_options() {
        let ast = parse_options_src("proc options; run;").unwrap();
        assert!(ast.option_names.is_empty());
        assert!(!ast.short);
        assert!(!ast.long);
    }

    #[test]
    fn parse_specific_options() {
        let ast = parse_options_src("proc options obs linesize; run;").unwrap();
        assert_eq!(ast.option_names, vec!["OBS", "LINESIZE"]);
    }

    #[test]
    fn parse_short_flag() {
        let ast = parse_options_src("proc options short; run;").unwrap();
        assert!(ast.short);
        // "short" is not added to option_names
        assert!(ast.option_names.is_empty());
    }

    #[test]
    fn parse_long_flag() {
        let ast = parse_options_src("proc options long; run;").unwrap();
        assert!(ast.long);
    }

    // ── Execute tests ─────────────────────────────────────────────────────────

    #[test]
    fn execute_all_options_writes_to_log() {
        let mut session = make_session();
        let ast = OptionsAst {
            option_names: vec![],
            short: false,
            long: false,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("OBS="), "log: {log}");
        assert!(log.contains("LINESIZE="), "log: {log}");
        assert!(log.contains("FIRSTOBS="), "log: {log}");
        // Boolean options appear with NO prefix when false
        assert!(log.contains("NODATE") || log.contains("DATE"), "log: {log}");
    }

    #[test]
    fn execute_specific_option_obs() {
        let mut session = make_session();
        let ast = OptionsAst {
            option_names: vec!["OBS".to_string()],
            short: false,
            long: false,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("OBS=MAX"), "log: {log}");
        // Should not contain other options
        assert!(!log.contains("LINESIZE="), "should not contain LINESIZE: {log}");
    }

    #[test]
    fn execute_specific_option_linesize() {
        let mut session = make_session();
        let ast = OptionsAst {
            option_names: vec!["LINESIZE".to_string()],
            short: false,
            long: false,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("LINESIZE=96"), "log: {log}");
    }

    #[test]
    fn execute_unknown_option_warns() {
        let mut session = make_session();
        let ast = OptionsAst {
            option_names: vec!["UNKNOWNOPTION123".to_string()],
            short: false,
            long: false,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("WARNING") || log.contains("not a recognized"), "log: {log}");
    }

    #[test]
    fn execute_boolean_option_format() {
        let mut session = make_session();
        // Default: MPRINT = false → should appear as NOMPRINT
        let ast = OptionsAst {
            option_names: vec!["MPRINT".to_string()],
            short: false,
            long: false,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("NOMPRINT"), "log: {log}");
    }

    #[test]
    fn execute_does_not_write_to_listing() {
        let mut session = make_session();
        let ast = OptionsAst {
            option_names: vec![],
            short: false,
            long: false,
        };
        execute(&ast, &mut session).unwrap();

        // PROC OPTIONS writes to log only, not listing
        let listing = session.listing.into_string();
        assert!(listing.is_empty(), "listing should be empty: {listing}");
    }
}
