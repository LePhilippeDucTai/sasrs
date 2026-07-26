// ───────────────────────── Formatting helpers ─────────────────────────

pub(super) fn fmt5(v: f64) -> String {
    format!("{v:.5}")
}

pub(super) fn fmt2(v: f64) -> String {
    format!("{v:.2}")
}

pub(super) fn fmt_fit4(v: f64) -> String {
    format!("{v:.4}")
}

/// Format a confidence level (e.g. 95, 90, or 97.5) without a trailing `.0`.
pub(super) fn fmt_level(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        // Trim trailing zeros from a fixed-precision rendering.
        let s = format!("{:.4}", v);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        s.to_string()
    }
}

// ───────────────────────── VarMeta helper ─────────────────────────

/// Build a PROC REG BY-group heading line (`<var>=<value> ...`), matching the
/// standard SAS BY-line used by the other procs (M36.7). Centered and emitted by
/// the per-model header path so it lands after "The REG Procedure".
pub(super) fn reg_by_heading_line(by_names: &[String], by_key: &[crate::value::Value]) -> String {
    let parts: Vec<String> = by_names
        .iter()
        .zip(by_key)
        .map(|(name, v)| format!("{}={}", name, by_value_cell(v)))
        .collect();
    parts.join(" ")
}

/// Render a BY-key cell value for the heading line (M36.7).
pub(super) fn by_value_cell(v: &crate::value::Value) -> String {
    match v {
        crate::value::Value::Num(f) => crate::value::format_best(*f, 12),
        crate::value::Value::Missing(k) => k.display(),
        crate::value::Value::Char(s) => s.trim_end().to_string(),
    }
}

/// Render an ID cell value for the diagnostic-listing leading column (M36.7).
pub(super) fn id_value_cell(v: &crate::value::Value) -> String {
    match v {
        crate::value::Value::Num(f) => crate::value::format_best(*f, 12),
        crate::value::Value::Missing(k) => k.display(),
        crate::value::Value::Char(s) => s.trim_end().to_string(),
    }
}
