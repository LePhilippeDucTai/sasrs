use super::*;

/// Jour SAS 0 = 1960-01-01.
pub(crate) fn sas_epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(1960, 1, 1).expect("1960-01-01 is a valid date")
}

/// Convertit un littéral chaîne (avec son suffixe) en `Expr`.
pub(crate) fn literal_from_string(value: &str, suffix: StrSuffix, span: Span) -> Result<Expr> {
    match suffix {
        StrSuffix::None | StrSuffix::Name => Ok(Expr::Str(value.to_string())),
        StrSuffix::Date => Ok(Expr::Num(parse_date_literal(value, span)?)),
        StrSuffix::Time => Ok(Expr::Num(parse_time_literal(value, span)?)),
        StrSuffix::DateTime => Ok(Expr::Num(parse_datetime_literal(value, span)?)),
    }
}

/// `ddMONyyyy` (insensible à la casse) → NaiveDate.
pub(crate) fn parse_date_ddmonyyyy(s: &str, span: Span) -> Result<NaiveDate> {
    let bytes = s.as_bytes();
    // Au minimum d + mmm + yyyy. Le jour fait 1 ou 2 chiffres.
    // On découpe : digits | 3 lettres | digits.
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let day_str = &s[..i];
    if day_str.is_empty() || i + 3 > s.len() {
        return Err(SasError::parse(format!("invalid date literal '{s}'"), span));
    }
    let mon_str = &s[i..i + 3];
    let year_str = &s[i + 3..];
    let day: u32 = day_str
        .parse()
        .map_err(|_| SasError::parse(format!("invalid date literal '{s}'"), span))?;
    let month = match mon_str.to_ascii_lowercase().as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => {
            return Err(SasError::parse(
                format!("invalid month in date literal '{s}'"),
                span,
            ));
        }
    };
    if year_str.is_empty() || !year_str.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SasError::parse(
            format!("invalid year in date literal '{s}'"),
            span,
        ));
    }
    let year: i32 = year_str
        .parse()
        .map_err(|_| SasError::parse(format!("invalid date literal '{s}'"), span))?;
    NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| SasError::parse(format!("invalid date literal '{s}'"), span))
}

/// `'ddMONyyyy'd` → jours depuis 1960-01-01 (f64).
pub(crate) fn parse_date_literal(s: &str, span: Span) -> Result<f64> {
    let date = parse_date_ddmonyyyy(s.trim(), span)?;
    let days = date.signed_duration_since(sas_epoch()).num_days();
    Ok(days as f64)
}

/// `hh:mm[:ss]` → secondes depuis minuit (f64).
pub(crate) fn parse_time_literal(s: &str, span: Span) -> Result<f64> {
    let s = s.trim();
    let mut parts = s.split(':');
    let h = parts.next();
    let m = parts.next();
    let sec = parts.next();
    if parts.next().is_some() {
        return Err(SasError::parse(format!("invalid time literal '{s}'"), span));
    }
    let (Some(h), Some(m)) = (h, m) else {
        return Err(SasError::parse(format!("invalid time literal '{s}'"), span));
    };
    let parse_int = |p: &str| -> Result<u32> {
        p.trim()
            .parse::<u32>()
            .map_err(|_| SasError::parse(format!("invalid time literal '{s}'"), span))
    };
    let hh = parse_int(h)?;
    let mm = parse_int(m)?;
    let ss = match sec {
        Some(p) => parse_int(p)?,
        None => 0,
    };
    if mm >= 60 || ss >= 60 {
        return Err(SasError::parse(format!("invalid time literal '{s}'"), span));
    }
    // Validation des composantes via NaiveTime (heures < 24 — SAS tolère
    // davantage, mais on reste strict pour les littéraux M1).
    if NaiveTime::from_hms_opt(hh, mm, ss).is_none() {
        return Err(SasError::parse(format!("invalid time literal '{s}'"), span));
    }
    Ok((hh * 3600 + mm * 60 + ss) as f64)
}

/// `'ddMONyyyy:hh:mm:ss'dt` ou `'ddMONyyyy hh:mm:ss'dt` → secondes depuis
/// 1960-01-01T00:00:00 (f64). SAS accepte un ESPACE ou un `:` entre la date
/// et l'heure ; on découpe au premier des deux.
pub(crate) fn parse_datetime_literal(s: &str, span: Span) -> Result<f64> {
    let s = s.trim();
    // Séparateur date/heure : premier espace, sinon premier `:`.
    let split = s
        .find(' ')
        .or_else(|| s.find(':'))
        .map(|i| (&s[..i], &s[i + 1..]));
    let Some((date_part, time_part)) = split else {
        return Err(SasError::parse(
            format!("invalid datetime literal '{s}'"),
            span,
        ));
    };
    let date = parse_date_ddmonyyyy(date_part.trim(), span)?;
    let secs_in_day = parse_time_literal(time_part.trim(), span)?;
    let days = date.signed_duration_since(sas_epoch()).num_days();
    Ok(days as f64 * 86400.0 + secs_in_day)
}
