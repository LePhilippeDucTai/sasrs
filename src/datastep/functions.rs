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
// M15.2 Mathematical functions (M15.2)
// ──────────────────────────────────────────────────────────────────────────────

/// CEIL(x): smallest integer ≥ x.
fn fn_ceil(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(f.ceil()),
        },
    }
}

/// FLOOR(x): largest integer ≤ x.
fn fn_floor(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(f.floor()),
        },
    }
}

/// SIGN(x): return -1.0 for negative, 0.0 for zero, 1.0 for positive.
fn fn_sign(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                let sign = if f < 0.0 {
                    -1.0
                } else if f > 0.0 {
                    1.0
                } else {
                    0.0
                };
                Value::Num(sign)
            }
        },
    }
}

/// SIN(x): sine (x in radians).
fn fn_sin(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(f.sin()),
        },
    }
}

/// COS(x): cosine (x in radians).
fn fn_cos(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(f.cos()),
        },
    }
}

/// TAN(x): tangent (x in radians).
fn fn_tan(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(f.tan()),
        },
    }
}

/// ARSIN(x): arcsine (domain -1 to +1, result in radians).
fn fn_arsin(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                if f < -1.0 || f > 1.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    Value::missing()
                } else {
                    Value::Num(f.asin())
                }
            }
        },
    }
}

/// ARCOS(x): arccosine (domain -1 to +1, result in radians).
fn fn_arcos(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                if f < -1.0 || f > 1.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    Value::missing()
                } else {
                    Value::Num(f.acos())
                }
            }
        },
    }
}

/// ATAN(x): arctangent (result in radians).
fn fn_atan(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(f.atan()),
        },
    }
}

/// ATAN2(y, x): two-argument arctangent (atan(y/x) with quadrant correction).
fn fn_atan2(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (None, _) | (_, None) => Value::missing(),
        (Some(y), Some(x)) => Value::Num(y.atan2(x)),
    }
}

/// SINH(x): hyperbolic sine.
fn fn_sinh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(f.sinh()),
        },
    }
}

/// COSH(x): hyperbolic cosine.
fn fn_cosh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(f.cosh()),
        },
    }
}

/// TANH(x): hyperbolic tangent.
fn fn_tanh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => Value::Num(f.tanh()),
        },
    }
}

/// FACT(n): factorial (n! where n ≥ 0 integer).
/// n < 0 or non-integer → missing + error.
/// overflow → missing + warning.
fn fn_fact(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                // Check if integer
                if f.fract() != 0.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    return Value::missing();
                }
                // Check if non-negative
                if f < 0.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    return Value::missing();
                }
                let n = f as u32;
                // Compute factorial with overflow check
                let mut result = 1i64;
                for i in 2..=n as i64 {
                    match result.checked_mul(i) {
                        Some(r) => result = r,
                        None => {
                            // Overflow
                            return Value::missing();
                        }
                    }
                }
                Value::Num(result as f64)
            }
        },
    }
}

/// COMB(n, k): binomial coefficient C(n,k) = n! / (k!(n-k)!).
/// k > n or k < 0 → 0.
/// invalid inputs → missing + error.
fn fn_comb(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (None, _) | (_, None) => Value::missing(),
        (Some(nf), Some(kf)) => {
            // Check if integers
            if nf.fract() != 0.0 || kf.fract() != 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            let n = nf as i64;
            let k = kf as i64;
            // Check if non-negative
            if n < 0 || k < 0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            // k > n → 0
            if k > n {
                return Value::Num(0.0);
            }
            // Compute C(n,k) = n! / (k!(n-k)!)
            // Use efficient formula: C(n,k) = n * (n-1) * ... * (n-k+1) / (k!)
            let k = k.min(n - k); // Use symmetry to reduce computation
            let mut result = 1i64;
            for i in 0..k {
                match result.checked_mul(n - i) {
                    Some(r) => result = r,
                    None => return Value::missing(), // Overflow
                }
                result /= i + 1;
            }
            Value::Num(result as f64)
        }
    }
}

/// PERM(n, k): permutation P(n,k) = n! / (n-k)!.
/// k > n or k < 0 → 0.
/// invalid inputs → missing + error.
fn fn_perm(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (None, _) | (_, None) => Value::missing(),
        (Some(nf), Some(kf)) => {
            // Check if integers
            if nf.fract() != 0.0 || kf.fract() != 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            let n = nf as i64;
            let k = kf as i64;
            // Check if non-negative
            if n < 0 || k < 0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            // k > n → 0
            if k > n {
                return Value::Num(0.0);
            }
            // Compute P(n,k) = n * (n-1) * ... * (n-k+1)
            let mut result = 1i64;
            for i in 0..k {
                match result.checked_mul(n - i) {
                    Some(r) => result = r,
                    None => return Value::missing(), // Overflow
                }
            }
            Value::Num(result as f64)
        }
    }
}

/// GAMMA(x): gamma function Γ(x).
/// x ≤ 0 integer → missing + error.
/// x > 170 → infinity.
fn fn_gamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => {
                // Check if x <= 0 and integer
                if x <= 0.0 && x.fract() == 0.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    return Value::missing();
                }
                // For x > 170, Gamma(x) overflows; return infinity
                if x > 170.0 {
                    return Value::Num(f64::INFINITY);
                }
                // Use Stirling's approximation for large x
                // For small x, use the recurrence relation or direct computation
                let result = gamma_approx(x);
                Value::Num(result)
            }
        },
    }
}

/// LGAMMA(x): log-gamma log|Γ(x)|.
/// x ≤ 0 integer → missing + error.
fn fn_lgamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => {
                // Check if x <= 0 and integer
                if x <= 0.0 && x.fract() == 0.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    return Value::missing();
                }
                let result = lgamma_approx(x);
                Value::Num(result)
            }
        },
    }
}

/// DIGAMMA(x): digamma ψ(x) = d/dx log Γ(x).
/// x ≤ 0 integer → missing + error.
fn fn_digamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => {
                // Check if x <= 0 and integer
                if x <= 0.0 && x.fract() == 0.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    return Value::missing();
                }
                let result = digamma_approx(x);
                Value::Num(result)
            }
        },
    }
}

/// BETA(a, b): beta function B(a,b) = Γ(a)Γ(b) / Γ(a+b).
/// a, b > 0 required; invalid → missing + error.
fn fn_beta(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (None, _) | (_, None) => Value::missing(),
        (Some(a), Some(b)) => {
            if a <= 0.0 || b <= 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            let result = (gamma_approx(a) * gamma_approx(b)) / gamma_approx(a + b);
            Value::Num(result)
        }
    }
}

