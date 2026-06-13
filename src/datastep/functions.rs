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
// Date functions
// ──────────────────────────────────────────────────────────────────────────────

fn fn_today(_args: &[Value], _ctx: &mut EvalCtx) -> Value {
    Value::Num(today_sas())
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
    // Conversion (PUT/INPUT) — délèguent au moteur de formats (M4).
    ("PUT", fn_put),
    ("INPUT", fn_input),
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
}
