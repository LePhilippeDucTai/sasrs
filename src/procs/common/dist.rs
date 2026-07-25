use super::*;

/// Upper-tail (survival) probability of the chi-square distribution with `df`
/// degrees of freedom evaluated at `x`: P(X²_df > x) = Q(df/2, x/2). Returns
/// 1.0 at x <= 0 and ~0 for large x. Accuracy ~1e-10.
pub(crate) fn chisq_sf(x: f64, df: f64) -> f64 {
    if df <= 0.0 {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 1.0;
    }
    gammq(df / 2.0, x / 2.0)
}
