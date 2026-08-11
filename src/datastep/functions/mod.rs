//! Bibliothèque de fonctions SAS (table de dispatch).
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : MOYENNE, mécanique et
//! table-driven — idéal pour un modèle économique, fonction par fonction)
//!
//! `call(name, args, ctx)` : nom matché en MAJUSCULES ; renvoie `None` si
//! la fonction est inconnue (l'évaluateur en fait une erreur).
//!
//! ## Lot M1/M2 (sémantique SAS exacte)
//! Statistiques sur arguments (IGNORENT les missings, contrairement aux
//! opérateurs !) :
//! - `SUM(a,b,...)`  somme des non-missings ; TOUS missings → `.`
//! - `MEAN`, `MIN`, `MAX`, `N` (nb non-missings), `NMISS`
//! - `COALESCE(a,b,...)` premier non-missing
//! - `MISSING(x)` → 1.0/0.0 (marche aussi sur char blanc)
//!
//! Math :
//! - `ABS`, `SQRT` (négatif → `.` + invalid note), `EXP`,
//!   `LOG`/`LOG2`/`LOG10` (≤0 → `.` + note), `INT` (troncature vers 0),
//!   `ROUND(x[,unit])` — ATTENTION : round SAS = demi-arrondi loin de
//!   zéro (`(x/unit).round()` Rust fait déjà half-away-from-zero),
//!   `MOD(a,b)` — signe du résultat = signe de a (comme `%` Rust f64).
//!
//! Caractères (les longueurs/blancs comptent — relire la doc SAS !) :
//! - `UPCASE`, `LOWCASE`, `TRIM` (blancs finaux ; chaîne blanche → ""),
//!   `STRIP`, `LEFT` (M1 : équivalent trim_start), `LENGTH` (sans blancs
//!   finaux, minimum 1 même pour ""), `SUBSTR(s, pos[, len])` (pos
//!   1-based ; hors bornes → "" + `_ERROR_` + note "Invalid ... argument"),
//!   `INDEX(s, sub)` (1-based, 0 si absent), `CAT` (concat brut),
//!   `CATS` (strip chaque arg), `CATX(sep, ...)` (strip + séparateur,
//!   args blancs sautés), `COMPRESS(s[,chars])` (défaut : enlève les
//!   espaces), `TRANWRD(s, from, to)`, `SCAN(s, n[, delims])` (n<0 =
//!   depuis la fin ; délimiteurs par défaut SAS : ` .<>()+&!$*);^-/,%|`).
//!
//! Dates (M4 affinera avec les formats) :
//! - `TODAY()`/`DATE()` → jours depuis 1960 (sous --deterministic,
//!   l'exécuteur peut figer la date — passer l'info via EvalCtx si
//!   nécessaire), `MDY(m,d,y)` (invalide → `.` + note), `YEAR`, `MONTH`,
//!   `DAY`, `WEEKDAY` (dimanche=1).
//!
//! Conversion :
//! - `INPUT(s, informat)` / `PUT(v, format)` → DÉLÉGUER au moteur
//!   formats/ (M4) ; M1-M3 : non disponibles (None).
//!
//! Arguments num : utiliser les helpers de coercition d'eval.rs (un char
//! passé à ABS déclenche la conversion automatique char→num).
//!
//! ## Tests
//! Table-driven : (nom, args, résultat attendu) — une trentaine de cas,
//! dont missings : `SUM(., .)` → `.`, `SUM(., 1)` → 1, `MEAN(1,.,3)` → 2.

use std::collections::HashMap;
use std::sync::LazyLock;

use super::eval::EvalCtx;
use crate::value::Value;

mod char;
mod datetime;
mod distributions;
mod math;
pub(crate) mod prx;
mod random;
mod stat;
#[cfg(test)]
mod tests;

use self::char::*;
use self::datetime::*;
use self::distributions::*;
use self::math::*;
use self::prx::{fn_prxchange, fn_prxmatch, fn_prxparen, fn_prxparse, fn_prxposn};
pub use self::random::streaminit_seed;
use self::random::*;
use self::stat::*;

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Coerce a Value to f64 for numeric functions.
/// Returns None if missing; char values are parsed (blank/invalid → None + ctx flag).
fn coerce_num(v: &Value, ctx: &mut EvalCtx) -> Option<f64> {
    match v {
        Value::Num(f) => Some(*f),
        Value::Missing(_) => None,
        Value::Char(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                ctx.missing_generated += 1;
                None
            } else {
                match trimmed.parse::<f64>() {
                    Ok(f) => Some(f),
                    Err(_) => {
                        ctx.invalid_data += 1;
                        ctx.error_flag = true;
                        None
                    }
                }
            }
        }
    }
}

/// Combinator for unary numeric functions: missing/absent arg propagates as
/// missing, otherwise applies `f` to the coerced number.
fn unary_num(args: &[Value], ctx: &mut EvalCtx, f: impl Fn(f64) -> f64) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => Value::Num(f(x)),
        },
    }
}

