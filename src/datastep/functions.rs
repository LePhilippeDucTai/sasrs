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
//! Math :
//! - `ABS`, `SQRT` (négatif → `.` + invalid note), `EXP`,
//!   `LOG`/`LOG2`/`LOG10` (≤0 → `.` + note), `INT` (troncature vers 0),
//!   `ROUND(x[,unit])` — ATTENTION : round SAS = demi-arrondi loin de
//!   zéro (`(x/unit).round()` Rust fait déjà half-away-from-zero),
//!   `MOD(a,b)` — signe du résultat = signe de a (comme `%` Rust f64).
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
//! Dates (M4 affinera avec les formats) :
//! - `TODAY()`/`DATE()` → jours depuis 1960 (sous --deterministic,
//!   l'exécuteur peut figer la date — passer l'info via EvalCtx si
//!   nécessaire), `MDY(m,d,y)` (invalide → `.` + note), `YEAR`, `MONTH`,
//!   `DAY`, `WEEKDAY` (dimanche=1).
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

use super::eval::EvalCtx;
use crate::value::Value;

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

fn today_sas() -> f64 {
    // Jours depuis 1960-01-01 : jours Unix + offset 1960→1970 (3653,
    // constante partagée avec dataset.rs — l'époque SAS précède Unix).
    let unix_days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86400) as f64)
        .unwrap_or(0.0);
    unix_days + crate::dataset::SAS_EPOCH_OFFSET_DAYS
}

/// Checks if a year/month/day combination is valid.
fn is_valid_date(year: i64, month: i64, day: i64) -> bool {
    if month < 1 || month > 12 || day < 1 {
        return false;
    }
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => return false,
    };
    day <= days_in_month
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Convert year/month/day to SAS date (days since 1960-01-01).
fn ymd_to_sas_date(year: i64, month: i64, day: i64) -> f64 {
    // Use a simple algorithm: count days from 1960-01-01.
    // We convert to days since some epoch via Julian Day Number or similar.
    days_since_1960(year, month, day) as f64
}

fn days_since_1960(year: i64, month: i64, day: i64) -> i64 {
    // Days since 1960-01-01
    // Compute Julian Day Number for both dates and subtract.
    jdn(year, month, day) - jdn(1960, 1, 1)
}

/// Julian Day Number (proleptic Gregorian).
fn jdn(year: i64, month: i64, day: i64) -> i64 {
    let a = (14 - month) / 12;
    let y = year + 4800 - a;
    let m = month + 12 * a - 3;
    day + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045
}

/// Convert SAS date to (year, month, day).
fn sas_date_to_ymd(sas_date: i64) -> (i64, i64, i64) {
    // Convert SAS date (days since 1960-01-01) to calendar date.
    let jd = sas_date + jdn(1960, 1, 1);
    jdn_to_ymd(jd)
}

/// Convert Julian Day Number to (year, month, day).
fn jdn_to_ymd(jd: i64) -> (i64, i64, i64) {
    // Algorithm from https://en.wikipedia.org/wiki/Julian_day
    let l = jd + 68569;
    let n = (4 * l) / 146097;
    let l = l - (146097 * n + 3) / 4;
    let i = (4000 * (l + 1)) / 1461001;
    let l = l - (1461 * i) / 4 + 31;
    let j = (80 * l) / 2447;
    let day = l - (2447 * j) / 80;
    let l = j / 11;
    let month = j + 2 - 12 * l;
    let year = 100 * (n - 49) + i + l;
    (year, month, day)
}

/// Day of week for a SAS date. Returns 1=Sunday, 2=Monday, ..., 7=Saturday.
fn sas_weekday(sas_date: i64) -> i64 {
    // 1960-01-01 was a Friday (=6 in SAS: 1=Sun,...,6=Fri,7=Sat).
    // JDN % 7: 0=Mon,1=Tue,2=Wed,3=Thu,4=Fri,5=Sat,6=Sun
    // 1960-01-01 JDN = 2436935, 2436935 % 7 = 4 => Friday in JDN scheme
    // SAS: Sun=1, Mon=2, Tue=3, Wed=4, Thu=5, Fri=6, Sat=7
    let jd = sas_date + jdn(1960, 1, 1);
    // jd % 7: 0=Mon,1=Tue,2=Wed,3=Thu,4=Fri,5=Sat,6=Sun
    let dow_0mon = ((jd % 7) + 7) % 7; // 0=Mon
    // Convert to SAS: Sun=1 means: Sun(0mon=6) → 1, Mon(0mon=0) → 2, ...
    // SAS_dow = (dow_0mon + 2) % 7 + 1? Let's check:
    // Fri(0mon=4): SAS=6. (4+2)%7+1 = 6+1=7. Wrong.
    // Let's use: Sun(0mon=6)→1, so: (dow_0mon+2)%7 gives 0 for Sun, then +1
    // (6+2)%7=1, 1+1=2. Wrong.
    // Direct mapping: 0mon → SAS: 0→2,1→3,2→4,3→5,4→6,5→7,6→1
    // i.e., SAS = (dow_0mon + 2) % 7 + 1 doesn't work.
    // Simpler: (dow_0mon + 1) % 7 + 1
    // 0→2, 1→3, 2→4, 3→5, 4→6, 5→7, 6→1. Check: 6=Sun→1✓, 4=Fri→6✓
    (dow_0mon + 1) % 7 + 1
}

/// Default SAS SCAN delimiters.
const SAS_SCAN_DELIMS: &str = " .<>()+&!$*);^-/,%|";

// ──────────────────────────────────────────────────────────────────────────────
// Statistical functions (ignore missings)
// ──────────────────────────────────────────────────────────────────────────────

fn fn_sum(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut total = 0.0f64;
    let mut n_valid = 0usize;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            total += f;
            n_valid += 1;
        }
    }
    if n_valid == 0 {
        Value::missing()
    } else {
        Value::Num(total)
    }
}

fn fn_mean(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut total = 0.0f64;
    let mut n_valid = 0usize;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            total += f;
            n_valid += 1;
        }
    }
    if n_valid == 0 {
        Value::missing()
    } else {
        Value::Num(total / n_valid as f64)
    }
}

fn fn_min(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut min_val: Option<f64> = None;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            min_val = Some(match min_val {
                None => f,
                Some(m) => if f < m { f } else { m },
            });
        }
    }
    match min_val {
        None => Value::missing(),
        Some(f) => Value::Num(f),
    }
}

fn fn_max(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut max_val: Option<f64> = None;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            max_val = Some(match max_val {
                None => f,
                Some(m) => if f > m { f } else { m },
            });
        }
    }
    match max_val {
        None => Value::missing(),
        Some(f) => Value::Num(f),
    }
}

fn fn_n(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    let count = args.iter().filter(|a| !a.is_missing()).count();
    Value::Num(count as f64)
}

fn fn_nmiss(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    let count = args.iter().filter(|a| a.is_missing()).count();
    Value::Num(count as f64)
}

fn fn_coalesce(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    for a in args {
        if !a.is_missing() {
            return a.clone();
        }
    }
    Value::missing()
}

fn fn_missing(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if let Some(a) = args.first() {
        Value::Num(if a.is_missing() { 1.0 } else { 0.0 })
    } else {
        Value::missing()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Math functions (propagate missing)
// ──────────────────────────────────────────────────────────────────────────────

fn fn_abs(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(f.abs()),
        },
    }
}

fn fn_sqrt(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                if f < 0.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    Value::missing()
                } else {
                    Value::Num(f.sqrt())
                }
            }
        },
    }
}

fn fn_exp(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(f.exp()),
        },
    }
}

fn fn_log(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                if f <= 0.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    Value::missing()
                } else {
                    Value::Num(f.ln())
                }
            }
        },
    }
}

fn fn_log2(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                if f <= 0.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    Value::missing()
                } else {
                    Value::Num(f.log2())
                }
            }
        },
    }
}

fn fn_log10(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                if f <= 0.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    Value::missing()
                } else {
                    Value::Num(f.log10())
                }
            }
        },
    }
}

fn fn_int(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            // INT truncates toward zero (like Rust's `as i64` cast for in-range values).
            Some(f) => Value::Num(f.trunc()),
        },
    }
}

fn fn_round(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => {
                let unit = if args.len() >= 2 {
                    match coerce_num(&args[1], ctx) {
                        None => return Value::missing(),
                        Some(u) => u,
                    }
                } else {
                    1.0
                };
                if unit == 0.0 {
                    return Value::Num(x);
                }
                // SAS ROUND: half-away-from-zero.
                // (x / unit).round() in Rust already uses half-away-from-zero for f64.
                let rounded = (x / unit).round() * unit;
                Value::Num(rounded)
            }
        },
    }
}

fn fn_mod(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (None, _) | (_, None) => Value::missing(),
        (Some(_), Some(b)) if b == 0.0 => {
            ctx.division_by_zero += 1;
            ctx.error_flag = true;
            Value::missing()
        }
        (Some(a), Some(b)) => {
            // SAS MOD: sign of result = sign of a (same as Rust's `%` for f64).
            Value::Num(a % b)
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Character functions
// ──────────────────────────────────────────────────────────────────────────────

fn fn_upcase(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => Value::Char(coerce_char(v).to_uppercase()),
    }
}

fn fn_lowcase(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => Value::Char(coerce_char(v).to_lowercase()),
    }
}

/// TRIM: remove trailing blanks. A fully-blank string becomes "".
fn fn_trim(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            Value::Char(s.trim_end_matches(' ').to_string())
        }
    }
}

/// STRIP: remove both leading and trailing blanks.
fn fn_strip(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            Value::Char(s.trim().to_string())
        }
    }
}

/// LEFT: remove leading blanks (trim_start).
fn fn_left(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            Value::Char(s.trim_start_matches(' ').to_string())
        }
    }
}

/// LENGTH: length without trailing blanks; minimum 1 even for blank string.
fn fn_length(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Num(1.0),
        Some(v) => {
            let s = coerce_char(v);
            // En caractères, pas en octets — convention de la crate
            // (troncature PDV et longueurs de dataset.rs comptent pareil).
            let trimmed_len = s.trim_end_matches(' ').chars().count();
            Value::Num(trimmed_len.max(1) as f64)
        }
    }
}

/// SUBSTR(s, pos[, len]) — 1-based; out of bounds → "" + _ERROR_.
fn fn_substr(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let chars: Vec<char> = s.chars().collect();
    let slen = chars.len() as i64;

    let pos = match args.get(1) {
        None => {
            ctx.error_flag = true;
            return Value::Char(String::new());
        }
        Some(v) => match coerce_num(v, ctx) {
            None => {
                ctx.error_flag = true;
                return Value::Char(String::new());
            }
            Some(f) => f as i64,
        },
    };

    // pos is 1-based; must be >= 1 and <= length.
    if pos < 1 || pos > slen {
        ctx.error_flag = true;
        ctx.invalid_data += 1;
        return Value::Char(String::new());
    }

    let start = (pos - 1) as usize;

    let end = if let Some(len_v) = args.get(2) {
        match coerce_num(len_v, ctx) {
            None => {
                ctx.error_flag = true;
                return Value::Char(String::new());
            }
            Some(l) => {
                let l = l as i64;
                if l < 0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    return Value::Char(String::new());
                }
                (start + l as usize).min(chars.len())
            }
        }
    } else {
        chars.len()
    };

    let result: String = chars[start..end].iter().collect();
    Value::Char(result)
}

/// INDEX(s, sub) → 1-based position, 0 if not found.
fn fn_index(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let sub = coerce_char(&args[1]);
    if sub.is_empty() {
        return Value::Num(0.0);
    }
    match s.find(&sub as &str) {
        None => Value::Num(0.0),
        // byte offset, but for ASCII this equals char offset.
        // For proper Unicode, count chars.
        Some(byte_pos) => {
            let char_pos = s[..byte_pos].chars().count() + 1;
            Value::Num(char_pos as f64)
        }
    }
}

/// CAT: concatenate all args without modification.
fn fn_cat(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    let mut result = String::new();
    for a in args {
        result.push_str(&coerce_char(a));
    }
    Value::Char(result)
}

/// CATS: strip each arg, then concatenate.
fn fn_cats(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    let mut result = String::new();
    for a in args {
        let s = coerce_char(a);
        result.push_str(s.trim());
    }
    Value::Char(result)
}

