use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// informat_builtin
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a SAS informat. Returns None if the informat name is unknown.
pub fn informat_builtin(s: &str, spec: &FormatSpec) -> Option<Value> {
    let name = spec.name.to_uppercase();
    let trimmed = s.trim();

    match name.as_str() {
        // ── w.d (plain numeric) ──────────────────────────────────────────────
        // THE FAMOUS PITFALL: if the source has NO decimal point and d>0,
        // divide by 10^d. If the source HAS a decimal point, ignore d.
        "" | "F" => {
            if trimmed.is_empty() || trimmed == "." {
                return Some(Value::missing());
            }
            let has_decimal = trimmed.contains('.');
            match trimmed.parse::<f64>() {
                Ok(mut v) => {
                    if !has_decimal {
                        let d = spec.d.unwrap_or(0) as u32;
                        if d > 0 {
                            v /= 10f64.powi(d as i32);
                        }
                    }
                    Some(Value::Num(v))
                }
                Err(_) => Some(Value::missing()),
            }
        }

        // ── COMMAw.d — strip $ and , then treat as w.d ───────────────────────
        "COMMA" | "DOLLAR" => {
            let cleaned: String = trimmed.chars().filter(|&c| c != ',' && c != '$').collect();
            if cleaned.is_empty() || cleaned == "." {
                return Some(Value::missing());
            }
            let has_decimal = cleaned.contains('.');
            match cleaned.parse::<f64>() {
                Ok(mut v) => {
                    if !has_decimal {
                        let d = spec.d.unwrap_or(0) as u32;
                        if d > 0 {
                            v /= 10f64.powi(d as i32);
                        }
                    }
                    Some(Value::Num(v))
                }
                Err(_) => Some(Value::missing()),
            }
        }

        // ── DATE9. → days since 1960-01-01 ───────────────────────────────────
        "DATE" => {
            // Formats: 01JAN2020 (9 chars) or 01JAN20 (7 chars)
            if trimmed.len() < 7 {
                return Some(Value::missing());
            }
            let day_str = &trimmed[..2];
            let mon_str = &trimmed[2..5].to_uppercase();
            let year_str = &trimmed[5..];
            let day: u32 = day_str.parse().ok()?;
            let month = MONTHS
                .iter()
                .position(|&m| m == mon_str)
                .map(|p| p as u32 + 1)?;
            let year: i32 = year_str.parse().ok()?;
            // 2-digit year: 00-99 → 2000-2099 (simple heuristic matching SAS)
            let year = if year_str.len() == 2 {
                if (0..100).contains(&year) {
                    2000 + year
                } else {
                    year
                }
            } else {
                year
            };
            let date = NaiveDate::from_ymd_opt(year, month, day)?;
            let days = date.signed_duration_since(sas_epoch()).num_days() as f64;
            Some(Value::Num(days))
        }

        // ── MMDDYY10. → days since 1960-01-01 ────────────────────────────────
        "MMDDYY" => {
            // Handles both mmddyyyy (8 chars, no sep) and mm/dd/yyyy (10 chars)
            let (mm, dd, yyyy) = parse_mdy_variants(trimmed)?;
            let date = NaiveDate::from_ymd_opt(yyyy, mm, dd)?;
            let days = date.signed_duration_since(sas_epoch()).num_days() as f64;
            Some(Value::Num(days))
        }

        // ── DDMMYY10. → days since 1960-01-01 ────────────────────────────────
        "DDMMYY" => {
            let (dd, mm, yyyy) = parse_dmy_variants(trimmed)?;
            let date = NaiveDate::from_ymd_opt(yyyy, mm, dd)?;
            let days = date.signed_duration_since(sas_epoch()).num_days() as f64;
            Some(Value::Num(days))
        }

        // ── YYMMDD10. → days since 1960-01-01 ────────────────────────────────
        "YYMMDD" => {
            let (yyyy, mm, dd) = parse_ymd_variants(trimmed)?;
            let date = NaiveDate::from_ymd_opt(yyyy, mm, dd)?;
            let days = date.signed_duration_since(sas_epoch()).num_days() as f64;
            Some(Value::Num(days))
        }

        // ── TIMEw. → seconds since midnight ──────────────────────────────────
        "TIME" => {
            // hh:mm:ss or hh:mm
            let parts: Vec<&str> = trimmed.split(':').collect();
            if parts.len() < 2 {
                return Some(Value::missing());
            }
            let hh: u64 = parts[0].trim().parse().ok()?;
            let mm: u64 = parts[1].trim().parse().ok()?;
            let ss: u64 = if parts.len() >= 3 {
                parts[2].trim().parse().ok()?
            } else {
                0
            };
            let secs = hh * 3600 + mm * 60 + ss;
            Some(Value::Num(secs as f64))
        }

        // ── $CHAR / $ ─────────────────────────────────────────────────────────
        "$" | "$CHAR" | "$F" => {
            let s = match spec.w {
                None => trimmed.to_string(),
                Some(w) => {
                    let mut out = trimmed.to_string();
                    out.truncate(w as usize);
                    out
                }
            };
            Some(Value::Char(s))
        }

        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Date parsing helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Parse MM/DD/YYYY or MMDDYYYY or MMDDYY (returns (month, day, year)).
pub(super) fn parse_mdy_variants(s: &str) -> Option<(u32, u32, i32)> {
    if s.contains('/') {
        // mm/dd/yyyy or mm/dd/yy
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 3 {
            return None;
        }
        let mm: u32 = parts[0].parse().ok()?;
        let dd: u32 = parts[1].parse().ok()?;
        let yyyy: i32 = expand_year(parts[2].parse().ok()?, parts[2].len());
        Some((mm, dd, yyyy))
    } else if s.len() >= 8 {
        let mm: u32 = s[..2].parse().ok()?;
        let dd: u32 = s[2..4].parse().ok()?;
        let yyyy: i32 = expand_year(s[4..].parse().ok()?, s.len() - 4);
        Some((mm, dd, yyyy))
    } else {
        None
    }
}

/// Parse DD/MM/YYYY or DDMMYYYY (returns (day, month, year)).
pub(super) fn parse_dmy_variants(s: &str) -> Option<(u32, u32, i32)> {
    if s.contains('/') {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 3 {
            return None;
        }
        let dd: u32 = parts[0].parse().ok()?;
        let mm: u32 = parts[1].parse().ok()?;
        let yyyy: i32 = expand_year(parts[2].parse().ok()?, parts[2].len());
        Some((dd, mm, yyyy))
    } else if s.len() >= 8 {
        let dd: u32 = s[..2].parse().ok()?;
        let mm: u32 = s[2..4].parse().ok()?;
        let yyyy: i32 = expand_year(s[4..].parse().ok()?, s.len() - 4);
        Some((dd, mm, yyyy))
    } else {
        None
    }
}

/// Parse YYYY/MM/DD or YYYYMMDD (returns (year, month, day)).
pub(super) fn parse_ymd_variants(s: &str) -> Option<(i32, u32, u32)> {
    if s.contains('/') {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 3 {
            return None;
        }
        let yyyy: i32 = expand_year(parts[0].parse().ok()?, parts[0].len());
        let mm: u32 = parts[1].parse().ok()?;
        let dd: u32 = parts[2].parse().ok()?;
        Some((yyyy, mm, dd))
    } else if s.len() >= 8 {
        let yyyy: i32 = expand_year(s[..4].parse().ok()?, 4);
        let mm: u32 = s[4..6].parse().ok()?;
        let dd: u32 = s[6..8].parse().ok()?;
        Some((yyyy, mm, dd))
    } else if s.len() == 6 {
        // yymmdd
        let yy: i32 = s[..2].parse().ok()?;
        let mm: u32 = s[2..4].parse().ok()?;
        let dd: u32 = s[4..6].parse().ok()?;
        Some((expand_year(yy, 2), mm, dd))
    } else {
        None
    }
}

/// Expand a 2-digit year to 4 digits (00-99 → 2000-2099).
pub(super) fn expand_year(y: i32, len: usize) -> i32 {
    if len == 2 { 2000 + y } else { y }
}
