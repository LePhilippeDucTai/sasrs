//! PROC PRINCOMP — principal component analysis (M27).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc princomp data=<ref> [cov] [n=<k>] [out=<ref>];
//!  var <name list>; run;`
//!
//! ## Périmètre
//! - Options du statement PROC : `data=`, `cov` (matrice de covariance au lieu
//!   de corrélation), `n=` (nombre de composantes à afficher), `out=`
//!   (parse-accepté ; les scores ne sont pas calculés en v1).
//! - `var` : variables numériques analysées (obligatoire, >= 2).
//! - Différé : `partial`, `weight`, `outstat=`, entrée TYPE=CORR, ODS plots.
//!
//! ## Sortie listing (titre "The PRINCOMP Procedure"), dans l'ordre SAS :
//! 1. Observations / Variables (n et p).
//! 2. Simple Statistics : Mean et StdDev par variable (échantillon, n-1).
//! 3. Correlation Matrix (ou Covariance Matrix si COV).
//! 4. Eigenvalues of the Correlation/Covariance Matrix : Eigenvalue,
//!    Difference, Proportion, Cumulative.
//! 5. Eigenvectors : matrice p×(k) des vecteurs propres.
//!
//! ## Conventions
//! - Observations : complete-case sur l'ENSEMBLE des variables `var` (une ligne
//!   est exclue si l'une quelconque des variables est missing).
//! - Écart-type / (co)variance : dénominateur n-1.
//! - Matrice de corrélation : diagonale forcée à 1.0, symétrisation exacte
//!   avant Jacobi (évite l'asymétrie de l'arrondi et un affichage 0.9999999).
//! - Convention de signe sur chaque vecteur propre : si l'élément de valeur
//!   absolue maximale (premier indice en cas d'égalité) est négatif, on inverse
//!   la colonne entière. Rend le snapshot stable.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::procs::common::{apply_sign_convention, complete_case_rows};
use crate::session::Session;
use crate::stat::eigenvectors_jacobi;
use crate::token::TokenKind;
use crate::value::VarType;

mod analysis;
mod report;

use analysis::*;
use report::*;

// ───────────────────────── AST ─────────────────────────

pub struct PrincompAst {
    pub data: Option<DatasetRef>,
    /// Use the covariance matrix instead of the (default) correlation matrix.
    pub cov: bool,
    /// Number of components to display (None = all p).
    pub n: Option<usize>,
    /// OUT= dataset (parse-accepted; scores not produced in v1).
    pub out: Option<DatasetRef>,
    /// VAR list (analysis variables, user order preserved).
    pub var: Vec<String>,
}

// ───────────────────────── Parser ─────────────────────────

/// Parse `proc princomp [data=a] [cov] [n=k] [out=b]; [var ...;] run;`.
/// Called AFTER "proc princomp" has been consumed. Consumes through run;/quit;.
pub fn parse(ts: &mut StatementStream) -> Result<PrincompAst> {
    let mut data: Option<DatasetRef> = None;
    let mut cov = false;
    let mut n: Option<usize> = None;
    let mut out: Option<DatasetRef> = None;

    // --- PROC PRINCOMP statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            data = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if ts.peek().is_kw("cov") || ts.peek().is_kw("covariance") {
            ts.next();
            cov = true;
        } else if ts.peek().is_kw("n") {
            common::consume_option_eq(ts, "N")?;
            let span = ts.peek().span;
            let k = match ts.peek().kind {
                TokenKind::Num(v) => v,
                _ => return Err(SasError::parse("expected a number after N=", span)),
            };
            ts.next();
            n = Some(k as usize);
        } else if ts.peek().is_kw("out") {
            out = Some(common::parse_out_opt(ts)?);
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC PRINCOMP statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC PRINCOMP statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; (combinateur partagé M31) ---
    let mut var: Vec<String> = Vec::new();
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next();
                var = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            _ => false,
        })
    })?;

    Ok(PrincompAst {
        data,
        cov,
        n,
        out,
        var,
    })
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &PrincompAst, session: &mut Session) -> Result<()> {
    // At least 2 variables required.
    if ast.var.len() < 2 {
        return Err(SasError::runtime(
            "PROC PRINCOMP requires at least 2 variables.",
        ));
    }

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

    // Number of components to display.
    let k = ast.n.map(|k| k.min(p)).unwrap_or(p).max(1).min(p);

    // ───────────────────────── listing ─────────────────────────
    session.listing.page_header();
    centered(session, "The PRINCOMP Procedure");
    session.listing.blank();

    session
        .listing
        .write_line(&format!(" Observations    {:>10}", n));
    session
        .listing
        .write_line(&format!(" Variables       {:>10}", p));
    session.listing.blank();

    // Simple Statistics: rows Mean / StdDev, columns = variables.
    print_simple_statistics(session, &names, &means, &stds);

    // Correlation / Covariance Matrix.
    print_analysis_matrix(session, ast.cov, &names, &amat);

    // Eigenvalues table.
    print_eigenvalue_table(session, ast.cov, &lambda, trace, p, k);

    // Eigenvectors table (6 decimals).
    print_eigenvectors(session, &names, &v, k);

    // OUT= : write input columns + Prin1..Prink component scores.
    //
    // Scoring method: for each complete-case observation, the score on
    // component j is the (standardized — or only centered, if COV) data vector
    // dotted with eigenvector column j (the SAME eigenvectors, with the SAME
    // sign convention, used for the Eigenvectors listing above). For
    // correlation-based PCA each variable is standardized by its sample mean
    // and std; for COV the variable is only centered by its mean. With this
    // convention the score on component j has sample variance equal to
    // eigenvalue_j. Observations with any missing analysis variable receive
    // missing scores (rows are kept in input order, mirroring SAS).
    if let Some(out_ref) = &ast.out {
        write_out_dataset(
            session, &ds, &decoded, &means, &stds, &v, ast.cov, p, k, out_ref,
        )?;
    }

    Ok(())
}

use crate::procs::common::centered;

#[cfg(test)]
mod tests;