/// ROUNDZ(x, unit): round x to nearest unit, ties to zero (vs. ROUND's half-away-from-zero).
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
                // Round to nearest unit, ties toward zero
                let scaled = x / unit;
                let rounded = if scaled >= 0.0 {
                    // For positive: if fractional part < 0.5, floor; >= 0.5, ceil
                    let int_part = scaled.floor();
                    let frac_part = scaled - int_part;
                    if frac_part < 0.5 {
                        int_part
                    } else if frac_part > 0.5 {
                        int_part + 1.0
                    } else {
                        // Tie: round toward zero
                        int_part
                    }
                } else {
                    // For negative: if fractional part > -0.5, ceil; <= -0.5, floor
                    let int_part = scaled.ceil();
                    let frac_part = scaled - int_part;
                    if frac_part > -0.5 {
                        int_part
                    } else if frac_part < -0.5 {
                        int_part - 1.0
                    } else {
                        // Tie: round toward zero
                        int_part
                    }
                };
                Value::Num(rounded * unit)
            }
        },
    }
}

/// RANGE(x1, x2, ...): max(args) - min(args); missing ignored.
fn fn_range(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut min_val: Option<f64> = None;
    let mut max_val: Option<f64> = None;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            min_val = Some(match min_val {
                None => f,
                Some(m) => if f < m { f } else { m },
            });
            max_val = Some(match max_val {
                None => f,
                Some(m) => if f > m { f } else { m },
            });
        }
    }
    match (min_val, max_val) {
        (Some(min), Some(max)) => Value::Num(max - min),
        _ => Value::missing(),
    }
}

/// LARGEST(k, x1, x2, ...): kth largest value.
/// k ≤ 0 or k > count → missing.
fn fn_largest(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::missing();
    }
    let k_v = &args[0];
    let k = match coerce_num(k_v, ctx) {
        None => return Value::missing(),
        Some(f) => {
            if f.fract() != 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            f as i64
        }
    };

    let mut values: Vec<f64> = Vec::new();
    for a in &args[1..] {
        if let Some(f) = coerce_num(a, ctx) {
            values.push(f);
        }
    }

    if k <= 0 || k > values.len() as i64 {
        return Value::missing();
    }

    // Sort in descending order and get kth element (1-based)
    values.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    Value::Num(values[(k - 1) as usize])
}

/// SMALLEST(k, x1, x2, ...): kth smallest value.
/// k ≤ 0 or k > count → missing.
fn fn_smallest(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::missing();
    }
    let k_v = &args[0];
    let k = match coerce_num(k_v, ctx) {
        None => return Value::missing(),
        Some(f) => {
            if f.fract() != 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            f as i64
        }
    };

    let mut values: Vec<f64> = Vec::new();
    for a in &args[1..] {
        if let Some(f) = coerce_num(a, ctx) {
            values.push(f);
        }
    }

    if k <= 0 || k > values.len() as i64 {
        return Value::missing();
    }

    // Sort in ascending order and get kth element (1-based)
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Value::Num(values[(k - 1) as usize])
}

/// ORDINAL(x): convert number to ordinal text ("1st", "2nd", "3rd", "4th", ...).
/// x must be integer; non-integer or invalid → empty string.
fn fn_ordinal(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::Char(String::new()),
            Some(f) => {
                if f.fract() != 0.0 {
                    return Value::Char(String::new());
                }
                let n = f as i64;
                let suffix = if n % 100 == 11 || n % 100 == 12 || n % 100 == 13 {
                    "th"
                } else {
                    match n % 10 {
                        1 => "st",
                        2 => "nd",
                        3 => "rd",
                        _ => "th",
                    }
                };
                Value::Char(format!("{}{}", n, suffix))
            }
        },
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Helper functions for special mathematical functions
// ──────────────────────────────────────────────────────────────────────────────

/// Stirling's approximation for gamma function.
/// Uses the formula: Γ(x) ≈ √(2π) * (x/e)^x * x^(-1/2)
/// More precisely: ln Γ(x) ≈ (x - 1/2) ln(x) - x + ln(2π)/2 + 1/(12x) - ...
fn gamma_approx(x: f64) -> f64 {
    if x < 0.5 {
        // Use reflection formula: Γ(x) = π / (sin(πx) * Γ(1-x))
        let pi = std::f64::consts::PI;
        pi / ((pi * x).sin() * gamma_approx(1.0 - x))
    } else {
        // Stirling's approximation for x >= 0.5
        let ln_gamma = lgamma_approx(x);
        ln_gamma.exp()
    }
}

/// Log-gamma approximation using Stirling's formula.
/// ln Γ(x) ≈ (x - 1/2) ln(x) - x + ln(2π)/2 + 1/(12x) - 1/(360x^3) + ...
fn lgamma_approx(x: f64) -> f64 {
    if x < 0.5 {
        // Use reflection formula: ln|Γ(x)| = ln(π) - ln|sin(πx)| - ln|Γ(1-x)|
        let pi = std::f64::consts::PI;
        pi.ln() - (pi * x).sin().abs().ln() - lgamma_approx(1.0 - x)
    } else if x < 1.5 {
        // For small x, use recursion: ln Γ(x+1) = ln(x) + ln Γ(x)
        lgamma_approx(x + 1.0) - x.ln()
    } else {
        // Stirling's approximation
        let ln_2pi = (2.0 * std::f64::consts::PI).ln();
        let x_minus_half = x - 0.5;
        x_minus_half * x.ln() - x + 0.5 * ln_2pi
            + 1.0 / (12.0 * x)
            - 1.0 / (360.0 * x * x * x)
    }
}

