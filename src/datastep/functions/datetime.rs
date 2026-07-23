// ──────────────────────────────────────────────────────────────────────────────
// Date functions
// ──────────────────────────────────────────────────────────────────────────────

use super::*;

pub(super) fn today_sas() -> f64 {
    // Jours depuis 1960-01-01 : jours Unix + offset 1960→1970 (3653,
    // constante partagée avec dataset.rs — l'époque SAS précède Unix).
    let unix_days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86400) as f64)
        .unwrap_or(0.0);
    unix_days + crate::dataset::SAS_EPOCH_OFFSET_DAYS
}

/// Checks if a year/month/day combination is valid.
pub(super) fn is_valid_date(year: i64, month: i64, day: i64) -> bool {
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

pub(super) fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Convert year/month/day to SAS date (days since 1960-01-01).
pub(super) fn ymd_to_sas_date(year: i64, month: i64, day: i64) -> f64 {
    // Use a simple algorithm: count days from 1960-01-01.
    // We convert to days since some epoch via Julian Day Number or similar.
    days_since_1960(year, month, day) as f64
}

pub(super) fn days_since_1960(year: i64, month: i64, day: i64) -> i64 {
    // Days since 1960-01-01
    // Compute Julian Day Number for both dates and subtract.
    jdn(year, month, day) - jdn(1960, 1, 1)
}

/// Julian Day Number (proleptic Gregorian).
pub(super) fn jdn(year: i64, month: i64, day: i64) -> i64 {
    let a = (14 - month) / 12;
    let y = year + 4800 - a;
    let m = month + 12 * a - 3;
    day + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045
}

/// Convert SAS date to (year, month, day).
pub(super) fn sas_date_to_ymd(sas_date: i64) -> (i64, i64, i64) {
    // Convert SAS date (days since 1960-01-01) to calendar date.
    let jd = sas_date + jdn(1960, 1, 1);
    jdn_to_ymd(jd)
}

/// Convert Julian Day Number to (year, month, day).
pub(super) fn jdn_to_ymd(jd: i64) -> (i64, i64, i64) {
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
pub(super) fn sas_weekday(sas_date: i64) -> i64 {
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

pub(super) fn fn_today(_args: &[Value], _ctx: &mut EvalCtx) -> Value {
    Value::Num(today_sas())
}

pub(super) fn fn_mdy(args: &[Value], ctx: &mut EvalCtx) -> Value {
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

pub(super) fn fn_year(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| sas_date_to_ymd(f as i64).0 as f64)
}

pub(super) fn fn_month(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| sas_date_to_ymd(f as i64).1 as f64)
}

pub(super) fn fn_day(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| sas_date_to_ymd(f as i64).2 as f64)
}

pub(super) fn fn_weekday(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| sas_weekday(f as i64) as f64)
}

// ──────────────────────────────────────────────────────────────────────────────
// Date/time functions (M15.3)
//
// Convention SAS :
//   - valeur date     = jours depuis 1960-01-01 (0 = 1960-01-01)
//   - valeur heure    = secondes dans la journée (0–86399)
//   - valeur datetime = secondes depuis 1960-01-01 00:00:00
// ──────────────────────────────────────────────────────────────────────────────

pub(super) const SECONDS_PER_DAY: f64 = 86400.0;

/// Abréviations de mois SAS (majuscules), index 0 = janvier.
pub(super) const MONTH_ABBR: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// Décompose un datetime (secondes) en (jours date SAS, secondes-dans-le-jour).
/// `floor` garantit un reste positif même pour les datetimes négatifs.
pub(super) fn split_datetime(dt: f64) -> (f64, f64) {
    let days = (dt / SECONDS_PER_DAY).floor();
    let secs = dt - days * SECONDS_PER_DAY;
    (days, secs)
}

pub(super) fn fn_datepart(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |dt| split_datetime(dt).0)
}

pub(super) fn fn_timepart(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |dt| split_datetime(dt).1.trunc())
}

