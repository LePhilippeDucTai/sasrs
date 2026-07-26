//! OPTIONS : application des options globales et traces macro (MQ9.6).

use super::*;

/// Valide une option numérique bornée : renvoie la valeur si elle parse et
/// tombe dans `range`, sinon émet le message d'erreur SAS
/// « The value X is not a valid LABEL value (lo..hi). » et renvoie `None`.
pub(crate) fn parse_bounded_usize(
    session: &mut Session,
    value: Option<&str>,
    range: std::ops::RangeInclusive<usize>,
    label: &str,
) -> Option<usize> {
    match value.and_then(|v| v.parse::<usize>().ok()) {
        Some(v) if range.contains(&v) => Some(v),
        _ => {
            session.log.error(&format!(
                "The value {} is not a valid {label} value ({}..{}).",
                value.unwrap_or(""),
                range.start(),
                range.end()
            ));
            None
        }
    }
}

/// Applique UNE option de `OPTIONS name=value;` (ou boolénne sans valeur) à la
/// session. Une option inconnue émet le WARNING « not yet supported ».
pub(crate) fn apply_option(session: &mut Session, name: &str, value: Option<&str>) {
    match name.to_ascii_lowercase().as_str() {
        "ls" | "linesize" => {
            if let Some(v) = parse_bounded_usize(session, value, 40..=256, "LINESIZE") {
                session.options.ls = v;
                session.listing.set_ls(v);
            }
        }
        // OBS=MAX (or unset) → no limit; OBS=n → process up to obs n.
        "obs" => match value {
            Some(v) if v.eq_ignore_ascii_case("max") => session.options.obs = None,
            Some(v) => match v.parse::<usize>() {
                Ok(n) => session.options.obs = Some(n),
                Err(_) => session
                    .log
                    .error(&format!("The value {v} is not a valid OBS value.")),
            },
            None => session.options.obs = None,
        },
        // FIRSTOBS=MAX is unusual; treat any non-number as an error.
        "firstobs" => match value {
            Some(v) if v.eq_ignore_ascii_case("max") => session.options.firstobs = usize::MAX,
            Some(v) => match v.parse::<usize>() {
                Ok(n) if n >= 1 => session.options.firstobs = n,
                _ => session
                    .log
                    .error(&format!("The value {v} is not a valid FIRSTOBS value.")),
            },
            None => {}
        },
        // M38.2 — PAGESIZE=/PS= : page length for listing output.
        // Valid range: 15..=32767. Stored but no pagination yet.
        "ps" | "pagesize" => {
            if let Some(v) = parse_bounded_usize(session, value, 15..=32767, "PAGESIZE") {
                session.options.pagesize = v;
            }
        }
        // M38.2 — MISSING= : single character used to display ordinary
        // numeric missing values (`.`) in the listing. Default '.'.
        // Spec: value is a single character (quoted or unquoted).
        "missing" => match value {
            Some(v) if v.chars().count() == 1 => {
                session.options.missing_char = v.chars().next().expect("length checked");
            }
            Some("") => {
                // OPTIONS MISSING=''; → space (SAS behaviour)
                session.options.missing_char = ' ';
            }
            _ => session
                .log
                .error("The value for the MISSING option must be a single character."),
        },
        // M38.2 — YEARCUTOFF= : lower bound of the 100-year sliding
        // window for interpreting 2-digit years. Valid: 1582..=9999.
        "yearcutoff" => match value.and_then(|v| v.parse::<u16>().ok()) {
            Some(v) if v >= 1582 => {
                session.options.yearcutoff = v;
            }
            _ => session.log.error(&format!(
                "The value {} is not a valid YEARCUTOFF value.",
                value.unwrap_or("")
            )),
        },
        // M38.2 — FMTSEARCH= : list of library refs / catalogues for
        // format search order. Stored; multi-library resolution deferred
        // to M39. Value arrives as space-separated entries (parser joins
        // the parenthesised list with spaces).
        "fmtsearch" => {
            let entries: Vec<String> = value
                .unwrap_or("")
                .split_whitespace()
                .map(|s| s.to_ascii_uppercase())
                .collect();
            session.options.fmtsearch = entries;
        }
        // M19.2 — SASAUTOS= fixe le(s) répertoire(s) de bibliothèques
        // autocall. On accepte une valeur simple (un répertoire) :
        //   OPTIONS SASAUTOS='dir';  ou  OPTIONS SASAUTOS=dir;
        // Les guillemets éventuels sont retirés par le parser
        // global ; le chemin relatif est résolu contre `base_dir`
        // (même base que %include/LIBNAME). La forme liste
        // `(d1 d2)` n'est pas gérée ici (différée).
        "sasautos" => match value {
            Some(v) if !v.is_empty() => {
                let dir = session.resolve_path(v);
                session.macro_engine.set_sasautos_path(vec![dir]);
            }
            _ => session
                .log
                .error("The value for the SASAUTOS option is missing."),
        },
        // M22.2 — options globales ODS booléennes (CENTER/NOCENTER,
        // DATE/NODATE, NUMBER/NONUMBER) posées sur `session.ods_options`.
        // Stockées seulement (application au rendu différée M22.3+) :
        // pas d'effet visible sur le listing texte par défaut.
        _ if value.is_none() && session.set_ods_option(name) => {}
        // M19.3 — options de trace booléennes : MPRINT/MLOGIC/
        // SYMBOLGEN (et leurs formes NO...). Appliquées à la session
        // ET propagées au processeur macro (qui décide de l'écho).
        _ if parse_macro_trace_flag(name).is_some() => match parse_macro_trace_flag(name) {
            Some(("mprint", on)) => {
                session.options.mprint = on;
                session.macro_engine.set_mprint(on);
            }
            Some(("mlogic", on)) => {
                session.options.mlogic = on;
                session.macro_engine.set_mlogic(on);
            }
            Some(("symbolgen", on)) => {
                session.options.symbolgen = on;
                session.macro_engine.set_symbolgen(on);
            }
            _ => {}
        },
        _ => {
            session.log.warning(&format!(
                "Option {} is not yet supported.",
                name.to_uppercase()
            ));
        }
    }
}

/// M19.3 — reconnaît une option de trace macro booléenne. Rend
/// `Some((canon, on))` où `canon` est `"mprint"`/`"mlogic"`/`"symbolgen"` et
/// `on` est `false` pour la forme préfixée `NO` (ex. `NOMPRINT`). `None` si
/// l'option n'est pas une option de trace.
pub(crate) fn parse_macro_trace_flag(name: &str) -> Option<(&'static str, bool)> {
    let lower = name.to_ascii_lowercase();
    let (body, on) = match lower.strip_prefix("no") {
        Some(rest) if matches!(rest, "mprint" | "mlogic" | "symbolgen") => {
            (rest.to_string(), false)
        }
        _ => (lower, true),
    };
    let canon = match body.as_str() {
        "mprint" => "mprint",
        "mlogic" => "mlogic",
        "symbolgen" => "symbolgen",
        _ => return None,
    };
    Some((canon, on))
}