/// Digamma approximation using Stirling's derivative.
/// ψ(x) = d/dx ln Γ(x) ≈ ln(x) - 1/(2x) - 1/(12x^2) + 1/(120x^4) - ...
fn digamma_approx(x: f64) -> f64 {
    if x < 0.5 {
        // Use reflection formula: ψ(x) = -ψ(1-x) - π/tan(πx)
        let pi = std::f64::consts::PI;
        -digamma_approx(1.0 - x) - pi / (pi * x).tan()
    } else if x < 1.5 {
        // Use recursion: ψ(x+1) = ψ(x) + 1/x
        digamma_approx(x + 1.0) - 1.0 / x
    } else {
        // Asymptotic expansion
        let ln_x = x.ln();
        let inv_x = 1.0 / x;
        ln_x - 0.5 * inv_x - inv_x * inv_x / 12.0 + inv_x * inv_x * inv_x / 120.0
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

/// FIND(s, target[, startPos[, modifiers]]): return 1-based position of first
/// occurrence of target in s, starting at startPos. If not found, return 0.
/// Modifiers: 'i' for case-insensitive.
fn fn_find(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let target = coerce_char(&args[1]);

    if target.is_empty() {
        return Value::Num(0.0);
    }

    let start_pos = if args.len() >= 3 {
        match coerce_num(&args[2], ctx) {
            None => return Value::Num(0.0),
            Some(f) => (f as i64).max(1),
        }
    } else {
        1
    };

    let case_insensitive = if args.len() >= 4 {
        let modifiers = coerce_char(&args[3]);
        modifiers.to_lowercase().contains('i')
    } else {
        false
    };

    let chars: Vec<char> = s.chars().collect();
    if start_pos < 1 || start_pos as usize > chars.len() {
        return Value::Num(0.0);
    }

    let search_from_char_idx = start_pos as usize;  // startPos is exclusive (1-based), skip to next char

    let target_search = if case_insensitive {
        target.to_lowercase()
    } else {
        target.clone()
    };

    // Search in the substring starting after startPos
    let search_text = chars[search_from_char_idx..].iter().collect::<String>();
    if case_insensitive && search_text.is_empty() {
        return Value::Num(0.0);
    }

    match if case_insensitive {
        search_text.to_lowercase().find(&target_search)
    } else {
        search_text.find(&target_search)
    } {
        None => Value::Num(0.0),
        Some(byte_pos) => {
            let found_char_idx = search_text[..byte_pos].chars().count();
            let char_pos = search_from_char_idx + found_char_idx + 1;
            Value::Num(char_pos as f64)
        }
    }
}

/// FINDC(s, target[, startPos[, modifiers]]): like FIND but target is a set of
/// characters; find first char from target in s.
fn fn_findc(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let target = coerce_char(&args[1]);

    if target.is_empty() {
        return Value::Num(0.0);
    }

    let start_pos = if args.len() >= 3 {
        match coerce_num(&args[2], ctx) {
            None => return Value::Num(0.0),
            Some(f) => (f as i64).max(1),
        }
    } else {
        1
    };

    let case_insensitive = if args.len() >= 4 {
        let modifiers = coerce_char(&args[3]);
        modifiers.to_lowercase().contains('i')
    } else {
        false
    };

    let chars: Vec<char> = s.chars().collect();
    if start_pos < 1 || start_pos as usize > chars.len() {
        return Value::Num(0.0);
    }

    let target_chars: Vec<char> = if case_insensitive {
        target.to_lowercase().chars().collect()
    } else {
        target.chars().collect()
    };

    for (i, &c) in chars.iter().enumerate().skip((start_pos - 1) as usize) {
        let test_c = if case_insensitive { c.to_lowercase().to_string() } else { c.to_string() };
        if target_chars.contains(&test_c.chars().next().unwrap_or('?')) {
            return Value::Num((i + 1) as f64);
        }
    }
    Value::Num(0.0)
}

/// COUNT(s, target[, modifiers]): count occurrences of target substring in s.
/// Modifiers: 'i' for case-insensitive.
fn fn_count(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let target = coerce_char(&args[1]);

    if target.is_empty() {
        return Value::Num(0.0);
    }

    let case_insensitive = if args.len() >= 3 {
        let modifiers = coerce_char(&args[2]);
        modifiers.to_lowercase().contains('i')
    } else {
        false
    };

    let search_str = if case_insensitive { s.to_lowercase() } else { s.clone() };
    let target_str = if case_insensitive { target.to_lowercase() } else { target.clone() };

    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = search_str[start..].find(&target_str as &str) {
        count += 1;
        start += pos + target_str.len();
    }
    Value::Num(count as f64)
}

/// COUNTC(s, target[, modifiers]): count occurrences of any character from
/// target set in s.
fn fn_countc(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let target = coerce_char(&args[1]);

    if target.is_empty() {
        return Value::Num(0.0);
    }

    let case_insensitive = if args.len() >= 3 {
        let modifiers = coerce_char(&args[2]);
        modifiers.to_lowercase().contains('i')
    } else {
        false
    };

    let target_chars: Vec<char> = if case_insensitive {
        target.to_lowercase().chars().collect()
    } else {
        target.chars().collect()
    };

    let count = s.chars().filter(|c| {
        let test_c = if case_insensitive {
            c.to_lowercase().next().unwrap_or('?')
        } else {
            *c
        };
        target_chars.contains(&test_c)
    }).count();

    Value::Num(count as f64)
}

/// VERIFY(s, target[, modifiers]): return 1-based position of first character
/// in s NOT in target set. Return 0 if all chars in s are in target.
fn fn_verify(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let target = coerce_char(&args[1]);

    if target.is_empty() {
        return if s.is_empty() { Value::Num(0.0) } else { Value::Num(1.0) };
    }

    let case_insensitive = if args.len() >= 3 {
        let modifiers = coerce_char(&args[2]);
        modifiers.to_lowercase().contains('i')
    } else {
        false
    };

    let target_chars: Vec<char> = if case_insensitive {
        target.to_lowercase().chars().collect()
    } else {
        target.chars().collect()
    };

    for (i, c) in s.chars().enumerate() {
        let test_c = if case_insensitive {
            c.to_lowercase().next().unwrap_or('?')
        } else {
            c
        };
        if !target_chars.contains(&test_c) {
            return Value::Num((i + 1) as f64);
        }
    }
    Value::Num(0.0)
}

/// TRANSLATE(s, to, from): replace each char in from with corresponding char in to.
/// If to is shorter than from, chars in from beyond len(to) are removed.
fn fn_translate(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let to = coerce_char(&args[1]);
    let from = coerce_char(&args[2]);

    if from.is_empty() {
        return Value::Char(s);
    }

    let to_chars: Vec<char> = to.chars().collect();
    let from_chars: Vec<char> = from.chars().collect();

    let result: String = s.chars().map(|c| {
        match from_chars.iter().position(|&fc| fc == c) {
            Some(pos) => {
                if pos < to_chars.len() {
                    to_chars[pos]
                } else {
                    // char in from beyond len(to) → remove it
                    return '\0';  // placeholder, will be filtered
                }
            }
            None => c,
        }
    }).filter(|&c| c != '\0').collect();

    Value::Char(result)
}

/// REVERSE(s): reverse the string s.
fn fn_reverse(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            let reversed: String = s.chars().rev().collect();
            Value::Char(reversed)
        }
    }
}

/// REPEAT(s, n): repeat string s n times. n is numeric, truncated to integer.
/// n<0 → "".
fn fn_repeat(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    match coerce_num(&args[1], ctx) {
        None => Value::Char(String::new()),
        Some(f) => {
            let n = f.trunc() as i64;
            if n < 0 {
                Value::Char(String::new())
            } else {
                let result = s.repeat(n as usize);
                Value::Char(result)
            }
        }
    }
}

