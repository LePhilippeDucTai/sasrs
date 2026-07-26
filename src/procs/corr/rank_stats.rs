use super::*;

/// Weighted Spearman rank correlation: weighted Pearson on the weighted
/// mid-ranks of x and y (weights from the WEIGHT variable, SAS exclusion rule).
/// With integer weights interpreted as replication counts this equals ordinary
/// Spearman on the replicated data. Returns (r, n) where n is the number of
/// usable (non-replicated) observations. None when n < 2 or a rank vector is
/// constant.
pub(super) fn spearman_weighted(
    xcol: &[Value],
    ycol: &[Value],
    wcol: &[Value],
) -> (Option<f64>, usize) {
    let (xs, ys, ws) = paired_complete_weighted(xcol, ycol, wcol);
    let n = xs.len();
    if n < 2 {
        return (None, n);
    }
    let rx = weighted_mean_ranks(&xs, &ws);
    let ry = weighted_mean_ranks(&ys, &ws);
    (weighted_pearson_xy(&rx, &ry, &ws), n)
}

/// Weighted Kendall tau-b. Each unordered pair (i, j) is weighted by w_i·w_j in
/// the concordant/discordant counts AND in the tie-corrected denominator
/// totals n0 = Σ_{i<j} w_i w_j, n1 = Σ ties-in-x w_i w_j, n2 = Σ ties-in-y.
/// τ_b = (C − D)/√((n0 − n1)(n0 − n2)). With integer weights this matches
/// ordinary tau-b on the replicated data: the within-block self-pairs of a
/// replicated observation are tied in both coordinates, so they cancel between
/// n0 and n1 (and n0 and n2) and never count as C or D. Returns (τ, n). None
/// when n < 2 or a denominator factor is zero.
pub(super) fn kendall_weighted(
    xcol: &[Value],
    ycol: &[Value],
    wcol: &[Value],
) -> (Option<f64>, usize) {
    let (xs, ys, ws) = paired_complete_weighted(xcol, ycol, wcol);
    let n = xs.len();
    if n < 2 {
        return (None, n);
    }
    let (mut concordant, mut discordant) = (0.0_f64, 0.0_f64);
    let (mut n0, mut n1, mut n2) = (0.0_f64, 0.0_f64, 0.0_f64);
    for i in 0..n {
        for j in (i + 1)..n {
            let ww = ws[i] * ws[j];
            n0 += ww;
            let dx = xs[i] - xs[j];
            let dy = ys[i] - ys[j];
            if dx == 0.0 {
                n1 += ww;
            }
            if dy == 0.0 {
                n2 += ww;
            }
            if dx != 0.0 && dy != 0.0 {
                if dx.signum() * dy.signum() > 0.0 {
                    concordant += ww;
                } else {
                    discordant += ww;
                }
            }
        }
    }
    let denom = (n0 - n1) * (n0 - n2);
    if denom <= 0.0 {
        return (None, n);
    }
    let tau = (concordant - discordant) / denom.sqrt();
    (Some(tau.clamp(-1.0, 1.0)), n)
}

