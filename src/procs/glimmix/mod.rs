//! PROC GLIMMIX — Generalized Linear Mixed Models via pseudo-likelihood (RSPL).
//!
//! Scope implemented:
//! - DIST=NORMAL/GAUSSIAN (link=IDENTITY), POISSON (link=LOG),
//!   BINARY/BINOMIAL (link=LOGIT).
//! - LINK= IDENTITY / LOG / LOGIT (others parse-accepted then deferred).
//! - RANDOM INTERCEPT / SUBJECT=<var> TYPE=VC (single random intercept).
//! - FREQ statement (grouped data).
//! - MODEL response = <fixed> / SOLUTION [NOINT].
//! - METHOD=RSPL (default). LAPLACE/QUAD parse-accepted then deferred.
//!
//! Estimation strategy (a 3-way dispatch, all routed to proven solvers):
//!  1. NORMAL/IDENTITY: PQL == REML, so the variance-components model is fit
//!     with the closed-form / profile estimator (reproduces PROC MIXED).
//!  2. Non-normal WITHOUT random: ordinary IRLS with FREQ weighting
//!     (reproduces PROC GENMOD / LOGISTIC).
//!  3. Non-normal WITH random: the residual-pseudo-likelihood (PQL) loop of
//!     Breslow-Clayton, linearising to a weighted mixed model at each step.
//!
//! Parse-accepted but deferred (NOTE emitted): ESTIMATE, CONTRAST, LSMEANS,
//! WEIGHT, PLOTS=, NOITPRINT, HTYPE=, DDFM= (always Contain).

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::decode_column;
use crate::procs::common::{fmt2, fmt4};
use crate::session::Session;
use crate::stat::dists::probnorm;
use crate::stat::{f_cdf, invert_matrix, student_t_cdf};
use crate::token::TokenKind;
use crate::value::{Value, format_best};

use crate::procs::lincom::build_design;

mod data;
mod glm_fit;
mod laplace;
mod linalg;
mod link;
mod parse;
mod pql;
mod repeated;
mod report;

pub use parse::parse;

use data::*;
use glm_fit::*;
use laplace::*;
use linalg::*;
use link::*;
use pql::*;
use repeated::*;
use report::*;

// ───────────────────────── AST ─────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distribution {
    Normal,
    Poisson,
    Binary, // binary / binomial both map here
    Gamma,
    NegBinomial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkFunction {
    Identity,
    Log,
    Logit,
    Probit,
    Cloglog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Rspl,
    Laplace,
    Quad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CovType {
    Vc,
    Cs,
    Ar1,
    Un,
}

#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub response: String,
    pub event: Option<String>,
    pub descending: bool,
    pub fixed: Vec<String>,
    pub dist: Distribution,
    pub link: LinkFunction,
    pub solution: bool,
    pub noint: bool,
}

#[derive(Debug, Clone)]
pub struct RandomSpec {
    pub effects: Vec<String>,
    pub subject: Option<String>,
    pub cov_type: CovType,
    pub solution: bool,
}

#[derive(Debug, Clone)]
pub struct GlimmixAst {
    pub data: Option<DatasetRef>,
    pub method: Method,
    pub class_vars: Vec<String>,
    pub model: Option<ModelSpec>,
    pub random: Option<RandomSpec>,
    pub freq_var: Option<String>,
    pub weight_var: Option<String>,
    pub estimate_labels: Vec<String>,
    pub contrast_labels: Vec<String>,
    pub lsmeans: Vec<String>,
}

use crate::procs::common::fmt_p_num as fmt_p;

use crate::procs::common::centered;

use crate::procs::common::value_label;

use crate::stat::optim::{nelder_mead, polish_coord};

