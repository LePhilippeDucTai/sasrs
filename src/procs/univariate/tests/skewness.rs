use super::super::*;
use super::*;

// ───────────────────────── skewness / kurtosis tests ───────────────────

#[test]
fn skewness_symmetric_is_zero() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let g1 = skewness(&xs).unwrap();
    assert!(g1.abs() < 1e-12, "g1 = {g1}");
}

#[test]
fn skewness_known_skewed_sample() {
    // [1,2,3,4,10] : computed with the SAS formula.
    // mean=4, s=sqrt(Σ(x-mean)^2/4)=sqrt((9+4+1+0+36)/4)=sqrt(12.5)
    // g1 = 5/((4)(3)) * Σ z^3, z=(x-4)/s.
    let xs = [1.0, 2.0, 3.0, 4.0, 10.0];
    let g1 = skewness(&xs).unwrap();
    // Reference value (SAS g1 formula): ~1.6970563
    assert!((g1 - 1.6970563).abs() < 1e-4, "g1 = {g1}");
}

#[test]
fn skewness_needs_n_ge_3() {
    assert!(skewness(&[1.0, 2.0]).is_none());
    assert!(skewness(&[1.0]).is_none());
}

#[test]
fn kurtosis_needs_n_ge_4() {
    assert!(kurtosis(&[1.0, 2.0, 3.0]).is_none());
    assert!(kurtosis(&[1.0, 2.0]).is_none());
}

#[test]
fn kurtosis_known_sample() {
    // [1,2,3,4,5] excess kurtosis (SAS) reference ~ -1.2
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let g2 = kurtosis(&xs).unwrap();
    assert!((g2 - (-1.2)).abs() < 1e-6, "g2 = {g2}");
}

// ───────────── M45.1 : skewness / kurtosis PONDÉRÉS (VARDEF=DF) ─────────

/// Weighted mean and weighted std (VARDEF=DF) of `(value, weight)` pairs — the
/// same two quantities `emit_variable_weighted` feeds to the weighted moments.
fn wmoments(pairs: &[(f64, f64)]) -> (f64, f64) {
    let sum_w: f64 = pairs.iter().map(|(_, w)| *w).sum();
    let mean = pairs.iter().map(|(x, w)| w * x).sum::<f64>() / sum_w;
    let css: f64 = pairs.iter().map(|(x, w)| w * (x - mean) * (x - mean)).sum();
    (mean, (css / (pairs.len() as f64 - 1.0)).sqrt())
}

#[test]
fn weighted_moments_reduce_to_unweighted_at_unit_weights() {
    // Oracle de non-régression demandé par le jalon : à w ≡ 1 les formules
    // pondérées doivent redonner EXACTEMENT les g1/g2 non pondérés. Les deux
    // chemins de calcul sont indépendants (moyenne/écart-type recalculés ici).
    for xs in [
        vec![1.0, 2.0, 3.0, 4.0, 5.0],
        vec![1.0, 2.0, 3.0, 4.0, 10.0],
        vec![-3.0, 0.5, 0.5, 7.25, 7.25, 11.0],
    ] {
        let pairs: Vec<(f64, f64)> = xs.iter().map(|&x| (x, 1.0)).collect();
        let (m, s) = wmoments(&pairs);
        let g1 = weighted_skewness(&pairs, m, s).unwrap();
        let g2 = weighted_kurtosis(&pairs, m, s).unwrap();
        assert!(
            (g1 - skewness(&xs).unwrap()).abs() < 1e-12,
            "g1 mismatch on {xs:?}: {g1}"
        );
        assert!(
            (g2 - kurtosis(&xs).unwrap()).abs() < 1e-12,
            "g2 mismatch on {xs:?}: {g2}"
        );
    }
}

#[test]
fn weighted_skewness_symmetric_is_zero() {
    // x=[1,2,3], w=[2,1,2] : distribution pondérée symétrique autour de 2.
    //   x̄_w = (2*1 + 1*2 + 2*3)/5 = 10/5 = 2
    //   Σ w^{3/2}(x-2)^3 = 2^1.5*(-1)^3 + 1*0 + 2^1.5*(1)^3 = -2√2 + 2√2 = 0
    // ⇒ g1 = 0 quelle que soit s_w.
    let pairs = [(1.0, 2.0), (2.0, 1.0), (3.0, 2.0)];
    let (m, s) = wmoments(&pairs);
    assert!((m - 2.0).abs() < 1e-12, "mean_w = {m}");
    let g1 = weighted_skewness(&pairs, m, s).unwrap();
    assert!(g1.abs() < 1e-12, "g1 = {g1}");
}

