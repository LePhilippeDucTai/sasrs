use super::*;

// ───────────────────────── Multi-way engine ─────────────────────────

/// Build a reference-cell (last-level-dropped) design matrix and run the model.
/// Returns the SSE of the ordinary least-squares fit `min ‖Xβ − y‖²`.
pub(super) fn fit_sse(x: &[Vec<f64>], y: &[f64]) -> f64 {
    if x.is_empty() || x[0].is_empty() {
        // Intercept-free / empty model: SSE around 0 is Σ y².
        return y.iter().map(|&v| v * v).sum();
    }
    let beta = match crate::stat::linalg::least_squares(x, y) {
        Ok(b) => b,
        Err(_) => return f64::NAN,
    };
    let mut sse = 0.0;
    for (i, row) in x.iter().enumerate() {
        let yhat: f64 = row.iter().zip(&beta).map(|(&xij, &bj)| xij * bj).sum();
        let r = y[i] - yhat;
        sse += r * r;
    }
    sse
}

/// Compute the per-level dummy columns (reference-cell, last level dropped) for
/// one CLASS variable. `codes[i]` is the level index of observation i.
/// Returns a Vec of columns, one per non-reference level (L−1 columns).
pub(super) fn main_effect_dummies(codes: &[usize], n_levels: usize, n: usize) -> Vec<Vec<f64>> {
    let mut cols: Vec<Vec<f64>> = Vec::new();
    // Drop the LAST level as the reference cell.
    for lvl in 0..n_levels.saturating_sub(1) {
        let mut col = vec![0.0; n];
        for i in 0..n {
            if codes[i] == lvl {
                col[i] = 1.0;
            }
        }
        cols.push(col);
    }
    cols
}

/// Compute the sum-to-zero (effect / deviation) coded columns for one CLASS
/// variable. `codes[i]` is the level index of observation i. Levels are ordered
/// 1..L by sas_cmp; column j (j=0..L−2) is +1 at level j, −1 at the LAST level
/// L−1, else 0. Returns L−1 columns. Building interaction terms from elementwise
/// products of these centered contrasts yields the SAS Type III estimable
/// function, so the partial SS matches SAS Type III even on unbalanced data.
pub(super) fn main_effect_effect_coded(
    codes: &[usize],
    n_levels: usize,
    n: usize,
) -> Vec<Vec<f64>> {
    let mut cols: Vec<Vec<f64>> = Vec::new();
    let last = n_levels.saturating_sub(1);
    for lvl in 0..n_levels.saturating_sub(1) {
        let mut col = vec![0.0; n];
        for i in 0..n {
            if codes[i] == lvl {
                col[i] = 1.0;
            } else if codes[i] == last {
                col[i] = -1.0;
            }
        }
        cols.push(col);
    }
    cols
}

/// Render a CLASS level value the way the existing one-way path does.
// Divergence volontaire avec `common::value_label` : ANOVA affiche les
// niveaux numériques via `format!("{f}")`, pas BESTw. — digits imprimés
// différents, ne pas converger sans re-valider les snapshots.
pub(super) fn value_label(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim_end().to_string(),
        Value::Num(f) => format!("{f}"),
        Value::Missing(k) => k.display(),
    }
}
