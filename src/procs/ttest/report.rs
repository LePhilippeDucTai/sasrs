use super::*;

// ───────────────────────── formatting ─────────────────────────

/// Column header for the probability, per the test sides.
pub(super) fn p_header(sides: TTestSides) -> &'static str {
    match sides {
        TTestSides::TwoTailed => "Pr > |t|",
        TTestSides::Upper => "Pr > t",
        TTestSides::Lower => "Pr < t",
    }
}

/// Confidence-level percentage label, e.g. "95%" for alpha=0.05.
pub(super) fn cl_pct(alpha: f64) -> String {
    let pct = 100.0 * (1.0 - alpha);
    if (pct - pct.round()).abs() < 1e-9 {
        format!("{}%", pct.round() as i64)
    } else {
        format!("{pct}%")
    }
}

/// Format a t-statistic / mean / CI value to 4 decimals; None → ".".
pub(super) fn fmt4(v: Option<f64>) -> String {
    match v {
        Some(f) => format!("{f:.4}"),
        None => ".".to_string(),
    }
}

/// Emit the standard BY-group heading line (`name=value name2=value2`), matching
/// the format used by PROC MEANS / UNIVARIATE / FREQ.
pub(super) fn emit_by_heading(session: &mut Session, by_names: &[String], by_key: &[Value]) {
    let by_cell = |v: &Value| -> String {
        match v {
            Value::Num(f) => crate::value::format_best(*f, 12),
            Value::Missing(k) => k.display(),
            Value::Char(s) => s.trim_end().to_string(),
        }
    };
    let parts: Vec<String> = by_names
        .iter()
        .zip(by_key)
        .map(|(name, v)| format!("{}={}", name, by_cell(v)))
        .collect();
    session.listing.write_line(&parts.join(" "));
    session.listing.blank();
}
