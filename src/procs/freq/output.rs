use super::*;

/// Emit the standard BY-group heading line (`var=val var2=val2`), matching the
/// MEANS/UNIVARIATE rendering.
pub(super) fn emit_by_heading(session: &mut Session, by_names: &[String], by_key: &[Value]) {
    let parts: Vec<String> = by_names
        .iter()
        .zip(by_key)
        .map(|(name, v)| format!("{}={}", name, category_label(v)))
        .collect();
    session.listing.write_line(&parts.join(" "));
    session.listing.blank();
}
