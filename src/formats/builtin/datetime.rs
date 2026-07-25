use super::*;

/// "DATE" arm of `format_builtin`.
pub(super) fn fmt_date(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    let day = date.day();
    let mon = MONTHS[(date.month() - 1) as usize];
    let year = date.year();
    let w = spec.w.unwrap_or(9) as usize;
    let s = if w >= 9 {
        format!("{:02}{}{:04}", day, mon, year)
    } else {
        // DATE7 or smaller: 2-digit year
        let yr2 = year.abs() % 100;
        format!("{:02}{}{:02}", day, mon, yr2)
    };
    Some(right_justify(&s, w))
}

/// "DDMMYY" arm of `format_builtin`.
pub(super) fn fmt_ddmmyy(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    let dd = date.day();
    let mm = date.month();
    let yyyy = date.year();
    let w = spec.w.unwrap_or(8) as usize;
    let s = if w >= 10 {
        format!("{:02}/{:02}/{:04}", dd, mm, yyyy)
    } else {
        let yy = yyyy.abs() % 100;
        format!("{:02}{:02}{:02}", dd, mm, yy)
    };
    Some(right_justify(&s, w))
}

/// "MMDDYY" arm of `format_builtin`.
pub(super) fn fmt_mmddyy(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    let dd = date.day();
    let mm = date.month();
    let yyyy = date.year();
    let w = spec.w.unwrap_or(8) as usize;
    let s = if w >= 10 {
        format!("{:02}/{:02}/{:04}", mm, dd, yyyy)
    } else {
        let yy = yyyy.abs() % 100;
        format!("{:02}{:02}{:02}", mm, dd, yy)
    };
    Some(right_justify(&s, w))
}

/// "YYMMDD" arm of `format_builtin`.
pub(super) fn fmt_yymmdd(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    let dd = date.day();
    let mm = date.month();
    let yyyy = date.year();
    let w = spec.w.unwrap_or(8) as usize;
    let s = if w >= 10 {
        format!("{:04}/{:02}/{:02}", yyyy, mm, dd)
    } else {
        let yy = yyyy.abs() % 100;
        format!("{:02}{:02}{:02}", yy, mm, dd)
    };
    Some(right_justify(&s, w))
}

/// "MONYY" arm of `format_builtin`.
pub(super) fn fmt_monyy(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    let mon = MONTHS[(date.month() - 1) as usize];
    let yyyy = date.year();
    let s = format!("{}{:04}", mon, yyyy);
    let w = spec.w.unwrap_or(7) as usize;
    Some(right_justify(&s, w))
}

/// "WORDDATE" arm of `format_builtin`.
pub(super) fn fmt_worddate(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    let mon = MONTHS_FULL[(date.month() - 1) as usize];
    let day = date.day();
    let year = date.year();
    let s = format!("{} {}, {}", mon, day, year);
    match spec.w {
        None => Some(s),
        Some(w) => Some(right_justify(&s, w as usize)),
    }
}

/// "DATETIME" arm of `format_builtin`.
pub(super) fn fmt_datetime(fval: f64, spec: &FormatSpec) -> Option<String> {
    let dt = secs_to_datetime(fval)?;
    let day = dt.day();
    let mon = MONTHS[(dt.month() - 1) as usize];
    let year = dt.year();
    let hh = dt.hour();
    let mm = dt.minute();
    let ss = dt.second();
    let s = format!("{:02}{}{:04}:{:02}:{:02}:{:02}", day, mon, year, hh, mm, ss);
    let w = spec.w.unwrap_or(19) as usize;
    Some(right_justify(&s, w))
}

/// "TIME" arm of `format_builtin`.
pub(super) fn fmt_time(fval: f64, spec: &FormatSpec) -> Option<String> {
    let (hh, mm, ss) = secs_to_time(fval);
    let s = format!("{:02}:{:02}:{:02}", hh, mm, ss);
    let w = spec.w.unwrap_or(8) as usize;
    Some(right_justify(&s, w))
}

/// "WEEKDATE" arm of `format_builtin`.
pub(super) fn fmt_weekdate(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    // chrono weekday: Monday=0 in num_days_from_monday(); Sunday=6
    // We need Sunday=0 for our DOW_SHORT array
    let dow = date.weekday().num_days_from_sunday() as usize;
    let s = DOW_SHORT[dow];
    let w = spec.w.unwrap_or(3) as usize;
    Some(right_justify(s, w))
}

