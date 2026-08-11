use super::*;
use crate::procs::common::centered;

// ───────────────────────────── normality tests ─────────────────────────────
//
// All four statistics are computed from the ascending-sorted non-missing
// values, the sample mean, and the sample standard deviation (VARDEF=DF, the
// same `sample_std` used elsewhere). p-values follow published approximations
// (Royston for Shapiro-Wilk; Stephens for Anderson-Darling / Cramér-von Mises;
// Lilliefors/Dallal-Wilkinson for the EDF Kolmogorov-Smirnov D). They are NOT
// bit-for-bit identical to SAS 9.4, but reproduce the documented reference
// values; this is noted in PROGRESS.md.

/// A computed normality test: name, statistic label, statistic value, and an
/// optional p-value (None → not computable, shown as ".").
pub(super) struct NormalityTest {
    pub(super) name: &'static str,
    pub(super) stat_label: &'static str,
    pub(super) stat: f64,
    pub(super) p: Option<f64>,
}

/// Emit the "Tests for Normality" block for one variable. Requires the sorted
/// non-missing values plus the sample mean/std already computed by the caller.
/// Degenerate inputs (n < 3, zero variance, …) print a centered NOTE instead
/// of the table — never a panic.
pub(super) fn emit_normality_tests(
    session: &mut Session,
    sorted: &[f64],
    mean: Option<f64>,
    std: Option<f64>,
    n: usize,
) {
    // M38.4 : le blank de séparation avec la section précédente est émis par
    // l'appelant (`section_sep` d'emit.rs), qui sait si cette section est la
    // première affichée sous la liste ODS SELECT/EXCLUDE courante.
    centered(session, "Tests for Normality");
    session.listing.blank();

    let (mean, std) = match (mean, std) {
        (Some(m), Some(s)) if s > 0.0 && n >= 3 => (m, s),
        _ => {
            centered(
                session,
                "Tests for Normality require at least 3 nonmissing values with positive variance.",
            );
            return;
        }
    };

    let tests = compute_normality_tests(sorted, mean, std, n);

    // Columns: Test | Statistic-label | Statistic-value | "p Value"-label |
    // p-value. SAS renders the p as `Pr < W`, `Pr > D`, etc.
    let rows: Vec<Vec<String>> = tests
        .iter()
        .map(|t| {
            let pcell = match t.p {
                Some(p) => fmt_num(p),
                None => ".".to_string(),
            };
            let plabel = match t.name {
                "Shapiro-Wilk" => "Pr < W",
                "Kolmogorov-Smirnov" => "Pr > D",
                "Cramer-von Mises" => "Pr > W-Sq",
                "Anderson-Darling" => "Pr > A-Sq",
                _ => "Pr",
            };
            vec![
                t.name.to_string(),
                t.stat_label.to_string(),
                fmt_num(t.stat),
                plabel.to_string(),
                pcell,
            ]
        })
        .collect();

    session.listing.write_table(
        &[
            "Test".into(),
            "StatLabel".into(),
            "StatValue".into(),
            "PLabel".into(),
            "PValue".into(),
        ],
        &[
            Align::Left,
            Align::Left,
            Align::Right,
            Align::Left,
            Align::Right,
        ],
        &rows,
    );
}

/// Compute the four normality statistics + p-values. `sorted` ascending,
/// `mean`/`std` the sample moments (std > 0), `n == sorted.len() >= 3`.
pub(super) fn compute_normality_tests(
    sorted: &[f64],
    mean: f64,
    std: f64,
    n: usize,
) -> Vec<NormalityTest> {
    let mut out = Vec::with_capacity(4);

    // Shapiro-Wilk (only defined for 3 <= n <= 2000).
    let (sw_w, sw_p) = shapiro_wilk(sorted);
    out.push(NormalityTest {
        name: "Shapiro-Wilk",
        stat_label: "W",
        stat: sw_w.unwrap_or(f64::NAN),
        p: sw_p,
    });

    // Standardized, sorted z_i = (x_(i) - mean) / std.
    let z: Vec<f64> = sorted.iter().map(|&x| (x - mean) / std).collect();

    // Kolmogorov-Smirnov D (Lilliefors, estimated parameters).
    let (ks_d, ks_p) = kolmogorov_smirnov(&z, n);
    out.push(NormalityTest {
        name: "Kolmogorov-Smirnov",
        stat_label: "D",
        stat: ks_d,
        p: ks_p,
    });

    // Cramér-von Mises W².
    let (cvm, cvm_p) = cramer_von_mises(&z, n);
    out.push(NormalityTest {
        name: "Cramer-von Mises",
        stat_label: "W-Sq",
        stat: cvm,
        p: cvm_p,
    });

    // Anderson-Darling A².
    let (ad, ad_p) = anderson_darling(&z, n);
    out.push(NormalityTest {
        name: "Anderson-Darling",
        stat_label: "A-Sq",
        stat: ad,
        p: ad_p,
    });

    out
}

