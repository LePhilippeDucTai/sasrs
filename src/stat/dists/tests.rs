use super::*;

fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * (1.0 + b.abs())
}

// ───────────────────────── promoted helpers ─────────────────────────

#[test]
fn test_ln_gamma() {
    // Γ(1)=1, Γ(5)=24, Γ(0.5)=√π.
    assert!(approx(ln_gamma(1.0), 0.0, 1e-10));
    assert!(approx(ln_gamma(5.0), 24f64.ln(), 1e-10));
    assert!(approx(
        ln_gamma(0.5),
        std::f64::consts::PI.sqrt().ln(),
        1e-10
    ));
}

#[test]
fn test_betai() {
    // I_x(a,b) edge cases and symmetry I_x(a,b) = 1 - I_{1-x}(b,a).
    assert_eq!(betai(2.0, 3.0, 0.0), 0.0);
    assert_eq!(betai(2.0, 3.0, 1.0), 1.0);
    assert!(approx(
        betai(2.0, 3.0, 0.5),
        1.0 - betai(3.0, 2.0, 0.5),
        1e-12
    ));
    // I_0.5(1,1) = 0.5 (uniform).
    assert!(approx(betai(1.0, 1.0, 0.5), 0.5, 1e-12));
}

#[test]
fn test_student_t_cdf() {
    // Symmetric: CDF(0) = 0.5.
    assert!(approx(student_t_cdf(0.0, 10.0), 0.5, 1e-12));
    // df=10, t=2.228 → ~0.975 (two-tailed 0.05 critical value).
    assert!(approx(student_t_cdf(2.228138852, 10.0), 0.975, 1e-6));
    // Symmetry: CDF(-t) = 1 - CDF(t).
    assert!(approx(
        student_t_cdf(-1.5, 7.0),
        1.0 - student_t_cdf(1.5, 7.0),
        1e-12
    ));
}

#[test]
fn test_gammq() {
    // Q(a,0)=1, monotone decreasing in x.
    assert_eq!(gammq(2.0, 0.0), 1.0);
    assert!(gammq(2.0, 1.0) > gammq(2.0, 3.0));
    // Q(1,x) = exp(-x) (exponential survival).
    assert!(approx(gammq(1.0, 2.0), (-2.0f64).exp(), 1e-10));
}

#[test]
fn test_erf() {
    // erf(0)=0, erf(∞)→1, odd function.
    assert_eq!(erf(0.0), 0.0);
    assert!(approx(erf(1.0), 0.8427007929, 1e-8));
    assert!(approx(erf(-0.5), -erf(0.5), 1e-12));
}

#[test]
fn test_probnorm() {
    // Φ(0)=0.5, Φ(1.959964)≈0.975, Φ(-z)=1-Φ(z).
    assert!(approx(probnorm(0.0), 0.5, 1e-12));
    assert!(approx(probnorm(1.959963985), 0.975, 1e-8));
    assert!(approx(probnorm(-1.0), 1.0 - probnorm(1.0), 1e-12));
}

#[test]
fn test_phi_inv() {
    // Φ⁻¹(0.5)=0, Φ⁻¹(0.975)≈1.9599640, round-trip.
    assert!(approx(phi_inv(0.5), 0.0, 1e-10));
    assert!(approx(phi_inv(0.975), 1.959963985, 1e-7));
    assert!(approx(probnorm(phi_inv(0.123)), 0.123, 1e-10));
}

#[test]
fn test_ln_factorial_choose() {
    // 5! = 120, C(5,2)=10, C(10,0)=1.
    assert!(approx(ln_factorial(5).exp(), 120.0, 1e-8));
    assert!(approx(ln_choose(5, 2).exp(), 10.0, 1e-8));
    assert!(approx(ln_choose(10, 0).exp(), 1.0, 1e-8));
    assert_eq!(ln_choose(2, 5), f64::NEG_INFINITY);
}

// ───────────────────────── chi-squared ─────────────────────────

#[test]
fn test_chisq_cdf() {
    assert_eq!(chisq_cdf(0.0, 2.0), 0.0);
    assert_eq!(chisq_cdf(-1.0, 2.0), 0.0);
    // SAS reference: df=2, x=5 → 0.91791.
    assert!(approx(chisq_cdf(5.0, 2.0), 0.9179150014, 1e-6));
    // Critical value: df=1, x=3.841459 → 0.95.
    assert!(approx(chisq_cdf(3.841458821, 1.0), 0.95, 1e-6));
}

