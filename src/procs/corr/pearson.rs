use super::*;

// ───────────────────────── numeric core ─────────────────────────

/// Pearson r over pairwise-complete observations. Returns (r, n) where n is
/// the number of complete pairs. r is `None` when n < 2 or either variable
/// has zero variance (constant) over the pairwise-complete set.
pub(super) fn pearson(xcol: &[Value], ycol: &[Value]) -> (Option<f64>, usize) {
    let (xs, ys) = paired_complete(xcol, ycol);
    let n = xs.len();
    if n < 2 {
        return (None, n);
    }
    (pearson_xy(&xs, &ys), n)
}

/// Collect the pairwise-complete numeric observations of two columns as
/// parallel `(xs, ys)` vectors (rows where either value is missing/NaN are
/// dropped). Shared by Pearson, Spearman and Kendall.
pub(super) fn paired_complete(xcol: &[Value], ycol: &[Value]) -> (Vec<f64>, Vec<f64>) {
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    let n_rows = xcol.len().min(ycol.len());
    for i in 0..n_rows {
        match (value_to_num(&xcol[i]), value_to_num(&ycol[i])) {
            (Some(x), Some(y)) if !x.is_nan() && !y.is_nan() => {
                xs.push(x);
                ys.push(y);
            }
            _ => {}
        }
    }
    (xs, ys)
}

/// Pearson r over two already paired-complete numeric vectors. Returns None
/// when n < 2 or either side is constant (zero variance).
pub(super) fn pearson_xy(xs: &[f64], ys: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let nf = n as f64;
    let mx = xs.iter().sum::<f64>() / nf;
    let my = ys.iter().sum::<f64>() / nf;
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for k in 0..n {
        let dx = xs[k] - mx;
        let dy = ys[k] - my;
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return None;
    }
    Some((sxy / (sxx.sqrt() * syy.sqrt())).clamp(-1.0, 1.0))
}

/// Weighted Pearson r over pairwise-complete observations. An observation is
/// usable only when x, y AND w are non-missing and w > 0 (SAS WEIGHT rule).
/// Weighted moments: mean_w = Σw·x/Σw, cov_w = Σw(x−mx)(y−my)/Σw, etc.
/// Returns (r, n) where n counts the usable triples. r is None when n < 2 or
/// either weighted variance is zero.
pub(super) fn pearson_weighted(
    xcol: &[Value],
    ycol: &[Value],
    wcol: &[Value],
) -> (Option<f64>, usize) {
    let n_rows = xcol.len().min(ycol.len()).min(wcol.len());
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    let mut ws: Vec<f64> = Vec::new();
    for i in 0..n_rows {
        match (
            value_to_num(&xcol[i]),
            value_to_num(&ycol[i]),
            value_to_num(&wcol[i]),
        ) {
            (Some(x), Some(y), Some(w)) if !x.is_nan() && !y.is_nan() && !w.is_nan() && w > 0.0 => {
                xs.push(x);
                ys.push(y);
                ws.push(w);
            }
            _ => {}
        }
    }
    let n = xs.len();
    if n < 2 {
        return (None, n);
    }
    let sw: f64 = ws.iter().sum();
    if sw <= 0.0 {
        return (None, n);
    }
    let mx: f64 = xs.iter().zip(&ws).map(|(x, w)| w * x).sum::<f64>() / sw;
    let my: f64 = ys.iter().zip(&ws).map(|(y, w)| w * y).sum::<f64>() / sw;
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for k in 0..n {
        let dx = xs[k] - mx;
        let dy = ys[k] - my;
        sxy += ws[k] * dx * dy;
        sxx += ws[k] * dx * dx;
        syy += ws[k] * dy * dy;
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return (None, n);
    }
    let r = (sxy / (sxx.sqrt() * syy.sqrt())).clamp(-1.0, 1.0);
    (Some(r), n)
}

/// Collect the pairwise-complete (x, y, w) triples usable under the SAS WEIGHT
/// rule (x, y, w all non-missing and w > 0). Shared by weighted Spearman /
/// Kendall.
pub(super) fn paired_complete_weighted(
    xcol: &[Value],
    ycol: &[Value],
    wcol: &[Value],
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let n_rows = xcol.len().min(ycol.len()).min(wcol.len());
    let (mut xs, mut ys, mut ws) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..n_rows {
        match (
            value_to_num(&xcol[i]),
            value_to_num(&ycol[i]),
            value_to_num(&wcol[i]),
        ) {
            (Some(x), Some(y), Some(w)) if !x.is_nan() && !y.is_nan() && !w.is_nan() && w > 0.0 => {
                xs.push(x);
                ys.push(y);
                ws.push(w);
            }
            _ => {}
        }
    }
    (xs, ys, ws)
}

/// Weighted mid-ranks: the rank a value would receive if every observation were
/// replicated `w` times. Sorting by value, a tie block of total weight `W`
/// starting at cumulative weight `c` occupies positions `c+1 .. c+W`, whose
/// average (the mid-rank) is `c + (W + 1)/2`; every member of the block gets
/// that mid-rank. With integer weights this is exactly the mid-rank vector of
/// the `w`-replicated data, which makes weighted Spearman reduce to ordinary
/// Spearman on the replicated dataset (see unit test).
pub(super) fn weighted_mean_ranks(xs: &[f64], ws: &[f64]) -> Vec<f64> {
    let n = xs.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| {
        xs[a]
            .partial_cmp(&xs[b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut ranks = vec![0.0_f64; n];
    let mut cum = 0.0_f64;
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && xs[idx[j]] == xs[idx[i]] {
            j += 1;
        }
        let wblock: f64 = idx[i..j].iter().map(|&k| ws[k]).sum();
        let mid = cum + (wblock + 1.0) / 2.0;
        for &k in &idx[i..j] {
            ranks[k] = mid;
        }
        cum += wblock;
        i = j;
    }
    ranks
}

/// Weighted Pearson over two already paired numeric vectors with weights `ws`.
/// Returns None when n < 2 or either weighted variance is zero.
pub(super) fn weighted_pearson_xy(xs: &[f64], ys: &[f64], ws: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let sw: f64 = ws.iter().sum();
    if sw <= 0.0 {
        return None;
    }
    let mx: f64 = xs.iter().zip(ws).map(|(x, w)| w * x).sum::<f64>() / sw;
    let my: f64 = ys.iter().zip(ws).map(|(y, w)| w * y).sum::<f64>() / sw;
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for k in 0..n {
        let dx = xs[k] - mx;
        let dy = ys[k] - my;
        sxy += ws[k] * dx * dy;
        sxx += ws[k] * dx * dx;
        syy += ws[k] * dy * dy;
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return None;
    }
    Some((sxy / (sxx.sqrt() * syy.sqrt())).clamp(-1.0, 1.0))
}

/// Two-sided p-value for Pearson r with n pairwise-complete observations:
/// t = r*sqrt((n-2)/(1-r^2)), p = P(|T_{n-2}| > |t|). Returns None when the
/// test is undefined (n < 3, or |r| == 1 exactly).
pub(super) fn pearson_pvalue(r: f64, n: usize) -> Option<f64> {
    if n < 3 {
        return None;
    }
    let df = (n - 2) as f64;
    if r.abs() >= 1.0 {
        return Some(0.0);
    }
    let t = r * (df / (1.0 - r * r)).sqrt();
    Some(student_t_sf_two_sided(t.abs(), df))
}