/// CATX(sep, ...): strip each arg; skip blank args; join with separator.
fn fn_catx(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let sep = coerce_char(&args[0]);
    let parts: Vec<String> = args[1..]
        .iter()
        .map(|a| coerce_char(a).trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Value::Char(parts.join(&sep))
}

/// COMPRESS(s[, chars]): remove specified chars from s; default removes spaces.
fn fn_compress(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let remove_chars: String = if args.len() >= 2 {
        coerce_char(&args[1])
    } else {
        " ".to_string()
    };
    let result: String = s.chars().filter(|c| !remove_chars.contains(*c)).collect();
    Value::Char(result)
}

/// TRANWRD(s, from, to): replace all occurrences of `from` with `to`.
fn fn_tranwrd(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let from = coerce_char(&args[1]);
    let to = coerce_char(&args[2]);
    if from.is_empty() {
        return Value::Char(s);
    }
    Value::Char(s.replace(&from as &str, &to as &str))
}

/// SCAN(s, n[, delims]): return nth word; n<0 means from end.
/// Default delimiters: " .<>()+&!$*);^-/,%|"
fn fn_scan(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let n = match coerce_num(&args[1], ctx) {
        None => return Value::Char(String::new()),
        Some(f) => f as i64,
    };
    if n == 0 {
        return Value::Char(String::new());
    }
    let delims: String = if args.len() >= 3 {
        coerce_char(&args[2])
    } else {
        SAS_SCAN_DELIMS.to_string()
    };

    // Split s into words (tokens between delimiter chars).
    let words: Vec<&str> = s
        .split(|c: char| delims.contains(c))
        .filter(|w| !w.is_empty())
        .collect();

    let idx = if n > 0 {
        n as usize - 1
    } else {
        // n < 0: count from end
        let abs_n = (-n) as usize;
        if abs_n > words.len() {
            return Value::Char(String::new());
        }
        words.len() - abs_n
    };

    match words.get(idx) {
        None => Value::Char(String::new()),
        Some(w) => Value::Char(w.to_string()),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Character functions — lot M15.1
// ──────────────────────────────────────────────────────────────────────────────

/// Modificateurs partagés par FIND/FINDC/COUNT/COUNTC.
/// `'i'` → comparaison insensible à la casse ; `'t'` → trim (blancs finaux
/// retirés de la chaîne ET de l'ensemble de caractères/sous-chaîne).
struct CharMods {
    ignore_case: bool,
    trim: bool,
}

fn parse_char_mods(s: &str) -> CharMods {
    let mut m = CharMods {
        ignore_case: false,
        trim: false,
    };
    for c in s.chars() {
        match c.to_ascii_lowercase() {
            'i' => m.ignore_case = true,
            't' => m.trim = true,
            _ => {}
        }
    }
    m
}

/// Distingue un argument modificateur (uniquement des lettres i/t/o, ou
/// blanc) d'un argument numérique de position : SAS examine le type, mais
/// nos `Value` peuvent être ambigus. On considère char composé de mods
/// connus comme modificateur, et tout Value::Num comme position.
fn is_mods_value(v: &Value) -> bool {
    match v {
        Value::Char(s) => s
            .chars()
            .all(|c| matches!(c.to_ascii_lowercase(), 'i' | 't' | 'o' | ' ')),
        _ => false,
    }
}

/// FIND(s, sub [, mods] [, start]) — 1-based, 0 si absent.
/// Les arguments optionnels mods (char) et start (num) peuvent venir dans
/// n'importe quel ordre après `sub`.
fn fn_find(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let mut mods = CharMods {
        ignore_case: false,
        trim: false,
    };
    let mut start: i64 = 1;
    for a in &args[2..] {
        if is_mods_value(a) {
            mods = parse_char_mods(&coerce_char(a));
        } else if let Some(f) = coerce_num(a, ctx) {
            start = f as i64;
        }
    }
    let mut s = coerce_char(&args[0]);
    let mut sub = coerce_char(&args[1]);
    if mods.trim {
        s = s.trim_end_matches(' ').to_string();
        sub = sub.trim_end_matches(' ').to_string();
    }
    find_impl(&s, &sub, mods.ignore_case, start)
}

/// Implémentation partagée d'une recherche de sous-chaîne 1-based avec
/// point de départ (négatif = recherche vers l'arrière depuis |start|).
fn find_impl(s: &str, sub: &str, ignore_case: bool, start: i64) -> Value {
    let chars: Vec<char> = s.chars().collect();
    let sub_chars: Vec<char> = sub.chars().collect();
    if sub_chars.is_empty() {
        return Value::Num(0.0);
    }
    let n = chars.len();
    let sl = sub_chars.len();
    let eq = |a: char, b: char| {
        if ignore_case {
            a.eq_ignore_ascii_case(&b)
        } else {
            a == b
        }
    };
    let matches_at = |i: usize| -> bool {
        if i + sl > n {
            return false;
        }
        (0..sl).all(|k| eq(chars[i + k], sub_chars[k]))
    };
    if start >= 0 {
        let from = if start <= 1 { 0 } else { (start - 1) as usize };
        for i in from..=n.saturating_sub(sl) {
            if i + sl <= n && matches_at(i) {
                return Value::Num((i + 1) as f64);
            }
        }
    } else {
        // start négatif : recherche vers l'arrière à partir de |start|.
        let from = ((-start) as usize).min(n);
        let begin = from.saturating_sub(1).min(n.saturating_sub(sl).max(0));
        let mut i = begin as i64;
        while i >= 0 {
            if matches_at(i as usize) {
                return Value::Num((i + 1) as f64);
            }
            i -= 1;
        }
    }
    Value::Num(0.0)
}

/// FINDC(s, chars [, mods] [, start]) — position du 1er caractère de `s`
/// présent dans l'ensemble `chars`. Modificateur `'v'` (inversé : 1er
/// caractère ABSENT) géré aussi. 0 si rien trouvé.
fn fn_findc(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let mut ignore_case = false;
    let mut trim = false;
    let mut invert = false;
    let mut start: i64 = 1;
    for a in &args[2..] {
        match a {
            Value::Char(s)
                if s.chars().all(|c| {
                    matches!(c.to_ascii_lowercase(), 'i' | 't' | 'o' | 'v' | ' ')
                }) =>
            {
                for c in s.chars() {
                    match c.to_ascii_lowercase() {
                        'i' => ignore_case = true,
                        't' => trim = true,
                        'v' => invert = true,
                        _ => {}
                    }
                }
            }
            _ => {
                if let Some(f) = coerce_num(a, ctx) {
                    start = f as i64;
                }
            }
        }
    }
    let mut s = coerce_char(&args[0]);
    let mut set = coerce_char(&args[1]);
    if trim {
        s = s.trim_end_matches(' ').to_string();
        set = set.trim_end_matches(' ').to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let in_set = |c: char| -> bool {
        if ignore_case {
            set.chars().any(|x| x.eq_ignore_ascii_case(&c))
        } else {
            set.contains(c)
        }
    };
    let n = chars.len();
    let fwd = start >= 0;
    let from = if fwd {
        if start <= 1 { 0 } else { (start - 1) as usize }
    } else {
        ((-start) as usize).saturating_sub(1).min(n.saturating_sub(1))
    };
    if fwd {
        for i in from..n {
            if in_set(chars[i]) != invert {
                return Value::Num((i + 1) as f64);
            }
        }
    } else if n > 0 {
        let mut i = from as i64;
        while i >= 0 {
            if in_set(chars[i as usize]) != invert {
                return Value::Num((i + 1) as f64);
            }
            i -= 1;
        }
    }
    Value::Num(0.0)
}

/// COUNT(s, sub [, mods]) — nombre d'occurrences non chevauchantes.
fn fn_count(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let mods = if args.len() >= 3 {
        parse_char_mods(&coerce_char(&args[2]))
    } else {
        CharMods { ignore_case: false, trim: false }
    };
    let mut s = coerce_char(&args[0]);
    let mut sub = coerce_char(&args[1]);
    if mods.trim {
        s = s.trim_end_matches(' ').to_string();
        sub = sub.trim_end_matches(' ').to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let sub_chars: Vec<char> = sub.chars().collect();
    if sub_chars.is_empty() {
        return Value::Num(0.0);
    }
    let n = chars.len();
    let sl = sub_chars.len();
    let eq = |a: char, b: char| {
        if mods.ignore_case {
            a.eq_ignore_ascii_case(&b)
        } else {
            a == b
        }
    };
    let mut count = 0u32;
    let mut i = 0usize;
    while i + sl <= n {
        if (0..sl).all(|k| eq(chars[i + k], sub_chars[k])) {
            count += 1;
            i += sl;
        } else {
            i += 1;
        }
    }
    Value::Num(count as f64)
}

/// COUNTC(s, chars [, mods]) — nombre de caractères de `s` présents dans
/// l'ensemble `chars`. Modificateur `'v'` (compte les ABSENTS).
fn fn_countc(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let mut ignore_case = false;
    let mut trim = false;
    let mut invert = false;
    if args.len() >= 3 {
        for c in coerce_char(&args[2]).chars() {
            match c.to_ascii_lowercase() {
                'i' => ignore_case = true,
                't' => trim = true,
                'v' => invert = true,
                _ => {}
            }
        }
    }
    let mut s = coerce_char(&args[0]);
    let mut set = coerce_char(&args[1]);
    if trim {
        s = s.trim_end_matches(' ').to_string();
        set = set.trim_end_matches(' ').to_string();
    }
    let in_set = |c: char| -> bool {
        if ignore_case {
            set.chars().any(|x| x.eq_ignore_ascii_case(&c))
        } else {
            set.contains(c)
        }
    };
    let count = s.chars().filter(|&c| in_set(c) != invert).count();
    Value::Num(count as f64)
}

/// VERIFY(s, chars) — position du 1er caractère de `s` ABSENT de
/// l'ensemble `chars` ; 0 si tous présents.
fn fn_verify(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    // VERIFY accepte plusieurs ensembles (concaténés).
    let mut set = String::new();
    for a in &args[1..] {
        set.push_str(&coerce_char(a));
    }
    for (i, c) in s.chars().enumerate() {
        if !set.contains(c) {
            return Value::Num((i + 1) as f64);
        }
    }
    Value::Num(0.0)
}

/// TRANSLATE(s, to, from) — remplace chaque caractère de `from` par le
/// caractère de même rang dans `to`. Caractères de `from` sans
/// correspondance dans `to` sont supprimés (comportement SAS : si `to`
/// plus court, le caractère est laissé inchangé en SAS 9.4 — on conserve
/// donc le caractère original).
fn fn_translate(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        return Value::Char(if args.is_empty() {
            String::new()
        } else {
            coerce_char(&args[0])
        });
    }
    let s = coerce_char(&args[0]);
    let to: Vec<char> = coerce_char(&args[1]).chars().collect();
    let from: Vec<char> = coerce_char(&args[2]).chars().collect();
    let result: String = s
        .chars()
        .map(|c| match from.iter().position(|&f| f == c) {
            Some(idx) => to.get(idx).copied().unwrap_or(c),
            None => c,
        })
        .collect();
    Value::Char(result)
}

/// REVERSE(s) — inverse l'ordre des caractères (blancs inclus).
fn fn_reverse(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => Value::Char(coerce_char(v).chars().rev().collect()),
    }
}

/// REPEAT(s, n) — renvoie `s` répété n+1 fois (piège SAS : n est le nombre
/// de répétitions SUPPLÉMENTAIRES, donc n+1 copies au total).
fn fn_repeat(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let n = match args.get(1) {
        None => 0,
        Some(v) => match coerce_num(v, ctx) {
            None => 0,
            Some(f) => f as i64,
        },
    };
    if n < 0 {
        return Value::Char(s);
    }
    let copies = (n + 1) as usize;
    Value::Char(s.repeat(copies))
}

/// PROPCASE(s [, delims]) — met en majuscule la 1re lettre de chaque mot,
/// le reste en minuscule. Délimiteurs par défaut : blanc et quelques
/// ponctuations courantes.
fn fn_propcase(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let delims: String = if args.len() >= 2 {
        coerce_char(&args[1])
    } else {
        " \t\r\n-/".to_string()
    };
    let mut result = String::with_capacity(s.len());
    let mut at_word_start = true;
    for c in s.chars() {
        if delims.contains(c) {
            result.push(c);
            at_word_start = true;
        } else if at_word_start {
            result.extend(c.to_uppercase());
            at_word_start = false;
        } else {
            result.extend(c.to_lowercase());
        }
    }
    Value::Char(result)
}

/// COMPBL(s) — réduit toute suite de blancs consécutifs à un seul espace.
fn fn_compbl(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            let mut result = String::with_capacity(s.len());
            let mut prev_space = false;
            for c in s.chars() {
                if c == ' ' {
                    if !prev_space {
                        result.push(' ');
                    }
                    prev_space = true;
                } else {
                    result.push(c);
                    prev_space = false;
                }
            }
            Value::Char(result)
        }
    }
}

/// SUBSTRN(s, pos [, len]) — comme SUBSTR mais TOLÈRE pos/len négatifs ou
/// hors borne sans erreur : la portion hors de [1, len(s)] est ignorée et
/// le résultat peut être vide.
fn fn_substrn(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let chars: Vec<char> = s.chars().collect();
    let slen = chars.len() as i64;
    let pos = match args.get(1) {
        None => 1,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::Char(String::new()),
            Some(f) => f.floor() as i64,
        },
    };
    let len = match args.get(2) {
        None => slen - pos + 1,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::Char(String::new()),
            Some(f) => f.floor() as i64,
        },
    };
    // Intervalle demandé [pos, pos+len) intersecté avec [1, slen].
    let req_start = pos;
    let req_end = pos + len; // exclusif
    let lo = req_start.max(1);
    let hi = req_end.min(slen + 1);
    if hi <= lo {
        return Value::Char(String::new());
    }
    let start = (lo - 1) as usize;
    let end = (hi - 1) as usize;
    Value::Char(chars[start..end].iter().collect())
}

/// CHAR(s, n) — n-ième caractère (1-based) ; blanc si hors borne.
fn fn_char(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let n = match coerce_num(&args[1], ctx) {
        None => return Value::Char(" ".to_string()),
        Some(f) => f as i64,
    };
    if n < 1 {
        return Value::Char(" ".to_string());
    }
    match s.chars().nth((n - 1) as usize) {
        Some(c) => Value::Char(c.to_string()),
        None => Value::Char(" ".to_string()),
    }
}

/// RANK(c) — code ASCII (position) du 1er caractère de `c`.
fn fn_rank(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => {
            let s = coerce_char(v);
            match s.chars().next() {
                Some(c) => Value::Num(c as u32 as f64),
                None => Value::missing(),
            }
        }
    }
}

/// BYTE(n) — caractère dont le code ASCII/Latin-1 est `n`.
fn fn_byte(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::Char(String::new()),
            Some(f) => {
                let code = f as u32;
                match char::from_u32(code) {
                    Some(c) => Value::Char(c.to_string()),
                    None => Value::Char(String::new()),
                }
            }
        },
    }
}

/// WHICHC(x, c1, c2, ...) — index (1-based) du 1er argument char égal à
/// `x` (comparaison ignorant les blancs finaux) ; 0 si aucun.
fn fn_whichc(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Num(0.0);
    }
    let target = coerce_char(&args[0]);
    let target = target.trim_end_matches(' ');
    for (i, a) in args[1..].iter().enumerate() {
        let s = coerce_char(a);
        if s.trim_end_matches(' ') == target {
            return Value::Num((i + 1) as f64);
        }
    }
    Value::Num(0.0)
}

/// CATQ([mods,] [delim,] item1, item2, ...) — concatène avec un
/// délimiteur, en entourant de guillemets les items contenant le
/// délimiteur ou un blanc. Version simplifiée : 1er arg = modificateurs
/// (char contenant uniquement des lettres de mods connus) optionnel ;
/// arg suivant = délimiteur si char ; défaut délimiteur = espace.
fn fn_catq(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let mut idx = 0usize;
    // Modificateurs optionnels.
    let mut strip = true; // CATQ strippe par défaut (proche de CATS)
    if let Some(Value::Char(s)) = args.first() {
        let is_mods = !s.is_empty()
            && s.chars().all(|c| {
                matches!(c.to_ascii_lowercase(), 'a' | 'b' | 'c' | 'd' | 'h' | 'm' | 'n' | 'o' | 'p' | 'q' | 's' | 't' | '1' | '2' | '3' | ' ')
            });
        if is_mods && args.len() > 1 {
            for c in s.chars() {
                if c.to_ascii_lowercase() == 't' {
                    strip = true;
                }
            }
            idx = 1;
        }
    }
    // Délimiteur : par défaut un espace. CATQ utilise un délimiteur fixe
    // (espace) sauf si le mod 'd' précise un délimiteur dans l'arg suivant.
    let delim = " ".to_string();
    let mut parts: Vec<String> = Vec::new();
    for a in &args[idx..] {
        let mut s = coerce_char(a);
        if strip {
            s = s.trim().to_string();
        }
        // Quote si l'item contient le délimiteur ou un blanc.
        if s.contains(&delim) || s.contains(' ') || s.contains('"') {
            let escaped = s.replace('"', "\"\"");
            parts.push(format!("\"{}\"", escaped));
        } else {
            parts.push(s);
        }
    }
    Value::Char(parts.join(&delim))
}

// ──────────────────────────────────────────────────────────────────────────────
// Math functions — lot M15.2
// ──────────────────────────────────────────────────────────────────────────────

/// Applique une fonction unaire `f` à l'argument numérique en propageant le
/// missing (arg manquant → résultat manquant ; pas d'argument → manquant).
fn unary_num(args: &[Value], ctx: &mut EvalCtx, f: impl Fn(f64) -> f64) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => Value::Num(f(x)),
        },
    }
}