pub(super) fn fn_datetime_combine(args: &[Value], ctx: &mut EvalCtx) -> Value {
    // DATETIME(date, time) — combine une date SAS et une heure-du-jour.
    let date = match args.first() {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let time = match args.get(1) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    Value::Num(date * SECONDS_PER_DAY + time)
}

pub(super) fn fn_hms(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let h = match args.first() {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let m = match args.get(1) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let s = match args.get(2) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    // h ≥ 0 ; m,s dans 0–59.
    if h < 0.0 || !(0.0..=59.0).contains(&m) || !(0.0..=59.0).contains(&s) {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    Value::Num(h.trunc() * 3600.0 + m.trunc() * 60.0 + s.trunc())
}

pub(super) fn fn_dhms(args: &[Value], ctx: &mut EvalCtx) -> Value {
    // DHMS(date, hour, minute, second) → datetime.
    let d = match args.first() {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let h = match args.get(1) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let m = match args.get(2) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let s = match args.get(3) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    if h < 0.0 || !(0.0..=59.0).contains(&m) || !(0.0..=59.0).contains(&s) {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    let time = h.trunc() * 3600.0 + m.trunc() * 60.0 + s.trunc();
    Value::Num(d * SECONDS_PER_DAY + time)
}

/// Bases de calcul partagées par YRDIF/DATDIF.
pub(super) enum DayBasis {
    Actual,
    A360,
    B360,
    Thirty30U,
    Thirty30E,
}

/// Parse la base (insensible à la casse). Renvoie None si inconnue.
pub(super) fn parse_basis(v: &Value) -> Option<DayBasis> {
    let s = match v {
        Value::Char(s) => s.trim().to_uppercase(),
        _ => return None,
    };
    match s.as_str() {
        "ACT" | "ACTUAL" | "ACT/ACT" => Some(DayBasis::Actual),
        "A360" | "ACT/360" => Some(DayBasis::A360),
        "B360" | "30/360" | "30/360 SAS" => Some(DayBasis::B360),
        "30U" | "30/360 US" => Some(DayBasis::Thirty30U),
        "30E" | "30/360 EUR" | "30E/360" => Some(DayBasis::Thirty30E),
        _ => None,
    }
}

/// Nombre de jours « 30/360 » selon la règle (us/eur/sas-business).
pub(super) fn days_30_360(d1: i64, d2: i64, basis: &DayBasis) -> i64 {
    let (y1, m1, mut dd1) = sas_date_to_ymd(d1);
    let (y2, m2, mut dd2) = sas_date_to_ymd(d2);
    match basis {
        DayBasis::B360 => {
            // Règle SAS business 30/360 : d1=31 → 30 ; d2=31 et d1∈{30,31} → 30.
            if dd1 == 31 {
                dd1 = 30;
            }
            if dd2 == 31 && dd1 == 30 {
                dd2 = 30;
            }
        }
        DayBasis::Thirty30U => {
            if dd1 == 31 {
                dd1 = 30;
            }
            if dd2 == 31 && dd1 == 30 {
                dd2 = 30;
            }
        }
        DayBasis::Thirty30E => {
            if dd1 == 31 {
                dd1 = 30;
            }
            if dd2 == 31 {
                dd2 = 30;
            }
        }
        _ => {}
    }
    360 * (y2 - y1) + 30 * (m2 - m1) + (dd2 - dd1)
}

/// Coeur de DATDIF : nombre de jours selon la base.
pub(super) fn datdif_days(d1: f64, d2: f64, basis: &DayBasis) -> f64 {
    match basis {
        DayBasis::Actual | DayBasis::A360 => (d2 - d1).trunc(),
        DayBasis::B360 | DayBasis::Thirty30U | DayBasis::Thirty30E => {
            days_30_360(d1.trunc() as i64, d2.trunc() as i64, basis) as f64
        }
    }
}

pub(super) fn fn_yrdif(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let d1 = match args.first() {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let d2 = match args.get(1) {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let basis = match args.get(2) {
        None => DayBasis::Actual,
        Some(v) => match parse_basis(v) {
            Some(b) => b,
            None => {
                ctx.invalid_data += 1;
                ctx.error_flag = true;
                return Value::missing();
            }
        },
    };
    let years = match basis {
        DayBasis::Actual => (d2 - d1) / 365.0,
        DayBasis::A360 => (d2 - d1) / 360.0,
        DayBasis::B360 | DayBasis::Thirty30U | DayBasis::Thirty30E => {
            days_30_360(d1.trunc() as i64, d2.trunc() as i64, &basis) as f64 / 360.0
        }
    };
    Value::Num(years)
}

pub(super) fn fn_datdif(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let d1 = match args.first() {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let d2 = match args.get(1) {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let basis = match args.get(2) {
        None => DayBasis::Actual,
        Some(v) => match parse_basis(v) {
            Some(b) => b,
            None => {
                ctx.invalid_data += 1;
                ctx.error_flag = true;
                return Value::missing();
            }
        },
    };
    Value::Num(datdif_days(d1, d2, &basis))
}

pub(super) fn fn_juldate(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| {
        let (year, _, _) = sas_date_to_ymd(f as i64);
        let jan1 = ymd_to_sas_date(year, 1, 1);
        ((f.trunc() - jan1) as i64 + 1) as f64 // 1-based
    })
}

pub(super) fn fn_datejul(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let jul = match args.first() {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f.trunc() as i64,
        },
    };
    if jul <= 0 {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    // Format YYDDD / YYYYDDD : les 3 derniers chiffres = jour de l'année.
    let day_of_year = jul % 1000;
    let year_part = jul / 1000;
    if day_of_year < 1 {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    // Interprétation de l'année à 2 chiffres via la fenêtre glissante YEARCUTOFF.
    // La fenêtre : pour yy (0..99), base = (yearcutoff / 100) * 100 ;
    // si base + yy < yearcutoff alors base += 100. Avec yearcutoff=1900
    // (défaut) cela donne 0–99 → 1900–1999, identique à l'ancien code en dur.
    let year = if year_part < 100 {
        let cutoff = ctx.yearcutoff as i64;
        let base = (cutoff / 100) * 100;
        let candidate = base + year_part;
        if candidate < cutoff { candidate + 100 } else { candidate }
    } else if year_part < 200 {
        2000 + (year_part - 100)
    } else {
        year_part
    };
    let max_day = if is_leap_year(year) { 366 } else { 365 };
    if day_of_year > max_day {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    let jan1 = ymd_to_sas_date(year, 1, 1);
    Value::Num(jan1 + (day_of_year - 1) as f64)
}

pub(super) fn fn_hour(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |dt| (split_datetime(dt).1 / 3600.0).floor())
}

pub(super) fn fn_minute(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |dt| ((split_datetime(dt).1 % 3600.0) / 60.0).floor())
}

pub(super) fn fn_second(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |dt| (split_datetime(dt).1 % 60.0).trunc())
}

/// Formate une date SAS en "DDMMMYYYY" (ex. "01JAN2020").
pub(super) fn format_date9(sas_date: i64) -> String {
    let (year, month, day) = sas_date_to_ymd(sas_date);
    let abbr = MONTH_ABBR
        .get((month - 1).clamp(0, 11) as usize)
        .copied()
        .unwrap_or("???");
    format!("{:02}{}{:04}", day, abbr, year)
}

pub(super) fn fn_nldate(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let date = match args.first() {
        None => return Value::Char(String::new()),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::Char(String::new()),
            Some(f) => f.trunc() as i64,
        },
    };
    // La langue (EN/FR/...) ne change rien dans cette implémentation simplifiée.
    let _lang = match args.get(1) {
        Some(Value::Char(s)) => s.trim().to_uppercase(),
        _ => "EN".to_string(),
    };
    Value::Char(format_date9(date))
}

// ──────────────────────────────────────────────────────────────────────────────
// Interval date functions : INTCK / INTNX
// ──────────────────────────────────────────────────────────────────────────────

/// Parsed interval keyword (premier argument caractère de INTCK/INTNX).
pub(super) enum Interval {
    Day,
    Week,
    Month,
    Qtr,
    Year,
}

/// Parse l'intervalle (insensible à la casse, blancs de bord supprimés).
/// Renvoie None pour un intervalle inconnu.
pub(super) fn parse_interval(v: &Value) -> Option<Interval> {
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
pub(super) fn week_index(sas_day: i64) -> i64 {
    (sas_day - 2).div_euclid(7)
}

/// INTCK('interval', from, to) → nombre discret de frontières d'intervalle
/// franchies (méthode "DISCRETE" par défaut de SAS). Intervalle inconnu ou
/// date manquante → missing.
pub(super) fn fn_intck(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(super) enum Align {
    Beginning,
    End,
    Same,
    Middle,
}

pub(super) fn parse_align(v: Option<&Value>) -> Align {
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
pub(super) fn last_day_of_month(year: i64, month: i64) -> i64 {
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
pub(super) fn normalize_ym(year: i64, month0: i64) -> (i64, i64) {
    // month0 est 0-based ici pour faciliter l'arithmétique modulaire.
    let y = year + month0.div_euclid(12);
    let m = month0.rem_euclid(12) + 1;
    (y, m)
}

/// INTNX('interval', start, increment [, 'alignment']) → date SAS.
/// Date manquante / intervalle inconnu → missing.
pub(super) fn fn_intnx(args: &[Value], ctx: &mut EvalCtx) -> Value {
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

