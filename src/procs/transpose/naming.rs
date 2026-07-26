use super::*;

/// Resolve a variable name to its column index (case-insensitive), erroring
/// like SAS when absent.
pub(crate) fn resolve_var(ds: &SasDataset, vname: &str) -> Result<usize> {
    ds.vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(vname))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", vname.to_uppercase())))
}

/// Format an ID value into a column name candidate (char trimmed; numeric
/// via BEST12. trimmed). Then normalize to a valid SAS name.
pub(crate) fn id_value_to_name(v: &Value) -> String {
    let raw = match v {
        Value::Char(s) => s.trim().to_string(),
        Value::Num(f) => format_best(*f, 12).trim().to_string(),
        Value::Missing(k) => k.display(),
    };
    normalize_name(&raw)
}

/// Normalize an arbitrary string into a valid SAS variable name: replace
/// invalid characters by `_`, prefix `_` when the first char is a digit or
/// the string is empty. (Conservative; SAS uses VALIDVARNAME=V7 by default.)
pub(crate) fn normalize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().max(1));
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let starts_bad = out
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(true);
    if out.is_empty() || starts_bad {
        let mut prefixed = String::with_capacity(out.len() + 1);
        prefixed.push('_');
        prefixed.push_str(&out);
        prefixed
    } else {
        out
    }
}

/// Convert a Value into its CHAR representation when transposed columns are
/// character (mixing rule). Numeric → BEST12. trimmed; missing → blank.
pub(crate) fn value_to_char(v: &Value) -> Option<String> {
    match v {
        Value::Char(s) => {
            let t = s.trim_end();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Value::Num(f) => Some(format_best(*f, 12).trim().to_string()),
        Value::Missing(_) => None,
    }
}

/// Group row indices by the BY-tuple, preserving first-appearance order of
/// the groups and input order within each group. With no BY columns, one
/// group containing all rows in input order.
pub(crate) fn group_by_tuple(
    by_values: &[Vec<Value>],
    n_obs: usize,
) -> Vec<(Vec<Value>, Vec<usize>)> {
    let mut groups: Vec<(Vec<Value>, Vec<usize>)> = Vec::new();
    for row in 0..n_obs {
        let key: Vec<Value> = by_values.iter().map(|c| c[row].clone()).collect();
        let pos = groups.iter().position(|(k, _)| {
            k.len() == key.len()
                && k.iter()
                    .zip(&key)
                    .all(|(a, b)| a.sas_cmp(b) == Ordering::Equal)
        });
        match pos {
            Some(p) => groups[p].1.push(row),
            None => groups.push((key, vec![row])),
        }
    }
    groups
}

/// Display form of an ID value for the duplicate-error message (char trimmed,
/// numeric via BEST12. trimmed, missing via its display char).
pub(crate) fn id_value_display(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim().to_string(),
        Value::Num(f) => format_best(*f, 12).trim().to_string(),
        Value::Missing(k) => k.display(),
    }
}

/// Encode a Value into an Option<String> for a CHAR output column (blank /
/// missing → None).
pub(crate) fn char_cell(v: &Value) -> Option<String> {
    match v {
        Value::Char(s) if s.trim_end().is_empty() => None,
        Value::Char(s) => Some(s.trim_end().to_string()),
        Value::Missing(_) => None,
        Value::Num(_) => None,
    }
}