/// "DOWNAME" arm of `format_builtin`.
pub(super) fn fmt_downame(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    let dow = date.weekday().num_days_from_sunday() as usize;
    let s = DOW_FULL[dow];
    let w = spec.w.unwrap_or(9) as usize; // "Wednesday" = 9 chars
    Some(right_justify(s, w))
}

/// "MONNAME" arm of `format_builtin`.
pub(super) fn fmt_monname(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    let s = MONTHS_FULL[(date.month() - 1) as usize];
    let w = spec.w.unwrap_or(9) as usize; // "September" = 9 chars
    Some(right_justify(s, w))
}

/// "QTR" | "QTRR" arm of `format_builtin`.
pub(super) fn fmt_qtr(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    let q = ((date.month() - 1) / 3) + 1;
    let s = format!("{}", q);
    let w = spec.w.unwrap_or(1) as usize;
    Some(right_justify(&s, w))
}

/// "YYQ" arm of `format_builtin`.
pub(super) fn fmt_yyq(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    let q = ((date.month() - 1) / 3) + 1;
    let s = format!("{}Q{}", date.year(), q);
    let w = spec.w.unwrap_or(6) as usize;
    Some(right_justify(&s, w))
}

/// "JULIAN" arm of `format_builtin`.
pub(super) fn fmt_julian(fval: f64, spec: &FormatSpec) -> Option<String> {
    let date = days_to_date(fval)?;
    let doy = date.ordinal(); // 1-based day of year
    let s = format!("{:04}{:03}", date.year(), doy);
    let w = spec.w.unwrap_or(7) as usize;
    Some(right_justify(&s, w))
}

/// "B8601DA" arm of `format_builtin`.
pub(super) fn fmt_b8601da(fval: f64, spec: &FormatSpec) -> Option<String> {
    // YYYYMMDD
    let date = days_to_date(fval)?;
    let s = format!("{:04}{:02}{:02}", date.year(), date.month(), date.day());
    let w = spec.w.unwrap_or(8) as usize;
    Some(right_justify(&s, w))
}

/// "B8601DT" arm of `format_builtin`.
pub(super) fn fmt_b8601dt(fval: f64, spec: &FormatSpec) -> Option<String> {
    // YYYYMMDDTHHmmss
    let dt = secs_to_datetime(fval)?;
    let s = format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}",
        dt.year(), dt.month(), dt.day(),
        dt.hour(), dt.minute(), dt.second()
    );
    let w = spec.w.unwrap_or(15) as usize;
    Some(right_justify(&s, w))
}

/// "B8601TM" arm of `format_builtin`.
pub(super) fn fmt_b8601tm(fval: f64, spec: &FormatSpec) -> Option<String> {
    // HHmmss
    let (hh, mm, ss) = secs_to_time(fval);
    let s = format!("{:02}{:02}{:02}", hh, mm, ss);
    let w = spec.w.unwrap_or(6) as usize;
    Some(right_justify(&s, w))
}

/// "E8601DA" arm of `format_builtin`.
pub(super) fn fmt_e8601da(fval: f64, spec: &FormatSpec) -> Option<String> {
    // YYYY-MM-DD
    let date = days_to_date(fval)?;
    let s = format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day());
    let w = spec.w.unwrap_or(10) as usize;
    Some(right_justify(&s, w))
}

/// "E8601DT" arm of `format_builtin`.
pub(super) fn fmt_e8601dt(fval: f64, spec: &FormatSpec) -> Option<String> {
    // YYYY-MM-DDTHH:mm:ss
    let dt = secs_to_datetime(fval)?;
    let s = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        dt.year(), dt.month(), dt.day(),
        dt.hour(), dt.minute(), dt.second()
    );
    let w = spec.w.unwrap_or(19) as usize;
    Some(right_justify(&s, w))
}

/// "E8601TM" arm of `format_builtin`.
pub(super) fn fmt_e8601tm(fval: f64, spec: &FormatSpec) -> Option<String> {
    // HH:mm:ss
    let (hh, mm, ss) = secs_to_time(fval);
    let s = format!("{:02}:{:02}:{:02}", hh, mm, ss);
    let w = spec.w.unwrap_or(8) as usize;
    Some(right_justify(&s, w))
}