/// PROPCASE(s[, delim]): proper case — capitalize first letter of each word
/// (words separated by delim, default ' ').
fn fn_propcase(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let delim = if args.len() >= 2 {
        coerce_char(&args[1])
    } else {
        " ".to_string()
    };

    if delim.is_empty() {
        // No delimiter: treat entire string as one word
        if s.is_empty() {
            return Value::Char(String::new());
        }
        let mut chars = s.chars();
        let first = chars.next().unwrap().to_uppercase().to_string();
        let rest: String = chars.map(|c| c.to_lowercase().to_string()).collect();
        return Value::Char(format!("{}{}", first, rest));
    }

    // Split by delimiter and capitalize each word
    let delim_chars: Vec<char> = delim.chars().collect();
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in s.chars() {
        if delim_chars.contains(&c) {
            result.push(c);
            capitalize_next = true;
        } else if capitalize_next {
            for ch in c.to_uppercase() {
                result.push(ch);
            }
            capitalize_next = false;
        } else {
            for ch in c.to_lowercase() {
                result.push(ch);
            }
        }
    }

    Value::Char(result)
}

/// COMPBL(s): compress multiple blanks to single, remove leading/trailing blanks.
fn fn_compbl(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let trimmed = s.trim();
    let result: String = trimmed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Value::Char(result)
}

/// SUBSTRN(s, pos[, len]): like SUBSTR but out-of-bounds pos returns ""
/// WITHOUT setting _ERROR_.
fn fn_substrn(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let chars: Vec<char> = s.chars().collect();
    let slen = chars.len() as i64;

    let pos = match args.get(1) {
        None => return Value::Char(String::new()),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::Char(String::new()),
            Some(f) => f as i64,
        },
    };

    // Out of bounds → "" WITHOUT setting _ERROR_
    if pos < 1 || pos > slen {
        return Value::Char(String::new());
    }

    let start = (pos - 1) as usize;
    let end = if let Some(len_v) = args.get(2) {
        match coerce_num(len_v, ctx) {
            None => return Value::Char(String::new()),
            Some(l) => {
                let l = l as i64;
                if l < 0 {
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

/// CHAR(n): return character with Unicode code point n (numeric input).
/// CHAR(0) returns empty string.
fn fn_char(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::Char(String::new()),
            Some(f) => {
                let code = f as u32;
                if code == 0 {
                    Value::Char(String::new())
                } else {
                    match std::char::from_u32(code) {
                        Some(c) => Value::Char(c.to_string()),
                        None => Value::Char(String::new()),
                    }
                }
            }
        }
    }
}

/// RANK(s): return Unicode code point of first character of s.
fn fn_rank(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Num(0.0),
        Some(v) => {
            let s = coerce_char(v);
            match s.chars().next() {
                None => Value::Num(0.0),
                Some(c) => Value::Num(c as u32 as f64),
            }
        }
    }
}

/// BYTE(n): alias for CHAR(n).
fn fn_byte(args: &[Value], ctx: &mut EvalCtx) -> Value {
    fn_char(args, ctx)
}

/// WHICHC(needle, haystack1[, haystack2, ...]): return 1-based position of
/// first argument (after needle) that equals needle. Return 0 if none found.
fn fn_whichc(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Num(0.0);
    }
    let needle = coerce_char(&args[0]);
    for (i, haystack) in args[1..].iter().enumerate() {
        if coerce_char(haystack) == needle {
            return Value::Num((i + 1) as f64);
        }
    }
    Value::Num(0.0)
}

