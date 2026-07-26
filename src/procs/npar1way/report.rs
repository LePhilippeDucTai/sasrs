use super::*;

// ───────────────────────── formatting ─────────────────────────

/// Format a statistic to 4 decimals; NaN → ".".
pub(super) fn fmt4(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.4}")
    } else {
        ".".to_string()
    }
}

/// Format a p-value SAS-style: `<.0001`, else 4 decimals; NaN → ".".
// Divergence volontaire avec `common::fmt_p_num` : NPAR1WAY affiche `.`
// pour une p-value non finie.
pub(super) fn fmt_p(p: f64) -> String {
    if !p.is_finite() {
        ".".to_string()
    } else if p < 0.0001 {
        "<.0001".to_string()
    } else {
        format!("{p:.4}")
    }
}

/// One-sided normal p-value `min(Φ(z), 1−Φ(z))`.
pub(super) fn normal_p1(z: f64) -> f64 {
    if !z.is_finite() {
        return f64::NAN;
    }
    let cdf = probnorm(z);
    cdf.min(1.0 - cdf).clamp(0.0, 1.0)
}

/// Emit the standard BY-group heading line (`name=value name2=value2`).
pub(super) fn emit_by_heading(session: &mut Session, by_names: &[String], by_key: &[Value]) {
    let cell = |v: &Value| -> String {
        match v {
            Value::Num(f) => format_best(*f, 12),
            Value::Missing(k) => k.display(),
            Value::Char(s) => s.trim_end().to_string(),
        }
    };
    let parts: Vec<String> = by_names
        .iter()
        .zip(by_key)
        .map(|(name, v)| format!("{}={}", name, cell(v)))
        .collect();
    session.listing.write_line(&parts.join(" "));
    session.listing.blank();
}

/// Write a 2-sample statistic table (Wilcoxon-shaped).
pub(super) fn write_two_sample_table(
    session: &mut Session,
    stat: f64,
    mean: f64,
    sd: f64,
    z: f64,
    p: f64,
) {
    let headers: Vec<String> = vec![
        "Statistic".into(),
        "Mean Under H0".into(),
        "Std Dev Under H0".into(),
        "Z".into(),
        "Pr > |Z|".into(),
    ];
    let aligns = vec![
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let rows = vec![vec![fmt4(stat), fmt4(mean), fmt4(sd), fmt4(z), fmt_p(p)]];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Write a one-way χ² table (`Chi-Square / DF / Pr > ChiSq`).
pub(super) fn write_one_way_table(session: &mut Session, chisq: f64, df: usize, p: f64) {
    let headers: Vec<String> = vec!["Chi-Square".into(), "DF".into(), "Pr > ChiSq".into()];
    let aligns = vec![Align::Right, Align::Right, Align::Right];
    let rows = vec![vec![fmt4(chisq), format!("{df}"), fmt_p(p)]];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Append the exact Wilcoxon block below the Wilcoxon table.
pub(super) fn write_exact_block(session: &mut Session, ex: &ExactWilcoxon) {
    centered(session, "Exact Test");
    session.listing.blank();
    session.listing.write_line(&format!(
        "One-Sided Pr <= S            {}",
        fmt_p(ex.p_lower)
    ));
    session
        .listing
        .write_line(&format!("Two-Sided Pr >= |S - Mean|   {}", fmt_p(ex.p_two)));
}
