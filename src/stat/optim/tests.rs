//! Tests des optimiseurs partagés.

use super::*;

/// MQ9.1 — un objectif qui rend NaN (vraisemblance divergente) faisait
/// paniquer le tri du simplexe (`partial_cmp(..).unwrap()`), donc tombait le
/// process au lieu de laisser la proc conclure à une non-convergence.
#[test]
fn nelder_mead_survives_a_nan_objective() {
    // Paraboloïde, mais NaN dès que x s'éloigne : le simplexe VA visiter la
    // zone NaN au premier pas.
    let eval = |x: &[f64]| -> f64 {
        if x[0] > 0.5 {
            f64::NAN
        } else {
            (x[0] - 0.25) * (x[0] - 0.25)
        }
    };
    let (best, fbest, _iters, _conv) = nelder_mead(&eval, &[0.0], 1.0, 200, 1e-10, 1e-10);
    assert!(!fbest.is_nan(), "le meilleur point ne doit pas être un NaN");
    assert!(best[0] <= 0.5, "un sommet NaN ne doit jamais gagner");
}

/// Le tri place le NaN en dernier, quel que soit l'ordre des arguments.
#[test]
fn nan_objective_sorts_last() {
    assert_eq!(cmp_objective(f64::NAN, 1.0), Ordering::Greater);
    assert_eq!(cmp_objective(1.0, f64::NAN), Ordering::Less);
    assert_eq!(cmp_objective(f64::NAN, f64::NAN), Ordering::Equal);
    assert_eq!(cmp_objective(1.0, 2.0), Ordering::Less);
}