/// Shapiro-Wilk W and its p-value (Royston 1992 algorithm AS R94).
/// Valid for 3 <= n <= 2000. Returns `(Some(W), Some(p))`, or `(None, None)`
/// when n is out of range. `sorted` must be ascending with positive variance.
pub(super) fn shapiro_wilk(sorted: &[f64]) -> (Option<f64>, Option<f64>) {
    let n = sorted.len();
    if !(3..=2000).contains(&n) {
        return (None, None);
    }
    let nf = n as f64;
    let mean = sorted.iter().sum::<f64>() / nf;
    let ss: f64 = sorted.iter().map(|x| (x - mean) * (x - mean)).sum();
    if ss <= 0.0 {
        return (None, None);
    }

    // Expected values of standard normal order statistics, m_i = Φ⁻¹((i-3/8)/(n+1/4)).
    let m: Vec<f64> = (1..=n)
        .map(|i| phi_inv((i as f64 - 0.375) / (nf + 0.25)))
        .collect();
    let m_sq_sum: f64 = m.iter().map(|v| v * v).sum();
    let rsn = 1.0 / nf.sqrt();

    // Royston polynomial corrections for a_n and a_{n-1}.
    let poly = |c: &[f64], x: f64| -> f64 {
        // Horner with c[0] the constant term.
        c.iter().rev().fold(0.0, |acc, &ci| acc * x + ci)
    };
    const C1: [f64; 6] = [0.0, 0.221157, -0.147981, -2.071190, 4.434685, -2.706056];
    const C2: [f64; 6] = [0.0, 0.042981, -0.293762, -1.752461, 5.682633, -3.582633];

    let mut a = vec![0.0_f64; n];
    let a_n = m[n - 1] / m_sq_sum.sqrt() + poly(&C1, rsn);
    let (i1, fac);
    if n > 5 {
        let a_n1 = m[n - 2] / m_sq_sum.sqrt() + poly(&C2, rsn);
        a[n - 1] = a_n;
        a[n - 2] = a_n1;
        a[0] = -a_n;
        a[1] = -a_n1;
        // Rescale the interior coefficients.
        let phi = (m_sq_sum - 2.0 * m[n - 1] * m[n - 1] - 2.0 * m[n - 2] * m[n - 2])
            / (1.0 - 2.0 * a_n * a_n - 2.0 * a_n1 * a_n1);
        fac = phi.sqrt();
        i1 = 2;
    } else {
        a[n - 1] = a_n;
        a[0] = -a_n;
        let phi = (m_sq_sum - 2.0 * m[n - 1] * m[n - 1]) / (1.0 - 2.0 * a_n * a_n);
        fac = phi.sqrt();
        i1 = 1;
    }
    for i in i1..(n - i1) {
        a[i] = m[i] / fac;
    }

    // W = (Σ a_i x_(i))² / Σ(x_i - x̄)².
    let num: f64 = a.iter().zip(sorted.iter()).map(|(&ai, &xi)| ai * xi).sum();
    let w = (num * num) / ss;
    let w = w.min(1.0);

    // p-value via Royston's normalizing transform.
    let p = shapiro_wilk_pvalue(w, n);
    (Some(w), Some(p))
}

/// Royston (1992) p-value for Shapiro-Wilk W, n >= 3.
pub(super) fn shapiro_wilk_pvalue(w: f64, n: usize) -> f64 {
    let nf = n as f64;
    if n == 3 {
        // Exact small-sample formula (Royston): p = 6/π · (asin(√W) − asin(√(3/4))).
        let pi = std::f64::consts::PI;
        let p = 6.0 / pi * ((w.sqrt()).asin() - (0.75_f64.sqrt()).asin());
        return (1.0 - p).clamp(0.0, 1.0);
    }
    let ln_n = nf.ln();
    let (mu, sigma, z);
    if n <= 11 {
        // Small-sample branch: γ-transform of (1 - W).
        const G: [f64; 2] = [-2.273, 0.459];
        const M: [f64; 4] = [0.5440, -0.39978, 0.025054, -6.714e-4];
        const S: [f64; 4] = [1.3822, -0.77857, 0.062767, -0.0020322];
        let gamma = G[0] + G[1] * nf;
        mu = M[0] + M[1] * nf + M[2] * nf * nf + M[3] * nf * nf * nf;
        let ln_sigma = S[0] + S[1] * nf + S[2] * nf * nf + S[3] * nf * nf * nf;
        sigma = ln_sigma.exp();
        let y = -(gamma - (1.0 - w).ln()).ln();
        z = (y - mu) / sigma;
    } else {
        // Large-sample branch (n >= 12): ln(1 - W) normalized in ln(n).
        const M: [f64; 4] = [-1.5861, -0.31082, -0.083751, 0.0038915];
        const S: [f64; 3] = [-0.4803, -0.082676, 0.0030302];
        mu = M[0] + M[1] * ln_n + M[2] * ln_n * ln_n + M[3] * ln_n * ln_n * ln_n;
        let ln_sigma = S[0] + S[1] * ln_n + S[2] * ln_n * ln_n;
        sigma = ln_sigma.exp();
        let y = (1.0 - w).ln();
        z = (y - mu) / sigma;
    }
    // p = P(Z > z) = upper tail of standard normal.
    1.0 - probnorm(z)
}

