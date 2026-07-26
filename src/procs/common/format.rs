use super::*;

/// Write a centered line within LINESIZE.
pub fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(text.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), text));
}

/// Format a p-value column cell: missing → `.`, tiny → `<.0001`,
/// otherwise 4 decimals (the layout shared by the model procs).
pub fn fmt_p(p: Option<f64>) -> String {
    match p {
        None => ".".to_string(),
        Some(v) if v < 0.0001 => "<.0001".to_string(),
        Some(v) => format!("{v:.4}"),
    }
}

/// `fmt_p` for a p-value that is always present.
pub fn fmt_p_num(p: f64) -> String {
    if p < 0.0001 {
        "<.0001".to_string()
    } else {
        format!("{p:.4}")
    }
}

/// Format a class-level / BY value for display: BESTw.-style numbers,
/// trailing blanks trimmed on character values.
pub fn value_label(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.trim_end().to_string(),
    }
}

/// Cellule numérique à 2 décimales fixes (MQ8.2 — unifie 5 copies).
pub fn fmt2(v: f64) -> String {
    format!("{v:.2}")
}

/// Cellule numérique à 4 décimales fixes (MQ8.2 — unifie 6 copies).
///
/// Les variantes qui rendent `.` sur une valeur absente ou non finie ne sont
/// PAS repliées ici : voir `ttest::report::fmt4_opt` (`Option<f64>`) et
/// `npar1way::report::fmt4_finite` (garde `is_finite`).
pub fn fmt4(v: f64) -> String {
    format!("{v:.4}")
}

/// Cellule numérique à 5 décimales fixes (MQ8.2 — unifie 3 copies).
pub fn fmt5(v: f64) -> String {
    format!("{v:.5}")
}

/// Cellule numérique à 6 décimales fixes (MQ8.2 — unifie 3 copies).
pub fn fmt6(v: f64) -> String {
    format!("{v:.6}")
}

/// Cellule numérique à 8 décimales fixes (matrices de produits croisés REG).
pub fn fmt8(v: f64) -> String {
    format!("{v:.8}")
}
