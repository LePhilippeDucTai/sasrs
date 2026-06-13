//! Helpers partagés par plusieurs PROCs.
//!
//! Ce module centralise les fonctions utilitaires communes afin d'éviter la
//! duplication de code entre `means`, `freq`, `univariate`, `sort`,
//! `transpose` et `append`. Chaque fonction est extraite verbatim de son
//! premier site d'apparition ; aucune logique n'est modifiée.

use crate::dataset::SasDataset;
use crate::error::Result;
use crate::missing::{num_to_value, value_to_num};
use crate::value::{Value, VarType};
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
