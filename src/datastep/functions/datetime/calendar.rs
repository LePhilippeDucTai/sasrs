pub(crate) fn today_sas() -> f64 {
    // Jours depuis 1960-01-01 : jours Unix + offset 1960→1970 (3653,
    // constante partagée avec dataset.rs — l'époque SAS précède Unix).
    let unix_days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86400) as f64)
        .unwrap_or(0.0);
    unix_days + crate::dataset::SAS_EPOCH_OFFSET_DAYS
}

/// Checks if a year/month/day combination is valid.
pub(crate) fn is_valid_date(year: i64, month: i64, day: i64) -> bool {
    if !(1..=12).contains(&month) || day < 1 {
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

pub(crate) fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Convert year/month/day to SAS date (days since 1960-01-01).
pub(crate) fn ymd_to_sas_date(year: i64, month: i64, day: i64) -> f64 {
    // Use a simple algorithm: count days from 1960-01-01.
    // We convert to days since some epoch via Julian Day Number or similar.
    days_since_1960(year, month, day) as f64
}

pub(crate) fn days_since_1960(year: i64, month: i64, day: i64) -> i64 {
    // Days since 1960-01-01
    // Compute Julian Day Number for both dates and subtract.
    jdn(year, month, day) - jdn(1960, 1, 1)
}

/// Julian Day Number (proleptic Gregorian).
pub(crate) fn jdn(year: i64, month: i64, day: i64) -> i64 {
    let a = (14 - month) / 12;
    let y = year + 4800 - a;
    let m = month + 12 * a - 3;
    day + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045
}

/// Convert SAS date to (year, month, day).
pub(crate) fn sas_date_to_ymd(sas_date: i64) -> (i64, i64, i64) {
    // Convert SAS date (days since 1960-01-01) to calendar date.
    let jd = sas_date + jdn(1960, 1, 1);
    jdn_to_ymd(jd)
}

/// Convert Julian Day Number to (year, month, day).
pub(crate) fn jdn_to_ymd(jd: i64) -> (i64, i64, i64) {
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
pub(crate) fn sas_weekday(sas_date: i64) -> i64 {
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

// ──────────────────────────────────────────────────────────────────────────────
// Date/time functions (M15.3)
//
// Convention SAS :
//   - valeur date     = jours depuis 1960-01-01 (0 = 1960-01-01)
//   - valeur heure    = secondes dans la journée (0–86399)
//   - valeur datetime = secondes depuis 1960-01-01 00:00:00
// ──────────────────────────────────────────────────────────────────────────────

pub(crate) const SECONDS_PER_DAY: f64 = 86400.0;

/// Abréviations de mois SAS (majuscules), index 0 = janvier.
pub(crate) const MONTH_ABBR: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// Décompose un datetime (secondes) en (jours date SAS, secondes-dans-le-jour).
/// `floor` garantit un reste positif même pour les datetimes négatifs.
pub(crate) fn split_datetime(dt: f64) -> (f64, f64) {
    let days = (dt / SECONDS_PER_DAY).floor();
    let secs = dt - days * SECONDS_PER_DAY;
    (days, secs)
}

/// Formate une date SAS en "DDMMMYYYY" (ex. "01JAN2020").
pub(crate) fn format_date9(sas_date: i64) -> String {
    let (year, month, day) = sas_date_to_ymd(sas_date);
    let abbr = MONTH_ABBR
        .get((month - 1).clamp(0, 11) as usize)
        .copied()
        .unwrap_or("???");
    format!("{:02}{}{:04}", day, abbr, year)
}

/// Dernier jour du mois (gère les années bissextiles).
pub(crate) fn last_day_of_month(year: i64, month: i64) -> i64 {
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
pub(crate) fn normalize_ym(year: i64, month0: i64) -> (i64, i64) {
    // month0 est 0-based ici pour faciliter l'arithmétique modulaire.
    let y = year + month0.div_euclid(12);
    let m = month0.rem_euclid(12) + 1;
    (y, m)
}