/// Kolmogorov-Smirnov D (Lilliefors test, parameters estimated from the data)
/// and an approximate p-value. `z` are the standardized sorted values; `n` is
/// the sample size.
pub(super) fn kolmogorov_smirnov(z: &[f64], n: usize) -> (f64, Option<f64>) {
    let nf = n as f64;
    let mut d = 0.0_f64;
    for (i, &zi) in z.iter().enumerate() {
        let f = probnorm(zi);
        let d_plus = (i as f64 + 1.0) / nf - f; // F_n(x_i) - F(x_i)
        let d_minus = f - (i as f64) / nf; // F(x_i) - F_n(x_i⁻)
        d = d.max(d_plus).max(d_minus);
    }
    let p = lilliefors_pvalue(d, n);
    (d, Some(p))
}

/// Approximate Lilliefors p-value for the KS D statistic with estimated
/// parameters, via the Dallal & Wilkinson (1986) analytic approximation.
///
/// This single-exponential form is the published upper-tail probability and is
/// accurate for the significant region p ≤ 0.10; for larger D the exponent
/// becomes < 1 (often > 1 before clamping), so values are clamped to 1.0 and
/// interpreted as "p > 0.10" (non-significant). Documented approximation — not
/// bit-identical to SAS's internal Lilliefors table.
pub(super) fn lilliefors_pvalue(d: f64, n: usize) -> f64 {
    if d <= 0.0 {
        return 1.0;
    }
    // For n > 100, scale D and cap the effective sample size at 100
    // (Dallal-Wilkinson extension).
    let (d_eff, n_eff) = if n > 100 {
        (d * (n as f64 / 100.0).powf(0.49), 100.0_f64)
    } else {
        (d, n as f64)
    };
    let expo = -7.01256 * d_eff * d_eff * (n_eff + 2.78019)
        + 2.99587 * d_eff * (n_eff + 2.78019).sqrt()
        - 0.122119
        + 0.974598 / n_eff.sqrt()
        + 1.67997 / n_eff;
    let pval = expo.exp();
    if pval.is_finite() {
        pval.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Cramér-von Mises W² (estimated parameters) and Stephens p-value.
pub(super) fn cramer_von_mises(z: &[f64], n: usize) -> (f64, Option<f64>) {
    let nf = n as f64;
    let mut w2 = 1.0 / (12.0 * nf);
    for (i, &zi) in z.iter().enumerate() {
        let f = probnorm(zi);
        let term = f - (2.0 * (i as f64 + 1.0) - 1.0) / (2.0 * nf);
        w2 += term * term;
    }
    // Modification for estimated parameters.
    let w2_star = w2 * (1.0 + 0.5 / nf);
    let p = cvm_pvalue(w2_star);
    (w2, Some(p))
}

/// Stephens (1974) p-value regions for the (modified) Cramér-von Mises W²*.
pub(super) fn cvm_pvalue(w: f64) -> f64 {
    // Piecewise upper-tail approximation (Stephens / D'Agostino & Stephens).
    if w < 0.0275 {
        1.0
    } else if w < 0.051 {
        1.0 - (-13.953 + 775.5 * w - 12542.61 * w * w).exp()
    } else if w < 0.092 {
        1.0 - (-5.903 + 179.546 * w - 1515.29 * w * w).exp()
    } else if w < 1.1 {
        (0.886 - 31.62 * w + 10.897 * w * w).exp()
    } else {
        0.0
    }
    .clamp(0.0, 1.0)
}

/// Anderson-Darling A² (estimated parameters) and Stephens p-value.
pub(super) fn anderson_darling(z: &[f64], n: usize) -> (f64, Option<f64>) {
    let nf = n as f64;
    let mut s = 0.0_f64;
    for i in 0..n {
        let fi = probnorm(z[i]); // Φ(z_(i))
        let fr = probnorm(z[n - 1 - i]); // Φ(z_(n+1-i)) with 0-based index
        // Guard the logs against 0/1 (degenerate tails).
        let a = fi.clamp(1e-300, 1.0 - 1e-16);
        let b = (1.0 - fr).clamp(1e-300, 1.0);
        s += (2.0 * (i as f64 + 1.0) - 1.0) * (a.ln() + b.ln());
    }
    let a2 = -nf - s / nf;
    let a2_star = a2 * (1.0 + 0.75 / nf + 2.25 / (nf * nf));
    let p = ad_pvalue(a2_star);
    (a2, Some(p))
}

/// Stephens (1974) p-value regions for the (modified) Anderson-Darling A²*.
pub(super) fn ad_pvalue(a: f64) -> f64 {
    if a < 0.2 {
        1.0 - (-13.436 + 101.14 * a - 223.73 * a * a).exp()
    } else if a < 0.34 {
        1.0 - (-8.318 + 42.796 * a - 59.938 * a * a).exp()
    } else if a < 0.6 {
        (0.9177 - 4.279 * a - 1.38 * a * a).exp()
    } else if a < 13.0 {
        (1.2937 - 5.709 * a + 0.0186 * a * a).exp()
    } else {
        0.0
    }
    .clamp(0.0, 1.0)
}
