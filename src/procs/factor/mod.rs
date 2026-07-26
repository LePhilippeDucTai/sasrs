//! PROC FACTOR — principal axis / principal component factor analysis (M27).
//!
//! `proc factor data=<ref> [nfactors=k] [method=principal] [rotate=varimax|none]
//!              [cov] [out=<ref>];
//!  var <name list>; run;`
//!
//! ## Périmètre
//! - `data=`, `cov`, `nfactors=`, `rotate=varimax|none|promax`, `out=`.
//! - Critère de rétention : Kaiser (λ>1) ou NFACTORS=k explicite.
//! - Rotation VARIMAX (Kaiser 1958) : orthogonale, normalisation Kaiser.
//! - Rotation PROMAX : oblique, partant de la solution VARIMAX (cible élevée à
//!   la puissance k=4, ajustement de Procrustes). Produit le « Rotated Factor
//!   Pattern » oblique et les « Inter-Factor Correlations ».
//! - OUT= : colonnes d'entrée + `Factor1..Factorm` (scores par régression).
//! - Différé : METHOD=ML/ITER, HEYWOOD, ALPHA, ROTATE=OBLIMIN.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::session::Session;
use crate::stat::{eigenvectors_jacobi, invert_matrix};
use crate::token::TokenKind;
use crate::value::VarType;

mod analysis;
mod output;
mod parse;
mod report;
mod rotate;

pub use parse::parse;
pub use rotate::PromaxResult;
pub use rotate::promax;
pub use rotate::varimax;

use analysis::*;
use output::*;
use parse::*;
use report::*;
use rotate::*;

// ───────────────────────── AST ─────────────────────────

pub struct FactorAst {
    pub data: Option<DatasetRef>,
    /// Use covariance matrix instead of correlation matrix.
    pub cov: bool,
    /// Number of factors to retain (None = Kaiser criterion λ>1).
    pub nfactors: Option<usize>,
    /// Factor extraction method (only "principal" supported).
    pub method: String,
    /// Rotation method: "none", "varimax", or "promax".
    pub rotate: String,
    /// OUT= dataset (factor scores).
    pub out: Option<DatasetRef>,
    /// VAR list (analysis variables, user order preserved).
    pub var: Vec<String>,
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &FactorAst, session: &mut Session) -> Result<()> {
    validate_options(ast)?;

    let (ds, display) = common::open_input_display(&ast.data, session)?;
    let n_read = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_read, display
    ));

    let cols = resolve_var_columns(&ds, ast, &display)?;
    let p = cols.len();
    let names: Vec<String> = cols.iter().map(|&c| ds.vars[c].name.clone()).collect();

    // Decode each analysis column once.
    let decoded: Vec<Vec<f64>> = cols
        .iter()
        .map(|&c| {
            decode_column(&ds, c).map(|vals| {
                vals.iter()
                    .map(|v| value_to_num(v).unwrap_or(f64::NAN))
                    .collect::<Vec<f64>>()
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let data_rows = complete_case_rows(&decoded, n_read);
    let n = data_rows.len();
    if n == 0 {
        return Err(SasError::runtime("No observations with complete data."));
    }

    let (means, stds, amat) = compute_analysis_matrix(&data_rows, p, ast.cov);

    // Eigen-decomposition: V columns = eigenvectors, lambda descending.
    let (mut v, lambda) = eigenvectors_jacobi(&amat)?;
    apply_sign_convention(&mut v, p);

    let trace: f64 = lambda.iter().sum();

    // Determine number of factors to retain.
    let k = if let Some(nf_req) = ast.nfactors {
        nf_req.max(1).min(p)
    } else {
        // Kaiser criterion: λ > 1.0
        let kaiser = lambda.iter().filter(|&&lam| lam > 1.0).count();
        kaiser.max(1)
    };

    let retention_msg = if ast.nfactors.is_some() {
        format!(
            "{} factor{} will be retained by the NFACTORS criterion.",
            k,
            if k == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "{} factor{} will be retained by the MINEIGEN criterion.",
            k,
            if k == 1 { "" } else { "s" }
        )
    };

    // Compute loadings: L[i][j] = V[i][j] * sqrt(λ[j])  for j in 0..k
    let loadings: Vec<Vec<f64>> = (0..p)
        .map(|i| {
            (0..k)
                .map(|j| v[i][j] * lambda[j].max(0.0).sqrt())
                .collect()
        })
        .collect();

    // Initial communalities: h²[i] = Σⱼ L[i][j]²
    let communalities: Vec<f64> = loadings
        .iter()
        .map(|row| row.iter().map(|&x| x * x).sum())
        .collect();

    // Variance explained by each factor (sum of squares of each column).
    let factor_variance: Vec<f64> = (0..k)
        .map(|j| loadings.iter().map(|row| row[j] * row[j]).sum::<f64>())
        .collect();

    // ───────────────────────── listing ─────────────────────────
    session.listing.page_header();
    centered(session, "The FACTOR Procedure");
    session.listing.blank();

    centered(session, "Initial Factor Method: Principal Components");
    session.listing.blank();

    centered(session, "Prior Communality Estimates: ONE");
    session.listing.blank();

    print_eigenvalue_table(session, &lambda, trace, p, ast.cov);

    // Retention criterion message.
    centered(session, &retention_msg);
    session.listing.blank();

    // Factor Pattern (initial loadings).
    print_factor_pattern(session, "Factor Pattern", &names, &loadings, k);

    // Variance Explained by Each Factor.
    print_variance_explained(session, &factor_variance, k);

    // Final Communality Estimates (before rotation).
    print_final_communalities(session, &names, &communalities);

    // Pattern used for OUT= factor scoring (rotated when a rotation applies).
    let mut final_pattern: Vec<Vec<f64>> = loadings.clone();

    // ───── VARIMAX rotation (if requested and k >= 2) ─────
    if ast.rotate == "varimax" && k >= 2 {
        final_pattern = print_varimax_section(session, &names, &loadings, k);
    }

    // ───── PROMAX oblique rotation (if requested and k >= 2) ─────
    if ast.rotate == "promax" && k >= 2 {
        final_pattern = print_promax_section(session, &names, &loadings, k)?;
    }

    // OUT= : write input columns + Factor1..Factorm regression factor scores.
    //
    // Scoring method (standard SAS regression scoring): with Z the matrix of
    // standardized analysis variables, R the correlation matrix and `pattern`
    // the (possibly rotated) factor pattern, the standardized scoring
    // coefficients are B = R⁻¹ · pattern (n_vars × k) and the factor scores are
    // F = Z · B. For COV analysis the variables are only centered. Observations
    // with any missing analysis variable receive missing scores.
    if let Some(out_ref) = &ast.out {
        write_out_dataset(
            session,
            &ds,
            &decoded,
            &means,
            &stds,
            &amat,
            &final_pattern,
            ast.cov,
            p,
            k,
            out_ref,
        )?;
    }

    Ok(())
}

use crate::procs::common::centered;

#[cfg(test)]
mod tests;