/// Hoeffding's D measure of dependence over pairwise-complete observations.
///
/// With bivariate mid-ranks R_i = rank(x_i), S_i = rank(y_i) and the bivariate
/// "CDF count" Q_i = 1 + Σ_{j≠i} c(x_j,x_i)·c(y_j,y_i), where c(a,b)=1 if a<b,
/// ½ if a==b, 0 otherwise (so a point tied in exactly one coordinate and less
/// in the other contributes ¼, and one tied in both contributes ¼):
///   D1 = Σ (Q_i−1)(Q_i−2)
///   D2 = Σ (R_i−1)(R_i−2)(S_i−1)(S_i−2)
///   D3 = Σ (R_i−2)(S_i−2)(Q_i−1)
///   D  = 30·[(n−2)(n−3)·D1 + D2 − 2(n−2)·D3] / [n(n−1)(n−2)(n−3)(n−4)]
/// Requires n ≥ 5; returns (None, n) otherwise. This reproduces SAS PROC CORR
/// HOEFFDING to 5 decimals (e.g. sashelp.class height×weight → 0.31609).
pub(super) fn hoeffding_d(xcol: &[Value], ycol: &[Value]) -> (Option<f64>, usize) {
    let (xs, ys) = paired_complete(xcol, ycol);
    let n = xs.len();
    if n < 5 {
        return (None, n);
    }
    let r = mean_ranks(&xs);
    let s = mean_ranks(&ys);
    let mut q = vec![1.0_f64; n];
    for i in 0..n {
        let mut c = 1.0_f64;
        for j in 0..n {
            if j == i {
                continue;
            }
            let cx = if xs[j] < xs[i] {
                1.0
            } else if xs[j] == xs[i] {
                0.5
            } else {
                0.0
            };
            let cy = if ys[j] < ys[i] {
                1.0
            } else if ys[j] == ys[i] {
                0.5
            } else {
                0.0
            };
            c += cx * cy;
        }
        q[i] = c;
    }
    let mut d1 = 0.0;
    let mut d2 = 0.0;
    let mut d3 = 0.0;
    for i in 0..n {
        d1 += (q[i] - 1.0) * (q[i] - 2.0);
        d2 += (r[i] - 1.0) * (r[i] - 2.0) * (s[i] - 1.0) * (s[i] - 2.0);
        d3 += (r[i] - 2.0) * (s[i] - 2.0) * (q[i] - 1.0);
    }
    let nf = n as f64;
    let num = (nf - 2.0) * (nf - 3.0) * d1 + d2 - 2.0 * (nf - 2.0) * d3;
    let den = nf * (nf - 1.0) * (nf - 2.0) * (nf - 3.0) * (nf - 4.0);
    let d = 30.0 * num / den;
    (Some(d), n)
}

/// Asymptotic upper-tail probability `Prob > D` for Hoeffding's D.
///
/// Under H0 (independence), the Blum-Kiefer-Rosenblatt (1961) limiting law of
/// the statistic `B = (n−1)·D/30` is the distribution of the χ²-mixture
/// `Σ_{j,k≥1} λ_jk·Z_jk²` with eigenvalues `λ_jk = 1/(π⁴ j² k²)`. The survival
/// function is evaluated by Imhof's (1961) numerical inversion on a fixed,
/// deterministic eigenvalue grid (j,k = 1..30).
///
/// DOCUMENTED APPROXIMATION: this is the *large-sample* p-value. SAS PROC CORR
/// uses a tabulated small-sample distribution for moderate n; reproducing that
/// table exactly is impractical here, so for small n the printed `Prob > D` is
/// an asymptotic approximation and may differ from SAS in the 4th decimal. The
/// D coefficient itself is computed exactly and matches SAS. Returns None when
/// n < 5.
pub(super) fn hoeffding_pvalue(d: f64, n: usize) -> Option<f64> {
    if n < 5 {
        return None;
    }
    let b = (n as f64 - 1.0) * d / 30.0;
    Some(bkr_survival(b))
}

/// Survival function P(B > b) of the Blum-Kiefer-Rosenblatt limiting law via
/// Imhof's method on the eigenvalue grid `λ_jk = 1/(π⁴ j² k²)`, j,k = 1..30.
pub(super) fn bkr_survival(b: f64) -> f64 {
    if b <= 0.0 {
        return 1.0;
    }
    const M: usize = 30;
    let pi4 = std::f64::consts::PI.powi(4);
    let mut lam: Vec<f64> = Vec::with_capacity(M * M);
    for j in 1..=M {
        for k in 1..=M {
            lam.push(1.0 / (pi4 * (j * j) as f64 * (k * k) as f64));
        }
    }
    // Imhof: P(Q>b) = 1/2 + (1/π)∫_0^U sin(θ(u))/(u·ρ(u)) du, with
    // θ(u) = ½ Σ atan(λ u) − ½ b u, ρ(u) = Π (1+(λ u)²)^{1/4}.
    const U: f64 = 1000.0;
    const STEPS: usize = 20_000;
    let h = U / STEPS as f64;
    let mut sum = 0.0;
    for i in 1..STEPS {
        let u = i as f64 * h;
        let mut theta = -0.5 * b * u;
        let mut lnrho = 0.0;
        for &l in &lam {
            let lu = l * u;
            theta += 0.5 * lu.atan();
            lnrho += 0.25 * (lu * lu).ln_1p();
        }
        sum += theta.sin() * (-lnrho).exp() / u;
    }
    sum *= h;
    (0.5 + sum / std::f64::consts::PI).clamp(0.0, 1.0)
}