#[test]
fn test_chisq_quantile() {
    // chisq_quantile(0.95, 1) ≈ 3.841459.
    assert!(approx(chisq_quantile(0.95, 1.0), 3.841458821, 1e-5));
    // df=10, 0.95 → 18.30704.
    assert!(approx(chisq_quantile(0.95, 10.0), 18.30703805, 1e-5));
    // Round-trip with CDF.
    assert!(approx(chisq_cdf(chisq_quantile(0.3, 5.0), 5.0), 0.3, 1e-8));
}

#[test]
fn test_chisq_edge() {
    assert_eq!(chisq_quantile(0.0, 3.0), 0.0);
    assert!(chisq_quantile(1.0, 3.0).is_infinite());
    assert!(chisq_cdf(1.0, -1.0).is_nan());
}

// ───────────────────────── F distribution ─────────────────────────

#[test]
fn test_f_cdf() {
    assert_eq!(f_cdf(0.0, 2.0, 10.0), 0.0);
    // df1=2, df2=10, x=1: CDF = betai(1,5,1/6) = 1-(5/6)^5 = 0.59812.
    // (The 0.40155 in the header is the upper-tail survival prob 1-CDF.)
    assert!(approx(f_cdf(1.0, 2.0, 10.0), 0.5981224280, 1e-9));
    // Critical: df1=2, df2=10, x=4.102821 → 0.95.
    assert!(approx(f_cdf(4.102821015, 2.0, 10.0), 0.95, 1e-6));
}

#[test]
fn test_f_quantile() {
    // f_quantile(0.95, 2, 10) ≈ 4.102821.
    assert!(approx(f_quantile(0.95, 2.0, 10.0), 4.102821015, 1e-4));
    // df1=5, df2=20, 0.95 → 2.71089.
    assert!(approx(f_quantile(0.95, 5.0, 20.0), 2.71089, 1e-3));
    // Round-trip.
    assert!(approx(
        f_cdf(f_quantile(0.4, 3.0, 12.0), 3.0, 12.0),
        0.4,
        1e-7
    ));
}

#[test]
fn test_f_edge() {
    assert_eq!(f_quantile(0.0, 2.0, 5.0), 0.0);
    assert!(f_quantile(1.0, 2.0, 5.0).is_infinite());
    assert!(f_cdf(1.0, -1.0, 5.0).is_nan());
}

#[test]
fn test_t_quantile() {
    // Classic table values.
    assert!(approx(t_quantile(0.975, 10.0), 2.228138852, 1e-6));
    assert!(approx(t_quantile(0.95, 5.0), 2.015048373, 1e-6));
    // Symmetry: q(1-p) == -q(p).
    assert!(approx(t_quantile(0.025, 10.0), -2.228138852, 1e-6));
    assert_eq!(t_quantile(0.5, 7.0), 0.0);
    // Round-trip against the CDF.
    assert!(approx(
        student_t_cdf(t_quantile(0.8, 12.0), 12.0),
        0.8,
        1e-7
    ));
    assert!(approx(student_t_cdf(t_quantile(0.3, 4.0), 4.0), 0.3, 1e-7));
    // Large df → standard normal quantile.
    assert!(approx(t_quantile(0.975, 1.0e6), 1.959963985, 1e-4));
}

#[test]
fn test_t_edge() {
    assert!(t_quantile(0.0, 5.0).is_infinite() && t_quantile(0.0, 5.0) < 0.0);
    assert!(t_quantile(1.0, 5.0).is_infinite() && t_quantile(1.0, 5.0) > 0.0);
    assert!(t_quantile(0.9, -1.0).is_nan());
}

// ───────────────────────── gamma ─────────────────────────

#[test]
fn test_gamma_cdf() {
    assert_eq!(gamma_cdf(0.0, 2.0, 1.0), 0.0);
    // Gamma(1, scale) = Exponential(1/scale): CDF = 1 - exp(-x/scale).
    assert!(approx(
        gamma_cdf(2.0, 1.0, 1.0),
        1.0 - (-2.0f64).exp(),
        1e-10
    ));
    // Gamma(2,1) at x=2: 1 - exp(-2)(1+2) = 1 - 3e^-2.
    assert!(approx(
        gamma_cdf(2.0, 2.0, 1.0),
        1.0 - 3.0 * (-2.0f64).exp(),
        1e-9
    ));
}

#[test]
fn test_gamma_quantile() {
    // Round-trip.
    assert!(approx(
        gamma_cdf(gamma_quantile(0.5, 2.0, 1.5), 2.0, 1.5),
        0.5,
        1e-9
    ));
    assert!(approx(
        gamma_cdf(gamma_quantile(0.9, 3.0, 2.0), 3.0, 2.0),
        0.9,
        1e-9
    ));
    // Exponential(scale=2): quantile(p) = -2 ln(1-p).
    assert!(approx(
        gamma_quantile(0.5, 1.0, 2.0),
        -2.0 * 0.5f64.ln(),
        1e-7
    ));
}