/// Combinator for unary numeric functions with a domain check: `f` returns
/// `None` when the argument is out of domain, which yields missing with the
/// standard error effects (`error_flag` set, then `invalid_data` bumped).
fn unary_num_checked(args: &[Value], ctx: &mut EvalCtx, f: impl Fn(f64) -> Option<f64>) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => match f(x) {
                None => {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    Value::missing()
                }
                Some(r) => Value::Num(r),
            },
        },
    }
}

/// Coerce a Value to String.
fn coerce_char(v: &Value) -> String {
    match v {
        Value::Char(s) => s.clone(),
        Value::Num(f) => {
            // BEST12. format: right-justified in 12 chars (SAS behaviour for ||).
            // For string functions, just give the raw representation.
            crate::value::format_best(*f, 12)
        }
        Value::Missing(_) => String::new(),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Conversion functions (PUT / INPUT) — délèguent au moteur formats/ (M4)
// ──────────────────────────────────────────────────────────────────────────────

/// `PUT(value, format)` : applique un format à une valeur, renvoie TOUJOURS
/// du caractère. Le second argument est le token de format (poussé en
/// `Value::Char` par le parser). Format invalide ou args manquants → chaîne
/// vide.
fn fn_put(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() != 2 {
        return Value::Char(String::new());
    }
    let token = match &args[1] {
        Value::Char(s) => s.clone(),
        _ => return Value::Char(String::new()),
    };
    let Some(spec) = crate::formats::FormatSpec::parse(&token) else {
        return Value::Char(String::new());
    };
    // Use the session's format catalog so user-defined formats (PROC FORMAT) are resolved.
    let result = ctx.format_catalog.format(&args[0], &spec);
    Value::Char(result)
}

/// `INPUT(source, informat)` : lit une chaîne selon un informat, renvoie un
/// numérique ou un caractère selon l'informat. Le second argument est le
/// token d'informat (poussé en `Value::Char` par le parser). Informat
/// invalide ou args manquants → missing.
fn fn_input(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() != 2 {
        return Value::missing();
    }
    let source = coerce_char(&args[0]);
    let token = match &args[1] {
        Value::Char(s) => s.clone(),
        _ => return Value::missing(),
    };
    let Some(spec) = crate::formats::FormatSpec::parse(&token) else {
        return Value::missing();
    };
    // Use the session's format catalog so user-defined informats (PROC FORMAT INVALUE) are resolved.
    ctx.format_catalog.informat(&source, &spec)
}

/// SYMGET (M11.5) : `symget('name')` lit la valeur de la variable macro
/// `name` (insensible casse) dans l'INSTANTANÉ pris au début de l'étape
/// (`ctx.macro_symbols`). Renvoie une valeur CARACTÈRE ; variable inconnue
/// → missing caractère (chaîne vide). Sous le build par défaut l'instantané
/// est vide, donc toujours missing — `symget` reste appelable sans effet.
fn fn_symget(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() != 1 {
        return Value::Char(String::new());
    }
    let name = coerce_char(&args[0]);
    let key = name.trim().to_uppercase();
    match ctx.macro_symbols.get(&key) {
        Some(v) => Value::Char(v.clone()),
        None => Value::Char(String::new()),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Dispatch table
// ──────────────────────────────────────────────────────────────────────────────

type SasFn = fn(&[Value], &mut EvalCtx) -> Value;

/// Static dispatch table: (UPPERCASE_NAME, function_pointer).
static DISPATCH: &[(&str, SasFn)] = &[
    // Statistical
    ("SUM", fn_sum),
    ("MEAN", fn_mean),
    ("MIN", fn_min),
    ("MAX", fn_max),
    ("N", fn_n),
    ("NMISS", fn_nmiss),
    ("COALESCE", fn_coalesce),
    ("MISSING", fn_missing),
    // Math
    ("ABS", fn_abs),
    ("SQRT", fn_sqrt),
    ("EXP", fn_exp),
    ("LOG", fn_log),
    ("LOG2", fn_log2),
    ("LOG10", fn_log10),
    ("INT", fn_int),
    ("ROUND", fn_round),
    ("MOD", fn_mod),
    // Math functions M15.2
    ("CEIL", fn_ceil),
    ("FLOOR", fn_floor),
    ("SIGN", fn_sign),
    ("SIN", fn_sin),
    ("COS", fn_cos),
    ("TAN", fn_tan),
    ("ARSIN", fn_arsin),
    ("ARCOS", fn_arcos),
    ("ATAN", fn_atan),
    ("ATAN2", fn_atan2),
    ("SINH", fn_sinh),
    ("COSH", fn_cosh),
    ("TANH", fn_tanh),
    ("FACT", fn_fact),
    ("COMB", fn_comb),
    ("PERM", fn_perm),
    ("GAMMA", fn_gamma),
    ("LGAMMA", fn_lgamma),
    ("DIGAMMA", fn_digamma),
    ("TRIGAMMA", fn_trigamma),
    ("BETA", fn_beta),
    ("ROUNDZ", fn_roundz),
    ("RANGE", fn_range),
    ("LARGEST", fn_largest),
    ("SMALLEST", fn_smallest),
    ("ORDINAL", fn_ordinal),
    // Character
    ("UPCASE", fn_upcase),
    ("LOWCASE", fn_lowcase),
    ("TRIM", fn_trim),
    ("STRIP", fn_strip),
    ("LEFT", fn_left),
    ("LENGTH", fn_length),
    ("SUBSTR", fn_substr),
    ("INDEX", fn_index),
    ("CAT", fn_cat),
    ("CATS", fn_cats),
    ("CATX", fn_catx),
    ("COMPRESS", fn_compress),
    ("TRANWRD", fn_tranwrd),
    ("SCAN", fn_scan),
    // Character functions M15.1
    ("FIND", fn_find),
    ("FINDC", fn_findc),
    ("COUNT", fn_count),
    ("COUNTC", fn_countc),
    ("VERIFY", fn_verify),
    ("TRANSLATE", fn_translate),
    ("REVERSE", fn_reverse),
    ("REPEAT", fn_repeat),
    ("PROPCASE", fn_propcase),
    ("COMPBL", fn_compbl),
    ("SUBSTRN", fn_substrn),
    ("CHAR", fn_char),
    ("RANK", fn_rank),
    ("BYTE", fn_byte),
    ("WHICHC", fn_whichc),
    ("CATQ", fn_catq),
    // Date
    ("TODAY", fn_today),
    ("DATE", fn_today), // DATE() is an alias for TODAY()
    ("MDY", fn_mdy),
    ("YEAR", fn_year),
    ("MONTH", fn_month),
    ("DAY", fn_day),
    ("WEEKDAY", fn_weekday),
    ("INTCK", fn_intck),
    ("INTNX", fn_intnx),
    // Date/time functions M15.3 (alphabetical)
    ("DATDIF", fn_datdif),
    ("DATEJUL", fn_datejul),
    ("DATEPART", fn_datepart),
    ("DATETIME", fn_datetime_combine),
    ("DHMS", fn_dhms),
    ("HMS", fn_hms),
    ("HOUR", fn_hour),
    ("JULDATE", fn_juldate),
    ("MINUTE", fn_minute),
    ("NLDATE", fn_nldate),
    ("SECOND", fn_second),
    ("TIMEPART", fn_timepart),
    ("YRDIF", fn_yrdif),
    // Conversion (PUT/INPUT) — délèguent au moteur de formats (M4).
    ("PUT", fn_put),
    ("INPUT", fn_input),
    // Probability distributions M15.4
    ("PROBNORM", fn_probnorm),
    ("PROBT", fn_probt),
    ("PROBF", fn_probf),
    ("PROBCHI", fn_probchi),
    ("PROBBETA", fn_probbeta),
    ("PROBGAM", fn_probgam),
    ("PROBBNML", fn_probbnml),
    ("POISSON", fn_poisson),
    ("CDF", fn_cdf),
    ("PDF", fn_pdf),
    ("QUANTILE", fn_quantile),
    ("SDF", fn_sdf),
    ("LOGCDF", fn_logcdf),
    // Perl regular expressions (M40.1) — table de patterns dans EvalCtx.prx.
    ("PRXPARSE", fn_prxparse),
    ("PRXMATCH", fn_prxmatch),
    ("PRXCHANGE", fn_prxchange),
    ("PRXPOSN", fn_prxposn),
    ("PRXPAREN", fn_prxparen),
    // Macro bridge (M11.5) — lit l'instantané de la table macro.
    ("SYMGET", fn_symget),
    // Random variate generation (M15.5)
    ("RAND", fn_rand),
    ("RANUNI", fn_ranuni),
    ("RANNOR", fn_rannor),
    ("RANEXP", fn_ranexp),
    ("RANBIN", fn_ranbin),
];

/// Index O(1) construit depuis `DISPATCH`. En cas de clé dupliquée dans la
/// table source, la PREMIÈRE occurrence gagne (`entry().or_insert`), comme
/// avec le scan linéaire historique — voir le test
/// `dispatch_map_matches_linear_scan`.
static DISPATCH_MAP: LazyLock<HashMap<&'static str, SasFn>> = LazyLock::new(|| {
    let mut map = HashMap::with_capacity(DISPATCH.len());
    for (name, f) in DISPATCH {
        map.entry(*name).or_insert(*f);
    }
    map
});

/// Renvoie None si la fonction est inconnue.
pub fn call(name: &str, args: &[Value], ctx: &mut EvalCtx) -> Option<Value> {
    // Une seule normalisation, et AUCUNE allocation quand le nom est déjà
    // en majuscules (pour un nom ASCII sans minuscule, `to_uppercase` est
    // l'identité) — cas des appelants qui passent un nom pré-normalisé.
    let f = if name.is_ascii() && !name.bytes().any(|b| b.is_ascii_lowercase()) {
        DISPATCH_MAP.get(name)?
    } else {
        DISPATCH_MAP.get(name.to_uppercase().as_str())?
    };
    Some(f(args, ctx))
}