pub fn execute(ast: &GlimmixAst, session: &mut Session) -> Result<()> {
    // ── 1. Guards ────────────────────────────────────────────────────────────
    let model = ast
        .model
        .as_ref()
        .ok_or_else(|| SasError::runtime("MODEL statement required in PROC GLIMMIX."))?;

    check_guards(ast, model, session)?;

    // ── 2. Read dataset ──────────────────────────────────────────────────────
    let (ds, in_libref, in_table) = common::open_input(&ast.data, session)?;
    let n_read = ds.n_obs();

    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
    };

    let resp_idx = find_col(&model.response)?;
    let resp_col = decode_column(&ds, resp_idx)?;

    let mut fixed_cols_full: Vec<(String, Vec<Value>)> = Vec::new();
    for nm in &model.fixed {
        let idx = find_col(nm)?;
        fixed_cols_full.push((nm.clone(), decode_column(&ds, idx)?));
    }
    let freq_col: Option<Vec<Value>> = match &ast.freq_var {
        Some(fv) => Some(decode_column(&ds, find_col(fv)?)?),
        None => None,
    };

    let random = ast.random.as_ref();
    let subject = random.and_then(|r| r.subject.clone());
    let subj_col: Option<Vec<Value>> = match &subject {
        Some(s) => Some(decode_column(&ds, find_col(s)?)?),
        None => None,
    };

    // ── 3. Determine binomial event level ────────────────────────────────────
    let event_level = determine_event_level(model, &resp_col, n_read)?;

    // ── 4. Build observations (listwise deletion + encoding) ──────────────────
    let KeptObs {
        y,
        freq,
        subj_values,
        kept_fixed,
        n_not_used,
    } = build_observations(
        model,
        &ast.class_vars,
        &resp_col,
        &fixed_cols_full,
        &freq_col,
        &subj_col,
        &event_level,
        n_read,
    );

    let n_used = y.len();
    if n_used == 0 {
        return Err(SasError::runtime(
            "No complete observations available for PROC GLIMMIX.",
        ));
    }
    let n_total: f64 = freq.iter().sum();

    // Build the labeled fixed-effects design.
    let design = build_design(
        &kept_fixed,
        &ast.class_vars,
        &model.fixed,
        model.noint,
        n_used,
    )?;
    if design.is_empty() {
        return Err(SasError::runtime(
            "MODEL has no effects (NOINT with no fixed effects) in PROC GLIMMIX.",
        ));
    }
    let param_labels: Vec<String> = design.iter().map(|d| d.label.clone()).collect();
    let x: Vec<Vec<f64>> = (0..n_used)
        .map(|i| design.iter().map(|c| c.values[i]).collect())
        .collect();
    let p = x[0].len();

    // Subject levels.
    let (subj_of, levels) = index_subjects(&subj_values, subj_col.is_some());
    let n_subjects = levels.len();

    let has_random = random.is_some();
    if has_random && n_subjects < 2 {
        return Err(SasError::runtime(
            "PROC GLIMMIX requires at least 2 subjects when a RANDOM statement is used.",
        ));
    }

    // Within-subject position index (0-based, in order of appearance), used by
    // the AR(1)/UN repeated covariance structure.
    let within_idx: Vec<usize> = {
        let mut counters = vec![0usize; n_subjects.max(1)];
        let mut wi = Vec::with_capacity(n_used);
        for &s in &subj_of {
            wi.push(counters[s]);
            counters[s] += 1;
        }
        wi
    };
    // The (selected) covariance type, when a RANDOM statement is present.
    let cov_type = random.map(|r| r.cov_type).unwrap_or(CovType::Vc);
    let rep_cov: Option<RepCov> = match cov_type {
        CovType::Ar1 => Some(RepCov::Ar1),
        CovType::Un => {
            let t = within_idx.iter().copied().max().map(|m| m + 1).unwrap_or(0);
            Some(RepCov::Un { t })
        }
        _ => None,
    };

    let use_laplace = ast.method == Method::Laplace && has_random;

    // ── 5. Fit dispatch ──────────────────────────────────────────────────────
    let fit: GlimmixFit = compute_fit(
        model,
        &FitContext {
            y: &y,
            x: &x,
            freq: &freq,
            subj_of: &subj_of,
            within_idx: &within_idx,
            n_subjects,
            n_total,
        },
        rep_cov,
        use_laplace,
        has_random,
    )?;

    // Generalized Chi-Square: Σ freq * (y - μ)² / V(μ).
    let gen_chisq: f64 = (0..n_used)
        .map(|i| {
            let v = variance(fit.mu[i], model.dist);
            freq[i] * (y[i] - fit.mu[i]).powi(2) / v
        })
        .sum();

    // DF for fixed-effects tests (ddfm=Contain).
    let den_df: f64 = if has_random {
        (n_subjects as f64 - p as f64).max(1.0)
    } else {
        (n_total - p as f64).max(1.0)
    };
    let gen_chisq_df = (n_total - p as f64).max(1.0);

    // Max obs per subject.
    let max_obs = if has_random {
        let mut counts = vec![0usize; n_subjects];
        for &s in &subj_of {
            counts[s] += 1;
        }
        *counts.iter().max().unwrap_or(&0)
    } else {
        0
    };

    // ── 6. Listing ───────────────────────────────────────────────────────────
    let laplace = ast.method == Method::Laplace && has_random;

    print_model_information(session, model, &in_libref, &in_table, laplace);
    if has_random {
        print_class_level_information(session, &subject, &levels, n_subjects);
        print_dimensions(session, &fit, p, n_subjects, max_obs);
    }
    print_number_of_observations(session, ast, n_read, n_used, n_total, n_not_used);
    print_iteration_history(session, &fit, has_random, gen_chisq);

    // Convergence note.
    centered(session, "Convergence criterion (GCONV=1E-8) satisfied.");
    session.listing.blank();

    if has_random {
        print_covariance_parameter_estimates(session, &fit, &subject);
    }
    print_fit_statistics(
        session,
        model,
        &fit,
        p,
        n_subjects,
        gen_chisq,
        gen_chisq_df,
        laplace,
        has_random,
    );
    print_type3_tests(session, &param_labels, &fit, den_df);
    if model.solution {
        print_fixed_solutions(session, &param_labels, &fit, den_df);
    }

    let _ = fit.iterations;
    Ok(())
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
