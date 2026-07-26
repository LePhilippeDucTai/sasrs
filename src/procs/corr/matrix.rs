// MQ7.2c — `needless_range_loop` assumé dans ce module : l'indice EST le
// langage du domaine (`a[i][j] * b[j][k]`, parcours colonne-major, triangle
// d'une matrice symétrique). La forme itérateur y coûte plus en lisibilité
// qu'elle n'en rend, et la revue a préféré garder les indices explicites.
#![allow(clippy::needless_range_loop)]

use super::*;

/// Correlation method requested by PROC CORR.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Method {
    Pearson,
    Spearman,
    Kendall,
    /// Hoeffding's D measure of dependence. Unlike the correlation methods,
    /// its cell statistic is D (not a correlation) and the probability column
    /// is `Prob > D` (see `hoeffding_d` / `hoeffding_pvalue`).
    Hoeffding,
}

impl Method {
    /// Heading line for the listing block.
    pub(super) fn heading(self) -> &'static str {
        match self {
            Method::Pearson => "Pearson Correlation Coefficients",
            Method::Spearman => "Spearman Correlation Coefficients",
            Method::Kendall => "Kendall Tau b Coefficients",
            Method::Hoeffding => "Hoeffding Dependence Coefficients",
        }
    }
}

/// `_TYPE_` value written in the TYPE=CORR output dataset's CORR rows. SAS uses
/// the literal "CORR" for Pearson, Spearman and Kendall datasets alike.
pub(super) const CORR_TYPE: &str = "CORR";

/// One computed (r, p, n) cell.
#[derive(Clone, Copy)]
pub(super) struct Cell {
    pub(super) r: Option<f64>,
    pub(super) p: Option<f64>,
    pub(super) n: usize,
}

/// Compute r/p/n for every (row_col, col_col) pair under `method`. WEIGHT (if
/// any) is applied to Pearson only.
pub(super) fn compute_matrix(
    method: Method,
    row_cols: &[usize],
    col_cols: &[usize],
    decoded: &std::collections::HashMap<usize, Vec<Value>>,
    weight: Option<&[Value]>,
) -> Vec<Vec<Cell>> {
    let mut out = vec![
        vec![
            Cell {
                r: None,
                p: None,
                n: 0
            };
            col_cols.len()
        ];
        row_cols.len()
    ];
    for (i, &rc) in row_cols.iter().enumerate() {
        for (j, &cc) in col_cols.iter().enumerate() {
            out[i][j] = compute_cell(method, &decoded[&rc], &decoded[&cc], rc == cc, weight);
        }
    }
    out
}

/// Compute a single cell. `same_var` marks the diagonal where r is exactly 1
/// and the p-value is left blank (SAS convention).
pub(super) fn compute_cell(
    method: Method,
    xcol: &[Value],
    ycol: &[Value],
    same_var: bool,
    weight: Option<&[Value]>,
) -> Cell {
    // Hoeffding's D is computed for every pair INCLUDING the diagonal: the
    // self-dependence D(x,x) is the maximum attainable D for the given n (SAS
    // prints this value, not a forced 1.0), so the `same_var` shortcut used by
    // the correlation methods does not apply here.
    if let Method::Hoeffding = method {
        let (d, n) = hoeffding_d(xcol, ycol);
        let p = d.and_then(|dv| hoeffding_pvalue(dv, n));
        return Cell { r: d, p, n };
    }
    if same_var {
        // N = non-missing count of the variable (weighted: usable count).
        let n = match (method, weight) {
            (Method::Pearson, Some(w)) => {
                let (pairs, _) = partition_weighted(xcol, w, &(0..xcol.len()).collect::<Vec<_>>());
                pairs.len()
            }
            _ => {
                let (xs, _) = partition_numeric(xcol, &(0..xcol.len()).collect::<Vec<_>>());
                xs.len()
            }
        };
        return Cell {
            r: Some(1.0),
            p: None,
            n,
        };
    }
    match method {
        Method::Pearson => {
            let (r, n) = match weight {
                Some(w) => pearson_weighted(xcol, ycol, w),
                None => pearson(xcol, ycol),
            };
            let p = r.and_then(|rv| pearson_pvalue(rv, n));
            Cell { r, p, n }
        }
        Method::Spearman => {
            let (r, n) = match weight {
                Some(w) => spearman_weighted(xcol, ycol, w),
                None => spearman(xcol, ycol),
            };
            let p = r.and_then(|rv| pearson_pvalue(rv, n));
            Cell { r, p, n }
        }
        Method::Kendall => {
            let (r, n) = match weight {
                Some(w) => kendall_weighted(xcol, ycol, w),
                None => kendall_tau_b(xcol, ycol),
            };
            let p = r.and_then(|rv| kendall_pvalue(rv, n));
            Cell { r, p, n }
        }
        // Hoeffding is handled by the early return above; unreachable here.
        Method::Hoeffding => {
            let (d, n) = hoeffding_d(xcol, ycol);
            let p = d.and_then(|dv| hoeffding_pvalue(dv, n));
            Cell { r: d, p, n }
        }
    }
}

