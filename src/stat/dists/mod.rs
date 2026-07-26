//! Distributions de probabilité et fonctions statistiques maison.
//!
//! ## Plan d'implémentation (M24.1)
//!
//! **Promotion depuis `src/procs/common.rs`** (verbatim, zéro changement logique) :
//! - `ln_gamma(x)` : Lanczos approximation Γ(x), x > 0, accuracy ~1e-13.
//! - `betai(a, b, x)` : regularized incomplete beta I_x(a,b), for t-CDF, F-CDF, etc.
//! - `betacf(a, b, x)` : continued fraction for betai, accuracy tuned.
//! - `student_t_cdf(t, df)` : CDF of Student-t, t∈ℝ, df>0 via betai.
//! - `gammq(a, x)` : upper incomplete gamma Q(a,x) = 1 - P(a,x), via gser/gcf.
//! - `gser(a, x)` : ascending series P(a,x).
//! - `gcf(a, x)` : continued fraction for P(a,x).
//! - `erf(x)` : error function, used in normal CDF.
//! - `probnorm(z)` : CDF of standard normal N(0,1), z∈ℝ, via erf.
//! - `phi_inv(p)` : inverse CDF (quantile) of standard normal, p∈(0,1),
//!   Acklam approximation + 1 Halley step. Validated: phi_inv(0.975)≈1.9599640.
//! - `ln_factorial(n)` : ln(n!) via ln_gamma, for binomial coefficients.
//! - `ln_choose(n, k)` : ln(C(n,k)) = ln(n!) - ln(k!) - ln((n-k)!),
//!   no overflow on large n (stored as ln).
//!
//! **Ajout M24.1 : distributions manquantes**
//!
//! Toutes sont testées contre valeurs SAS documentées ou J.H.Maindonald et al.
//!
//! ### Chi-squared distribution
//! - `chisq_cdf(x: f64, df: f64) -> f64`
//!   CDF of χ²(df). Implemented as:
//!   ```text
//!   P(χ²(df) ≤ x) = gammq(df/2, x/2)  if x > 0 else 0.0
//!   ```
//!   Reuses existing `gammq`. Validation (SAS reference):
//!   - chisq_cdf(5.0, 2.0) ≈ 0.91795 (df=2, x=5)
//!   - chisq_cdf(3.841, 1.0) ≈ 0.95000 (critical value)
//!
//! - `chisq_quantile(p: f64, df: f64) -> f64`
//!   Inverse CDF. Implemented via Newton-Raphson on `chisq_cdf`:
//!   Initial guess: `(df + 2*p*df)^(1/3) * df` (Wilson-Hilferty approx)
//!   Loop: x_{n+1} = x_n - (chisq_cdf(x_n) - p) / chisq_pdf(x_n)
//!   where `chisq_pdf(x) = (x^(df/2-1) * exp(-x/2)) / (2^(df/2) * Γ(df/2))`
//!   Converge to ~1e-12 relative error, max ~20 iterations.
//!   Validation: chisq_quantile(0.95, 1) ≈ 3.841459.
//!
//! ### F distribution (ratio of scaled chi-squared)
//! - `f_cdf(x: f64, df1: f64, df2: f64) -> f64`
//!   CDF of F(df1, df2). Relationship:
//!   ```text
//!   P(F(d1,d2) ≤ x) = betai(d1/2, d2/2, d1*x / (d1*x + d2))
//!   ```
//!   Validation (SAS, df1=2, df2=10):
//!   - f_cdf(1.0, 2, 10) ≈ 0.40155
//!   - f_cdf(4.103, 2, 10) ≈ 0.95000 (critical value)
//!
//! - `f_quantile(p: f64, df1: f64, df2: f64) -> f64`
//!   Inverse CDF. Implemented via Newton-Raphson on f_cdf:
//!   Initial guess: `df1 / (df2 - 2.0)` (crude but reasonable)
//!   Loop: x_{n+1} = x_n - (f_cdf(x_n) - p) / f_pdf(x_n)
//!   where f_pdf requires `betai` derivative (numerical or exact).
//!   For stability: use Acklam-like approximation with refinement.
//!   Converge to ~1e-12 relative error.
//!   Validation: f_quantile(0.95, 2, 10) ≈ 4.10281.
//!
//! ### Gamma distribution
//! - `gamma_cdf(x: f64, shape: f64, scale: f64) -> f64`
//!   CDF of Gamma(α, β) parameterized as pdf ∝ x^(α-1) exp(-x/β).
//!   Implemented as: P(X ≤ x) = P(α, x/β) = gammq(α, x/β)
//!   (Note: SAS uses shape α, scale β; some sources use rate γ=1/β).
//!   Validation: Gamma(2, 1) CDF at x=2 should match exponential sum.
//!
//! - `gamma_quantile(p: f64, shape: f64, scale: f64) -> f64`
//!   Inverse CDF via Newton-Raphson on gamma_cdf.
//!   Initial guess via normal approx or Cornish-Fisher transform.
//!
//! ### Beta distribution
//! - `beta_cdf(x: f64, alpha: f64, beta: f64) -> f64`
//!   CDF of Beta(α, β) on [0, 1]. Directly:
//!   ```text
//!   P(X ≤ x) = betai(α, β, x)
//!   ```
//!   Validation: Beta(2, 2) mode at 0.5, CDF(0.5) = 0.5.
//!
//! - `beta_quantile(p: f64, alpha: f64, beta: f64) -> f64`
//!   Inverse CDF. No closed form; use Newton-Raphson on beta_cdf.
//!   Initial guess: simple bisection or approximation.
//!   Converge to ~1e-12 relative error on [0, 1].
//!
//! ## Tests
//!
//! All functions include unit tests validating against:
//! - Edge cases (x=0, p=0.5, p→0/→1)
//! - SAS PROBT/PROBF/PROBCHI/PROBGAM/PROBBETA reference values
//! - Monotonicity of CDF, identity of CDF∘quantile.
//! - Numerical stability (no NaN/Inf on valid inputs).
//!
//! Special case handling:
//! - p < 0 or p > 1 → ERROR (or clamp to [0,1])
//! - df ≤ 0 → ERROR
//! - x < 0 (for χ², gamma, beta) → 0.0
//!
//! ## Performance considerations
//!
//! All algorithms avoid iterative loops where possible (precomputed ln_gamma,
//! betai via continued fraction). Newton-Raphson limited to ~20 iterations
//! with exit on convergence; no infinite loops or pathological cases.

mod chisq_f;
mod gamma_beta;
mod normal_t;
mod special;

pub use chisq_f::chisq_cdf;
pub use chisq_f::chisq_quantile;
pub use chisq_f::f_cdf;
pub use chisq_f::f_quantile;
pub use gamma_beta::beta_cdf;
pub use gamma_beta::beta_quantile;
pub use gamma_beta::gamma_cdf;
pub use gamma_beta::gamma_quantile;
pub use normal_t::phi_inv;
pub use normal_t::probnorm;
pub use normal_t::student_t_cdf;
pub use normal_t::t_quantile;
pub use special::betai;
pub use special::digamma;
pub use special::erf;
pub use special::gammq;
pub use special::ln_choose;
pub use special::ln_factorial;
pub use special::ln_gamma;
pub use special::trigamma;

use special::*;

#[cfg(test)]
mod tests;