#[test]
fn test_gamma_edge() {
    assert_eq!(gamma_quantile(0.0, 2.0, 1.0), 0.0);
    assert!(gamma_quantile(1.0, 2.0, 1.0).is_infinite());
    assert!(gamma_cdf(1.0, -1.0, 1.0).is_nan());
}

// ───────────────────────── beta ─────────────────────────

#[test]
fn test_beta_cdf() {
    assert_eq!(beta_cdf(0.0, 2.0, 2.0), 0.0);
    assert_eq!(beta_cdf(1.0, 2.0, 2.0), 1.0);
    // Beta(2,2) symmetric: CDF(0.5)=0.5.
    assert!(approx(beta_cdf(0.5, 2.0, 2.0), 0.5, 1e-10));
    // Beta(1,1) uniform: CDF(x)=x.
    assert!(approx(beta_cdf(0.3, 1.0, 1.0), 0.3, 1e-10));
}

#[test]
fn test_beta_quantile() {
    // Beta(2,2) median = 0.5.
    assert!(approx(beta_quantile(0.5, 2.0, 2.0), 0.5, 1e-8));
    // Uniform: quantile(p)=p.
    assert!(approx(beta_quantile(0.42, 1.0, 1.0), 0.42, 1e-9));
    // Round-trip.
    assert!(approx(
        beta_cdf(beta_quantile(0.7, 3.0, 5.0), 3.0, 5.0),
        0.7,
        1e-8
    ));
}

#[test]
fn test_beta_edge() {
    assert_eq!(beta_quantile(0.0, 2.0, 2.0), 0.0);
    assert_eq!(beta_quantile(1.0, 2.0, 2.0), 1.0);
    assert!(beta_cdf(0.5, -1.0, 2.0).is_nan());
    // Quantile stays within [0,1].
    let q = beta_quantile(0.99, 2.0, 8.0);
    assert!((0.0..=1.0).contains(&q));
}

// ─────────────────────── digamma / trigamma (M37.3) ───────────────────────

const EULER_GAMMA: f64 = 0.577_215_664_901_532_9;

// NOTE: `digamma` is the byte-identical legacy DATA-step algorithm (truncated
// asymptotic series + single recursion step), accurate only to ~few·1e-4.
// Tolerances below reflect that, NOT a more precise expansion.
#[test]
fn test_digamma_at_one() {
    // ψ(1) = −γ.
    assert!(
        approx(digamma(1.0), -EULER_GAMMA, 1e-3),
        "digamma(1)={}",
        digamma(1.0)
    );
}

#[test]
fn test_digamma_recurrence() {
    // ψ(x+1) − ψ(x) = 1/x (exact in theory; the truncated series introduces
    // a small numeric residual).
    for &x in &[2.5_f64, 7.0, 0.75, 13.2] {
        let lhs = digamma(x + 1.0) - digamma(x);
        assert!(approx(lhs, 1.0 / x, 1e-3), "x={x}: {lhs} vs {}", 1.0 / x);
    }
}

// `trigamma` is the new high-accuracy implementation (~1e-9 or better).
#[test]
fn test_trigamma_at_one() {
    // ψ′(1) = π²/6.
    let pi2_6 = std::f64::consts::PI * std::f64::consts::PI / 6.0;
    assert!(
        approx(trigamma(1.0), pi2_6, 1e-8),
        "trigamma(1)={}",
        trigamma(1.0)
    );
}

#[test]
fn test_trigamma_recurrence() {
    // ψ′(x) − ψ′(x+1) = 1/x².
    for &x in &[2.5_f64, 7.0, 0.75, 13.2, 0.3] {
        let lhs = trigamma(x) - trigamma(x + 1.0);
        assert!(
            approx(lhs, 1.0 / (x * x), 1e-8),
            "x={x}: {lhs} vs {}",
            1.0 / (x * x)
        );
    }
}

#[test]
fn test_trigamma_known_values() {
    // ψ′(2) = π²/6 − 1; ψ′(0.5) = π²/2.
    let pi = std::f64::consts::PI;
    assert!(
        approx(trigamma(2.0), pi * pi / 6.0 - 1.0, 1e-8),
        "trigamma(2)={}",
        trigamma(2.0)
    );
    assert!(
        approx(trigamma(0.5), pi * pi / 2.0, 1e-8),
        "trigamma(0.5)={}",
        trigamma(0.5)
    );
}