/// Comme `unary_num` mais le domaine est validé : si `domain_ok(x)` est faux,
/// renvoie missing et incrémente `ctx.invalid_data` + `ctx.error_flag` (style
/// SQRT/LOG existant).
fn unary_num_domain(
    args: &[Value],
    ctx: &mut EvalCtx,
    domain_ok: impl Fn(f64) -> bool,
    f: impl Fn(f64) -> f64,
) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => {
                if domain_ok(x) {
                    Value::Num(f(x))
                } else {
                    ctx.invalid_data += 1;
                    ctx.error_flag = true;
                    Value::missing()
                }
            }
        },
    }
}

fn fn_ceil(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |x| x.ceil())
}

fn fn_floor(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |x| x.floor())
}

/// SIGN(x) → -1, 0 ou 1 (SAS : SIGN(0)=0, signe du non-nul sinon).
fn fn_sign(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |x| {
        if x > 0.0 {
            1.0
        } else if x < 0.0 {
            -1.0
        } else {
            0.0
        }
    })
}

fn fn_sin(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |x| x.sin())
}

fn fn_cos(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |x| x.cos())
}

fn fn_tan(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |x| x.tan())
}

/// ARSIN(x) — domaine [-1, 1].
fn fn_arsin(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_domain(args, ctx, |x| (-1.0..=1.0).contains(&x), |x| x.asin())
}

/// ARCOS(x) — domaine [-1, 1].
fn fn_arcos(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_domain(args, ctx, |x| (-1.0..=1.0).contains(&x), |x| x.acos())
}

fn fn_atan(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |x| x.atan())
}

/// ATAN2(y, x) — arc-tangente à deux arguments (manquant propagé).
fn fn_atan2(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (Some(y), Some(x)) => Value::Num(y.atan2(x)),
        _ => Value::missing(),
    }
}

fn fn_sinh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |x| x.sinh())
}

fn fn_cosh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |x| x.cosh())
}

fn fn_tanh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |x| x.tanh())
}

/// FACT(n) — factorielle. n doit être un entier ≥ 0 ; sinon missing + erreur.
fn fn_fact(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => {
                // n doit être un entier non négatif.
                if x < 0.0 || x.fract() != 0.0 {
                    ctx.invalid_data += 1;
                    ctx.error_flag = true;
                    return Value::missing();
                }
                let n = x as u64;
                // Au-delà de 170!, dépassement f64 → +inf (comme SAS qui pose
                // une erreur d'overflow). On borne pour rester déterministe.
                if n > 170 {
                    ctx.invalid_data += 1;
                    ctx.error_flag = true;
                    return Value::missing();
                }
                let mut acc = 1.0f64;
                for k in 2..=n {
                    acc *= k as f64;
                }
                Value::Num(acc)
            }
        },
    }
}

/// COMB(n, k) — nombre de combinaisons C(n, k) = n! / (k!(n-k)!).
/// Calcul multiplicatif stable (évite l'overflow des factorielles).
fn fn_comb(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    let (n, k) = match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (Some(n), Some(k)) => (n, k),
        _ => return Value::missing(),
    };
    if n < 0.0 || k < 0.0 || n.fract() != 0.0 || k.fract() != 0.0 {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    let n = n as i64;
    let k = k as i64;
    if k > n {
        return Value::Num(0.0);
    }
    // C(n, k) symétrique : prendre le plus petit k.
    let k = k.min(n - k);
    let mut acc = 1.0f64;
    for i in 0..k {
        acc = acc * (n - i) as f64 / (i + 1) as f64;
    }
    Value::Num(acc.round())
}

/// PERM(n[, k]) — arrangements P(n, k) = n! / (n-k)! ; PERM(n) = n!.
fn fn_perm(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let n = match args.first() {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(x) => x,
        },
    };
    let k = match args.get(1) {
        None => n,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(x) => x,
        },
    };
    if n < 0.0 || k < 0.0 || n.fract() != 0.0 || k.fract() != 0.0 {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    let n = n as i64;
    let k = k as i64;
    if k > n {
        // SAS : k > n → erreur.
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    let mut acc = 1.0f64;
    for i in 0..k {
        acc *= (n - i) as f64;
    }
    Value::Num(acc)
}

/// Coefficients de Lanczos (g = 7, n = 9) pour l'approximation de la fonction
/// gamma. Déterministe, pas de dépendance externe.
const LANCZOS_G: f64 = 7.0;
const LANCZOS_COEF: [f64; 9] = [
    0.999_999_999_999_809_93,
    676.520_368_121_885_1,
    -1_259.139_216_722_402_8,
    771.323_428_777_653_1,
    -176.615_029_162_140_6,
    12.507_343_278_686_905,
    -0.138_571_095_265_720_12,
    9.984_369_578_019_572e-6,
    1.505_632_735_149_311_6e-7,
];

/// ln(Γ(x)) par l'approximation de Lanczos (gère x ≤ 0 via la réflexion).
fn lgamma_lanczos(x: f64) -> f64 {
    use std::f64::consts::PI;
    if x < 0.5 {
        // Réflexion : Γ(x)Γ(1-x) = π / sin(πx).
        let log_sin = (PI * x).sin().abs().ln();
        (PI).ln() - log_sin - lgamma_lanczos(1.0 - x)
    } else {
        let x = x - 1.0;
        let mut a = LANCZOS_COEF[0];
        let t = x + LANCZOS_G + 0.5;
        for (i, &c) in LANCZOS_COEF.iter().enumerate().skip(1) {
            a += c / (x + i as f64);
        }
        0.5 * (2.0 * PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
    }
}

/// Γ(x) par Lanczos. Reflète pour x < 0.5.
fn gamma_lanczos(x: f64) -> f64 {
    use std::f64::consts::PI;
    if x < 0.5 {
        PI / ((PI * x).sin() * gamma_lanczos(1.0 - x))
    } else {
        let x = x - 1.0;
        let mut a = LANCZOS_COEF[0];
        let t = x + LANCZOS_G + 0.5;
        for (i, &c) in LANCZOS_COEF.iter().enumerate().skip(1) {
            a += c / (x + i as f64);
        }
        (2.0 * PI).sqrt() * t.powf(x + 0.5) * (-t).exp() * a
    }
}

/// GAMMA(x) — fonction gamma. Pôles aux entiers ≤ 0 → missing + erreur.
fn fn_gamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_domain(
        args,
        ctx,
        |x| !(x <= 0.0 && x.fract() == 0.0),
        gamma_lanczos,
    )
}

/// LGAMMA(x) — ln de la fonction gamma. SAS exige x > 0.
fn fn_lgamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_domain(args, ctx, |x| x > 0.0, lgamma_lanczos)
}

/// Fonction digamma ψ(x) = d/dx ln Γ(x), via récurrence + série asymptotique.
fn digamma_impl(mut x: f64) -> f64 {
    use std::f64::consts::PI;
    let mut result = 0.0;
    // Réflexion pour x ≤ 0 : ψ(1-x) - ψ(x) = π·cot(πx).
    if x <= 0.0 && x.fract() == 0.0 {
        return f64::NAN; // pôle (filtré en amont)
    }
    if x < 0.0 {
        result -= PI / (PI * x).tan();
        x = 1.0 - x;
    }
    // Récurrence ascendante jusqu'à x ≥ 6 pour la série asymptotique.
    while x < 6.0 {
        result -= 1.0 / x;
        x += 1.0;
    }
    // Série asymptotique : ψ(x) ≈ ln x - 1/(2x) - Σ B_2k/(2k x^{2k}).
    let inv = 1.0 / x;
    let inv2 = inv * inv;
    result += x.ln() - 0.5 * inv
        - inv2
            * (1.0 / 12.0
                - inv2 * (1.0 / 120.0 - inv2 * (1.0 / 252.0 - inv2 / 240.0)));
    result
}

/// DIGAMMA(x) — SAS exige x > 0 (ou non-entier ; ici on borne à x > 0 comme SAS).
fn fn_digamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_domain(args, ctx, |x| x > 0.0, digamma_impl)
}

/// BETA(a, b) = Γ(a)Γ(b)/Γ(a+b). SAS exige a > 0 et b > 0.
fn fn_beta(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    let (a, b) = match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (Some(a), Some(b)) => (a, b),
        _ => return Value::missing(),
    };
    if a <= 0.0 || b <= 0.0 {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    // Via lgamma pour la stabilité numérique.
    let log_beta = lgamma_lanczos(a) + lgamma_lanczos(b) - lgamma_lanczos(a + b);
    Value::Num(log_beta.exp())
}

/// ROUNDZ(x[, u]) — arrondi au multiple de u le plus proche, demi-arrondi vers
/// le PAIR (round-half-even / banker's rounding), contrairement à ROUND.
fn fn_roundz(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => {
                let unit = if args.len() >= 2 {
                    match coerce_num(&args[1], ctx) {
                        None => return Value::missing(),
                        Some(u) => u,
                    }
                } else {
                    1.0
                };
                if unit == 0.0 {
                    return Value::Num(x);
                }
                let scaled = x / unit;
                // round-half-to-even : Rust f64::round_ties_even.
                let rounded = scaled.round_ties_even() * unit;
                Value::Num(rounded)
            }
        },
    }
}

/// RANGE(...) — étendue (max − min) des arguments non manquants. Tous
/// manquants → missing.
fn fn_range(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut lo: Option<f64> = None;
    let mut hi: Option<f64> = None;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            lo = Some(lo.map_or(f, |m| m.min(f)));
            hi = Some(hi.map_or(f, |m| m.max(f)));
        }
    }
    match (lo, hi) {
        (Some(l), Some(h)) => Value::Num(h - l),
        _ => Value::missing(),
    }
}

/// Collecte les valeurs numériques non manquantes des arguments `args`.
fn collect_nonmissing(args: &[Value], ctx: &mut EvalCtx) -> Vec<f64> {
    args.iter().filter_map(|a| coerce_num(a, ctx)).collect()
}

/// LARGEST(k, v1, v2, ...) — k-ième plus grande valeur non manquante.
/// k hors borne → missing.
fn fn_largest(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::missing();
    }
    let k = match coerce_num(&args[0], ctx) {
        None => return Value::missing(),
        Some(f) => f as i64,
    };
    let mut vals = collect_nonmissing(&args[1..], ctx);
    if k < 1 || (k as usize) > vals.len() {
        return Value::missing();
    }
    // Tri décroissant.
    vals.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    Value::Num(vals[(k - 1) as usize])
}

/// SMALLEST(k, v1, v2, ...) — k-ième plus petite valeur non manquante.
fn fn_smallest(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::missing();
    }
    let k = match coerce_num(&args[0], ctx) {
        None => return Value::missing(),
        Some(f) => f as i64,
    };
    let mut vals = collect_nonmissing(&args[1..], ctx);
    if k < 1 || (k as usize) > vals.len() {
        return Value::missing();
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Value::Num(vals[(k - 1) as usize])
}

/// ORDINAL(k, v1, v2, ...) — k-ième plus petite valeur (synonyme de SMALLEST
/// en SAS, sur les valeurs non manquantes).
fn fn_ordinal(args: &[Value], ctx: &mut EvalCtx) -> Value {
    fn_smallest(args, ctx)
}

// ──────────────────────────────────────────────────────────────────────────────
// Date functions
// ──────────────────────────────────────────────────────────────────────────────

fn fn_today(_args: &[Value], ctx: &mut EvalCtx) -> Value {
    // Sous --deterministic, FIGE la date au 01JAN1960 (date SAS 0) pour des
    // snapshots stables ; sinon horloge réelle.
    if ctx.deterministic {
        Value::Num(0.0)
    } else {
        Value::Num(today_sas())
    }
}

fn fn_mdy(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        ctx.invalid_data += 1;
        return Value::missing();
    }
    let m = match coerce_num(&args[0], ctx) {
        None => return Value::missing(),
        Some(f) => f as i64,
    };
    let d = match coerce_num(&args[1], ctx) {
        None => return Value::missing(),
        Some(f) => f as i64,
    };
    let y = match coerce_num(&args[2], ctx) {
        None => return Value::missing(),
        Some(f) => f as i64,
    };
    if !is_valid_date(y, m, d) {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    Value::Num(ymd_to_sas_date(y, m, d))
}

fn fn_year(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                let (year, _, _) = sas_date_to_ymd(f as i64);
                Value::Num(year as f64)
            }
        },
    }
}

fn fn_month(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                let (_, month, _) = sas_date_to_ymd(f as i64);
                Value::Num(month as f64)
            }
        },
    }
}

fn fn_day(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                let (_, _, day) = sas_date_to_ymd(f as i64);
                Value::Num(day as f64)
            }
        },
    }
}

fn fn_weekday(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(sas_weekday(f as i64) as f64),
        },
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Interval date functions : INTCK / INTNX
// ──────────────────────────────────────────────────────────────────────────────

/// Parsed interval keyword (premier argument caractère de INTCK/INTNX).
enum Interval {
    Day,
    Week,
    Month,
    Qtr,
    Year,
}

/// Parse l'intervalle (insensible à la casse, blancs de bord supprimés).
/// Renvoie None pour un intervalle inconnu.
fn parse_interval(v: &Value) -> Option<Interval> {
    let s = match v {
        Value::Char(s) => s.trim().to_uppercase(),
        _ => return None,
    };
    match s.as_str() {
        "DAY" => Some(Interval::Day),
        "WEEK" => Some(Interval::Week),
        "MONTH" => Some(Interval::Month),
        "QTR" | "QUARTER" => Some(Interval::Qtr),
        "YEAR" => Some(Interval::Year),
        _ => None,
    }
}

/// Index de semaine SAS (les semaines commencent le DIMANCHE). Le jour SAS 0
/// (1960-01-01) est un VENDREDI ; le dimanche le plus récent à cette date est
/// le jour -5 (1959-12-27), et le dimanche suivant est le jour 2 (1960-01-03).
/// `floor((d - 2) / 7)` place donc chaque dimanche (… -5, 2, 9 …) sur une
/// frontière. On utilise une division euclidienne pour gérer correctement les
/// jours négatifs.
fn week_index(sas_day: i64) -> i64 {
    (sas_day - 2).div_euclid(7)
}

/// INTCK('interval', from, to) → nombre discret de frontières d'intervalle
/// franchies (méthode "DISCRETE" par défaut de SAS). Intervalle inconnu ou
/// date manquante → missing.
fn fn_intck(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        ctx.invalid_data += 1;
        return Value::missing();
    }
    let Some(interval) = parse_interval(&args[0]) else {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    };
    let from = match coerce_num(&args[1], ctx) {
        None => return Value::missing(),
        Some(f) => f.floor() as i64,
    };
    let to = match coerce_num(&args[2], ctx) {
        None => return Value::missing(),
        Some(f) => f.floor() as i64,
    };
    let (y1, m1, _d1) = sas_date_to_ymd(from);
    let (y2, m2, _d2) = sas_date_to_ymd(to);
    let count = match interval {
        Interval::Day => (to - from) as f64,
        Interval::Week => (week_index(to) - week_index(from)) as f64,
        Interval::Month => ((y2 * 12 + m2) - (y1 * 12 + m1)) as f64,
        Interval::Qtr => {
            let q1 = (m1 - 1) / 3; // 0-based quarter index
            let q2 = (m2 - 1) / 3;
            ((y2 * 4 + q2) - (y1 * 4 + q1)) as f64
        }
        Interval::Year => (y2 - y1) as f64,
    };
    Value::Num(count)
}

