//! Fonctions spéciales LOCALES à PROC CORR.
//!
//! MQ8.9 — `betai`/`betacf`/`ln_gamma` ont été retirées d'ici : elles étaient
//! byte-identiques à celles de `stat::dists::special`, contrairement à ce
//! qu'affirmait la note de MQ1.2. Ce qui reste est réellement différent :
//! `erfc` est l'approximation rationnelle de Numerical Recipes (|erreur| <
//! 1.2e-7), là où `stat::dists::erf` passe par la gamma incomplète — les
//! digits imprimés par CORR dépendent de ce choix, ne pas replier.

use crate::stat::dists::betai;

/// Upper-tail standard-normal survival function 1 − Φ(z) for z >= 0, via the
/// complementary error function relation Φ(z) = ½ erfc(−z/√2). Accuracy ~1e-7,
/// ample for a documented normal approximation.
pub(super) fn normal_sf(z: f64) -> f64 {
    0.5 * erfc(z / std::f64::consts::SQRT_2)
}

/// erfc(x) — Numerical Recipes rational (Chebyshev) approximation, |error| < 1.2e-7.
pub(super) fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let ans = t
        * (-z * z - 1.26551223
            + t * (1.00002368
                + t * (0.37409196
                    + t * (0.09678418
                        + t * (-0.18628806
                            + t * (0.27886807
                                + t * (-1.13520398
                                    + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277)))))))))
            .exp();
    if x >= 0.0 { ans } else { 2.0 - ans }
}

/// Two-sided survival function of Student's t: P(|T_df| > t) for t >= 0.
/// Uses the identity P(|T| > t) = I_{df/(df+t^2)}(df/2, 1/2), where I is the
/// regularized incomplete beta function. Accurate to ~1e-10 over the usual
/// range of t and df encountered here.
pub(super) fn student_t_sf_two_sided(t: f64, df: f64) -> f64 {
    if t <= 0.0 {
        return 1.0;
    }
    let x = df / (df + t * t);
    betai(df / 2.0, 0.5, x)
}
