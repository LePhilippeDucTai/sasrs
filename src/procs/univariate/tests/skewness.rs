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