/// Alignement de INTNX (4e argument optionnel, défaut BEGINNING).
enum Align {
    Beginning,
    End,
    Same,
    Middle,
}

fn parse_align(v: Option<&Value>) -> Align {
    let s = match v {
        Some(Value::Char(s)) => s.trim().to_uppercase(),
        _ => return Align::Beginning,
    };
    // On matche sur le premier caractère significatif (B/E/S/M).
    match s.chars().next() {
        Some('E') => Align::End,
        Some('S') => Align::Same,
        Some('M') => Align::Middle,
        _ => Align::Beginning, // 'B'/BEG/BEGINNING et tout le reste
    }
}

/// Dernier jour du mois (gère les années bissextiles).
fn last_day_of_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Normalise (year, month) après avoir ajouté des mois (month 1-based).
fn normalize_ym(year: i64, month0: i64) -> (i64, i64) {
    // month0 est 0-based ici pour faciliter l'arithmétique modulaire.
    let y = year + month0.div_euclid(12);
    let m = month0.rem_euclid(12) + 1;
    (y, m)
}

/// INTNX('interval', start, increment [, 'alignment']) → date SAS.
/// Date manquante / intervalle inconnu → missing.
fn fn_intnx(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        ctx.invalid_data += 1;
        return Value::missing();
    }
    let Some(interval) = parse_interval(&args[0]) else {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    };
    let start = match coerce_num(&args[1], ctx) {
        None => return Value::missing(),
        Some(f) => f.floor() as i64,
    };
    let inc = match coerce_num(&args[2], ctx) {
        None => return Value::missing(),
        Some(f) => f.trunc() as i64,
    };
    let align = parse_align(args.get(3));
    let (sy, sm, sd) = sas_date_to_ymd(start);

    let (y, m, d) = match interval {
        Interval::Day => {
            // Période = 1 jour ; alignement sans objet (B=E=S=start+inc).
            return Value::Num((start + inc) as f64);
        }
        Interval::Week => {
            // Période = 7 jours débutant un dimanche.
            // Le dimanche d'index k est le jour 7*k + 2 (cf. week_index :
            // … -5, 2, 9 …). Dimanche de la semaine de `start` :
            let start_sunday = week_index(start) * 7 + 2;
            let target_sunday = start_sunday + inc * 7;
            let day = match align {
                Align::Beginning => target_sunday,
                Align::End => target_sunday + 6,         // samedi
                Align::Same => target_sunday + (start - start_sunday), // même jour de semaine
                Align::Middle => target_sunday + 3,      // milieu : mercredi
            };
            return Value::Num(day as f64);
        }
        Interval::Month => {
            // Période = mois civil. Début de période = (sy, sm, 1).
            let (ny, nm) = normalize_ym(sy, (sm - 1) + inc);
            let last = last_day_of_month(ny, nm);
            let d = match align {
                Align::Beginning => 1,
                Align::End => last,
                Align::Same => sd.min(last),
                Align::Middle => 15,
            };
            (ny, nm, d)
        }
        Interval::Qtr => {
            // Période = trimestre (mois de début 1, 4, 7, 10).
            let q0 = (sm - 1) / 3; // 0-based quarter of start
            let total_q = sy * 4 + q0 + inc;
            let ny = total_q.div_euclid(4);
            let nq = total_q.rem_euclid(4); // 0..3
            let first_month = nq * 3 + 1;
            let d = match align {
                Align::Beginning => (ny, first_month, 1),
                Align::End => {
                    let last_month = first_month + 2;
                    (ny, last_month, last_day_of_month(ny, last_month))
                }
                Align::Same => {
                    // Même offset (mois dans le trimestre + jour) que start.
                    let month_in_q = (sm - 1) % 3; // 0..2
                    let tm = first_month + month_in_q;
                    let last = last_day_of_month(ny, tm);
                    (ny, tm, sd.min(last))
                }
                Align::Middle => {
                    // Milieu du trimestre ≈ 15 du mois central.
                    (ny, first_month + 1, 15)
                }
            };
            d
        }
        Interval::Year => {
            let ny = sy + inc;
            match align {
                Align::Beginning => (ny, 1, 1),
                Align::End => (ny, 12, 31),
                Align::Same => {
                    let last = last_day_of_month(ny, sm);
                    (ny, sm, sd.min(last))
                }
                Align::Middle => (ny, 7, 1),
            }
        }
    };

    Value::Num(days_since_1960(y, m, d) as f64)
}

// ──────────────────────────────────────────────────────────────────────────────
// Date/time functions — lot M15.3
//
// Modèle SAS :
//   date     = jours depuis 1960-01-01
//   datetime = secondes depuis 1960-01-01 00:00:00
//   time     = secondes depuis minuit (0 ≤ t < 86400)
//
// Propagation des manquants : tout argument manquant → résultat manquant,
// sauf sémantique spécifique. Les modulos utilisent `rem_euclid` pour rester
// corrects sur les datetimes NÉGATIFS (avant 1960).
// ──────────────────────────────────────────────────────────────────────────────

/// Nombre de secondes dans une journée.
const SECS_PER_DAY: f64 = 86400.0;

/// `DATEPART(datetime)` → date SAS = floor(dt / 86400).
fn fn_datepart(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(dt) => Value::Num((dt / SECS_PER_DAY).floor()),
        },
    }
}

/// `TIMEPART(datetime)` → time (secondes du jour) = dt mod 86400, modulo
/// euclidien pour rester dans [0, 86400) même pour un datetime négatif.
fn fn_timepart(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(dt) => Value::Num(dt.rem_euclid(SECS_PER_DAY)),
        },
    }
}

/// `DATETIME()` → datetime courant (secondes depuis 1960-01-01 00:00:00).
/// Sous --deterministic, FIGE au datetime 0 (01JAN1960 00:00:00), cohérent
/// avec `TODAY()` figé au 01JAN1960 (date 0 × 86400 = datetime 0).
fn fn_datetime(_args: &[Value], ctx: &mut EvalCtx) -> Value {
    if ctx.deterministic {
        Value::Num(0.0)
    } else {
        let unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as f64)
            .unwrap_or(0.0);
        // datetime SAS = secondes Unix + offset d'époque 1960→1970 en secondes.
        Value::Num(unix_secs + crate::dataset::SAS_EPOCH_OFFSET_DAYS * SECS_PER_DAY)
    }
}

/// `HMS(hour, minute, second)` → time = h*3600 + m*60 + s (secondes).
/// Un argument manquant → missing.
fn fn_hms(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        ctx.invalid_data += 1;
        return Value::missing();
    }
    let h = match coerce_num(&args[0], ctx) {
        None => return Value::missing(),
        Some(f) => f,
    };
    let m = match coerce_num(&args[1], ctx) {
        None => return Value::missing(),
        Some(f) => f,
    };
    let s = match coerce_num(&args[2], ctx) {
        None => return Value::missing(),
        Some(f) => f,
    };
    Value::Num(h * 3600.0 + m * 60.0 + s)
}

/// `DHMS(date, hour, minute, second)` → datetime
/// = date*86400 + h*3600 + m*60 + s. Un argument manquant → missing.
fn fn_dhms(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 4 {
        ctx.invalid_data += 1;
        return Value::missing();
    }
    let date = match coerce_num(&args[0], ctx) {
        None => return Value::missing(),
        Some(f) => f,
    };
    let h = match coerce_num(&args[1], ctx) {
        None => return Value::missing(),
        Some(f) => f,
    };
    let m = match coerce_num(&args[2], ctx) {
        None => return Value::missing(),
        Some(f) => f,
    };
    let s = match coerce_num(&args[3], ctx) {
        None => return Value::missing(),
        Some(f) => f,
    };
    Value::Num(date * SECS_PER_DAY + h * 3600.0 + m * 60.0 + s)
}

/// Composantes horaires. Acceptent un time OU un datetime : on ramène d'abord
/// dans la journée (`rem_euclid` → [0, 86400)) puis on décompose.
fn time_of_day(v: &Value, ctx: &mut EvalCtx) -> Option<i64> {
    coerce_num(v, ctx).map(|f| (f.rem_euclid(SECS_PER_DAY)).floor() as i64)
}

/// `HOUR(time | datetime)` → heure (0..23).
fn fn_hour(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match time_of_day(v, ctx) {
            None => Value::missing(),
            Some(secs) => Value::Num((secs / 3600) as f64),
        },
    }
}

/// `MINUTE(time | datetime)` → minute (0..59).
fn fn_minute(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match time_of_day(v, ctx) {
            None => Value::missing(),
            Some(secs) => Value::Num(((secs % 3600) / 60) as f64),
        },
    }
}

/// `SECOND(time | datetime)` → seconde (0..59). SAS conserve la partie
/// fractionnaire des secondes ; on la récupère via le reste réel modulo 60.
fn fn_second(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                let in_day = f.rem_euclid(SECS_PER_DAY);
                Value::Num(in_day.rem_euclid(60.0))
            }
        },
    }
}

/// Base de calcul de jours/années pour YRDIF / DATDIF.
enum Basis {
    ActAct,
    Act360,
    Act365,
    Thirty360,
    Age,
}

/// Parse la base (insensible casse, blancs supprimés). Défaut ACT/ACT.
fn parse_basis(v: Option<&Value>) -> Option<Basis> {
    let s = match v {
        None => return Some(Basis::ActAct),
        Some(Value::Char(s)) => s.trim().to_uppercase(),
        Some(_) => return None,
    };
    match s.as_str() {
        "" | "ACT/ACT" | "ACTUAL" => Some(Basis::ActAct),
        "ACT/360" => Some(Basis::Act360),
        "ACT/365" => Some(Basis::Act365),
        "30/360" | "360" => Some(Basis::Thirty360),
        "AGE" => Some(Basis::Age),
        _ => None,
    }
}

/// Nombre de jours selon la convention US (NASD) 30/360.
/// Règle : si d1 = 31 → d1 = 30 ; si d2 = 31 ET d1 = 30 (après ajustement) → d2 = 30.
/// jours = (y2-y1)*360 + (m2-m1)*30 + (d2-d1).
fn days_30_360(start: i64, end: i64) -> i64 {
    let (y1, m1, d1) = sas_date_to_ymd(start);
    let (y2, m2, d2) = sas_date_to_ymd(end);
    let mut d1 = d1;
    let mut d2 = d2;
    if d1 == 31 {
        d1 = 30;
    }
    if d2 == 31 && d1 == 30 {
        d2 = 30;
    }
    (y2 - y1) * 360 + (m2 - m1) * 30 + (d2 - d1)
}

/// `YRDIF(start, end, basis)` → années fractionnaires entre deux dates SAS.
///
/// Bases gérées : ACT/ACT (défaut), ACT/360, ACT/365, 30/360.
/// - ACT/ACT : nombre réel de jours rapporté à la longueur réelle de chaque
///   année traversée (méthode composite SAS : la fraction de chaque année
///   civile est divisée par le nombre de jours de cette année — 365 ou 366).
/// - ACT/360 : jours réels / 360.
/// - ACT/365 : jours réels / 365.
/// - 30/360  : jours convention 30/360 / 360.
///
/// AGE : non implémenté ici (calcul d'âge anniversaire, sémantique distincte) ;
/// renvoie une erreur "not yet implemented." documentée plutôt qu'un résultat
/// faux. Argument manquant ou base inconnue → missing.
fn fn_yrdif(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        ctx.invalid_data += 1;
        return Value::missing();
    }
    let start = match coerce_num(&args[0], ctx) {
        None => return Value::missing(),
        Some(f) => f.floor() as i64,
    };
    let end = match coerce_num(&args[1], ctx) {
        None => return Value::missing(),
        Some(f) => f.floor() as i64,
    };
    let Some(basis) = parse_basis(args.get(2)) else {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    };
    let years = match basis {
        Basis::Act360 => (end - start) as f64 / 360.0,
        Basis::Act365 => (end - start) as f64 / 365.0,
        Basis::Thirty360 => days_30_360(start, end) as f64 / 360.0,
        Basis::ActAct => yrdif_act_act(start, end),
        Basis::Age => {
            ctx.error_flag = true;
            return Value::missing();
        }
    };
    Value::Num(years)
}

/// ACT/ACT pour YRDIF : méthode composite SAS. On découpe l'intervalle par
/// année civile ; pour chaque année traversée, la fraction de jours y tombant
/// est divisée par la longueur de cette année (365 ou 366). La somme des
/// fractions est le nombre d'années. Symétrique : YRDIF(a,b)=-YRDIF(b,a).
fn yrdif_act_act(start: i64, end: i64) -> f64 {
    if start == end {
        return 0.0;
    }
    if start > end {
        return -yrdif_act_act(end, start);
    }
    let (y1, _, _) = sas_date_to_ymd(start);
    let (y2, _, _) = sas_date_to_ymd(end);
    if y1 == y2 {
        let len = year_length(y1) as f64;
        return (end - start) as f64 / len;
    }
    // Première année partielle : de `start` jusqu'au 1er janvier de y1+1.
    let next_year_start = days_since_1960(y1 + 1, 1, 1);
    let mut total = (next_year_start - start) as f64 / year_length(y1) as f64;
    // Années entières intermédiaires (chacune compte pour 1).
    total += ((y2 - 1) - (y1 + 1) + 1).max(0) as f64;
    // Dernière année partielle : du 1er janvier de y2 jusqu'à `end`.
    let this_year_start = days_since_1960(y2, 1, 1);
    total += (end - this_year_start) as f64 / year_length(y2) as f64;
    total
}

/// Nombre de jours d'une année civile (365 ou 366).
fn year_length(year: i64) -> i64 {
    if is_leap_year(year) {
        366
    } else {
        365
    }
}

/// `DATDIF(start, end, basis)` → nombre de jours entre deux dates SAS.
/// Bases : ACT/ACT (= jours réels = end - start) ou 30/360.
/// Argument manquant ou base inconnue → missing.
fn fn_datdif(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        ctx.invalid_data += 1;
        return Value::missing();
    }
    let start = match coerce_num(&args[0], ctx) {
        None => return Value::missing(),
        Some(f) => f.floor() as i64,
    };
    let end = match coerce_num(&args[1], ctx) {
        None => return Value::missing(),
        Some(f) => f.floor() as i64,
    };
    let Some(basis) = parse_basis(args.get(2)) else {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    };
    let days = match basis {
        // Pour DATDIF, ACT/ACT, ACT/360 et ACT/365 donnent tous le nombre
        // réel de jours (la base ne s'applique qu'au dénominateur, absent ici).
        Basis::ActAct | Basis::Act360 | Basis::Act365 => (end - start) as f64,
        Basis::Thirty360 => days_30_360(start, end) as f64,
        Basis::Age => {
            ctx.error_flag = true;
            return Value::missing();
        }
    };
    Value::Num(days)
}