#[test]
fn weighted_moments_oracle_hand_computed() {
    // Jeu de la fixture m33 : x=[1,2,3,4], w=[1,2,3,4]. Tout est calculable
    // à la main.
    //   Σw = 10 ; Σwx = 1+4+9+16 = 30 ⇒ x̄_w = 3
    //   CSS_w = 1*4 + 2*1 + 3*0 + 4*1 = 10 ⇒ s_w² = 10/3 (VARDEF=DF, n-1 = 3)
    //   Σ w^{3/2}(x-3)^3 = 1*(-8) + 2√2*(-1) + 0 + 8*(1) = -2√2
    //     g1 = 4/((3)(2)) * (-2√2) / (10/3)^{3/2} = -0.30983867…
    //   Σ w²(x-3)^4   = 1*16 + 4*1 + 9*0 + 16*1 = 36 ; s_w^4 = 100/9
    //     g2 = [4*5/((3)(2)(1))] * 36/(100/9) - 3*(3)²/((2)(1))
    //        = (10/3)*3.24 - 13.5 = 10.8 - 13.5 = -2.7   (exact)
    let pairs = [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0), (4.0, 4.0)];
    let (m, s) = wmoments(&pairs);
    assert!((m - 3.0).abs() < 1e-12, "mean_w = {m}");
    assert!((s * s - 10.0 / 3.0).abs() < 1e-12, "var_w = {}", s * s);

    let g1 = weighted_skewness(&pairs, m, s).unwrap();
    let expect_g1 = (4.0 / 6.0) * (-2.0 * 2.0_f64.sqrt()) / (10.0_f64 / 3.0).powf(1.5);
    assert!((g1 - expect_g1).abs() < 1e-12, "g1 = {g1}");
    assert!((g1 - (-0.309_838_67)).abs() < 1e-7, "g1 = {g1}");

    let g2 = weighted_kurtosis(&pairs, m, s).unwrap();
    assert!((g2 - (-2.7)).abs() < 1e-12, "g2 = {g2}");
}

#[test]
fn weighted_moments_guards() {
    // n < 3 / n < 4 → None, comme les versions non pondérées.
    let two = [(1.0, 1.0), (2.0, 3.0)];
    assert!(weighted_skewness(&two, 1.75, 0.7).is_none());
    let three = [(1.0, 1.0), (2.0, 2.0), (5.0, 1.0)];
    assert!(weighted_skewness(&three, 2.5, 1.7).is_some());
    assert!(weighted_kurtosis(&three, 2.5, 1.7).is_none());

    // s_w = 0 (toutes les valeurs égales) → None, sans NaN ni panic.
    let flat = [(4.0, 1.0), (4.0, 2.0), (4.0, 3.0), (4.0, 4.0)];
    let (m, s) = wmoments(&flat);
    assert_eq!(s, 0.0);
    assert!(weighted_skewness(&flat, m, s).is_none());
    assert!(weighted_kurtosis(&flat, m, s).is_none());
}

#[test]
fn mode_smallest_repeat_or_none() {
    let mut a = [1.0, 1.0, 2.0, 2.0, 3.0]; // 1 and 2 both twice -> smallest = 1
    a.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert_eq!(mode(&a), Some(1.0));

    let mut b = [1.0, 2.0, 3.0]; // all unique -> no mode
    b.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert_eq!(mode(&b), None);
}

#[test]
fn weighted_quantile_def5_oracle() {
    // Fixture data: x=[1,2,3,4], w=[1,2,3,4].
    //   Total weight W = 1+2+3+4 = 10.
    //   Cumulative weights W_i: 1, 3, 6, 10.
    // For p: target t = p*W; first i with W_i >= t; if W_i==t exactly →
    // average x(i),x(i+1), else x(i).
    let pairs = [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0), (4.0, 4.0)];
    // Q1 p=0.25: t=2.5 → W_2=3 is first ≥ → x(2)=2.
    assert_eq!(wq(&pairs, 0.25), 2.0);
    // Median p=0.5: t=5.0 → W_3=6 first ≥, 6≠5 → x(3)=3.
    assert_eq!(wq(&pairs, 0.50), 3.0);
    // Q3 p=0.75: t=7.5 → W_4=10 first ≥ → x(4)=4.
    assert_eq!(wq(&pairs, 0.75), 4.0);
    // p=0.10: t=1.0 == W_1 exactly → (x(1)+x(2))/2 = (1+2)/2 = 1.5.
    assert_eq!(wq(&pairs, 0.10), 1.5);
    // p=0.05: t=0.5 → W_1=1 first ≥ → x(1)=1.
    assert_eq!(wq(&pairs, 0.05), 1.0);
    // Edges.
    assert_eq!(wq(&pairs, 1.0), 4.0); // 100% Max
    assert_eq!(wq(&pairs, 0.0), 1.0); // 0% Min
}

#[test]
fn weighted_quantile_reduces_to_def5_when_unit_weights() {
    // Unit weights → must equal the unweighted Definition 5 results.
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let pairs: Vec<(f64, f64)> = xs.iter().map(|&x| (x, 1.0)).collect();
    for &p in &[0.0, 0.25, 0.5, 0.75, 1.0, 0.1, 0.9] {
        let mut sp = pairs.clone();
        sp.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut s = xs.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(
            weighted_quantile_def5(&sp, p),
            quantile_def5(&s, p),
            "mismatch at p={p}"
        );
    }
}