/// Mean (midrank) ranks of a slice: ties receive the average of the ranks they
/// would otherwise occupy (1-based). Used by Spearman.
pub(super) fn mean_ranks(xs: &[f64]) -> Vec<f64> {
    let n = xs.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| {
        xs[a]
            .partial_cmp(&xs[b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut ranks = vec![0.0_f64; n];
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && xs[idx[j]] == xs[idx[i]] {
            j += 1;
        }
        // Ranks i+1 .. j (1-based) averaged.
        let avg = ((i + 1 + j) as f64) / 2.0; // sum (i+1..=j) / count
        for &k in &idx[i..j] {
            ranks[k] = avg;
        }
        i = j;
    }
    ranks
}

/// Spearman rank correlation over pairwise-complete observations: Pearson on
/// the midranks of x and y. Returns (r, n). None when n < 2 or a rank vector
/// is constant (all values tied).
pub(super) fn spearman(xcol: &[Value], ycol: &[Value]) -> (Option<f64>, usize) {
    let (xs, ys) = paired_complete(xcol, ycol);
    let n = xs.len();
    if n < 2 {
        return (None, n);
    }
    let rx = mean_ranks(&xs);
    let ry = mean_ranks(&ys);
    (pearson_xy(&rx, &ry), n)
}

/// Kendall tau-b over pairwise-complete observations.
/// τ_b = (n_c − n_d)/√((n0 − n1)(n0 − n2)), with n0 = n(n−1)/2,
/// n1 = Σ t_i(t_i−1)/2 (ties in x), n2 = Σ u_j(u_j−1)/2 (ties in y).
/// Returns (tau, n). None when n < 2 or a denominator factor is zero (i.e. all
/// x tied or all y tied).
pub(super) fn kendall_tau_b(xcol: &[Value], ycol: &[Value]) -> (Option<f64>, usize) {
    let (xs, ys) = paired_complete(xcol, ycol);
    let n = xs.len();
    if n < 2 {
        return (None, n);
    }
    let mut concordant: i64 = 0;
    let mut discordant: i64 = 0;
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = xs[i] - xs[j];
            let dy = ys[i] - ys[j];
            let s = dx.signum() * dy.signum();
            if dx != 0.0 && dy != 0.0 {
                if s > 0.0 {
                    concordant += 1;
                } else {
                    discordant += 1;
                }
            }
        }
    }
    let n0 = (n as f64) * (n as f64 - 1.0) / 2.0;
    let n1 = tie_term(&xs);
    let n2 = tie_term(&ys);
    let denom = (n0 - n1) * (n0 - n2);
    if denom <= 0.0 {
        return (None, n);
    }
    let tau = (concordant - discordant) as f64 / denom.sqrt();
    (Some(tau.clamp(-1.0, 1.0)), n)
}

/// Σ t(t−1)/2 over groups of equal values (tie correction term for Kendall).
pub(super) fn tie_term(xs: &[f64]) -> f64 {
    let mut v: Vec<f64> = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut total = 0.0;
    let mut i = 0;
    let n = v.len();
    while i < n {
        let mut j = i + 1;
        while j < n && v[j] == v[i] {
            j += 1;
        }
        let t = (j - i) as f64;
        total += t * (t - 1.0) / 2.0;
        i = j;
    }
    total
}

/// Two-sided p-value for Kendall tau-b via the normal approximation
/// z = 3·τ·√(n(n−1)) / √(2(2n+5)); p = 2·(1 − Φ(|z|)). v1 ignores the
/// ties correction in the variance (documented). None when n < 2.
pub(super) fn kendall_pvalue(tau: f64, n: usize) -> Option<f64> {
    if n < 2 {
        return None;
    }
    let nf = n as f64;
    let z = 3.0 * tau * (nf * (nf - 1.0)).sqrt() / (2.0 * (2.0 * nf + 5.0)).sqrt();
    Some(2.0 * normal_sf(z.abs()))
}