/// `JULDATE(date)` → date julienne SAS au format `YYDDD` (années 1960–2059,
/// 5 chiffres) ou `YYYYDDD` (7 chiffres) selon l'option YEARCUTOFF. Ici on
/// suit la convention SAS la plus simple et réciproque de DATEJUL : on émet
/// `YYYYDDD` quand l'année n'est pas dans la fenêtre [1900, 1999], sinon
/// `YYDDD` (2 derniers chiffres de l'année). DATEJUL inverse exactement cela.
/// Date manquante → missing.
fn fn_juldate(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                let date = f.floor() as i64;
                let (year, _, _) = sas_date_to_ymd(date);
                let jan1 = days_since_1960(year, 1, 1);
                let doy = date - jan1 + 1; // jour de l'année, 1-based
                let jul = if (1900..=1999).contains(&year) {
                    (year % 100) * 1000 + doy
                } else {
                    year * 1000 + doy
                };
                Value::Num(jul as f64)
            }
        },
    }
}

/// `DATEJUL(juldate)` → date SAS. Inverse de JULDATE : décode `YYDDD` (5
/// chiffres → année 19YY) ou `YYYYDDD` (7 chiffres). Manquant → missing.
fn fn_datejul(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                let jul = f.floor() as i64;
                if jul <= 0 {
                    ctx.invalid_data += 1;
                    ctx.error_flag = true;
                    return Value::missing();
                }
                let doy = jul % 1000;
                let yy = jul / 1000;
                // ≤ 99 → forme YYDDD (année 1900+yy) ; sinon année complète.
                let year = if yy <= 99 { 1900 + yy } else { yy };
                if doy < 1 || doy > year_length(year) {
                    ctx.invalid_data += 1;
                    ctx.error_flag = true;
                    return Value::missing();
                }
                Value::Num((days_since_1960(year, 1, 1) + doy - 1) as f64)
            }
        },
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Conversion functions (PUT / INPUT) — délèguent au moteur formats/ (M4)
// ──────────────────────────────────────────────────────────────────────────────

/// `PUT(value, format)` : applique un format à une valeur, renvoie TOUJOURS
/// du caractère. Le second argument est le token de format (poussé en
/// `Value::Char` par le parser). Format invalide ou args manquants → chaîne
/// vide.
fn fn_put(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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
    let result = crate::formats::FormatCatalog::default().format(&args[0], &spec);
    Value::Char(result)
}

/// `INPUT(source, informat)` : lit une chaîne selon un informat, renvoie un
/// numérique ou un caractère selon l'informat. Le second argument est le
/// token d'informat (poussé en `Value::Char` par le parser). Informat
/// invalide ou args manquants → missing.
fn fn_input(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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
    crate::formats::FormatCatalog::default().informat(&source, &spec)
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
    // Character — lot M15.1
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
    // Math — lot M15.2
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
    ("BETA", fn_beta),
    ("ROUNDZ", fn_roundz),
    ("RANGE", fn_range),
    ("LARGEST", fn_largest),
    ("SMALLEST", fn_smallest),
    ("ORDINAL", fn_ordinal),
    // Date
    ("TODAY", fn_today),
    ("DATE", fn_today),   // DATE() is an alias for TODAY()
    ("MDY", fn_mdy),
    ("YEAR", fn_year),
    ("MONTH", fn_month),
    ("DAY", fn_day),
    ("WEEKDAY", fn_weekday),
    ("INTCK", fn_intck),
    ("INTNX", fn_intnx),
    // Date/time — lot M15.3
    ("DATETIME", fn_datetime),
    ("DATEPART", fn_datepart),
    ("TIMEPART", fn_timepart),
    ("HMS", fn_hms),
    ("DHMS", fn_dhms),
    ("HOUR", fn_hour),
    ("MINUTE", fn_minute),
    ("SECOND", fn_second),
    ("YRDIF", fn_yrdif),
    ("DATDIF", fn_datdif),
    ("JULDATE", fn_juldate),
    ("DATEJUL", fn_datejul),
    // NLDATE/NLDATEMN (formatage localisé), INTFMT, INTSHIFT : DIFFÉRÉS — non
    // enregistrés ici, donc résolus comme "fonction inconnue" par le chemin
    // existant (cohérent avec les autres fonctions non encore implémentées).
    // Conversion (PUT/INPUT) — délèguent au moteur de formats (M4).
    ("PUT", fn_put),
    ("INPUT", fn_input),
    // Macro bridge (M11.5) — lit l'instantané de la table macro.
    ("SYMGET", fn_symget),
];

