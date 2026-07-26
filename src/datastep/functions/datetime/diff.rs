use super::*;

/// Bases de calcul partagées par YRDIF/DATDIF.
pub(crate) enum DayBasis {
    Actual,
    A360,
    B360,
    Thirty30U,
    Thirty30E,
}

/// Parse la base (insensible à la casse). Renvoie None si inconnue.
pub(crate) fn parse_basis(v: &Value) -> Option<DayBasis> {
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
pub(crate) fn days_30_360(d1: i64, d2: i64, basis: &DayBasis) -> i64 {
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
pub(crate) fn datdif_days(d1: f64, d2: f64, basis: &DayBasis) -> f64 {
    match basis {
        DayBasis::Actual | DayBasis::A360 => (d2 - d1).trunc(),
        DayBasis::B360 | DayBasis::Thirty30U | DayBasis::Thirty30E => {
            days_30_360(d1.trunc() as i64, d2.trunc() as i64, basis) as f64
        }
    }
}

pub(crate) fn fn_yrdif(args: &[Value], ctx: &mut EvalCtx) -> Value {
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

pub(crate) fn fn_datdif(args: &[Value], ctx: &mut EvalCtx) -> Value {
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

pub(crate) fn fn_juldate(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| {
        let (year, _, _) = sas_date_to_ymd(f as i64);
        let jan1 = ymd_to_sas_date(year, 1, 1);
        ((f.trunc() - jan1) as i64 + 1) as f64 // 1-based
    })
}

pub(crate) fn fn_datejul(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
        if candidate < cutoff {
            candidate + 100
        } else {
            candidate
        }
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
