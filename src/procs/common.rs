//! Helpers partagés par plusieurs PROCs.
//!
//! Ce module centralise les fonctions utilitaires communes afin d'éviter la
//! duplication de code entre `means`, `freq`, `univariate`, `sort`,
//! `transpose` et `append`. Chaque fonction est extraite verbatim de son
//! premier site d'apparition ; aucune logique n'est modifiée.

use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use crate::missing::{num_to_value, value_to_num};
use crate::value::{format_best, Value, VarType};
use std::cmp::Ordering;

/// Decode one column of a SasDataset into a `Vec<Value>` (downcast once;
/// never decode per cell).
pub fn decode_column(ds: &SasDataset, col_idx: usize) -> Result<Vec<Value>> {
    let series = ds.df.get_columns()[col_idx].as_materialized_series();
    let values = match ds.vars[col_idx].ty {
        VarType::Num => series.f64()?.iter().map(num_to_value).collect(),
        VarType::Char => series
            .str()?
            .iter()
            .map(|o| Value::Char(o.unwrap_or("").to_string()))
            .collect(),
    };
    Ok(values)
}

/// Sample standard deviation (divisor n-1). Needs n>=2, else None.
pub fn sample_std(xs: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    let ss: f64 = xs.iter().map(|v| (v - mean) * (v - mean)).sum();
    Some((ss / (n as f64 - 1.0)).sqrt())
}

/// Split a column's values for one set of row indices into (non-missing
/// numbers, missing count). Char values are treated as missing for numeric
/// statistics.
pub fn partition_numeric(col: &[Value], rows: &[usize]) -> (Vec<f64>, usize) {
    let mut xs = Vec::with_capacity(rows.len());
    let mut nmiss = 0usize;
    for &r in rows {
        match value_to_num(&col[r]) {
            Some(f) if !f.is_nan() => xs.push(f),
            _ => nmiss += 1,
        }
    }
    (xs, nmiss)
}

/// A resolved BY variable: dataset column index, declared DESCENDING flag,
/// and the variable name (display, original case).
pub struct ByCol {
    pub col_idx: usize,
    pub descending: bool,
    pub name: String,
}

/// Resolve a BY var list against a dataset, validating each name exists.
/// `by` is the parsed (name, descending) list.
pub fn resolve_by_cols(ds: &SasDataset, by: &[(String, bool)]) -> Result<Vec<ByCol>> {
    let mut cols = Vec::with_capacity(by.len());
    for (vname, descending) in by {
        match ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(vname))
        {
            Some(i) => cols.push(ByCol {
                col_idx: i,
                descending: *descending,
                name: ds.vars[i].name.clone(),
            }),
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    vname.to_uppercase()
                )));
            }
        }
    }
    Ok(cols)
}

/// Render a BY-key cell value (for the heading line / error message).
fn by_cell(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.trim_end().to_string(),
    }
}

/// Verify the rows are sorted by the BY key (per each var's direction) using
/// `sas_cmp`, then partition the rows (in their existing order) into
/// contiguous BY groups. Returns one (key values, row indices) pair per group
/// in input order.
///
/// On the first out-of-order adjacent pair, returns the SAS error
/// `Data set <display> is not sorted in ascending sequence. ...`.
pub fn by_groups(
    by_values: &[Vec<Value>],
    descending: &[bool],
    n_obs: usize,
    by_names: &[String],
    display: &str,
) -> Result<Vec<(Vec<Value>, Vec<usize>)>> {
    // Compare the BY key of two rows honoring per-key direction.
    let cmp = |a: usize, b: usize| -> Ordering {
        for (k, col) in by_values.iter().enumerate() {
            let mut c = col[a].sas_cmp(&col[b]);
            if descending[k] {
                c = c.reverse();
            }
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    };

    let mut groups: Vec<(Vec<Value>, Vec<usize>)> = Vec::new();
    for row in 0..n_obs {
        if row > 0 {
            match cmp(row - 1, row) {
                Ordering::Greater => {
                    // Find the first key var that differs to name it in the error.
                    let prev = row - 1;
                    let (vname, v1, v2) = by_values
                        .iter()
                        .enumerate()
                        .find_map(|(k, col)| {
                            if col[prev].sas_cmp(&col[row]) != Ordering::Equal {
                                Some((
                                    by_names[k].clone(),
                                    by_cell(&col[prev]),
                                    by_cell(&col[row]),
                                ))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| {
                            (by_names[0].clone(), by_cell(&by_values[0][prev]), by_cell(&by_values[0][row]))
                        });
                    return Err(SasError::runtime(format!(
                        "Data set {display} is not sorted in ascending sequence. \
                         The current BY group has {vname}={v1} and the next BY group has {vname}={v2}."
                    )));
                }
                Ordering::Equal => {
                    // Same group as the previous row.
                    let key: Vec<Value> = by_values.iter().map(|c| c[row].clone()).collect();
                    let _ = key; // group key already recorded; just append.
                    groups.last_mut().unwrap().1.push(row);
                    continue;
                }
                Ordering::Less => {}
            }
        }
        let key: Vec<Value> = by_values.iter().map(|c| c[row].clone()).collect();
        groups.push((key, vec![row]));
    }
    Ok(groups)
}

/// Group all rows by the tuple of the given class columns' values, in
/// `sas_cmp` order. Returns (key tuple, row indices) pairs.
pub fn group_by_keys(
    class_values: &[&Vec<Value>],
    n_obs: usize,
) -> Vec<(Vec<Value>, Vec<usize>)> {
    let mut groups: Vec<(Vec<Value>, Vec<usize>)> = Vec::new();
    for row in 0..n_obs {
        let key: Vec<Value> = class_values.iter().map(|c| c[row].clone()).collect();
        // Find an existing group with an equal key (sas_cmp equality).
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
    // Order groups by the key tuple via sas_cmp.
    groups.sort_by(|(a, _), (b, _)| {
        for (x, y) in a.iter().zip(b) {
            let c = x.sas_cmp(y);
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    });
    groups
}
