//! Module de calcul statistique maison (déterministe, snap-stable).
//!
//! Ce module centralise la numérique statistique pour SAS : distributions
//! (normale/t/F/χ²/gamma/bêta), quantiles, fonctions de répartition (CDF),
//! et algèbre linéaire (Cholesky, QR, moindres carrés). Tout est implémenté
//! en Rust pur pour le déterminisme et la stabilité des snapshots.
//!
//! ## Architecture
//!
//! - **`dists.rs`** : distributions de probabilité et leurs CDFs/quantiles.
//!   Promotion des helpers existants de `src/procs/common.rs` (qui reste
//!   inchangé) : `ln_gamma`, `betai`, `student_t_cdf`, `gammq`, `erf`.
//!   Ajout : CDF et quantile pour F, χ², gamma, bêta. Tests unitaires
//!   contre valeurs SAS documentées (Handbook, J.H.Maindonald & W.J.Braun).
//!
//! - **`linalg.rs`** : algèbre linéaire maison.
//!   - Décomposition Cholesky (matrice sym. def. pos., `L @ L.T() = A`)
//!   - Décomposition QR via Householder (base orthonormale)
//!   - Moindres carrés : résoudre `(X'X)b = X'y` via Cholesky ou QR
//!   - Inversion matricielle (via QR ou Cholesky)
//!   - Valeurs/vecteurs propres symétriques via Jacobi (rotations planes)
//!   Tous les algorithmes sont testés contre matrices de référence.
//!
//! ## Intégration
//!
//! M24.2 (PROC TTEST) et M24.3 (PROC NPAR1WAY) réduisent en dépendances :
//! pour t-tests : CDF/quantile t (déjà public en common.rs, promu ici).
//! Pour tests non-paramétriques : pas de linalg, peu de dist. avancées.
//!
//! M25 (REG/ANOVA/GLM) : déterminant à partir de QR, moindres carrés,
//! inversion pour tests F (type I/III SS). M26+ : Jacobi pour corrélations
//! partielles/vecteurs propres (PCA).

pub mod dists;
pub mod linalg;
pub mod optim;

// Re-export fréquent vers l'extérieur
pub use dists::{
    beta_cdf,
    beta_quantile,
    betai,
    // Ajout M24.1 : F, χ², gamma, bêta CDFs et quantiles
    chisq_cdf,
    chisq_quantile,
    // Ajout M37.3 : digamma ψ et trigamma ψ′
    digamma,
    erf,
    f_cdf,
    f_quantile,
    gamma_cdf,
    gamma_quantile,
    gammq,
    ln_choose,
    ln_factorial,
    ln_gamma,
    phi_inv,
    probnorm,
    student_t_cdf,
    t_quantile,
    trigamma,
};

pub use linalg::{
    cholesky, eigenvalues_jacobi, eigenvectors_jacobi, invert_matrix, least_squares,
    qr_decomposition,
};