/// Two-sided p-value for a PARTIAL Pearson correlation with `k` partialled
/// variables: df = n − k − 2 (vs n − 2 for an ordinary correlation).
pub(super) fn partial_pvalue(r: f64, n: usize, k: usize) -> Option<f64> {
    let df = (n as i64) - (k as i64) - 2;
    if df < 1 {
        return None;
    }
    let df = df as f64;
    if r.abs() >= 1.0 {
        return Some(0.0);
    }
    let t = r * (df / (1.0 - r * r)).sqrt();
    Some(student_t_sf_two_sided(t.abs(), df))
}

/// Partial Pearson correlation matrix (rows × cols), controlling for
/// `partial_cols`. Observations are **listwise-complete** across the union of
/// all row, column and partial variables. Each analysis variable is regressed
/// on `[1, partial vars]` (ordinary least squares via `stat::linalg`) and the
/// residuals are Pearson-correlated. The p-value uses df = n − k − 2.
pub(super) fn partial_pearson_matrix(
    row_cols: &[usize],
    col_cols: &[usize],
    partial_cols: &[usize],
    decoded: &std::collections::HashMap<usize, Vec<Value>>,
) -> Vec<Vec<Cell>> {
    let k = partial_cols.len();
    let mut involved: Vec<usize> = Vec::new();
    for &c in row_cols.iter().chain(col_cols).chain(partial_cols) {
        if !involved.contains(&c) {
            involved.push(c);
        }
    }
    let n_obs = involved
        .first()
        .and_then(|c| decoded.get(c))
        .map(|v| v.len())
        .unwrap_or(0);

    // Listwise-complete rows: every involved variable non-missing numeric.
    let mut rows_idx: Vec<usize> = Vec::new();
    'row: for i in 0..n_obs {
        for &c in &involved {
            match value_to_num(&decoded[&c][i]) {
                Some(f) if !f.is_nan() => {}
                _ => continue 'row,
            }
        }
        rows_idx.push(i);
    }
    let n = rows_idx.len();

    // Design matrix P = [1, partial vars] over the complete rows.
    let design: Vec<Vec<f64>> = rows_idx
        .iter()
        .map(|&i| {
            let mut row = Vec::with_capacity(k + 1);
            row.push(1.0);
            for &c in partial_cols {
                row.push(value_to_num(&decoded[&c][i]).unwrap());
            }
            row
        })
        .collect();

    // Residualise one involved variable on the partial set (None if rank-deficient).
    let residual = |c: usize| -> Option<Vec<f64>> {
        if n < k + 2 {
            return None;
        }
        let y: Vec<f64> = rows_idx
            .iter()
            .map(|&i| value_to_num(&decoded[&c][i]).unwrap())
            .collect();
        let beta = crate::stat::linalg::least_squares(&design, &y).ok()?;
        Some(
            y.iter()
                .zip(&design)
                .map(|(yi, xr)| yi - xr.iter().zip(&beta).map(|(a, b)| a * b).sum::<f64>())
                .collect(),
        )
    };
    let mut resid: std::collections::HashMap<usize, Option<Vec<f64>>> =
        std::collections::HashMap::new();
    for &c in &involved {
        resid.insert(c, residual(c));
    }

    let mut out = vec![
        vec![
            Cell {
                r: None,
                p: None,
                n
            };
            col_cols.len()
        ];
        row_cols.len()
    ];
    for (i, &rc) in row_cols.iter().enumerate() {
        for (j, &cc) in col_cols.iter().enumerate() {
            if rc == cc {
                out[i][j] = Cell {
                    r: Some(1.0),
                    p: None,
                    n,
                };
                continue;
            }
            let rx = resid.get(&rc).and_then(|o| o.as_ref());
            let ry = resid.get(&cc).and_then(|o| o.as_ref());
            out[i][j] = match (rx, ry) {
                (Some(rx), Some(ry)) => {
                    let r = pearson_xy(rx, ry);
                    let p = r.and_then(|rv| partial_pvalue(rv, n, k));
                    Cell { r, p, n }
                }
                _ => Cell {
                    r: None,
                    p: None,
                    n,
                },
            };
        }
    }
    out
}