/// Renvoie None si la fonction est inconnue.
pub fn call(name: &str, args: &[Value], ctx: &mut EvalCtx) -> Option<Value> {
    let upper = name.to_uppercase();
    for (fn_name, f) in DISPATCH {
        if *fn_name == upper.as_str() {
            return Some(f(args, ctx));
        }
    }
    None
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{MissingKind, Value};

    fn ctx() -> EvalCtx {
        EvalCtx::default()
    }

    fn num(f: f64) -> Value {
        Value::Num(f)
    }

    fn miss() -> Value {
        Value::missing()
    }

    fn miss_a() -> Value {
        Value::Missing(MissingKind::Letter(0))
    }

    fn chr(s: &str) -> Value {
        Value::Char(s.to_string())
    }

    fn invoke(name: &str, args: &[Value]) -> Value {
        let mut c = ctx();
        call(name, args, &mut c).expect("function should be known")
    }

    fn invoke_ctx<'a>(name: &str, args: &[Value], c: &'a mut EvalCtx) -> Value {
        call(name, args, c).expect("function should be known")
    }

    // ── Unknown function → None ───────────────────────────────────────────────

    #[test]
    fn unknown_function_returns_none() {
        let mut c = ctx();
        assert!(call("NOSUCHFN", &[], &mut c).is_none());
    }

    // ── SUM ──────────────────────────────────────────────────────────────────

    #[test]
    fn sum_nominal() {
        assert_eq!(invoke("SUM", &[num(1.0), num(2.0), num(3.0)]), num(6.0));
    }

    #[test]
    fn sum_ignores_missing_in_middle() {
        // SUM(., 1) = 1  (missing ignored, not propagated)
        assert_eq!(invoke("SUM", &[miss(), num(1.0)]), num(1.0));
        assert_eq!(invoke("SUM", &[num(1.0), miss(), num(3.0)]), num(4.0));
    }

    #[test]
    fn sum_all_missing_returns_missing() {
        assert_eq!(invoke("SUM", &[miss(), miss()]), miss());
    }

    #[test]
    fn sum_special_missing_ignored() {
        assert_eq!(invoke("SUM", &[miss_a(), num(5.0)]), num(5.0));
    }

    // ── MEAN ─────────────────────────────────────────────────────────────────

    #[test]
    fn mean_nominal() {
        assert_eq!(invoke("MEAN", &[num(1.0), num(3.0)]), num(2.0));
    }

    #[test]
    fn mean_ignores_missing() {
        // MEAN(1, ., 3) = 2.0
        assert_eq!(invoke("MEAN", &[num(1.0), miss(), num(3.0)]), num(2.0));
    }

    #[test]
    fn mean_all_missing() {
        assert_eq!(invoke("MEAN", &[miss()]), miss());
    }

    // ── MIN / MAX ─────────────────────────────────────────────────────────────

    #[test]
    fn min_nominal() {
        assert_eq!(invoke("MIN", &[num(3.0), num(1.0), num(2.0)]), num(1.0));
    }

    #[test]
    fn min_ignores_missing() {
        assert_eq!(invoke("MIN", &[miss(), num(5.0)]), num(5.0));
    }

    #[test]
    fn min_all_missing() {
        assert_eq!(invoke("MIN", &[miss()]), miss());
    }

    #[test]
    fn max_nominal() {
        assert_eq!(invoke("MAX", &[num(3.0), num(1.0), num(2.0)]), num(3.0));
    }

    #[test]
    fn max_ignores_missing() {
        assert_eq!(invoke("MAX", &[miss(), num(5.0)]), num(5.0));
    }

    // ── N / NMISS ─────────────────────────────────────────────────────────────

    #[test]
    fn n_counts_nonmissing() {
        assert_eq!(invoke("N", &[num(1.0), miss(), num(3.0)]), num(2.0));
        assert_eq!(invoke("N", &[miss(), miss()]), num(0.0));
    }

    #[test]
    fn nmiss_counts_missing() {
        assert_eq!(invoke("NMISS", &[num(1.0), miss(), num(3.0)]), num(1.0));
        assert_eq!(invoke("NMISS", &[miss(), miss()]), num(2.0));
    }

    // ── COALESCE ──────────────────────────────────────────────────────────────

    #[test]
    fn coalesce_first_nonmissing() {
        assert_eq!(invoke("COALESCE", &[miss(), num(2.0), num(3.0)]), num(2.0));
    }

    #[test]
    fn coalesce_all_missing() {
        assert_eq!(invoke("COALESCE", &[miss(), miss()]), miss());
    }

    // ── MISSING ───────────────────────────────────────────────────────────────

    #[test]
    fn missing_fn_numeric_missing() {
        assert_eq!(invoke("MISSING", &[miss()]), num(1.0));
    }

    #[test]
    fn missing_fn_numeric_nonmissing() {
        assert_eq!(invoke("MISSING", &[num(0.0)]), num(0.0));
    }

    #[test]
    fn missing_fn_blank_char() {
        assert_eq!(invoke("MISSING", &[chr("   ")]), num(1.0));
    }

    #[test]
    fn missing_fn_nonblank_char() {
        assert_eq!(invoke("MISSING", &[chr("hi")]), num(0.0));
    }

    // ── ABS ───────────────────────────────────────────────────────────────────

    #[test]
    fn abs_nominal() {
        assert_eq!(invoke("ABS", &[num(-5.0)]), num(5.0));
        assert_eq!(invoke("ABS", &[num(3.0)]), num(3.0));
    }

    #[test]
    fn abs_missing_propagates() {
        assert_eq!(invoke("ABS", &[miss()]), miss());
    }

    // ── SQRT ──────────────────────────────────────────────────────────────────

    #[test]
    fn sqrt_nominal() {
        assert_eq!(invoke("SQRT", &[num(4.0)]), num(2.0));
    }

    #[test]
    fn sqrt_negative_returns_missing_and_flags_error() {
        let mut c = ctx();
        let result = invoke_ctx("SQRT", &[num(-1.0)], &mut c);
        assert_eq!(result, miss());
        assert!(c.error_flag);
    }

    #[test]
    fn sqrt_missing_propagates() {
        assert_eq!(invoke("SQRT", &[miss()]), miss());
    }

    // ── EXP ───────────────────────────────────────────────────────────────────

    #[test]
    fn exp_nominal() {
        let result = invoke("EXP", &[num(0.0)]);
        assert_eq!(result, num(1.0));
    }

    #[test]
    fn exp_missing_propagates() {
        assert_eq!(invoke("EXP", &[miss()]), miss());
    }

    // ── LOG / LOG2 / LOG10 ────────────────────────────────────────────────────

    #[test]
    fn log_nominal() {
        let result = invoke("LOG", &[num(1.0)]);
        assert_eq!(result, num(0.0));
    }

    #[test]
    fn log_nonpositive_returns_missing_and_flags() {
        let mut c = ctx();
        let r = invoke_ctx("LOG", &[num(0.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);

        let mut c2 = ctx();
        let r2 = invoke_ctx("LOG", &[num(-1.0)], &mut c2);
        assert_eq!(r2, miss());
        assert!(c2.error_flag);
    }

    #[test]
    fn log2_nominal() {
        assert_eq!(invoke("LOG2", &[num(8.0)]), num(3.0));
    }

    #[test]
    fn log10_nominal() {
        assert_eq!(invoke("LOG10", &[num(100.0)]), num(2.0));
    }

    // ── INT ───────────────────────────────────────────────────────────────────

    #[test]
    fn int_truncates_toward_zero() {
        assert_eq!(invoke("INT", &[num(3.7)]), num(3.0));
        assert_eq!(invoke("INT", &[num(-3.7)]), num(-3.0));
    }

    #[test]
    fn int_missing_propagates() {
        assert_eq!(invoke("INT", &[miss()]), miss());
    }

    // ── ROUND ─────────────────────────────────────────────────────────────────

    #[test]
    fn round_default_unit() {
        assert_eq!(invoke("ROUND", &[num(2.5)]), num(3.0));
        assert_eq!(invoke("ROUND", &[num(-2.5)]), num(-3.0));
    }

    #[test]
    fn round_with_unit() {
        assert_eq!(invoke("ROUND", &[num(2.567), num(0.01)]), num(2.57));
    }

    #[test]
    fn round_missing_propagates() {
        assert_eq!(invoke("ROUND", &[miss()]), miss());
    }

    // ── MOD ───────────────────────────────────────────────────────────────────

    #[test]
    fn mod_nominal() {
        assert_eq!(invoke("MOD", &[num(10.0), num(3.0)]), num(1.0));
    }

    #[test]
    fn mod_sign_follows_dividend() {
        assert_eq!(invoke("MOD", &[num(-7.0), num(3.0)]), num(-1.0));
    }

    #[test]
    fn mod_div_by_zero_returns_missing() {
        let mut c = ctx();
        let r = invoke_ctx("MOD", &[num(5.0), num(0.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    // ── UPCASE / LOWCASE ──────────────────────────────────────────────────────

    #[test]
    fn upcase_nominal() {
        assert_eq!(invoke("UPCASE", &[chr("hello")]), chr("HELLO"));
    }

    #[test]
    fn lowcase_nominal() {
        assert_eq!(invoke("LOWCASE", &[chr("HELLO")]), chr("hello"));
    }

    // ── TRIM ──────────────────────────────────────────────────────────────────

    #[test]
    fn trim_trailing_blanks() {
        assert_eq!(invoke("TRIM", &[chr("hello   ")]), chr("hello"));
    }

    #[test]
    fn trim_all_blank_becomes_empty() {
        assert_eq!(invoke("TRIM", &[chr("   ")]), chr(""));
    }

    // ── STRIP ─────────────────────────────────────────────────────────────────

    #[test]
    fn strip_both_ends() {
        assert_eq!(invoke("STRIP", &[chr("  hello  ")]), chr("hello"));
    }

    // ── LEFT ──────────────────────────────────────────────────────────────────

    #[test]
    fn left_removes_leading_blanks() {
        assert_eq!(invoke("LEFT", &[chr("  hello")]), chr("hello"));
    }

    // ── LENGTH ────────────────────────────────────────────────────────────────

    #[test]
    fn length_without_trailing_blanks() {
        assert_eq!(invoke("LENGTH", &[chr("hello   ")]), num(5.0));
    }

    #[test]
    fn length_blank_string_min_one() {
        assert_eq!(invoke("LENGTH", &[chr("")]), num(1.0));
        assert_eq!(invoke("LENGTH", &[chr("   ")]), num(1.0));
    }

    // ── SUBSTR ────────────────────────────────────────────────────────────────

    #[test]
    fn substr_nominal() {
        assert_eq!(invoke("SUBSTR", &[chr("Hello"), num(2.0), num(3.0)]), chr("ell"));
    }

    #[test]
    fn substr_no_length() {
        assert_eq!(invoke("SUBSTR", &[chr("Hello"), num(3.0)]), chr("llo"));
    }

    #[test]
    fn substr_out_of_bounds_flags_error() {
        let mut c = ctx();
        let r = invoke_ctx("SUBSTR", &[chr("abc"), num(0.0)], &mut c);
        assert_eq!(r, chr(""));
        assert!(c.error_flag);
    }

    // ── INDEX ─────────────────────────────────────────────────────────────────

    #[test]
    fn index_found() {
        assert_eq!(invoke("INDEX", &[chr("Hello World"), chr("World")]), num(7.0));
    }

    #[test]
    fn index_not_found() {
        assert_eq!(invoke("INDEX", &[chr("Hello"), chr("xyz")]), num(0.0));
    }

    // ── CAT / CATS / CATX ────────────────────────────────────────────────────

    #[test]
    fn cat_concatenates_raw() {
        assert_eq!(invoke("CAT", &[chr("Hello "), chr("World")]), chr("Hello World"));
    }

    #[test]
    fn cats_strips_each() {
        assert_eq!(invoke("CATS", &[chr("  Hello  "), chr("  World  ")]), chr("HelloWorld"));
    }

    #[test]
    fn catx_sep_skips_blank() {
        // CATX("-", "a", "", "c") = "a-c"
        assert_eq!(
            invoke("CATX", &[chr("-"), chr("a"), chr(""), chr("c")]),
            chr("a-c")
        );
    }

    // ── COMPRESS ──────────────────────────────────────────────────────────────

    #[test]
    fn compress_default_removes_spaces() {
        assert_eq!(invoke("COMPRESS", &[chr("hello world")]), chr("helloworld"));
    }

    #[test]
    fn compress_custom_chars() {
        assert_eq!(invoke("COMPRESS", &[chr("hello123"), chr("123")]), chr("hello"));
    }

    // ── TRANWRD ───────────────────────────────────────────────────────────────

    #[test]
    fn tranwrd_replaces_substring() {
        assert_eq!(
            invoke("TRANWRD", &[chr("Hello World"), chr("World"), chr("Rust")]),
            chr("Hello Rust")
        );
    }

    // ── SCAN ──────────────────────────────────────────────────────────────────

    #[test]
    fn scan_first_word() {
        assert_eq!(invoke("SCAN", &[chr("hello world foo"), num(1.0)]), chr("hello"));
    }

    #[test]
    fn scan_second_word() {
        assert_eq!(invoke("SCAN", &[chr("hello world foo"), num(2.0)]), chr("world"));
    }

    #[test]
    fn scan_negative_index_from_end() {
        // n=-1 → last word
        assert_eq!(invoke("SCAN", &[chr("hello world foo"), num(-1.0)]), chr("foo"));
    }

    #[test]
    fn scan_out_of_range() {
        assert_eq!(invoke("SCAN", &[chr("hello world"), num(5.0)]), chr(""));
    }

    #[test]
    fn scan_custom_delim() {
        assert_eq!(
            invoke("SCAN", &[chr("a,b,c"), num(2.0), chr(",")]),
            chr("b")
        );
    }

    // ── TODAY / DATE ──────────────────────────────────────────────────────────

    #[test]
    fn today_returns_numeric() {
        let mut c = ctx();
        let r = call("TODAY", &[], &mut c).unwrap();
        // Croise les deux chemins de calcul de date : la valeur de TODAY()
        // redécodée par le chemin JDN doit donner une année plausible
        // (>= 2026, l'horloge ne recule pas) — attrape toute erreur
        // d'offset d'époque 1960/1970.
        match r {
            Value::Num(f) => {
                let (y, _, _) = sas_date_to_ymd(f as i64);
                assert!(y >= 2026, "TODAY() decodes to year {y}");
            }
            _ => panic!("expected numeric"),
        }
        let r2 = call("DATE", &[], &mut c).unwrap();
        assert_eq!(r, r2);
    }

    // ── MDY ───────────────────────────────────────────────────────────────────

    #[test]
    fn mdy_nominal() {
        // 1960-01-01 should be day 0.
        let r = invoke("MDY", &[num(1.0), num(1.0), num(1960.0)]);
        assert_eq!(r, num(0.0));
    }

    #[test]
    fn mdy_known_date() {
        // 2000-01-01 = SAS date 14610.
        let r = invoke("MDY", &[num(1.0), num(1.0), num(2000.0)]);
        assert_eq!(r, num(14610.0));
    }

    #[test]
    fn mdy_invalid_date_returns_missing() {
        let mut c = ctx();
        let r = invoke_ctx("MDY", &[num(13.0), num(1.0), num(2000.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    // ── YEAR / MONTH / DAY / WEEKDAY ─────────────────────────────────────────

    #[test]
    fn year_from_sas_date() {
        // 14610 = 2000-01-01
        assert_eq!(invoke("YEAR", &[num(14610.0)]), num(2000.0));
    }

    #[test]
    fn month_from_sas_date() {
        assert_eq!(invoke("MONTH", &[num(14610.0)]), num(1.0));
    }

    #[test]
    fn day_from_sas_date() {
        assert_eq!(invoke("DAY", &[num(14610.0)]), num(1.0));
    }

    #[test]
    fn weekday_sas_date_0_is_friday() {
        // 1960-01-01 = SAS date 0 = Friday = 6 in SAS (Sun=1).
        assert_eq!(invoke("WEEKDAY", &[num(0.0)]), num(6.0));
    }

    #[test]
    fn weekday_known_sunday() {
        // 2000-01-02 = Sunday. SAS date 14611.
        assert_eq!(invoke("WEEKDAY", &[num(14611.0)]), num(1.0));
    }

    // ── DATETIME / DATEPART / TIMEPART (lot M15.3) ─────────────────────────────

    #[test]
    fn datetime_deterministic_is_zero() {
        let mut c = ctx();
        c.deterministic = true;
        assert_eq!(invoke_ctx("DATETIME", &[], &mut c), num(0.0));
    }

    #[test]
    fn datepart_timepart_of_known_datetime() {
        // 2000-01-01 = date 14610 ; datetime à 01:30:00 du jour =
        // 14610*86400 + 5400.
        let dt = 14610.0 * 86400.0 + 5400.0;
        assert_eq!(invoke("DATEPART", &[num(dt)]), num(14610.0));
        assert_eq!(invoke("TIMEPART", &[num(dt)]), num(5400.0));
    }

    #[test]
    fn timepart_negative_datetime_wraps_into_day() {
        // Datetime négatif (avant 1960) : TIMEPART doit rester dans [0, 86400).
        // -1 seconde = 23:59:59 de la veille → 86399.
        assert_eq!(invoke("TIMEPART", &[num(-1.0)]), num(86399.0));
        assert_eq!(invoke("DATEPART", &[num(-1.0)]), num(-1.0));
    }

    // ── HMS / DHMS ─────────────────────────────────────────────────────────────

    #[test]
    fn hms_known() {
        // HMS(1, 30, 0) = 1*3600 + 30*60 = 5400.
        assert_eq!(invoke("HMS", &[num(1.0), num(30.0), num(0.0)]), num(5400.0));
        assert_eq!(
            invoke("HMS", &[num(23.0), num(59.0), num(59.0)]),
            num(86399.0)
        );
    }

    #[test]
    fn hms_missing_arg_propagates() {
        assert_eq!(invoke("HMS", &[miss(), num(30.0), num(0.0)]), miss());
    }

    #[test]
    fn dhms_known() {
        // DHMS(14610, 1, 30, 0) = 14610*86400 + 5400.
        let expect = 14610.0 * 86400.0 + 5400.0;
        assert_eq!(
            invoke("DHMS", &[num(14610.0), num(1.0), num(30.0), num(0.0)]),
            num(expect)
        );
    }

    #[test]
    fn dhms_missing_arg_propagates() {
        assert_eq!(
            invoke("DHMS", &[num(14610.0), miss(), num(30.0), num(0.0)]),
            miss()
        );
    }

    // ── HOUR / MINUTE / SECOND ────────────────────────────────────────────────

    #[test]
    fn hour_minute_second_of_time() {
        // time = 01:30:45 = 5445 s.
        let t = 5445.0;
        assert_eq!(invoke("HOUR", &[num(t)]), num(1.0));
        assert_eq!(invoke("MINUTE", &[num(t)]), num(30.0));
        assert_eq!(invoke("SECOND", &[num(t)]), num(45.0));
    }

    #[test]
    fn hour_minute_second_accept_datetime() {
        // datetime 2000-01-01 02:03:04 → composantes 2,3,4 (modulo jour).
        let dt = 14610.0 * 86400.0 + 2.0 * 3600.0 + 3.0 * 60.0 + 4.0;
        assert_eq!(invoke("HOUR", &[num(dt)]), num(2.0));
        assert_eq!(invoke("MINUTE", &[num(dt)]), num(3.0));
        assert_eq!(invoke("SECOND", &[num(dt)]), num(4.0));
    }

    #[test]
    fn hour_minute_second_negative_datetime() {
        // -1 s = 23:59:59 de la veille → H=23, M=59, S=59.
        assert_eq!(invoke("HOUR", &[num(-1.0)]), num(23.0));
        assert_eq!(invoke("MINUTE", &[num(-1.0)]), num(59.0));
        assert_eq!(invoke("SECOND", &[num(-1.0)]), num(59.0));
    }

    #[test]
    fn second_keeps_fraction() {
        assert_eq!(invoke("SECOND", &[num(45.5)]), num(45.5));
    }

    // ── YRDIF ──────────────────────────────────────────────────────────────────

    #[test]
    fn yrdif_act_act_full_year() {
        // 2001-01-01 → 2002-01-01 : exactement 1 an (2001 = 365 jours).
        let s = invoke("MDY", &[num(1.0), num(1.0), num(2001.0)]);
        let e = invoke("MDY", &[num(1.0), num(1.0), num(2002.0)]);
        let r = invoke("YRDIF", &[s, e, chr("ACT/ACT")]);
        match r {
            Value::Num(f) => assert!((f - 1.0).abs() < 1e-9, "got {f}"),
            _ => panic!("expected numeric"),
        }
    }

    #[test]
    fn yrdif_act_act_default_basis() {
        // Sans base → ACT/ACT.
        let s = invoke("MDY", &[num(1.0), num(1.0), num(2001.0)]);
        let e = invoke("MDY", &[num(1.0), num(1.0), num(2002.0)]);
        let r = invoke("YRDIF", &[s, e]);
        match r {
            Value::Num(f) => assert!((f - 1.0).abs() < 1e-9, "got {f}"),
            _ => panic!("expected numeric"),
        }
    }

    #[test]
    fn yrdif_30_360_known() {
        // 2000-01-01 → 2000-07-01 : 30/360 = 180 jours / 360 = 0.5 an.
        let s = invoke("MDY", &[num(1.0), num(1.0), num(2000.0)]);
        let e = invoke("MDY", &[num(7.0), num(1.0), num(2000.0)]);
        let r = invoke("YRDIF", &[s, e, chr("30/360")]);
        match r {
            Value::Num(f) => assert!((f - 0.5).abs() < 1e-9, "got {f}"),
            _ => panic!("expected numeric"),
        }
    }

    #[test]
    fn yrdif_30_360_end_of_month_rule() {
        // 2000-01-31 → 2000-02-29 : règle 30/360 (d1=31→30, d2=29 reste) =
        // (0)*360 + (1)*30 + (29-30) = 29 jours / 360.
        let s = invoke("MDY", &[num(1.0), num(31.0), num(2000.0)]);
        let e = invoke("MDY", &[num(2.0), num(29.0), num(2000.0)]);
        let r = invoke("YRDIF", &[s, e, chr("30/360")]);
        match r {
            Value::Num(f) => assert!((f - 29.0 / 360.0).abs() < 1e-9, "got {f}"),
            _ => panic!("expected numeric"),
        }
    }

    #[test]
    fn yrdif_act_360_and_365() {
        // 2000-01-01 → 2000-12-31 : 365 jours réels (2000-12-31 - 2000-01-01).
        let s = invoke("MDY", &[num(1.0), num(1.0), num(2000.0)]);
        let e = invoke("MDY", &[num(12.0), num(31.0), num(2000.0)]);
        let days = match (&s, &e) {
            (Value::Num(a), Value::Num(b)) => (b - a) as f64,
            _ => panic!(),
        };
        assert_eq!(days, 365.0);
        let r360 = invoke("YRDIF", &[s.clone(), e.clone(), chr("ACT/360")]);
        assert_eq!(r360, num(365.0 / 360.0));
        let r365 = invoke("YRDIF", &[s, e, chr("ACT/365")]);
        assert_eq!(r365, num(365.0 / 365.0));
    }

    #[test]
    fn yrdif_symmetric_act_act() {
        let s = invoke("MDY", &[num(3.0), num(15.0), num(1998.0)]);
        let e = invoke("MDY", &[num(11.0), num(20.0), num(2003.0)]);
        let fwd = invoke("YRDIF", &[s.clone(), e.clone()]);
        let bwd = invoke("YRDIF", &[e, s]);
        match (fwd, bwd) {
            (Value::Num(a), Value::Num(b)) => assert!((a + b).abs() < 1e-9),
            _ => panic!(),
        }
    }

    #[test]
    fn yrdif_missing_arg() {
        assert_eq!(invoke("YRDIF", &[miss(), num(100.0)]), miss());
    }

    #[test]
    fn yrdif_age_basis_errors() {
        // AGE différé : erreur signalée, résultat missing (pas de valeur fausse).
        let mut c = ctx();
        let r = invoke_ctx("YRDIF", &[num(0.0), num(365.0), chr("AGE")], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    // ── DATDIF ─────────────────────────────────────────────────────────────────

    #[test]
    fn datdif_act_act_is_real_days() {
        let s = invoke("MDY", &[num(1.0), num(1.0), num(2000.0)]);
        let e = invoke("MDY", &[num(1.0), num(11.0), num(2000.0)]);
        assert_eq!(invoke("DATDIF", &[s, e, chr("ACT/ACT")]), num(10.0));
    }

    #[test]
    fn datdif_30_360() {
        // 2000-01-31 → 2000-02-29 : 30/360 = 29 jours (cf. yrdif test).
        let s = invoke("MDY", &[num(1.0), num(31.0), num(2000.0)]);
        let e = invoke("MDY", &[num(2.0), num(29.0), num(2000.0)]);
        assert_eq!(invoke("DATDIF", &[s, e, chr("30/360")]), num(29.0));
    }

    #[test]
    fn datdif_missing_arg() {
        assert_eq!(invoke("DATDIF", &[miss(), num(5.0)]), miss());
    }

    // ── JULDATE / DATEJUL ──────────────────────────────────────────────────────

    #[test]
    fn juldate_known() {
        // 2000-01-01 = date 14610 ; année hors [1900,1999] → forme YYYYDDD.
        assert_eq!(invoke("JULDATE", &[num(14610.0)]), num(2000001.0));
        // 1995-01-01 → forme YYDDD = 95001.
        let d = invoke("MDY", &[num(1.0), num(1.0), num(1995.0)]);
        assert_eq!(invoke("JULDATE", &[d]), num(95001.0));
    }

    #[test]
    fn datejul_known() {
        // 2000001 → 2000-01-01 = 14610.
        assert_eq!(invoke("DATEJUL", &[num(2000001.0)]), num(14610.0));
        // 95001 → 1995-01-01.
        let expect = invoke("MDY", &[num(1.0), num(1.0), num(1995.0)]);
        assert_eq!(invoke("DATEJUL", &[num(95001.0)]), expect);
    }

    #[test]
    fn juldate_datejul_roundtrip() {
        // Réciprocité sur quelques dates.
        for &date in &[14610.0_f64, 0.0, 20000.0, 25000.0] {
            let jul = invoke("JULDATE", &[num(date)]);
            assert_eq!(invoke("DATEJUL", &[jul]), num(date));
        }
    }

    #[test]
    fn juldate_datejul_missing() {
        assert_eq!(invoke("JULDATE", &[miss()]), miss());
        assert_eq!(invoke("DATEJUL", &[miss()]), miss());
    }

    // ── Case insensitivity ────────────────────────────────────────────────────

    #[test]
    fn function_names_case_insensitive() {
        assert_eq!(invoke("sum", &[num(1.0), num(2.0)]), num(3.0));
        assert_eq!(invoke("Abs", &[num(-3.0)]), num(3.0));
    }

    // ── PUT / INPUT : délégation au moteur de formats (M4) ────────────────
    // Le 2e argument est le TOKEN de format poussé en Value::Char par le
    // parser (cf. parse_call dans expr.rs).

    #[test]
    fn put_dollar_format_returns_char() {
        // PUT(1234.5, dollar10.2) → "$1,234.50".
        let r = invoke("PUT", &[num(1234.5), chr("dollar10.2")]);
        match r {
            Value::Char(s) => assert!(
                s.contains("$1,234.50"),
                "expected '$1,234.50' inside {s:?}"
            ),
            _ => panic!("PUT must return character, got {r:?}"),
        }
    }

    #[test]
    fn put_date_format_returns_char() {
        // 2020-01-01 = 21915 jours après 1960-01-01 (croise avec MDY).
        assert_eq!(invoke("MDY", &[num(1.0), num(1.0), num(2020.0)]), num(21915.0));
        let r = invoke("PUT", &[num(21915.0), chr("date9.")]);
        match r {
            Value::Char(s) => assert!(
                s.contains("01JAN2020"),
                "expected '01JAN2020' inside {s:?}"
            ),
            _ => panic!("PUT must return character, got {r:?}"),
        }
    }

    #[test]
    fn put_invalid_format_returns_empty() {
        // Token non parsable → chaîne vide (pas de panique).
        assert_eq!(invoke("PUT", &[num(1.0), chr("")]), chr(""));
    }

    #[test]
    fn put_wrong_arity_returns_empty() {
        assert_eq!(invoke("PUT", &[num(1.0)]), chr(""));
    }

    #[test]
    fn input_implicit_decimal() {
        // INPUT("123", 5.2) → 1.23 (le `.2` impose 2 décimales implicites).
        assert_eq!(invoke("INPUT", &[chr("123"), chr("5.2")]), num(1.23));
    }

    #[test]
    fn input_date_informat() {
        // INPUT("01JAN2020", date9.) → 21915.
        assert_eq!(invoke("INPUT", &[chr("01JAN2020"), chr("date9.")]), num(21915.0));
    }

    #[test]
    fn input_wrong_arity_returns_missing() {
        assert_eq!(invoke("INPUT", &[chr("123")]), miss());
    }

    // ── INTCK ─────────────────────────────────────────────────────────────────

    fn sas_day(y: i64, m: i64, d: i64) -> f64 {
        days_since_1960(y, m, d) as f64
    }

    #[test]
    fn intck_day_diff() {
        let d1 = sas_day(2020, 1, 1);
        let d2 = sas_day(2020, 1, 11);
        assert_eq!(invoke("INTCK", &[chr("day"), num(d1), num(d2)]), num(10.0));
    }

    #[test]
    fn intck_month() {
        // 15jan2020 → 01mar2020 = 2 month boundaries.
        let d1 = sas_day(2020, 1, 15);
        let d2 = sas_day(2020, 3, 1);
        assert_eq!(invoke("INTCK", &[chr("month"), num(d1), num(d2)]), num(2.0));
    }

    #[test]
    fn intck_qtr() {
        // jan2020 (Q1) → jul2020 (Q3) = 2 quarter boundaries.
        let d1 = sas_day(2020, 1, 15);
        let d2 = sas_day(2020, 7, 1);
        assert_eq!(invoke("INTCK", &[chr("qtr"), num(d1), num(d2)]), num(2.0));
    }

    #[test]
    fn intck_year() {
        let d1 = sas_day(2018, 6, 1);
        let d2 = sas_day(2021, 3, 1);
        assert_eq!(invoke("INTCK", &[chr("year"), num(d1), num(d2)]), num(3.0));
    }

    #[test]
    fn intck_week_boundary() {
        // SAS day 0 = Friday; day 2 (1960-01-03) = Sunday → new SAS week.
        assert_eq!(invoke("INTCK", &[chr("week"), num(0.0), num(2.0)]), num(1.0));
        // days 0..6 within the SAS week of day 0: day 0 (Fri) → day 1 (Sat)
        // are in the same week (Sunday boundary not crossed).
        assert_eq!(invoke("INTCK", &[chr("week"), num(0.0), num(1.0)]), num(0.0));
    }

    #[test]
    fn intck_week_negative() {
        // Going backward across a Sunday boundary.
        assert_eq!(invoke("INTCK", &[chr("week"), num(2.0), num(0.0)]), num(-1.0));
    }

    #[test]
    fn intck_unknown_interval_is_missing() {
        let mut c = ctx();
        let r = invoke_ctx("INTCK", &[chr("fortnight"), num(0.0), num(14.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    #[test]
    fn intck_missing_date_is_missing() {
        assert_eq!(invoke("INTCK", &[chr("day"), miss(), num(5.0)]), miss());
    }

    // ── INTNX ─────────────────────────────────────────────────────────────────

    #[test]
    fn intnx_month_beginning_default() {
        // INTNX('month', 15jan2020, 1) → 01feb2020.
        let start = sas_day(2020, 1, 15);
        assert_eq!(
            invoke("INTNX", &[chr("month"), num(start), num(1.0)]),
            num(sas_day(2020, 2, 1))
        );
    }

    #[test]
    fn intnx_month_same() {
        // INTNX('month', 15jan2020, 1, 'same') → 15feb2020.
        let start = sas_day(2020, 1, 15);
        assert_eq!(
            invoke("INTNX", &[chr("month"), num(start), num(1.0), chr("same")]),
            num(sas_day(2020, 2, 15))
        );
    }

    #[test]
    fn intnx_month_end() {
        // INTNX('month', 15jan2020, 1, 'e') → 29feb2020 (leap year).
        let start = sas_day(2020, 1, 15);
        assert_eq!(
            invoke("INTNX", &[chr("month"), num(start), num(1.0), chr("e")]),
            num(sas_day(2020, 2, 29))
        );
    }

    #[test]
    fn intnx_month_same_clamps_to_last_day() {
        // 31jan2020 + 1 month, 'same' → clamp to 29feb2020.
        let start = sas_day(2020, 1, 31);
        assert_eq!(
            invoke("INTNX", &[chr("month"), num(start), num(1.0), chr("same")]),
            num(sas_day(2020, 2, 29))
        );
    }

    #[test]
    fn intnx_year_end() {
        // INTNX('year', d, 0, 'e') → 31dec of that year.
        let start = sas_day(2020, 5, 17);
        assert_eq!(
            invoke("INTNX", &[chr("year"), num(start), num(0.0), chr("e")]),
            num(sas_day(2020, 12, 31))
        );
    }

    #[test]
    fn intnx_year_beginning() {
        let start = sas_day(2020, 5, 17);
        assert_eq!(
            invoke("INTNX", &[chr("year"), num(start), num(0.0)]),
            num(sas_day(2020, 1, 1))
        );
    }

    #[test]
    fn intnx_qtr_beginning() {
        // 17may2020 is in Q2 (apr-jun); +1 qtr → Q3 → 01jul2020.
        let start = sas_day(2020, 5, 17);
        assert_eq!(
            invoke("INTNX", &[chr("qtr"), num(start), num(1.0)]),
            num(sas_day(2020, 7, 1))
        );
    }

    #[test]
    fn intnx_qtr_end() {
        // Q2 of 2020, 0 increment, end → 30jun2020.
        let start = sas_day(2020, 5, 17);
        assert_eq!(
            invoke("INTNX", &[chr("qtr"), num(start), num(0.0), chr("e")]),
            num(sas_day(2020, 6, 30))
        );
    }

    #[test]
    fn intnx_day() {
        let start = sas_day(2020, 1, 1);
        assert_eq!(
            invoke("INTNX", &[chr("day"), num(start), num(10.0)]),
            num(sas_day(2020, 1, 11))
        );
    }

    #[test]
    fn intnx_week_beginning() {
        // SAS day 0 = Friday; its week begins Sunday day -5 (1959-12-27).
        // +0 weeks, B → day -5.
        assert_eq!(
            invoke("INTNX", &[chr("week"), num(0.0), num(0.0)]),
            num(-5.0)
        );
        // +1 week beginning → next Sunday = day 2 (1960-01-03).
        assert_eq!(
            invoke("INTNX", &[chr("week"), num(0.0), num(1.0)]),
            num(2.0)
        );
    }

    #[test]
    fn intnx_week_same_weekday() {
        // day 0 = Friday; +1 week 'same' → next Friday = day 7.
        assert_eq!(
            invoke("INTNX", &[chr("week"), num(0.0), num(1.0), chr("s")]),
            num(7.0)
        );
    }

    #[test]
    fn intnx_unknown_interval_is_missing() {
        let mut c = ctx();
        let r = invoke_ctx("INTNX", &[chr("decade"), num(0.0), num(1.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    #[test]
    fn intnx_missing_date_is_missing() {
        assert_eq!(invoke("INTNX", &[chr("month"), miss(), num(1.0)]), miss());
    }

    // ══════════════════════════════════════════════════════════════════════════
    // M15.1 — fonctions caractère
    // ══════════════════════════════════════════════════════════════════════════

    // ── FIND ──────────────────────────────────────────────────────────────────

    #[test]
    fn find_table() {
        // (s, sub, extra_args, expected)
        let cases: &[(&str, &str, Vec<Value>, f64)] = &[
            ("Hello World", "World", vec![], 7.0),
            ("Hello", "xyz", vec![], 0.0),
            ("abcabc", "bc", vec![], 2.0),
            // start argument: chercher après la position 3.
            ("abcabc", "bc", vec![num(3.0)], 5.0),
        ];
        for (s, sub, extra, exp) in cases {
            let mut args = vec![chr(s), chr(sub)];
            args.extend(extra.clone());
            assert_eq!(invoke("FIND", &args), num(*exp), "FIND({s:?},{sub:?},{extra:?})");
        }
    }

    #[test]
    fn find_mod_i_case_insensitive() {
        // Sans 'i' : pas trouvé ; avec 'i' : trouvé.
        assert_eq!(invoke("FIND", &[chr("Hello"), chr("hello")]), num(0.0));
        assert_eq!(invoke("FIND", &[chr("Hello"), chr("hello"), chr("i")]), num(1.0));
    }

    #[test]
    fn find_missing_like_blank() {
        // Sous-chaîne vide → 0.
        assert_eq!(invoke("FIND", &[chr("abc"), chr("")]), num(0.0));
    }

    // ── FINDC ─────────────────────────────────────────────────────────────────

    #[test]
    fn findc_table() {
        // 1er caractère de s présent dans l'ensemble.
        assert_eq!(invoke("FINDC", &[chr("abc123"), chr("0123456789")]), num(4.0));
        // aucun présent → 0.
        assert_eq!(invoke("FINDC", &[chr("abc"), chr("xyz")]), num(0.0));
        // modificateur 'v' : 1er caractère ABSENT de l'ensemble.
        assert_eq!(invoke("FINDC", &[chr("123a"), chr("0123456789"), chr("v")]), num(4.0));
    }

    #[test]
    fn findc_mod_i() {
        assert_eq!(invoke("FINDC", &[chr("ABC"), chr("abc"), chr("i")]), num(1.0));
    }

    // ── COUNT / COUNTC ────────────────────────────────────────────────────────

    #[test]
    fn count_table() {
        assert_eq!(invoke("COUNT", &[chr("abcabcabc"), chr("abc")]), num(3.0));
        assert_eq!(invoke("COUNT", &[chr("aaaa"), chr("aa")]), num(2.0)); // non chevauchant
        assert_eq!(invoke("COUNT", &[chr("xyz"), chr("a")]), num(0.0));
    }

    #[test]
    fn count_mod_i() {
        assert_eq!(invoke("COUNT", &[chr("AbAbAb"), chr("ab"), chr("i")]), num(3.0));
    }

    #[test]
    fn countc_table() {
        assert_eq!(invoke("COUNTC", &[chr("a1b2c3"), chr("0123456789")]), num(3.0));
        // 'v' : compte les absents de l'ensemble.
        assert_eq!(invoke("COUNTC", &[chr("a1b2c3"), chr("0123456789"), chr("v")]), num(3.0));
    }

    // ── VERIFY ────────────────────────────────────────────────────────────────

    #[test]
    fn verify_table() {
        // 1er caractère absent de l'ensemble.
        assert_eq!(invoke("VERIFY", &[chr("12345"), chr("0123456789")]), num(0.0));
        assert_eq!(invoke("VERIFY", &[chr("123a5"), chr("0123456789")]), num(4.0));
    }

    // ── TRANSLATE ─────────────────────────────────────────────────────────────

    #[test]
    fn translate_table() {
        assert_eq!(
            invoke("TRANSLATE", &[chr("abc"), chr("xyz"), chr("abc")]),
            chr("xyz")
        );
        // caractère sans correspondance laissé inchangé.
        assert_eq!(
            invoke("TRANSLATE", &[chr("a-b"), chr("X"), chr("a")]),
            chr("X-b")
        );
    }

    // ── REVERSE ───────────────────────────────────────────────────────────────

    #[test]
    fn reverse_table() {
        assert_eq!(invoke("REVERSE", &[chr("abc")]), chr("cba"));
        assert_eq!(invoke("REVERSE", &[chr("")]), chr(""));
    }

    // ── REPEAT (piège : n+1 copies) ───────────────────────────────────────────

    #[test]
    fn repeat_n_plus_one_copies() {
        // REPEAT("ab", 2) → 3 copies = "ababab".
        assert_eq!(invoke("REPEAT", &[chr("ab"), num(2.0)]), chr("ababab"));
        // REPEAT("x", 0) → 1 copie = "x".
        assert_eq!(invoke("REPEAT", &[chr("x"), num(0.0)]), chr("x"));
    }

    // ── PROPCASE ──────────────────────────────────────────────────────────────

    #[test]
    fn propcase_table() {
        assert_eq!(invoke("PROPCASE", &[chr("hello world")]), chr("Hello World"));
        assert_eq!(invoke("PROPCASE", &[chr("JOHN SMITH")]), chr("John Smith"));
    }

    // ── COMPBL ────────────────────────────────────────────────────────────────

    #[test]
    fn compbl_table() {
        assert_eq!(invoke("COMPBL", &[chr("a   b    c")]), chr("a b c"));
        assert_eq!(invoke("COMPBL", &[chr("no  double")]), chr("no double"));
    }

    // ── SUBSTRN (tolère pos/len négatifs/hors borne) ──────────────────────────

    #[test]
    fn substrn_table() {
        // nominal.
        assert_eq!(invoke("SUBSTRN", &[chr("Hello"), num(2.0), num(3.0)]), chr("ell"));
        // pos négatif : tronque la partie hors [1,len].
        assert_eq!(invoke("SUBSTRN", &[chr("Hello"), num(-1.0), num(3.0)]), chr("H"));
        // pos après la fin → vide (pas d'erreur).
        assert_eq!(invoke("SUBSTRN", &[chr("Hello"), num(10.0), num(3.0)]), chr(""));
        // len négatif → vide.
        assert_eq!(invoke("SUBSTRN", &[chr("Hello"), num(2.0), num(-1.0)]), chr(""));
    }

    #[test]
    fn substrn_no_error_flag() {
        // Contrairement à SUBSTR, SUBSTRN ne lève pas _ERROR_.
        let mut c = ctx();
        let r = invoke_ctx("SUBSTRN", &[chr("abc"), num(-5.0), num(2.0)], &mut c);
        assert_eq!(r, chr(""));
        assert!(!c.error_flag);
    }

    // ── CHAR ──────────────────────────────────────────────────────────────────

    #[test]
    fn char_table() {
        assert_eq!(invoke("CHAR", &[chr("Hello"), num(1.0)]), chr("H"));
        assert_eq!(invoke("CHAR", &[chr("Hello"), num(5.0)]), chr("o"));
        // hors borne → blanc.
        assert_eq!(invoke("CHAR", &[chr("Hi"), num(9.0)]), chr(" "));
    }

    // ── RANK / BYTE ───────────────────────────────────────────────────────────

    #[test]
    fn rank_table() {
        assert_eq!(invoke("RANK", &[chr("A")]), num(65.0));
        assert_eq!(invoke("RANK", &[chr("a")]), num(97.0));
    }

    #[test]
    fn byte_table() {
        assert_eq!(invoke("BYTE", &[num(65.0)]), chr("A"));
        assert_eq!(invoke("BYTE", &[num(97.0)]), chr("a"));
    }

    #[test]
    fn rank_byte_roundtrip() {
        let c = invoke("BYTE", &[num(66.0)]);
        assert_eq!(invoke("RANK", &[c]), num(66.0));
    }

    // ── WHICHC ────────────────────────────────────────────────────────────────

    #[test]
    fn whichc_table() {
        assert_eq!(
            invoke("WHICHC", &[chr("b"), chr("a"), chr("b"), chr("c")]),
            num(2.0)
        );
        assert_eq!(
            invoke("WHICHC", &[chr("z"), chr("a"), chr("b")]),
            num(0.0)
        );
        // comparaison ignore les blancs finaux.
        assert_eq!(
            invoke("WHICHC", &[chr("a"), chr("a  ")]),
            num(1.0)
        );
    }

    // ── CATQ ──────────────────────────────────────────────────────────────────

    #[test]
    fn catq_quotes_when_needed() {
        // item avec espace → entre guillemets. (Le 1er arg "x y" n'est pas une
        // chaîne de modificateurs valide — il est donc traité comme un item.)
        let r = invoke("CATQ", &[chr("x y"), chr("z")]);
        match r {
            Value::Char(s) => assert!(s.contains('"'), "CATQ should quote spaced item: {s:?}"),
            _ => panic!("CATQ must return char"),
        }
        // items simples → pas de guillemets, séparés par espace.
        assert_eq!(invoke("CATQ", &[chr("foo"), chr("bar")]), chr("foo bar"));
    }

    // ══════════════════════════════════════════════════════════════════════════
    // M15.2 — fonctions mathématiques
    // ══════════════════════════════════════════════════════════════════════════

    fn approx(v: Value, expected: f64) {
        match v {
            Value::Num(f) => assert!(
                (f - expected).abs() < 1e-9,
                "expected ~{expected}, got {f}"
            ),
            _ => panic!("expected numeric, got {v:?}"),
        }
    }

    fn approx_tol(v: Value, expected: f64, tol: f64) {
        match v {
            Value::Num(f) => assert!(
                (f - expected).abs() < tol,
                "expected ~{expected} (tol {tol}), got {f}"
            ),
            _ => panic!("expected numeric, got {v:?}"),
        }
    }

    // ── CEIL / FLOOR ──────────────────────────────────────────────────────────

    #[test]
    fn ceil_floor_table() {
        assert_eq!(invoke("CEIL", &[num(2.1)]), num(3.0));
        assert_eq!(invoke("CEIL", &[num(-2.1)]), num(-2.0));
        assert_eq!(invoke("FLOOR", &[num(2.9)]), num(2.0));
        assert_eq!(invoke("FLOOR", &[num(-2.1)]), num(-3.0));
    }

    #[test]
    fn ceil_floor_missing() {
        assert_eq!(invoke("CEIL", &[miss()]), miss());
        assert_eq!(invoke("FLOOR", &[miss()]), miss());
    }

    // ── SIGN ──────────────────────────────────────────────────────────────────

    #[test]
    fn sign_table() {
        assert_eq!(invoke("SIGN", &[num(5.0)]), num(1.0));
        assert_eq!(invoke("SIGN", &[num(-5.0)]), num(-1.0));
        assert_eq!(invoke("SIGN", &[num(0.0)]), num(0.0));
        assert_eq!(invoke("SIGN", &[miss()]), miss());
    }

    // ── Trigonométrie ─────────────────────────────────────────────────────────

    #[test]
    fn trig_table() {
        approx(invoke("SIN", &[num(0.0)]), 0.0);
        approx(invoke("COS", &[num(0.0)]), 1.0);
        approx(invoke("TAN", &[num(0.0)]), 0.0);
        approx(invoke("SIN", &[num(std::f64::consts::FRAC_PI_2)]), 1.0);
        assert_eq!(invoke("SIN", &[miss()]), miss());
    }

    #[test]
    fn arc_trig_table() {
        approx(invoke("ARSIN", &[num(1.0)]), std::f64::consts::FRAC_PI_2);
        approx(invoke("ARCOS", &[num(1.0)]), 0.0);
        approx(invoke("ATAN", &[num(0.0)]), 0.0);
        approx(invoke("ATAN2", &[num(1.0), num(1.0)]), std::f64::consts::FRAC_PI_4);
    }

    #[test]
    fn arsin_domain_error() {
        let mut c = ctx();
        let r = invoke_ctx("ARSIN", &[num(2.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
        assert_eq!(c.invalid_data, 1);
    }

    #[test]
    fn arcos_domain_error() {
        let mut c = ctx();
        let r = invoke_ctx("ARCOS", &[num(-2.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    #[test]
    fn hyperbolic_table() {
        approx(invoke("SINH", &[num(0.0)]), 0.0);
        approx(invoke("COSH", &[num(0.0)]), 1.0);
        approx(invoke("TANH", &[num(0.0)]), 0.0);
        approx(invoke("TANH", &[num(100.0)]), 1.0);
    }

    // ── FACT / COMB / PERM ────────────────────────────────────────────────────

    #[test]
    fn fact_known_values() {
        assert_eq!(invoke("FACT", &[num(0.0)]), num(1.0));
        assert_eq!(invoke("FACT", &[num(1.0)]), num(1.0));
        assert_eq!(invoke("FACT", &[num(5.0)]), num(120.0));
        assert_eq!(invoke("FACT", &[num(10.0)]), num(3628800.0));
    }

    #[test]
    fn fact_negative_or_fraction_errors() {
        let mut c = ctx();
        assert_eq!(invoke_ctx("FACT", &[num(-1.0)], &mut c), miss());
        assert!(c.error_flag);
        let mut c2 = ctx();
        assert_eq!(invoke_ctx("FACT", &[num(2.5)], &mut c2), miss());
        assert!(c2.error_flag);
    }

    #[test]
    fn fact_missing() {
        assert_eq!(invoke("FACT", &[miss()]), miss());
    }

    #[test]
    fn comb_table() {
        assert_eq!(invoke("COMB", &[num(5.0), num(2.0)]), num(10.0));
        assert_eq!(invoke("COMB", &[num(10.0), num(0.0)]), num(1.0));
        assert_eq!(invoke("COMB", &[num(10.0), num(10.0)]), num(1.0));
        // k > n → 0.
        assert_eq!(invoke("COMB", &[num(3.0), num(5.0)]), num(0.0));
        assert_eq!(invoke("COMB", &[num(52.0), num(5.0)]), num(2598960.0));
    }

    #[test]
    fn perm_table() {
        // PERM(n) = n!.
        assert_eq!(invoke("PERM", &[num(5.0)]), num(120.0));
        // PERM(5, 2) = 5*4 = 20.
        assert_eq!(invoke("PERM", &[num(5.0), num(2.0)]), num(20.0));
        assert_eq!(invoke("PERM", &[num(10.0), num(3.0)]), num(720.0));
    }

    #[test]
    fn perm_missing() {
        assert_eq!(invoke("PERM", &[miss()]), miss());
    }

    // ── GAMMA / LGAMMA ────────────────────────────────────────────────────────

    #[test]
    fn gamma_known_values() {
        // Γ(n) = (n-1)!.
        approx_tol(invoke("GAMMA", &[num(1.0)]), 1.0, 1e-7);
        approx_tol(invoke("GAMMA", &[num(5.0)]), 24.0, 1e-6);
        approx_tol(invoke("GAMMA", &[num(6.0)]), 120.0, 1e-5);
        // Γ(0.5) = sqrt(pi).
        approx_tol(invoke("GAMMA", &[num(0.5)]), std::f64::consts::PI.sqrt(), 1e-7);
    }

    #[test]
    fn gamma_pole_errors() {
        let mut c = ctx();
        let r = invoke_ctx("GAMMA", &[num(0.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
        let mut c2 = ctx();
        assert_eq!(invoke_ctx("GAMMA", &[num(-3.0)], &mut c2), miss());
        assert!(c2.error_flag);
    }

    #[test]
    fn gamma_missing() {
        assert_eq!(invoke("GAMMA", &[miss()]), miss());
    }

    #[test]
    fn lgamma_known_values() {
        // ln Γ(5) = ln(24).
        approx_tol(invoke("LGAMMA", &[num(5.0)]), 24.0_f64.ln(), 1e-7);
        approx_tol(invoke("LGAMMA", &[num(1.0)]), 0.0, 1e-9);
        // ln Γ(100) connu.
        approx_tol(invoke("LGAMMA", &[num(10.0)]), 362880.0_f64.ln(), 1e-6);
    }

    #[test]
    fn lgamma_nonpositive_errors() {
        let mut c = ctx();
        assert_eq!(invoke_ctx("LGAMMA", &[num(0.0)], &mut c), miss());
        assert!(c.error_flag);
    }

    // ── DIGAMMA ───────────────────────────────────────────────────────────────

    #[test]
    fn digamma_known_values() {
        // ψ(1) = -γ (constante d'Euler-Mascheroni ≈ -0.5772156649).
        approx_tol(invoke("DIGAMMA", &[num(1.0)]), -0.577_215_664_9, 1e-8);
        // ψ(2) = 1 - γ.
        approx_tol(invoke("DIGAMMA", &[num(2.0)]), 1.0 - 0.577_215_664_9, 1e-8);
    }

    #[test]
    fn digamma_nonpositive_errors() {
        let mut c = ctx();
        assert_eq!(invoke_ctx("DIGAMMA", &[num(0.0)], &mut c), miss());
        assert!(c.error_flag);
    }

    // ── BETA ──────────────────────────────────────────────────────────────────

    #[test]
    fn beta_known_values() {
        // BETA(a, b) = Γ(a)Γ(b)/Γ(a+b). BETA(1, 1) = 1.
        approx_tol(invoke("BETA", &[num(1.0), num(1.0)]), 1.0, 1e-9);
        // BETA(2, 3) = 1!·2!/4! = 2/24 = 1/12.
        approx_tol(invoke("BETA", &[num(2.0), num(3.0)]), 1.0 / 12.0, 1e-9);
    }

    #[test]
    fn beta_invalid_domain_errors() {
        let mut c = ctx();
        assert_eq!(invoke_ctx("BETA", &[num(0.0), num(1.0)], &mut c), miss());
        assert!(c.error_flag);
    }

    // ── ROUNDZ (round-half-even) ──────────────────────────────────────────────

    #[test]
    fn roundz_half_even() {
        // 2.5 → 2 (vers le pair), contrairement à ROUND qui donne 3.
        assert_eq!(invoke("ROUNDZ", &[num(2.5)]), num(2.0));
        assert_eq!(invoke("ROUNDZ", &[num(3.5)]), num(4.0));
        assert_eq!(invoke("ROUNDZ", &[num(0.5)]), num(0.0));
        assert_eq!(invoke("ROUNDZ", &[num(1.5)]), num(2.0));
    }

    #[test]
    fn roundz_with_unit() {
        assert_eq!(invoke("ROUNDZ", &[num(2.125), num(0.01)]), num(2.12));
    }

    #[test]
    fn roundz_missing() {
        assert_eq!(invoke("ROUNDZ", &[miss()]), miss());
    }

    // ── RANGE ─────────────────────────────────────────────────────────────────

    #[test]
    fn range_table() {
        assert_eq!(invoke("RANGE", &[num(1.0), num(5.0), num(3.0)]), num(4.0));
        // ignore les manquants.
        assert_eq!(invoke("RANGE", &[num(2.0), miss(), num(8.0)]), num(6.0));
        // tous manquants → missing.
        assert_eq!(invoke("RANGE", &[miss(), miss()]), miss());
    }

    // ── LARGEST / SMALLEST / ORDINAL ──────────────────────────────────────────

    #[test]
    fn largest_table() {
        // 2e plus grand parmi 1,3,5,7.
        assert_eq!(
            invoke("LARGEST", &[num(2.0), num(1.0), num(3.0), num(5.0), num(7.0)]),
            num(5.0)
        );
    }

    #[test]
    fn largest_ignores_missing() {
        // LARGEST(1, ., 4, .) = 4 (manquants ignorés).
        assert_eq!(
            invoke("LARGEST", &[num(1.0), miss(), num(4.0), miss()]),
            num(4.0)
        );
        // k au-delà du nombre de non-manquants → missing.
        assert_eq!(
            invoke("LARGEST", &[num(3.0), num(1.0), miss(), num(2.0)]),
            miss()
        );
    }

    #[test]
    fn smallest_table() {
        assert_eq!(
            invoke("SMALLEST", &[num(1.0), num(5.0), num(3.0), num(8.0)]),
            num(3.0)
        );
        // 2e plus petit, manquants ignorés.
        assert_eq!(
            invoke("SMALLEST", &[num(2.0), num(5.0), miss(), num(3.0), num(8.0)]),
            num(5.0)
        );
    }

    #[test]
    fn ordinal_is_smallest() {
        // ORDINAL(2, 5, 3, 8) = 2e plus petit = 5.
        assert_eq!(
            invoke("ORDINAL", &[num(2.0), num(5.0), num(3.0), num(8.0)]),
            num(5.0)
        );
    }

    #[test]
    fn smallest_k_out_of_range() {
        assert_eq!(invoke("SMALLEST", &[num(5.0), num(1.0), num(2.0)]), miss());
    }
}