/// CATQ(delim, item1, item2, ...): concatenate items with delimiter, quoting
/// items that contain delimiter or quotes. Escape internal quotes with double quotes.
fn fn_catq(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let delim = coerce_char(&args[0]);
    let mut result = Vec::new();

    for item in &args[1..] {
        let s = coerce_char(item);
        let needs_quoting = s.contains(&delim) || s.contains('"');
        let quoted = if needs_quoting {
            let escaped = s.replace('"', "\"\"");
            format!("\"{}\"", escaped)
        } else {
            s
        };
        result.push(quoted);
    }

    Value::Char(result.join(&delim))
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

    // ── M15.2 Mathematical Functions ───────────────────────────────────────────

    // ── CEIL ───────────────────────────────────────────────────────────────────

    #[test]
    fn ceil_positive() {
        assert_eq!(invoke("CEIL", &[num(3.2)]), num(4.0));
    }

    #[test]
    fn ceil_negative() {
        assert_eq!(invoke("CEIL", &[num(-3.2)]), num(-3.0));
    }

    #[test]
    fn ceil_integer() {
        assert_eq!(invoke("CEIL", &[num(3.0)]), num(3.0));
    }

    // ── FLOOR ──────────────────────────────────────────────────────────────────

    #[test]
    fn floor_positive() {
        assert_eq!(invoke("FLOOR", &[num(3.7)]), num(3.0));
    }

    #[test]
    fn floor_negative() {
        assert_eq!(invoke("FLOOR", &[num(-3.7)]), num(-4.0));
    }

    #[test]
    fn floor_integer() {
        assert_eq!(invoke("FLOOR", &[num(3.0)]), num(3.0));
    }

    // ── SIGN ───────────────────────────────────────────────────────────────────

    #[test]
    fn sign_positive() {
        assert_eq!(invoke("SIGN", &[num(5.0)]), num(1.0));
    }

    #[test]
    fn sign_negative() {
        assert_eq!(invoke("SIGN", &[num(-5.0)]), num(-1.0));
    }

    #[test]
    fn sign_zero() {
        assert_eq!(invoke("SIGN", &[num(0.0)]), num(0.0));
    }

    // ── SIN ────────────────────────────────────────────────────────────────────

    #[test]
    fn sin_zero() {
        assert_eq!(invoke("SIN", &[num(0.0)]), num(0.0));
    }

    #[test]
    fn sin_pi_half() {
        let result = invoke("SIN", &[num(std::f64::consts::PI / 2.0)]);
        assert!((coerce_num(&result, &mut ctx()).unwrap() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn sin_missing() {
        assert_eq!(invoke("SIN", &[miss()]), miss());
    }

    // ── COS ────────────────────────────────────────────────────────────────────

    #[test]
    fn cos_zero() {
        assert_eq!(invoke("COS", &[num(0.0)]), num(1.0));
    }

    #[test]
    fn cos_pi() {
        let result = invoke("COS", &[num(std::f64::consts::PI)]);
        assert!((coerce_num(&result, &mut ctx()).unwrap() + 1.0).abs() < 1e-10);
    }

    #[test]
    fn cos_missing() {
        assert_eq!(invoke("COS", &[miss()]), miss());
    }

    // ── TAN ────────────────────────────────────────────────────────────────────

    #[test]
    fn tan_zero() {
        assert_eq!(invoke("TAN", &[num(0.0)]), num(0.0));
    }

    #[test]
    fn tan_pi_quarter() {
        let result = invoke("TAN", &[num(std::f64::consts::PI / 4.0)]);
        assert!((coerce_num(&result, &mut ctx()).unwrap() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn tan_missing() {
        assert_eq!(invoke("TAN", &[miss()]), miss());
    }

    // ── ARSIN ──────────────────────────────────────────────────────────────────

    #[test]
    fn arsin_zero() {
        assert_eq!(invoke("ARSIN", &[num(0.0)]), num(0.0));
    }

    #[test]
    fn arsin_one() {
        let result = invoke("ARSIN", &[num(1.0)]);
        assert!((coerce_num(&result, &mut ctx()).unwrap() - std::f64::consts::PI / 2.0).abs() < 1e-10);
    }

    #[test]
    fn arsin_out_of_domain() {
        let mut c = ctx();
        let r = invoke_ctx("ARSIN", &[num(1.5)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    // ── ARCOS ──────────────────────────────────────────────────────────────────

    #[test]
    fn arcos_one() {
        assert_eq!(invoke("ARCOS", &[num(1.0)]), num(0.0));
    }

    #[test]
    fn arcos_zero() {
        let result = invoke("ARCOS", &[num(0.0)]);
        assert!((coerce_num(&result, &mut ctx()).unwrap() - std::f64::consts::PI / 2.0).abs() < 1e-10);
    }

    #[test]
    fn arcos_out_of_domain() {
        let mut c = ctx();
        let r = invoke_ctx("ARCOS", &[num(-1.5)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    // ── ATAN ───────────────────────────────────────────────────────────────────

    #[test]
    fn atan_zero() {
        assert_eq!(invoke("ATAN", &[num(0.0)]), num(0.0));
    }

    #[test]
    fn atan_one() {
        let result = invoke("ATAN", &[num(1.0)]);
        assert!((coerce_num(&result, &mut ctx()).unwrap() - std::f64::consts::PI / 4.0).abs() < 1e-10);
    }

    #[test]
    fn atan_missing() {
        assert_eq!(invoke("ATAN", &[miss()]), miss());
    }

    // ── ATAN2 ──────────────────────────────────────────────────────────────────

    #[test]
    fn atan2_one_one() {
        let result = invoke("ATAN2", &[num(1.0), num(1.0)]);
        assert!((coerce_num(&result, &mut ctx()).unwrap() - std::f64::consts::PI / 4.0).abs() < 1e-10);
    }

    #[test]
    fn atan2_zero_one() {
        assert_eq!(invoke("ATAN2", &[num(0.0), num(1.0)]), num(0.0));
    }

    #[test]
    fn atan2_missing_first() {
        assert_eq!(invoke("ATAN2", &[miss(), num(1.0)]), miss());
    }

    // ── SINH ───────────────────────────────────────────────────────────────────

    #[test]
    fn sinh_zero() {
        assert_eq!(invoke("SINH", &[num(0.0)]), num(0.0));
    }

    #[test]
    fn sinh_positive() {
        let result = invoke("SINH", &[num(1.0)]);
        assert!(coerce_num(&result, &mut ctx()).unwrap() > 1.0);
    }

    #[test]
    fn sinh_missing() {
        assert_eq!(invoke("SINH", &[miss()]), miss());
    }

    // ── COSH ───────────────────────────────────────────────────────────────────

    #[test]
    fn cosh_zero() {
        assert_eq!(invoke("COSH", &[num(0.0)]), num(1.0));
    }

    #[test]
    fn cosh_positive() {
        let result = invoke("COSH", &[num(1.0)]);
        assert!(coerce_num(&result, &mut ctx()).unwrap() > 1.0);
    }

    #[test]
    fn cosh_missing() {
        assert_eq!(invoke("COSH", &[miss()]), miss());
    }

    // ── TANH ───────────────────────────────────────────────────────────────────

    #[test]
    fn tanh_zero() {
        assert_eq!(invoke("TANH", &[num(0.0)]), num(0.0));
    }

    #[test]
    fn tanh_large_positive() {
        let result = invoke("TANH", &[num(100.0)]);
        assert!((coerce_num(&result, &mut ctx()).unwrap() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn tanh_missing() {
        assert_eq!(invoke("TANH", &[miss()]), miss());
    }

    // ── FACT ───────────────────────────────────────────────────────────────────

    #[test]
    fn fact_five() {
        assert_eq!(invoke("FACT", &[num(5.0)]), num(120.0));
    }

    #[test]
    fn fact_zero() {
        assert_eq!(invoke("FACT", &[num(0.0)]), num(1.0));
    }

    #[test]
    fn fact_non_integer() {
        let mut c = ctx();
        let r = invoke_ctx("FACT", &[num(3.5)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    #[test]
    fn fact_negative() {
        let mut c = ctx();
        let r = invoke_ctx("FACT", &[num(-1.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    // ── COMB ───────────────────────────────────────────────────────────────────

    #[test]
    fn comb_basic() {
        // C(5, 2) = 10
        assert_eq!(invoke("COMB", &[num(5.0), num(2.0)]), num(10.0));
    }

    #[test]
    fn comb_k_greater_than_n() {
        // C(3, 5) = 0
        assert_eq!(invoke("COMB", &[num(3.0), num(5.0)]), num(0.0));
    }

    #[test]
    fn comb_k_equals_zero() {
        // C(5, 0) = 1
        assert_eq!(invoke("COMB", &[num(5.0), num(0.0)]), num(1.0));
    }

    #[test]
    fn comb_non_integer() {
        let mut c = ctx();
        let r = invoke_ctx("COMB", &[num(5.0), num(2.5)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    // ── PERM ───────────────────────────────────────────────────────────────────

    #[test]
    fn perm_basic() {
        // P(5, 2) = 20
        assert_eq!(invoke("PERM", &[num(5.0), num(2.0)]), num(20.0));
    }

    #[test]
    fn perm_k_greater_than_n() {
        // P(3, 5) = 0
        assert_eq!(invoke("PERM", &[num(3.0), num(5.0)]), num(0.0));
    }

    #[test]
    fn perm_k_equals_zero() {
        // P(5, 0) = 1
        assert_eq!(invoke("PERM", &[num(5.0), num(0.0)]), num(1.0));
    }

    #[test]
    fn perm_non_integer() {
        let mut c = ctx();
        let r = invoke_ctx("PERM", &[num(5.0), num(2.5)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    // ── GAMMA ──────────────────────────────────────────────────────────────────

    #[test]
    fn gamma_one() {
        let result = invoke("GAMMA", &[num(1.0)]);
        let val = coerce_num(&result, &mut ctx()).unwrap();
        assert!((val - 1.0).abs() < 0.001);
    }

    #[test]
    fn gamma_two() {
        let result = invoke("GAMMA", &[num(2.0)]);
        let val = coerce_num(&result, &mut ctx()).unwrap();
        assert!((val - 1.0).abs() < 0.001);
    }

    #[test]
    fn gamma_zero_or_negative_integer() {
        let mut c = ctx();
        let r = invoke_ctx("GAMMA", &[num(0.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    #[test]
    fn gamma_large_x() {
        let result = invoke("GAMMA", &[num(171.0)]);
        assert_eq!(result, num(f64::INFINITY));
    }

    // ── LGAMMA ────────────────────────────────────────────────────────────────

    #[test]
    fn lgamma_one() {
        let result = invoke("LGAMMA", &[num(1.0)]);
        let val = coerce_num(&result, &mut ctx()).unwrap();
        assert!(val.abs() < 0.001);
    }

    #[test]
    fn lgamma_two() {
        let result = invoke("LGAMMA", &[num(2.0)]);
        let val = coerce_num(&result, &mut ctx()).unwrap();
        assert!(val.abs() < 0.001);
    }

    #[test]
    fn lgamma_zero_or_negative_integer() {
        let mut c = ctx();
        let r = invoke_ctx("LGAMMA", &[num(-1.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    // ── DIGAMMA ───────────────────────────────────────────────────────────────

    #[test]
    fn digamma_one() {
        // ψ(1) ≈ -0.5772 (Euler-Mascheroni constant)
        let result = invoke("DIGAMMA", &[num(1.0)]);
        let val = coerce_num(&result, &mut ctx()).unwrap();
        assert!((val - (-0.5772156649)).abs() < 0.001);
    }

    #[test]
    fn digamma_zero_integer() {
        let mut c = ctx();
        let r = invoke_ctx("DIGAMMA", &[num(0.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    // ── BETA ───────────────────────────────────────────────────────────────────

    #[test]
    fn beta_one_one() {
        let result = invoke("BETA", &[num(1.0), num(1.0)]);
        let val = coerce_num(&result, &mut ctx()).unwrap();
        assert!((val - 1.0).abs() < 0.001);
    }

    #[test]
    fn beta_positive() {
        let result = invoke("BETA", &[num(2.0), num(2.0)]);
        let val = coerce_num(&result, &mut ctx()).unwrap();
        assert!((val - 1.0 / 6.0).abs() < 0.001);
    }

    #[test]
    fn beta_invalid_negative() {
        let mut c = ctx();
        let r = invoke_ctx("BETA", &[num(-1.0), num(1.0)], &mut c);
        assert_eq!(r, miss());
        assert!(c.error_flag);
    }

    // ── ROUNDZ ────────────────────────────────────────────────────────────────

    #[test]
    fn roundz_tie_toward_zero_positive() {
        // 2.5 should round to 2 (toward zero)
        assert_eq!(invoke("ROUNDZ", &[num(2.5)]), num(2.0));
    }

    #[test]
    fn roundz_tie_toward_zero_negative() {
        // -2.5 should round to -2 (toward zero)
        assert_eq!(invoke("ROUNDZ", &[num(-2.5)]), num(-2.0));
    }

    #[test]
    fn roundz_normal_positive() {
        // 2.3 should round to 2
        assert_eq!(invoke("ROUNDZ", &[num(2.3)]), num(2.0));
    }

    #[test]
    fn roundz_with_unit() {
        // 2.55 with unit 0.1 should round to 2.5
        assert_eq!(invoke("ROUNDZ", &[num(2.55), num(0.1)]), num(2.5));
    }

    // ── RANGE ─────────────────────────────────────────────────────────────────

    #[test]
    fn range_basic() {
        assert_eq!(invoke("RANGE", &[num(1.0), num(5.0), num(3.0)]), num(4.0));
    }

    #[test]
    fn range_ignores_missing() {
        assert_eq!(invoke("RANGE", &[miss(), num(1.0), num(5.0)]), num(4.0));
    }

    #[test]
    fn range_all_missing() {
        assert_eq!(invoke("RANGE", &[miss(), miss()]), miss());
    }

    #[test]
    fn range_negative() {
        assert_eq!(invoke("RANGE", &[num(-5.0), num(3.0)]), num(8.0));
    }

    // ── LARGEST ───────────────────────────────────────────────────────────────

    #[test]
    fn largest_second() {
        // 2nd largest of (3, 1, 5, 2)
        assert_eq!(invoke("LARGEST", &[num(2.0), num(3.0), num(1.0), num(5.0), num(2.0)]), num(3.0));
    }

    #[test]
    fn largest_first() {
        // 1st largest of (3, 1, 5)
        assert_eq!(invoke("LARGEST", &[num(1.0), num(3.0), num(1.0), num(5.0)]), num(5.0));
    }

    #[test]
    fn largest_out_of_range() {
        // 10th largest of only 3 values
        assert_eq!(invoke("LARGEST", &[num(10.0), num(1.0), num(2.0), num(3.0)]), miss());
    }

    #[test]
    fn largest_k_zero() {
        assert_eq!(invoke("LARGEST", &[num(0.0), num(1.0), num(2.0)]), miss());
    }

    // ── SMALLEST ──────────────────────────────────────────────────────────────

    #[test]
    fn smallest_second() {
        // 2nd smallest of (3, 1, 5, 2)
        assert_eq!(invoke("SMALLEST", &[num(2.0), num(3.0), num(1.0), num(5.0), num(2.0)]), num(2.0));
    }

    #[test]
    fn smallest_first() {
        // 1st smallest of (3, 1, 5)
        assert_eq!(invoke("SMALLEST", &[num(1.0), num(3.0), num(1.0), num(5.0)]), num(1.0));
    }

    #[test]
    fn smallest_out_of_range() {
        // 10th smallest of only 3 values
        assert_eq!(invoke("SMALLEST", &[num(10.0), num(1.0), num(2.0), num(3.0)]), miss());
    }

    #[test]
    fn smallest_k_negative() {
        assert_eq!(invoke("SMALLEST", &[num(-1.0), num(1.0), num(2.0)]), miss());
    }

    // ── ORDINAL ───────────────────────────────────────────────────────────────

    #[test]
    fn ordinal_first() {
        assert_eq!(invoke("ORDINAL", &[num(1.0)]), chr("1st"));
    }

    #[test]
    fn ordinal_second() {
        assert_eq!(invoke("ORDINAL", &[num(2.0)]), chr("2nd"));
    }

    #[test]
    fn ordinal_third() {
        assert_eq!(invoke("ORDINAL", &[num(3.0)]), chr("3rd"));
    }

    #[test]
    fn ordinal_fourth() {
        assert_eq!(invoke("ORDINAL", &[num(4.0)]), chr("4th"));
    }

    #[test]
    fn ordinal_eleventh() {
        assert_eq!(invoke("ORDINAL", &[num(11.0)]), chr("11th"));
    }

    #[test]
    fn ordinal_twelfth() {
        assert_eq!(invoke("ORDINAL", &[num(12.0)]), chr("12th"));
    }

    #[test]
    fn ordinal_thirteenth() {
        assert_eq!(invoke("ORDINAL", &[num(13.0)]), chr("13th"));
    }

    #[test]
    fn ordinal_twenty_first() {
        assert_eq!(invoke("ORDINAL", &[num(21.0)]), chr("21st"));
    }

    #[test]
    fn ordinal_non_integer() {
        assert_eq!(invoke("ORDINAL", &[num(3.5)]), chr(""));
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

    // ── FIND ──────────────────────────────────────────────────────────────────

    #[test]
    fn find_basic() {
        assert_eq!(invoke("FIND", &[chr("hello world"), chr("world")]), num(7.0));
    }

    #[test]
    fn find_not_found() {
        assert_eq!(invoke("FIND", &[chr("hello"), chr("xyz")]), num(0.0));
    }

    #[test]
    fn find_with_start_pos() {
        // Find "o" starting from position 5 in "hello world"
        assert_eq!(invoke("FIND", &[chr("hello world"), chr("o"), num(5.0)]), num(8.0));
    }

    #[test]
    fn find_case_insensitive() {
        assert_eq!(invoke("FIND", &[chr("Hello World"), chr("WORLD"), num(1.0), chr("i")]), num(7.0));
    }

    #[test]
    fn find_empty_target() {
        assert_eq!(invoke("FIND", &[chr("hello"), chr("")]), num(0.0));
    }

    // ── FINDC ─────────────────────────────────────────────────────────────────

    #[test]
    fn findc_basic() {
        assert_eq!(invoke("FINDC", &[chr("hello"), chr("lo")]), num(3.0));
    }

    #[test]
    fn findc_not_found() {
        assert_eq!(invoke("FINDC", &[chr("hello"), chr("xyz")]), num(0.0));
    }

    #[test]
    fn findc_with_start_pos() {
        assert_eq!(invoke("FINDC", &[chr("hello"), chr("lo"), num(4.0)]), num(4.0));
    }

    #[test]
    fn findc_case_insensitive() {
        assert_eq!(invoke("FINDC", &[chr("Hello"), chr("EL"), num(1.0), chr("i")]), num(2.0));
    }

    // ── COUNT ────────────────────────────────────────────────────────────────

    #[test]
    fn count_basic() {
        assert_eq!(invoke("COUNT", &[chr("hello hello"), chr("hello")]), num(2.0));
    }

    #[test]
    fn count_zero() {
        assert_eq!(invoke("COUNT", &[chr("hello"), chr("xyz")]), num(0.0));
    }

    #[test]
    fn count_overlapping() {
        assert_eq!(invoke("COUNT", &[chr("aaa"), chr("aa")]), num(1.0));
    }

    #[test]
    fn count_case_insensitive() {
        assert_eq!(invoke("COUNT", &[chr("Hello hello"), chr("HELLO"), chr("i")]), num(2.0));
    }

    // ── COUNTC ───────────────────────────────────────────────────────────────

    #[test]
    fn countc_basic() {
        assert_eq!(invoke("COUNTC", &[chr("hello"), chr("lo")]), num(3.0));
    }

    #[test]
    fn countc_zero() {
        assert_eq!(invoke("COUNTC", &[chr("hello"), chr("xyz")]), num(0.0));
    }

    #[test]
    fn countc_all_chars_in_set() {
        assert_eq!(invoke("COUNTC", &[chr("aaa"), chr("a")]), num(3.0));
    }

    #[test]
    fn countc_case_insensitive() {
        assert_eq!(invoke("COUNTC", &[chr("Hello"), chr("EL"), chr("i")]), num(3.0));
    }

    // ── VERIFY ───────────────────────────────────────────────────────────────

    #[test]
    fn verify_basic() {
        assert_eq!(invoke("VERIFY", &[chr("hello"), chr("helo")]), num(0.0));
    }

    #[test]
    fn verify_first_not_in_set() {
        assert_eq!(invoke("VERIFY", &[chr("xhello"), chr("helo")]), num(1.0));
    }

    #[test]
    fn verify_middle_not_in_set() {
        assert_eq!(invoke("VERIFY", &[chr("hello world"), chr("hello")]), num(6.0));
    }

    #[test]
    fn verify_empty_target() {
        assert_eq!(invoke("VERIFY", &[chr("hello"), chr("")]), num(1.0));
        assert_eq!(invoke("VERIFY", &[chr(""), chr("")]), num(0.0));
    }

    // ── TRANSLATE ────────────────────────────────────────────────────────────

    #[test]
    fn translate_basic() {
        assert_eq!(invoke("TRANSLATE", &[chr("hello"), chr("HELLO"), chr("hello")]), chr("HELLO"));
    }

    #[test]
    fn translate_partial_mapping() {
        assert_eq!(invoke("TRANSLATE", &[chr("hello"), chr("12"), chr("he")]), chr("12llo"));
    }

    #[test]
    fn translate_removal() {
        assert_eq!(invoke("TRANSLATE", &[chr("hello"), chr("1"), chr("helo")]), chr("1"));
    }

    #[test]
    fn translate_no_change() {
        assert_eq!(invoke("TRANSLATE", &[chr("hello"), chr("abc"), chr("xyz")]), chr("hello"));
    }

    // ── REVERSE ───────────────────────────────────────────────────────────────

    #[test]
    fn reverse_basic() {
        assert_eq!(invoke("REVERSE", &[chr("hello")]), chr("olleh"));
    }

    #[test]
    fn reverse_empty() {
        assert_eq!(invoke("REVERSE", &[chr("")]), chr(""));
    }

    #[test]
    fn reverse_single_char() {
        assert_eq!(invoke("REVERSE", &[chr("a")]), chr("a"));
    }

    // ── REPEAT ────────────────────────────────────────────────────────────────

    #[test]
    fn repeat_basic() {
        assert_eq!(invoke("REPEAT", &[chr("ab"), num(3.0)]), chr("ababab"));
    }

    #[test]
    fn repeat_zero_times() {
        assert_eq!(invoke("REPEAT", &[chr("hello"), num(0.0)]), chr(""));
    }

    #[test]
    fn repeat_negative_times() {
        assert_eq!(invoke("REPEAT", &[chr("hello"), num(-5.0)]), chr(""));
    }

    #[test]
    fn repeat_single_time() {
        assert_eq!(invoke("REPEAT", &[chr("hello"), num(1.0)]), chr("hello"));
    }

    #[test]
    fn repeat_truncates_decimal() {
        assert_eq!(invoke("REPEAT", &[chr("a"), num(3.7)]), chr("aaa"));
    }

    // ── PROPCASE ──────────────────────────────────────────────────────────────

    #[test]
    fn propcase_basic() {
        assert_eq!(invoke("PROPCASE", &[chr("hello world")]), chr("Hello World"));
    }

    #[test]
    fn propcase_mixed_case() {
        assert_eq!(invoke("PROPCASE", &[chr("HELLO world")]), chr("Hello World"));
    }

    #[test]
    fn propcase_custom_delimiter() {
        assert_eq!(invoke("PROPCASE", &[chr("hello-world"), chr("-")]), chr("Hello-World"));
    }

    #[test]
    fn propcase_empty() {
        assert_eq!(invoke("PROPCASE", &[chr("")]), chr(""));
    }

    #[test]
    fn propcase_single_word() {
        assert_eq!(invoke("PROPCASE", &[chr("hello")]), chr("Hello"));
    }

    // ── COMPBL ───────────────────────────────────────────────────────────────

    #[test]
    fn compbl_multiple_spaces() {
        assert_eq!(invoke("COMPBL", &[chr("hello    world")]), chr("hello world"));
    }

    #[test]
    fn compbl_leading_trailing() {
        assert_eq!(invoke("COMPBL", &[chr("  hello world  ")]), chr("hello world"));
    }

    #[test]
    fn compbl_mixed_whitespace() {
        assert_eq!(invoke("COMPBL", &[chr("hello  \t  world")]), chr("hello world"));
    }

    #[test]
    fn compbl_empty() {
        assert_eq!(invoke("COMPBL", &[chr("")]), chr(""));
    }

    // ── SUBSTRN ───────────────────────────────────────────────────────────────

    #[test]
    fn substrn_basic() {
        assert_eq!(invoke("SUBSTRN", &[chr("hello"), num(2.0), num(3.0)]), chr("ell"));
    }

    #[test]
    fn substrn_no_length() {
        assert_eq!(invoke("SUBSTRN", &[chr("hello"), num(3.0)]), chr("llo"));
    }

    #[test]
    fn substrn_out_of_bounds_no_error() {
        let mut c = ctx();
        let r = invoke_ctx("SUBSTRN", &[chr("abc"), num(10.0)], &mut c);
        assert_eq!(r, chr(""));
        assert!(!c.error_flag);  // Unlike SUBSTR, no error flag
    }

    #[test]
    fn substrn_pos_zero_no_error() {
        let mut c = ctx();
        let r = invoke_ctx("SUBSTRN", &[chr("abc"), num(0.0)], &mut c);
        assert_eq!(r, chr(""));
        assert!(!c.error_flag);
    }

    // ── CHAR ──────────────────────────────────────────────────────────────────

    #[test]
    fn char_ascii() {
        assert_eq!(invoke("CHAR", &[num(65.0)]), chr("A"));
    }

    #[test]
    fn char_space() {
        assert_eq!(invoke("CHAR", &[num(32.0)]), chr(" "));
    }

    #[test]
    fn char_zero() {
        assert_eq!(invoke("CHAR", &[num(0.0)]), chr(""));
    }

    #[test]
    fn char_unicode() {
        assert_eq!(invoke("CHAR", &[num(233.0)]), chr("é"));
    }

    // ── RANK ──────────────────────────────────────────────────────────────────

    #[test]
    fn rank_ascii() {
        assert_eq!(invoke("RANK", &[chr("A")]), num(65.0));
    }

    #[test]
    fn rank_space() {
        assert_eq!(invoke("RANK", &[chr(" ")]), num(32.0));
    }

    #[test]
    fn rank_empty() {
        assert_eq!(invoke("RANK", &[chr("")]), num(0.0));
    }

    #[test]
    fn rank_first_char_only() {
        assert_eq!(invoke("RANK", &[chr("ABC")]), num(65.0));
    }

    #[test]
    fn rank_unicode() {
        assert_eq!(invoke("RANK", &[chr("é")]), num(233.0));
    }

    // ── BYTE ──────────────────────────────────────────────────────────────────

    #[test]
    fn byte_basic() {
        assert_eq!(invoke("BYTE", &[num(65.0)]), chr("A"));
    }

    #[test]
    fn byte_same_as_char() {
        assert_eq!(invoke("BYTE", &[num(72.0)]), invoke("CHAR", &[num(72.0)]));
    }

    // ── WHICHC ───────────────────────────────────────────────────────────────

    #[test]
    fn whichc_first_match() {
        assert_eq!(
            invoke("WHICHC", &[chr("b"), chr("a"), chr("b"), chr("c")]),
            num(2.0)
        );
    }

    #[test]
    fn whichc_no_match() {
        assert_eq!(
            invoke("WHICHC", &[chr("x"), chr("a"), chr("b"), chr("c")]),
            num(0.0)
        );
    }

    #[test]
    fn whichc_first_is_match() {
        assert_eq!(
            invoke("WHICHC", &[chr("a"), chr("a"), chr("b"), chr("c")]),
            num(1.0)
        );
    }

    #[test]
    fn whichc_empty_needle() {
        assert_eq!(
            invoke("WHICHC", &[chr(""), chr(""), chr("b")]),
            num(1.0)
        );
    }

    // ── CATQ ──────────────────────────────────────────────────────────────────

    #[test]
    fn catq_no_quoting_needed() {
        assert_eq!(
            invoke("CATQ", &[chr(","), chr("a"), chr("b")]),
            chr("a,b")
        );
    }

    #[test]
    fn catq_quote_on_delimiter() {
        assert_eq!(
            invoke("CATQ", &[chr(","), chr("a,b"), chr("c")]),
            chr("\"a,b\",c")
        );
    }

    #[test]
    fn catq_quote_on_internal_quote() {
        assert_eq!(
            invoke("CATQ", &[chr(","), chr("a\"b"), chr("c")]),
            chr("\"a\"\"b\",c")
        );
    }

    #[test]
    fn catq_both_conditions() {
        assert_eq!(
            invoke("CATQ", &[chr(","), chr("a,\"b"), chr("c")]),
            chr("\"a,\"\"b\",c")
        );
    }

    #[test]
    fn catq_empty_items() {
        assert_eq!(
            invoke("CATQ", &[chr(","), chr("a"), chr(""), chr("c")]),
            chr("a,,c")
        );
    }
}
