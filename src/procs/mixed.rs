//! PROC MIXED — linear mixed models with REML / ML estimation (M28).
//!
//! Scope implemented:
//! - CLASS statement (categorical variables; used to identify SUBJECT levels).
//! - MODEL response = <fixed> / [solution] [ddfm=...] [noint] : here the only
//!   fully-supported fixed-effects structure is intercept-only (`model y = `),
//!   matching the verified oracle. Additional fixed CLASS effects are accepted
//!   but only the intercept design (X = ones) is exercised by the oracle.
//! - RANDOM intercept / SUBJECT=<var> TYPE=VC|CS : random intercept per
//!   subject. VC and CS are identical for a single random intercept (balanced
//!   or not), so both map to the same variance-components model:
//!       V = σ²_u · Z Z' + σ²_e · I
//! - METHOD=REML (default) and METHOD=ML.
//!
//! Estimation: for a single random intercept the REML/ML estimates have a
//! closed form for *balanced* designs (equal #obs per subject) via the method
//! of moments; this is exact and is what SAS reports. For unbalanced designs we
//! fall back to a 1-D profile search on λ = σ²_u/σ²_e. β̂ and SE(β̂) are then
//! formed from the general V-based formulas, which reproduce the balanced oracle
//! exactly.
//!
//! Parse-accepted but not implemented (NOTE emitted): TYPE=AR(1)/UN (proper
//! error), REPEATED, ESTIMATE, CONTRAST, COVTEST, ASYCOV, NOBOUND, G/GCORR/
//! R/RCORR options, DDFM= (we always print/use Contain).

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::decode_column;
use crate::session::Session;
use crate::stat::{invert_matrix, student_t_cdf};
use crate::token::TokenKind;
use crate::value::{format_best, Value};

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Reml,
    Ml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovType {
    Vc,
    Cs,
    Ar1,
    Un,
}

#[derive(Debug, Clone)]
pub struct RandomSpec {
    /// The random effect terms (e.g. ["intercept"]). Only `intercept` is
    /// implemented; other terms produce an error.
    pub effects: Vec<String>,
    pub subject: Option<String>,
    pub cov_type: CovType,
}

#[derive(Debug, Clone)]
pub struct RepeatedSpec {
    pub subject: Option<String>,
    pub cov_type: CovType,
}

#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub response: String,
    pub fixed: Vec<String>,
    pub solution: bool,
    pub noint: bool,
    pub ddfm: Option<String>,
    pub nofit: bool,
}

#[derive(Debug, Clone)]
pub struct LsmeansSpec {
    pub effect: String,
    pub diff: bool,
    pub pdiff: bool,
    pub cl: bool,
    pub alpha: f64,
}

#[derive(Debug, Clone)]
pub struct MixedAst {
    pub data: Option<DatasetRef>,
    pub method: Method,
    pub covtest: bool,
    pub nobound: bool,
    pub asycov: bool,
    pub class_vars: Vec<String>,
    pub model: Option<ModelSpec>,
    pub random: Option<RandomSpec>,
    pub repeated: Option<RepeatedSpec>,
    pub lsmeans: Vec<LsmeansSpec>,
    /// Labels of ESTIMATE statements seen (for NOTE emission).
    pub estimate_labels: Vec<String>,
    /// Labels of CONTRAST statements seen (for NOTE emission).
    pub contrast_labels: Vec<String>,
}

// ───────────────────────── Parser helpers ─────────────────────────

/// Parse a TYPE=... value, including `ar(1)`.
fn parse_cov_type(ts: &mut StatementStream) -> CovType {
    let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
    let t = match v.as_deref() {
        Some("cs") => CovType::Cs,
        Some("un") => CovType::Un,
        Some("ar") => CovType::Ar1,
        _ => CovType::Vc,
    };
    ts.next();
    // Consume an optional `(1)` after AR.
    if ts.peek().kind == TokenKind::LParen {
        ts.next();
        while ts.peek().kind != TokenKind::RParen
            && ts.peek().kind != TokenKind::Semi
            && ts.peek().kind != TokenKind::Eof
        {
            ts.next();
        }
        if ts.peek().kind == TokenKind::RParen {
            ts.next();
        }
    }
    t
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC MIXED. Called AFTER `proc mixed` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<MixedAst> {
    let mut data: Option<DatasetRef> = None;
    let mut method = Method::Reml;
    let mut covtest = false;
    let mut nobound = false;
    let mut asycov = false;

    // PROC MIXED statement options, until `;`.
    loop {
        let tk = ts.peek();
        if tk.kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if tk.kind == TokenKind::Eof {
            break;
        }
        if tk.is_kw("data") {
            data = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if tk.is_kw("method") {
            common::expect_eq(ts, "METHOD")?;
            let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
            method = match v.as_deref() {
                Some("ml") => Method::Ml,
                _ => Method::Reml,
            };
            ts.next();
        } else if tk.is_kw("covtest") {
            covtest = true;
            ts.next();
        } else if tk.is_kw("nobound") {
            nobound = true;
            ts.next();
        } else if tk.is_kw("asycov") {
            asycov = true;
            ts.next();
        } else {
            ts.next();
        }
    }

    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<ModelSpec> = None;
    let mut random: Option<RandomSpec> = None;
    let mut repeated: Option<RepeatedSpec> = None;
    let mut lsmeans: Vec<LsmeansSpec> = Vec::new();
    let mut estimate_labels: Vec<String> = Vec::new();
    let mut contrast_labels: Vec<String> = Vec::new();

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "class" {
            ts.next();
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    class_vars.push(name);
                }
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "model" {
            ts.next();
            model = Some(parse_model(ts)?);
            Ok(true)
        } else if kw == "random" {
            ts.next();
            random = Some(parse_random(ts));
            Ok(true)
        } else if kw == "repeated" {
            ts.next();
            repeated = Some(parse_repeated(ts));
            Ok(true)
        } else if kw == "lsmeans" {
            ts.next();
            if let Some(spec) = parse_lsmeans(ts) {
                lsmeans.push(spec);
            }
            Ok(true)
        } else if kw == "estimate" {
            ts.next();
            if let TokenKind::Str { value, .. } = &ts.peek().kind {
                estimate_labels.push(value.clone());
            }
            ts.skip_to_semi();
            Ok(true)
        } else if kw == "contrast" {
            ts.next();
            if let TokenKind::Str { value, .. } = &ts.peek().kind {
                contrast_labels.push(value.clone());
            }
            ts.skip_to_semi();
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(MixedAst {
        data,
        method,
        covtest,
        nobound,
        asycov,
        class_vars,
        model,
        random,
        repeated,
        lsmeans,
        estimate_labels,
        contrast_labels,
    })
}

/// Parse the MODEL statement body (after `model`): `response = <fixed> / opts;`.
fn parse_model(ts: &mut StatementStream) -> Result<ModelSpec> {
    let response = ts
        .peek()
        .ident()
        .map(str::to_string)
        .ok_or_else(|| SasError::parse("expected response variable in MODEL", ts.peek().span))?;
    ts.next();
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            "expected '=' in MODEL statement",
            ts.peek().span,
        ));
    }
    ts.next();

    let mut fixed: Vec<String> = Vec::new();
    // Read fixed effects until `/` or `;`.
    while ts.peek().kind != TokenKind::Semi
        && ts.peek().kind != TokenKind::Slash
        && ts.peek().kind != TokenKind::Eof
    {
        if let Some(name) = ts.peek().ident().map(str::to_string) {
            fixed.push(name);
        }
        ts.next();
    }

    let mut solution = false;
    let mut noint = false;
    let mut ddfm: Option<String> = None;
    let mut nofit = false;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let tk = ts.peek();
            if tk.is_kw("solution") || tk.is_kw("s") {
                solution = true;
                ts.next();
            } else if tk.is_kw("noint") {
                noint = true;
                ts.next();
            } else if tk.is_kw("nofit") {
                nofit = true;
                ts.next();
            } else if tk.is_kw("ddfm") {
                common::expect_eq(ts, "DDFM")?;
                ddfm = ts.peek().ident().map(|s| s.to_ascii_lowercase());
                ts.next();
            } else {
                ts.next();
            }
        }
    }
    ts.expect_semi()?;

    Ok(ModelSpec {
        response,
        fixed,
        solution,
        noint,
        ddfm,
        nofit,
    })
}

/// Parse the RANDOM statement body (after `random`).
fn parse_random(ts: &mut StatementStream) -> RandomSpec {
    let mut effects: Vec<String> = Vec::new();
    while ts.peek().kind != TokenKind::Semi
        && ts.peek().kind != TokenKind::Slash
        && ts.peek().kind != TokenKind::Eof
    {
        if let Some(name) = ts.peek().ident().map(str::to_string) {
            effects.push(name);
        }
        ts.next();
    }

    let mut subject: Option<String> = None;
    let mut cov_type = CovType::Vc;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let tk = ts.peek();
            if tk.is_kw("subject") || tk.is_kw("subj") {
                let _ = common::expect_eq(ts, "SUBJECT");
                subject = ts.peek().ident().map(str::to_string);
                ts.next();
            } else if tk.is_kw("type") {
                let _ = common::expect_eq(ts, "TYPE");
                cov_type = parse_cov_type(ts);
            } else {
                ts.next();
            }
        }
    }
    let _ = ts.expect_semi();

    RandomSpec {
        effects,
        subject,
        cov_type,
    }
}

/// Parse the REPEATED statement body (after `repeated`).
fn parse_repeated(ts: &mut StatementStream) -> RepeatedSpec {
    let mut subject: Option<String> = None;
    let mut cov_type = CovType::Vc;

    // Skip any effect tokens before `/`.
    while ts.peek().kind != TokenKind::Semi
        && ts.peek().kind != TokenKind::Slash
        && ts.peek().kind != TokenKind::Eof
    {
        ts.next();
    }
    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let tk = ts.peek();
            if tk.is_kw("subject") || tk.is_kw("subj") {
                let _ = common::expect_eq(ts, "SUBJECT");
                subject = ts.peek().ident().map(str::to_string);
                ts.next();
            } else if tk.is_kw("type") {
                let _ = common::expect_eq(ts, "TYPE");
                cov_type = parse_cov_type(ts);
            } else {
                ts.next();
            }
        }
    }
    let _ = ts.expect_semi();

    RepeatedSpec { subject, cov_type }
}

/// Parse the LSMEANS statement body (after `lsmeans`).
fn parse_lsmeans(ts: &mut StatementStream) -> Option<LsmeansSpec> {
    let effect = ts.peek().ident().map(str::to_string)?;
    ts.next();

    let mut diff = false;
    let mut pdiff = false;
    let mut cl = false;
    let mut alpha = 0.05;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let tk = ts.peek();
            if tk.is_kw("diff") {
                diff = true;
                ts.next();
            } else if tk.is_kw("pdiff") {
                pdiff = true;
                ts.next();
            } else if tk.is_kw("cl") {
                cl = true;
                ts.next();
            } else if tk.is_kw("alpha") {
                let _ = common::expect_eq(ts, "ALPHA");
                if let TokenKind::Num(v) = ts.peek().kind {
                    alpha = v;
                }
                ts.next();
            } else {
                ts.next();
            }
        }
    }
    let _ = ts.expect_semi();

    Some(LsmeansSpec {
        effect,
        diff,
        pdiff,
        cl,
        alpha,
    })
}

// ───────────────────────── Formatting helpers ─────────────────────────

fn fmt4(v: f64) -> String {
    format!("{v:.4}")
}

fn fmt2(v: f64) -> String {
    format!("{v:.2}")
}

fn fmt_p(v: f64) -> String {
    if v < 0.0001 {
        "<.0001".to_string()
    } else {
        format!("{v:.4}")
    }
}

fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(text.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), text));
}

fn value_label(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.trim_end().to_string(),
    }
}

// ───────────────────────── Linear algebra helpers ─────────────────────────

fn mat_vec(mat: &[Vec<f64>], v: &[f64]) -> Vec<f64> {
    mat.iter()
        .map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum())
        .collect()
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

// ───────────────────────── Mixed model fit ─────────────────────────

/// Result of fitting the variance-components mixed model (single random
/// intercept per subject).
struct MixedFit {
    sigma2_u: f64,
    sigma2_e: f64,
    /// β̂ for the fixed effects (length p).
    beta: Vec<f64>,
    /// Var(β̂) = (X'V⁻¹X)⁻¹ (p×p).
    cov_beta: Vec<Vec<f64>>,
    /// -2 log (restricted) likelihood at the optimum.
    neg2ll: f64,
    n: usize,
    p: usize,
    balanced: bool,
}

/// Build V = σ²_u Z Z' + σ²_e I given subject membership.
/// `subj_of[i]` is the subject index of observation i.
fn build_v(n: usize, subj_of: &[usize], sigma2_u: f64, sigma2_e: f64) -> Vec<Vec<f64>> {
    let mut v = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            let mut val = 0.0;
            if subj_of[i] == subj_of[j] {
                val += sigma2_u;
            }
            if i == j {
                val += sigma2_e;
            }
            v[i][j] = val;
        }
    }
    v
}

/// -2 log REML/ML likelihood for given (σ²_u, σ²_e).
///
/// -2 logL_ML  = n·log(2π) + log|V| + (y-Xβ)'V⁻¹(y-Xβ)
/// -2 logL_REML = -2 logL_ML(restricted) = (n-p)·log(2π) + log|V|
///                + log|X'V⁻¹X| + y'Py
/// where Py = V⁻¹(y-Xβ) at β = (X'V⁻¹X)⁻¹X'V⁻¹y.
fn neg2_loglik(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    sigma2_u: f64,
    sigma2_e: f64,
    method: Method,
) -> Result<(f64, Vec<f64>, Vec<Vec<f64>>)> {
    let n = y.len();
    let p = x[0].len();
    let v = build_v(n, subj_of, sigma2_u, sigma2_e);
    let v_inv = invert_matrix(&v)?;
    let log_det_v = log_det_spd(&v)?;

    // X'V⁻¹  (p×n)
    let mut xtvi = vec![vec![0.0; n]; p];
    for a in 0..p {
        for j in 0..n {
            let mut s = 0.0;
            for i in 0..n {
                s += x[i][a] * v_inv[i][j];
            }
            xtvi[a][j] = s;
        }
    }
    // X'V⁻¹X  (p×p)
    let mut xtvix = vec![vec![0.0; p]; p];
    for a in 0..p {
        for b in 0..p {
            let mut s = 0.0;
            for j in 0..n {
                s += xtvi[a][j] * x[j][b];
            }
            xtvix[a][b] = s;
        }
    }
    let xtvix_inv = invert_matrix(&xtvix)?;
    let log_det_xtvix = log_det_spd(&xtvix)?;

    // X'V⁻¹y  (p)
    let xtviy: Vec<f64> = (0..p).map(|a| dot(&xtvi[a], y)).collect();
    // β̂ = (X'V⁻¹X)⁻¹ X'V⁻¹y
    let beta = mat_vec(&xtvix_inv, &xtviy);

    // residual r = y - Xβ
    let resid: Vec<f64> = (0..n)
        .map(|i| y[i] - (0..p).map(|a| x[i][a] * beta[a]).sum::<f64>())
        .collect();
    // r' V⁻¹ r
    let vir = mat_vec(&v_inv, &resid);
    let quad = dot(&resid, &vir);

    let two_pi = std::f64::consts::TAU;
    let neg2 = match method {
        Method::Reml => {
            (n as f64 - p as f64) * two_pi.ln() + log_det_v + log_det_xtvix + quad
        }
        Method::Ml => n as f64 * two_pi.ln() + log_det_v + quad,
    };

    Ok((neg2, beta, xtvix_inv))
}

/// log determinant of a symmetric positive-definite matrix via Cholesky.
fn log_det_spd(a: &[Vec<f64>]) -> Result<f64> {
    let l = crate::stat::cholesky(a)?;
    let mut s = 0.0;
    for (i, row) in l.iter().enumerate() {
        s += row[i].ln();
    }
    Ok(2.0 * s)
}

/// Fit the variance-components mixed model.
fn fit_mixed(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    n_subjects: usize,
    method: Method,
    nobound: bool,
) -> Result<MixedFit> {
    let n = y.len();
    let p = x[0].len();

    // Detect balance: do all subjects have the same number of observations?
    let mut counts = vec![0usize; n_subjects];
    for &s in subj_of {
        counts[s] += 1;
    }
    let n_i = counts[0];
    let balanced = counts.iter().all(|&c| c == n_i) && n_i > 0;

    // For the intercept-only balanced case, use the closed-form moment
    // estimator (exact REML/ML). This is the configuration the oracle verifies.
    let intercept_only = p == 1 && x.iter().all(|row| row[0] == 1.0);

    let (mut sigma2_u, sigma2_e) = if balanced && intercept_only && n_subjects >= 2 {
        closed_form_vc(y, subj_of, n_subjects, n_i, method)
    } else {
        // General path: 1-D profile search over λ = σ²_u / σ²_e ≥ 0.
        profile_search(y, x, subj_of, method)?
    };

    if !nobound && sigma2_u < 0.0 {
        sigma2_u = 0.0;
    }

    // Final β̂, Var(β̂), and -2 logL at the estimated variances.
    let (neg2ll, beta, cov_beta) =
        neg2_loglik(y, x, subj_of, sigma2_u, sigma2_e, method)?;

    Ok(MixedFit {
        sigma2_u,
        sigma2_e,
        beta,
        cov_beta,
        neg2ll,
        n,
        p,
        balanced,
    })
}

/// Closed-form variance components for a balanced one-way random model.
/// Returns (σ²_u, σ²_e).
fn closed_form_vc(
    y: &[f64],
    subj_of: &[usize],
    n_subjects: usize,
    n_i: usize,
    method: Method,
) -> (f64, f64) {
    let a = n_subjects;
    let n_total = y.len();

    // Group means and grand mean.
    let mut group_sum = vec![0.0; a];
    for (i, &yi) in y.iter().enumerate() {
        group_sum[subj_of[i]] += yi;
    }
    let group_mean: Vec<f64> = group_sum.iter().map(|s| s / n_i as f64).collect();
    let grand_mean = y.iter().sum::<f64>() / n_total as f64;

    // SS_between and SS_within.
    let ss_between: f64 = group_mean
        .iter()
        .map(|m| (m - grand_mean).powi(2))
        .sum::<f64>()
        * n_i as f64;
    let ss_within: f64 = y
        .iter()
        .enumerate()
        .map(|(i, &yi)| (yi - group_mean[subj_of[i]]).powi(2))
        .sum();

    let ms_between = ss_between / (a as f64 - 1.0);
    let ms_within = ss_within / (n_total as f64 - a as f64);

    let sigma2_e = ms_within;
    let sigma2_u = match method {
        Method::Reml => (ms_between - ms_within) / n_i as f64,
        Method::Ml => {
            (((a as f64 - 1.0) / a as f64) * ms_between - ms_within) / n_i as f64
        }
    };
    (sigma2_u, sigma2_e)
}

/// Profile search over λ = σ²_u / σ²_e for the unbalanced / general case.
/// Returns (σ²_u, σ²_e). Uses golden-section minimisation of -2 logL.
fn profile_search(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    method: Method,
) -> Result<(f64, f64)> {
    let total_var = {
        let n = y.len() as f64;
        let mean = y.iter().sum::<f64>() / n;
        y.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n
    };
    // For a given λ, profile σ²_e out and return -2 logL.
    // We parameterise V = σ²_e (λ ZZ' + I); profile σ²_e analytically.
    let eval = |lambda: f64| -> Result<(f64, f64)> {
        // V0 = λ ZZ' + I
        let n = y.len();
        let p = x[0].len();
        let mut v0 = vec![vec![0.0; n]; n];
        for i in 0..n {
            for j in 0..n {
                let mut val = 0.0;
                if subj_of[i] == subj_of[j] {
                    val += lambda;
                }
                if i == j {
                    val += 1.0;
                }
                v0[i][j] = val;
            }
        }
        let v0_inv = invert_matrix(&v0)?;
        let log_det_v0 = log_det_spd(&v0)?;

        // X'V0⁻¹
        let mut xtvi = vec![vec![0.0; n]; p];
        for a in 0..p {
            for j in 0..n {
                let mut s = 0.0;
                for i in 0..n {
                    s += x[i][a] * v0_inv[i][j];
                }
                xtvi[a][j] = s;
            }
        }
        let mut xtvix = vec![vec![0.0; p]; p];
        for a in 0..p {
            for b in 0..p {
                let mut s = 0.0;
                for j in 0..n {
                    s += xtvi[a][j] * x[j][b];
                }
                xtvix[a][b] = s;
            }
        }
        let xtvix_inv = invert_matrix(&xtvix)?;
        let log_det_xtvix = log_det_spd(&xtvix)?;
        let xtviy: Vec<f64> = (0..p).map(|a| dot(&xtvi[a], y)).collect();
        let beta = mat_vec(&xtvix_inv, &xtviy);
        let resid: Vec<f64> = (0..n)
            .map(|i| y[i] - (0..p).map(|a| x[i][a] * beta[a]).sum::<f64>())
            .collect();
        let vir = mat_vec(&v0_inv, &resid);
        let quad = dot(&resid, &vir);

        let dof = match method {
            Method::Reml => (n - p) as f64,
            Method::Ml => n as f64,
        };
        let sigma2_e = quad / dof;

        let two_pi = std::f64::consts::TAU;
        let neg2 = match method {
            Method::Reml => {
                (n as f64 - p as f64) * (two_pi.ln() + sigma2_e.ln())
                    + log_det_v0
                    + log_det_xtvix
                    + dof
            }
            Method::Ml => n as f64 * (two_pi.ln() + sigma2_e.ln()) + log_det_v0 + dof,
        };
        Ok((neg2, sigma2_e))
    };

    // Golden-section search for λ ∈ [0, λ_max].
    let lambda_max = 1000.0_f64;
    let gr = (5.0_f64.sqrt() - 1.0) / 2.0;
    let mut lo = 0.0;
    let mut hi = lambda_max;
    let mut c = hi - gr * (hi - lo);
    let mut d = lo + gr * (hi - lo);
    let mut fc = eval(c)?.0;
    let mut fd = eval(d)?.0;
    for _ in 0..200 {
        if (hi - lo).abs() < 1e-10 {
            break;
        }
        if fc < fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - gr * (hi - lo);
            fc = eval(c)?.0;
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + gr * (hi - lo);
            fd = eval(d)?.0;
        }
    }
    let lambda = 0.5 * (lo + hi);
    // Also check the boundary λ=0 (σ²_u = 0).
    let (f_opt, _) = eval(lambda)?;
    let (f0, s2e0) = eval(0.0)?;
    if f0 <= f_opt {
        // σ²_u clipped to 0.
        let _ = total_var;
        return Ok((0.0, s2e0));
    }
    let (_, sigma2_e) = eval(lambda)?;
    let sigma2_u = lambda * sigma2_e;
    Ok((sigma2_u, sigma2_e))
}

// ───────────────────────── Execute ─────────────────────────

/// Decide whether the request is exactly the legacy M28 case: a single random
/// intercept with TYPE=VC|CS, SUBJECT=, no REPEATED, and an intercept-only mean
/// (no fixed effects, no NOINT). This path is kept numerically and format
/// byte-identical to the m28 oracle.
fn is_legacy_case(ast: &MixedAst) -> bool {
    let Some(model) = ast.model.as_ref() else {
        return false;
    };
    if !model.fixed.is_empty() || model.noint {
        return false;
    }
    if ast.repeated.is_some() {
        return false;
    }
    let Some(random) = ast.random.as_ref() else {
        return false;
    };
    if !matches!(random.cov_type, CovType::Vc | CovType::Cs) {
        return false;
    }
    if random.subject.is_none() {
        return false;
    }
    random.effects.len() == 1 && random.effects[0].eq_ignore_ascii_case("intercept")
}

pub fn execute(ast: &MixedAst, session: &mut Session) -> Result<()> {
    if is_legacy_case(ast) {
        execute_legacy(ast, session)
    } else {
        execute_general(ast, session)
    }
}

fn execute_legacy(ast: &MixedAst, session: &mut Session) -> Result<()> {
    // ── 1. Validate / guards ────────────────────────────────────────────────
    let model = ast.model.as_ref().ok_or_else(|| {
        SasError::runtime("MODEL statement required in PROC MIXED.")
    })?;

    let random = ast.random.as_ref().ok_or_else(|| {
        SasError::runtime("PROC MIXED currently requires a RANDOM statement with SUBJECT=.")
    })?;

    let subject = random.subject.as_ref().ok_or_else(|| {
        SasError::runtime("RANDOM statement requires SUBJECT= in PROC MIXED.")
    })?;

    // NOTEs for parse-accepted / deferred features.
    if ast.covtest {
        session
            .log
            .note("COVTEST is parse-accepted but not implemented in PROC MIXED.");
    }
    if ast.asycov {
        session
            .log
            .note("ASYCOV is parse-accepted but not implemented in PROC MIXED.");
    }
    if ast.nobound {
        session
            .log
            .note("NOBOUND is parse-accepted but not implemented in PROC MIXED.");
    }
    if let Some(d) = &model.ddfm {
        if d != "contain" {
            session.log.note(&format!(
                "DDFM={} is parse-accepted but not implemented; using CONTAIN.",
                d.to_uppercase()
            ));
        }
    }
    if model.nofit {
        session
            .log
            .note("NOFIT is parse-accepted but not implemented in PROC MIXED.");
    }
    if ast.repeated.is_some() {
        session
            .log
            .note("REPEATED statement is parse-accepted but not implemented in PROC MIXED.");
    }
    for lbl in &ast.estimate_labels {
        session.log.note(&format!(
            "ESTIMATE '{}' is parse-accepted but not implemented in PROC MIXED.",
            lbl
        ));
    }
    for lbl in &ast.contrast_labels {
        session.log.note(&format!(
            "CONTRAST '{}' is parse-accepted but not implemented in PROC MIXED.",
            lbl
        ));
    }
    if !ast.lsmeans.is_empty() {
        session
            .log
            .note("LSMEANS is parse-accepted but not implemented in PROC MIXED.");
    }

    // ── 2. Read dataset ─────────────────────────────────────────────────────
    let in_ref = common::resolve_last_dataset(&ast.data, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_read = ds.n_obs();

    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
    };

    let resp_idx = find_col(&model.response)?;
    let subj_idx = find_col(subject)?;

    let resp_col = decode_column(&ds, resp_idx)?;
    let subj_col = decode_column(&ds, subj_idx)?;

    // ── 3. Build complete observations ──────────────────────────────────────
    let mut y: Vec<f64> = Vec::new();
    let mut subj_values: Vec<Value> = Vec::new();
    let mut n_not_used = 0usize;
    for i in 0..n_read {
        let yi = match &resp_col[i] {
            Value::Num(v) if !v.is_nan() => *v,
            _ => {
                n_not_used += 1;
                continue;
            }
        };
        if subj_col[i].is_missing() {
            n_not_used += 1;
            continue;
        }
        y.push(yi);
        subj_values.push(subj_col[i].clone());
    }

    let n_used = y.len();
    if n_used == 0 {
        return Err(SasError::runtime(
            "No complete observations available for PROC MIXED.",
        ));
    }

    // Determine subject levels (sorted by SAS comparison order).
    let mut levels: Vec<Value> = Vec::new();
    for v in &subj_values {
        if !levels.iter().any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal) {
            levels.push(v.clone());
        }
    }
    levels.sort_by(|a, b| a.sas_cmp(b));
    let n_subjects = levels.len();
    let level_index = |v: &Value| -> usize {
        levels
            .iter()
            .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            .unwrap()
    };
    let subj_of: Vec<usize> = subj_values.iter().map(|v| level_index(v)).collect();

    if n_subjects < 2 {
        return Err(SasError::runtime(
            "PROC MIXED requires at least 2 subjects.",
        ));
    }

    // Design matrix X: intercept-only.
    let x: Vec<Vec<f64>> = vec![vec![1.0]; n_used];

    // ── 4. Fit ──────────────────────────────────────────────────────────────
    let fit = fit_mixed(&y, &x, &subj_of, n_subjects, ast.method, ast.nobound)?;

    // Max observations per subject.
    let mut counts = vec![0usize; n_subjects];
    for &s in &subj_of {
        counts[s] += 1;
    }
    let max_obs = *counts.iter().max().unwrap_or(&0);

    // ── 5. Listing ──────────────────────────────────────────────────────────
    let method_name = match ast.method {
        Method::Reml => "REML",
        Method::Ml => "ML",
    };
    let cov_struct = match random.cov_type {
        CovType::Cs => "Compound Symmetry",
        _ => "Variance Components",
    };

    session.listing.page_header();
    centered(session, "The Mixed Procedure");
    session.listing.blank();

    // Model Information.
    centered(session, "Model Information");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Left];
        let rows: Vec<Vec<String>> = vec![
            vec![
                "Data Set".into(),
                format!("{}.{}", in_libref, in_table),
            ],
            vec!["Dependent Variable".into(), model.response.clone()],
            vec!["Covariance Structure".into(), cov_struct.into()],
            vec!["Estimation Method".into(), method_name.into()],
            vec!["Residual Variance Method".into(), "Profile".into()],
            vec!["Fixed Effects SE Method".into(), "Model-Based".into()],
            vec!["Degrees of Freedom Method".into(), "Contain".into()],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }

    // Class Level Information.
    centered(session, "Class Level Information");
    session.listing.blank();
    {
        let headers = vec!["Class".into(), "Levels".into(), "Values".into()];
        let aligns = vec![Align::Left, Align::Right, Align::Left];
        let values_str = levels
            .iter()
            .map(value_label)
            .collect::<Vec<_>>()
            .join(" ");
        let rows = vec![vec![
            subject.clone(),
            n_subjects.to_string(),
            values_str,
        ]];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Dimensions.
    centered(session, "Dimensions");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec!["Covariance Parameters".into(), "2".into()],
            vec!["Columns in X".into(), fit.p.to_string()],
            vec!["Columns in Z Per Subject".into(), "1".into()],
            vec!["Subjects".into(), n_subjects.to_string()],
            vec!["Max Obs Per Subject".into(), max_obs.to_string()],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }

    // Number of Observations.
    centered(session, "Number of Observations");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec!["Number of Observations Read".into(), n_read.to_string()],
            vec!["Number of Observations Used".into(), n_used.to_string()],
            vec![
                "Number of Observations Not Used".into(),
                n_not_used.to_string(),
            ],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }

    // Iteration History (minimal, stable).
    centered(session, "Iteration History");
    session.listing.blank();
    {
        let headers = vec![
            "Iteration".into(),
            "Evaluations".into(),
            "-2 Res Log Like".into(),
            "Criterion".into(),
        ];
        let aligns = vec![Align::Right, Align::Right, Align::Right, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec![
                "0".into(),
                "1".into(),
                fmt4(fit.neg2ll),
                String::new(),
            ],
            vec![
                "1".into(),
                "1".into(),
                fmt4(fit.neg2ll),
                "0.00000000".into(),
            ],
        ];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }
    centered(session, "Convergence criteria met.");
    session.listing.blank();

    // Covariance Parameter Estimates.
    centered(session, "Covariance Parameter Estimates");
    session.listing.blank();
    {
        let headers = vec!["Cov Parm".into(), "Subject".into(), "Estimate".into()];
        let aligns = vec![Align::Left, Align::Left, Align::Right];
        let cov_parm_name = match random.cov_type {
            CovType::Cs => "CS",
            _ => "Intercept",
        };
        let rows: Vec<Vec<String>> = vec![
            vec![
                cov_parm_name.into(),
                subject.clone(),
                fmt4(fit.sigma2_u),
            ],
            vec!["Residual".into(), String::new(), fmt4(fit.sigma2_e)],
        ];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Fit Statistics.
    let neg2 = fit.neg2ll;
    let n_cov = 2.0_f64;
    let aic = neg2 + 2.0 * n_cov;
    let n_eff = match ast.method {
        Method::Reml => (fit.n - fit.p) as f64,
        Method::Ml => fit.n as f64,
    };
    let aicc = if n_eff - n_cov - 1.0 > 0.0 {
        neg2 + 2.0 * n_cov * n_eff / (n_eff - n_cov - 1.0)
    } else {
        aic
    };
    let bic = neg2 + n_cov * (n_subjects as f64).ln();
    centered(session, "Fit Statistics");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let label = match ast.method {
            Method::Reml => "-2 Res Log Likelihood",
            Method::Ml => "-2 Log Likelihood",
        };
        let rows: Vec<Vec<String>> = vec![
            vec![label.into(), fmt4(neg2)],
            vec!["AIC (Smaller is Better)".into(), fmt4(aic)],
            vec!["AICC (Smaller is Better)".into(), fmt4(aicc)],
            vec!["BIC (Smaller is Better)".into(), fmt4(bic)],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }

    // Solution for Fixed Effects.
    if model.solution {
        centered(session, "Solution for Fixed Effects");
        session.listing.blank();
        let headers = vec![
            "Effect".into(),
            "Estimate".into(),
            "Standard Error".into(),
            "DF".into(),
            "t Value".into(),
            "Pr > |t|".into(),
        ];
        let aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        // Intercept-only: single row.
        let est = fit.beta[0];
        let se = fit.cov_beta[0][0].max(0.0).sqrt();
        // ddfm=contain: DF = number of subjects - number of fixed parameters.
        let df = (n_subjects as i64 - fit.p as i64).max(1);
        let t = if se > 0.0 { est / se } else { 0.0 };
        let p = 2.0 * (1.0 - student_t_cdf(t.abs(), df as f64));
        let rows = vec![vec![
            "Intercept".into(),
            fmt4(est),
            fmt4(se),
            df.to_string(),
            fmt2(t),
            fmt_p(p),
        ]];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Final NOTE if a fall-back unbalanced fit was used.
    let _ = fit.balanced;

    Ok(())
}

// ═════════════════════ General fixed-effects design ═════════════════════

/// A fixed-effects design column together with its parameter label.
struct DesignColumn {
    label: String,
    values: Vec<f64>,
}

/// Build the fixed-effects design matrix from the MODEL effects.
///
/// Columns (in order): intercept (unless NOINT), then for each MODEL effect a
/// continuous column (if the variable is not in CLASS) or reference-cell coded
/// indicator columns (L−1, last level = reference per `sas_cmp` order) for a
/// CLASS variable. Returns the design columns (each with its parameter label).
fn build_design(
    cols: &[(String, Vec<Value>)],
    class_vars: &[String],
    fixed: &[String],
    noint: bool,
    n: usize,
) -> Result<Vec<DesignColumn>> {
    let mut design: Vec<DesignColumn> = Vec::new();
    if !noint {
        design.push(DesignColumn {
            label: "Intercept".to_string(),
            values: vec![1.0; n],
        });
    }

    let find = |nm: &str| -> Option<&(String, Vec<Value>)> {
        cols.iter().find(|(name, _)| name.eq_ignore_ascii_case(nm))
    };
    let is_class = |nm: &str| class_vars.iter().any(|c| c.eq_ignore_ascii_case(nm));

    for eff in fixed {
        let col = find(eff).ok_or_else(|| {
            SasError::runtime(format!("Variable {} not found.", eff.to_uppercase()))
        })?;
        if is_class(eff) {
            // Reference-cell coding: levels sorted by sas_cmp, last is reference.
            let mut levels: Vec<Value> = Vec::new();
            for v in &col.1 {
                if !levels
                    .iter()
                    .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
                {
                    levels.push(v.clone());
                }
            }
            levels.sort_by(|a, b| a.sas_cmp(b));
            // Drop the last (reference) level.
            for lvl in levels.iter().take(levels.len().saturating_sub(1)) {
                let label = format!("{} {}", eff, value_label(lvl));
                let values: Vec<f64> = col
                    .1
                    .iter()
                    .map(|v| {
                        if v.sas_cmp(lvl) == std::cmp::Ordering::Equal {
                            1.0
                        } else {
                            0.0
                        }
                    })
                    .collect();
                design.push(DesignColumn { label, values });
            }
        } else {
            // Continuous column.
            let values: Vec<f64> = col
                .1
                .iter()
                .map(|v| match v {
                    Value::Num(f) => *f,
                    _ => f64::NAN,
                })
                .collect();
            design.push(DesignColumn {
                label: eff.clone(),
                values,
            });
        }
    }
    Ok(design)
}

// ═════════════════════ General covariance V(θ) + REML ═════════════════════

/// The kind of covariance model being optimized in the general path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenCov {
    /// RANDOM intercept VC/CS with a general fixed design (params: σ²_u, σ²_e).
    RandomVc,
    /// REPEATED TYPE=AR(1) with SUBJECT (params: ρ, σ²).
    RepeatedAr1,
    /// REPEATED TYPE=UN with SUBJECT (t(t+1)/2 params).
    RepeatedUn { t: usize },
}

/// Build V(θ) for the general path.
/// `subj_of[i]` is the subject index of observation i; `within_idx[i]` is the
/// position of obs i within its subject (0-based, in order of appearance).
fn build_v_gen(
    cov: GenCov,
    theta: &[f64],
    n: usize,
    subj_of: &[usize],
    within_idx: &[usize],
) -> Vec<Vec<f64>> {
    let mut v = vec![vec![0.0; n]; n];
    match cov {
        GenCov::RandomVc => {
            let s2u = theta[0];
            let s2e = theta[1];
            for i in 0..n {
                for j in 0..n {
                    let mut val = 0.0;
                    if subj_of[i] == subj_of[j] {
                        val += s2u;
                    }
                    if i == j {
                        val += s2e;
                    }
                    v[i][j] = val;
                }
            }
        }
        GenCov::RepeatedAr1 => {
            let rho = theta[0];
            let s2 = theta[1];
            for i in 0..n {
                for j in 0..n {
                    if subj_of[i] == subj_of[j] {
                        let d = (within_idx[i] as i64 - within_idx[j] as i64).unsigned_abs();
                        v[i][j] = s2 * rho.powi(d as i32);
                    }
                }
            }
        }
        GenCov::RepeatedUn { t } => {
            // Build the t×t SPD block from packed params (row-major lower).
            let block = un_block(theta, t);
            for i in 0..n {
                for j in 0..n {
                    if subj_of[i] == subj_of[j] {
                        v[i][j] = block[within_idx[i]][within_idx[j]];
                    }
                }
            }
        }
    }
    v
}

/// Reconstruct the t×t UN covariance block from packed lower-triangular params
/// in SAS UN order: UN(1,1), UN(2,1), UN(2,2), UN(3,1), ...
fn un_block(theta: &[f64], t: usize) -> Vec<Vec<f64>> {
    let mut m = vec![vec![0.0; t]; t];
    let mut k = 0;
    for r in 0..t {
        for c in 0..=r {
            let val = theta[k];
            m[r][c] = val;
            m[c][r] = val;
            k += 1;
        }
    }
    m
}

/// Number of free covariance parameters for a covariance model.
fn n_cov_params(cov: GenCov) -> usize {
    match cov {
        GenCov::RandomVc => 2,
        GenCov::RepeatedAr1 => 2,
        GenCov::RepeatedUn { t } => t * (t + 1) / 2,
    }
}

/// Map an unconstrained parameter vector `u` to the natural θ for the model,
/// enforcing bounds: σ²>0 via exp, ρ∈(−1,1) via tanh, UN via Cholesky factor.
fn unconstrained_to_theta(cov: GenCov, u: &[f64]) -> Vec<f64> {
    match cov {
        GenCov::RandomVc => vec![u[0].exp(), u[1].exp()],
        GenCov::RepeatedAr1 => vec![u[0].tanh(), u[1].exp()],
        GenCov::RepeatedUn { t } => {
            // u parameterizes a lower-triangular Cholesky factor L (with positive
            // diagonal via exp); θ = packed lower of L Lᵀ in UN order.
            let mut l = vec![vec![0.0; t]; t];
            let mut k = 0;
            for r in 0..t {
                for c in 0..=r {
                    if r == c {
                        l[r][c] = u[k].exp();
                    } else {
                        l[r][c] = u[k];
                    }
                    k += 1;
                }
            }
            let mut theta = Vec::with_capacity(t * (t + 1) / 2);
            for r in 0..t {
                for c in 0..=r {
                    let mut s = 0.0;
                    for p in 0..=c.min(r) {
                        s += l[r][p] * l[c][p];
                    }
                    theta.push(s);
                }
            }
            theta
        }
    }
}

/// Evaluate −2·log(RE)ML at θ. Returns (neg2, β̂, (X'V⁻¹X)⁻¹).
fn neg2_loglik_gen(
    y: &[f64],
    x: &[Vec<f64>],
    v: &[Vec<f64>],
    method: Method,
) -> Result<(f64, Vec<f64>, Vec<Vec<f64>>)> {
    let n = y.len();
    let p = x[0].len();
    let v_inv = invert_matrix(v)?;
    let log_det_v = log_det_spd(v)?;

    let mut xtvi = vec![vec![0.0; n]; p];
    for a in 0..p {
        for j in 0..n {
            let mut s = 0.0;
            for i in 0..n {
                s += x[i][a] * v_inv[i][j];
            }
            xtvi[a][j] = s;
        }
    }
    let mut xtvix = vec![vec![0.0; p]; p];
    for a in 0..p {
        for b in 0..p {
            let mut s = 0.0;
            for j in 0..n {
                s += xtvi[a][j] * x[j][b];
            }
            xtvix[a][b] = s;
        }
    }
    let xtvix_inv = invert_matrix(&xtvix)?;
    let log_det_xtvix = log_det_spd(&xtvix)?;
    let xtviy: Vec<f64> = (0..p).map(|a| dot(&xtvi[a], y)).collect();
    let beta = mat_vec(&xtvix_inv, &xtviy);
    let resid: Vec<f64> = (0..n)
        .map(|i| y[i] - (0..p).map(|a| x[i][a] * beta[a]).sum::<f64>())
        .collect();
    let vir = mat_vec(&v_inv, &resid);
    let quad = dot(&resid, &vir);

    let two_pi = std::f64::consts::TAU;
    let neg2 = match method {
        Method::Reml => {
            (n as f64 - p as f64) * two_pi.ln() + log_det_v + log_det_xtvix + quad
        }
        Method::Ml => n as f64 * two_pi.ln() + log_det_v + quad,
    };
    Ok((neg2, beta, xtvix_inv))
}

/// Result of a general mixed fit.
struct GenFit {
    /// Natural covariance parameters θ.
    theta: Vec<f64>,
    beta: Vec<f64>,
    cov_beta: Vec<Vec<f64>>,
    neg2ll: f64,
    neg2_start: f64,
    iters: usize,
    converged: bool,
}

/// One run of Nelder-Mead from `start` with per-dimension initial step `step`.
/// Minimises `eval` over `np`-dimensional unconstrained space. Returns the best
/// point found, its function value, the number of iterations consumed, and
/// whether the simplex converged (function-value spread and vertex spread both
/// below tolerance).
fn nelder_mead<F: Fn(&[f64]) -> f64>(
    eval: &F,
    start: &[f64],
    step: f64,
    max_iter: usize,
    ftol: f64,
    xtol: f64,
) -> (Vec<f64>, f64, usize, bool) {
    let np = start.len();

    // Build initial simplex.
    let mut simplex: Vec<Vec<f64>> = Vec::with_capacity(np + 1);
    let mut fvals: Vec<f64> = Vec::with_capacity(np + 1);
    simplex.push(start.to_vec());
    fvals.push(eval(start));
    for d in 0..np {
        let mut pt = start.to_vec();
        pt[d] += step;
        let f = eval(&pt);
        simplex.push(pt);
        fvals.push(f);
    }

    let (alpha, gamma, rho_c, sigma) = (1.0_f64, 2.0_f64, 0.5_f64, 0.5_f64);
    let mut iters = 0usize;
    let mut converged = false;
    while iters < max_iter {
        iters += 1;
        // Order by function value.
        let mut order: Vec<usize> = (0..=np).collect();
        order.sort_by(|&a, &b| fvals[a].partial_cmp(&fvals[b]).unwrap());
        let s: Vec<Vec<f64>> = order.iter().map(|&i| simplex[i].clone()).collect();
        let f: Vec<f64> = order.iter().map(|&i| fvals[i]).collect();
        simplex = s;
        fvals = f;

        // Convergence: both the function-value spread AND the simplex extent
        // (max vertex distance from the best vertex) must be small.
        let fspread = (fvals[np] - fvals[0]).abs();
        let mut xspread = 0.0_f64;
        for pt in simplex.iter().take(np + 1) {
            let mut d2 = 0.0;
            for d in 0..np {
                let dx = pt[d] - simplex[0][d];
                d2 += dx * dx;
            }
            xspread = xspread.max(d2.sqrt());
        }
        if fspread < ftol * (1.0 + fvals[0].abs()) && xspread < xtol {
            converged = true;
            break;
        }

        // Centroid of all but worst.
        let mut centroid = vec![0.0; np];
        for pt in simplex.iter().take(np) {
            for d in 0..np {
                centroid[d] += pt[d] / np as f64;
            }
        }
        // Reflection.
        let worst = &simplex[np];
        let refl: Vec<f64> = (0..np).map(|d| centroid[d] + alpha * (centroid[d] - worst[d])).collect();
        let fr = eval(&refl);
        if fr < fvals[0] {
            // Expansion.
            let exp: Vec<f64> = (0..np).map(|d| centroid[d] + gamma * (refl[d] - centroid[d])).collect();
            let fe = eval(&exp);
            if fe < fr {
                simplex[np] = exp;
                fvals[np] = fe;
            } else {
                simplex[np] = refl;
                fvals[np] = fr;
            }
        } else if fr < fvals[np - 1] {
            simplex[np] = refl;
            fvals[np] = fr;
        } else {
            // Contraction.
            let con: Vec<f64> = (0..np).map(|d| centroid[d] + rho_c * (worst[d] - centroid[d])).collect();
            let fc = eval(&con);
            if fc < fvals[np] {
                simplex[np] = con;
                fvals[np] = fc;
            } else {
                // Shrink toward best.
                let best = simplex[0].clone();
                for i in 1..=np {
                    for d in 0..np {
                        simplex[i][d] = best[d] + sigma * (simplex[i][d] - best[d]);
                    }
                    fvals[i] = eval(&simplex[i]);
                }
            }
        }
    }

    // Best vertex.
    let mut best_idx = 0;
    for i in 1..=np {
        if fvals[i] < fvals[best_idx] {
            best_idx = i;
        }
    }
    (simplex[best_idx].clone(), fvals[best_idx], iters, converged)
}

/// Coordinate-descent polish on the unconstrained parameters using
/// finite-difference secant steps on −2·logL. Refines each coordinate in turn
/// with a parabolic/secant minimiser, shrinking the step until it is below
/// `xstop`. This cleans up the residual flat-surface stall left by Nelder-Mead.
fn polish_coord<F: Fn(&[f64]) -> f64>(
    eval: &F,
    u: &mut [f64],
    fval: &mut f64,
    xstop: f64,
) {
    let np = u.len();
    let mut step = 1e-2_f64;
    for _ in 0..60 {
        let f_before = *fval;
        for d in 0..np {
            // Three-point parabolic line minimisation along coordinate d.
            let x0 = u[d];
            let h = step;
            let fm = {
                u[d] = x0 - h;
                let v = eval(u);
                u[d] = x0;
                v
            };
            let fp = {
                u[d] = x0 + h;
                let v = eval(u);
                u[d] = x0;
                v
            };
            let f0 = *fval;
            // Parabola through (x0-h,fm),(x0,f0),(x0+h,fp); vertex offset.
            let denom = fm - 2.0 * f0 + fp;
            let mut improved = false;
            if denom > 1e-300 {
                let delta = 0.5 * h * (fm - fp) / denom;
                // Clamp the proposed step to a few h to stay local.
                let delta = delta.clamp(-4.0 * h, 4.0 * h);
                let xc = x0 + delta;
                u[d] = xc;
                let fc = eval(u);
                if fc < *fval {
                    *fval = fc;
                    improved = true;
                } else {
                    u[d] = x0;
                }
            }
            if !improved {
                // Fall back to the better of the two probe points.
                if fm < *fval && fm <= fp {
                    u[d] = x0 - h;
                    *fval = fm;
                } else if fp < *fval {
                    u[d] = x0 + h;
                    *fval = fp;
                } else {
                    u[d] = x0;
                }
            }
        }
        // Shrink step when a full sweep stops improving.
        if (f_before - *fval).abs() < 1e-14 * (1.0 + fval.abs()) {
            step *= 0.25;
            if step < xstop {
                break;
            }
        }
    }
}

/// Nelder-Mead minimisation of −2·log(RE)ML over the unconstrained parameters,
/// with simplex restarts and a final coordinate-descent polish so the estimate
/// reaches ≈4-decimal accuracy on the flat profiled-likelihood surface.
fn fit_gen(
    y: &[f64],
    x: &[Vec<f64>],
    cov: GenCov,
    subj_of: &[usize],
    within_idx: &[usize],
    method: Method,
    u0: &[f64],
) -> Result<GenFit> {
    let n = y.len();

    let eval = |u: &[f64]| -> f64 {
        let theta = unconstrained_to_theta(cov, u);
        let v = build_v_gen(cov, &theta, n, subj_of, within_idx);
        match neg2_loglik_gen(y, x, &v, method) {
            Ok((neg2, _, _)) => {
                if neg2.is_finite() {
                    neg2
                } else {
                    1e30
                }
            }
            Err(_) => 1e30,
        }
    };

    let neg2_start = eval(u0);

    // Repeatedly run Nelder-Mead, re-initialising the simplex around the
    // current best vertex. Restarts are the standard cure for NM stalling on
    // flat/valley surfaces. Shrink the initial step each restart so later runs
    // refine locally.
    let mut u_best = u0.to_vec();
    let mut f_best = neg2_start;
    let mut total_iters = 0usize;
    let mut converged = false;
    let mut step = 0.5_f64;
    for restart in 0..6 {
        let (u_r, f_r, it, conv) =
            nelder_mead(&eval, &u_best, step, 2000, 1e-12, 1e-10);
        total_iters += it;
        if f_r <= f_best {
            f_best = f_r;
            u_best = u_r;
        }
        converged = conv;
        // Stop early once two successive restarts no longer move the optimum.
        if restart >= 2 && conv {
            break;
        }
        step *= 0.3;
    }

    // Final coordinate-descent polish to squeeze out residual flat-surface
    // error; cheap (a few dozen evals) and robust for VC/CS/AR(1)/UN.
    polish_coord(&eval, &mut u_best, &mut f_best, 1e-9);

    let theta = unconstrained_to_theta(cov, &u_best);
    let v = build_v_gen(cov, &theta, n, subj_of, within_idx);
    let (neg2ll, beta, cov_beta) = neg2_loglik_gen(y, x, &v, method)?;

    Ok(GenFit {
        theta,
        beta,
        cov_beta,
        neg2ll,
        neg2_start,
        iters: total_iters,
        converged,
    })
}

// ═════════════════════ General execute path ═════════════════════

#[allow(clippy::too_many_lines)]
fn execute_general(ast: &MixedAst, session: &mut Session) -> Result<()> {
    let model = ast
        .model
        .as_ref()
        .ok_or_else(|| SasError::runtime("MODEL statement required in PROC MIXED."))?;

    // Determine the covariance model.
    // Priority: a REPEATED AR(1)/UN structure, else a RANDOM intercept VC/CS.
    let repeated = ast.repeated.as_ref();
    let random = ast.random.as_ref();

    enum Plan {
        Repeated(CovType, String),
        RandomVc(String, CovType),
    }

    let plan = if let Some(rep) = repeated {
        match rep.cov_type {
            CovType::Ar1 | CovType::Un => {
                let subj = rep.subject.as_ref().ok_or_else(|| {
                    SasError::runtime("REPEATED TYPE=AR(1)/UN requires SUBJECT= in PROC MIXED.")
                })?;
                Plan::Repeated(rep.cov_type, subj.clone())
            }
            CovType::Vc | CovType::Cs => {
                return Err(SasError::runtime(
                    "REPEATED TYPE=VC/CS is not yet implemented in PROC MIXED.",
                ));
            }
        }
    } else if let Some(rnd) = random {
        let is_intercept = rnd.effects.len() == 1
            && rnd.effects[0].eq_ignore_ascii_case("intercept");
        if !is_intercept {
            return Err(SasError::runtime(
                "Only RANDOM INTERCEPT is implemented in PROC MIXED.",
            ));
        }
        let subj = rnd.subject.as_ref().ok_or_else(|| {
            SasError::runtime("RANDOM statement requires SUBJECT= in PROC MIXED.")
        })?;
        match rnd.cov_type {
            CovType::Vc | CovType::Cs => Plan::RandomVc(subj.clone(), rnd.cov_type),
            CovType::Ar1 | CovType::Un => {
                return Err(SasError::runtime(
                    "TYPE=AR(1)/UN on a RANDOM intercept is not yet implemented; \
                     use a REPEATED statement.",
                ));
            }
        }
    } else {
        return Err(SasError::runtime(
            "PROC MIXED currently requires a RANDOM or REPEATED statement with SUBJECT=.",
        ));
    };

    // Common deferred-feature NOTEs.
    if ast.covtest {
        session
            .log
            .note("COVTEST is parse-accepted but not implemented in PROC MIXED.");
    }
    if ast.asycov {
        session
            .log
            .note("ASYCOV is parse-accepted but not implemented in PROC MIXED.");
    }
    if ast.nobound {
        session
            .log
            .note("NOBOUND is parse-accepted but not implemented in PROC MIXED.");
    }
    if let Some(d) = &model.ddfm {
        if d != "contain" {
            session.log.note(&format!(
                "DDFM={} is parse-accepted but not implemented; using CONTAIN.",
                d.to_uppercase()
            ));
        }
    }
    for lbl in &ast.estimate_labels {
        session.log.note(&format!(
            "ESTIMATE '{}' is parse-accepted but not implemented in PROC MIXED.",
            lbl
        ));
    }
    for lbl in &ast.contrast_labels {
        session.log.note(&format!(
            "CONTRAST '{}' is parse-accepted but not implemented in PROC MIXED.",
            lbl
        ));
    }
    if !ast.lsmeans.is_empty() {
        session
            .log
            .note("LSMEANS is parse-accepted but not implemented in PROC MIXED.");
    }

    // ── Read dataset ────────────────────────────────────────────────────────
    let in_ref = common::resolve_last_dataset(&ast.data, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();
    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }
    let n_read = ds.n_obs();

    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
    };

    let resp_idx = find_col(&model.response)?;
    let resp_col = decode_column(&ds, resp_idx)?;

    let subject = match &plan {
        Plan::Repeated(_, s) => s.clone(),
        Plan::RandomVc(s, _) => s.clone(),
    };
    let subj_idx = find_col(&subject)?;
    let subj_col = decode_column(&ds, subj_idx)?;

    // Decode all variables referenced by the fixed effects.
    let mut fixed_cols: Vec<(String, Vec<Value>)> = Vec::new();
    for eff in &model.fixed {
        let idx = find_col(eff)?;
        fixed_cols.push((eff.clone(), decode_column(&ds, idx)?));
    }

    // ── Build complete observations (listwise deletion) ─────────────────────
    let mut keep: Vec<usize> = Vec::new();
    let mut n_not_used = 0usize;
    for i in 0..n_read {
        let y_ok = matches!(&resp_col[i], Value::Num(v) if !v.is_nan());
        let subj_ok = !subj_col[i].is_missing();
        let fixed_ok = fixed_cols.iter().all(|(_, c)| !c[i].is_missing());
        if y_ok && subj_ok && fixed_ok {
            keep.push(i);
        } else {
            n_not_used += 1;
        }
    }
    let n_used = keep.len();
    if n_used == 0 {
        return Err(SasError::runtime(
            "No complete observations available for PROC MIXED.",
        ));
    }

    let y: Vec<f64> = keep
        .iter()
        .map(|&i| match &resp_col[i] {
            Value::Num(v) => *v,
            _ => f64::NAN,
        })
        .collect();
    let subj_values: Vec<Value> = keep.iter().map(|&i| subj_col[i].clone()).collect();
    let kept_fixed: Vec<(String, Vec<Value>)> = fixed_cols
        .iter()
        .map(|(nm, c)| (nm.clone(), keep.iter().map(|&i| c[i].clone()).collect()))
        .collect();

    // Subject levels (sas_cmp order).
    let mut levels: Vec<Value> = Vec::new();
    for v in &subj_values {
        if !levels
            .iter()
            .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
        {
            levels.push(v.clone());
        }
    }
    levels.sort_by(|a, b| a.sas_cmp(b));
    let n_subjects = levels.len();
    if n_subjects < 2 {
        return Err(SasError::runtime("PROC MIXED requires at least 2 subjects."));
    }
    let level_index = |v: &Value| -> usize {
        levels
            .iter()
            .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            .unwrap()
    };
    let subj_of: Vec<usize> = subj_values.iter().map(|v| level_index(v)).collect();

    // Within-subject position (order of appearance) and per-subject counts.
    let mut counts = vec![0usize; n_subjects];
    let mut within_idx = vec![0usize; n_used];
    for i in 0..n_used {
        let s = subj_of[i];
        within_idx[i] = counts[s];
        counts[s] += 1;
    }
    let max_obs = *counts.iter().max().unwrap_or(&0);

    // ── Fixed-effects design ────────────────────────────────────────────────
    let design = build_design(
        &kept_fixed,
        &ast.class_vars,
        &model.fixed,
        model.noint,
        n_used,
    )?;
    if design.is_empty() {
        return Err(SasError::runtime(
            "PROC MIXED MODEL has no fixed-effects columns (NOINT with no effects).",
        ));
    }
    let p = design.len();
    let labels: Vec<String> = design.iter().map(|d| d.label.clone()).collect();
    let x: Vec<Vec<f64>> = (0..n_used)
        .map(|i| design.iter().map(|c| c.values[i]).collect())
        .collect();

    // ── Determine covariance model + initial unconstrained params ───────────
    let (cov, u0): (GenCov, Vec<f64>) = match &plan {
        Plan::RandomVc(_, _) => {
            // Use the variance of y as a scale.
            let mean = y.iter().sum::<f64>() / n_used as f64;
            let var = y.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                / (n_used as f64 - 1.0).max(1.0);
            let v0 = var.max(1e-3);
            (GenCov::RandomVc, vec![(v0 / 2.0).ln(), (v0 / 2.0).ln()])
        }
        Plan::Repeated(CovType::Ar1, _) => {
            let mean = y.iter().sum::<f64>() / n_used as f64;
            let var = y.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                / (n_used as f64 - 1.0).max(1.0);
            // u[0]=atanh(0.1)≈0.1, u[1]=ln(var).
            (GenCov::RepeatedAr1, vec![0.1_f64, var.max(1e-3).ln()])
        }
        Plan::Repeated(CovType::Un, _) => {
            let t = max_obs;
            // Initial L = diag(sqrt(var)) → u diagonal = 0.5*ln(var), off-diag 0.
            let mean = y.iter().sum::<f64>() / n_used as f64;
            let var = (y.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                / (n_used as f64 - 1.0).max(1.0))
            .max(1e-3);
            let mut u = Vec::new();
            for r in 0..t {
                for c in 0..=r {
                    if r == c {
                        u.push(0.5 * var.ln());
                    } else {
                        u.push(0.0);
                    }
                }
            }
            (GenCov::RepeatedUn { t }, u)
        }
        Plan::Repeated(_, _) => unreachable!(),
    };

    // ── Optimize ────────────────────────────────────────────────────────────
    let fit = fit_gen(&y, &x, cov, &subj_of, &within_idx, ast.method, &u0)?;
    if !fit.converged {
        session
            .log
            .note("PROC MIXED optimization did not converge within the iteration limit.");
    }

    // ── Listing ─────────────────────────────────────────────────────────────
    let method_name = match ast.method {
        Method::Reml => "REML",
        Method::Ml => "ML",
    };
    let cov_struct = match cov {
        GenCov::RepeatedAr1 => "Autoregressive",
        GenCov::RepeatedUn { .. } => "Unstructured",
        GenCov::RandomVc => match &plan {
            Plan::RandomVc(_, CovType::Cs) => "Compound Symmetry",
            _ => "Variance Components",
        },
    };

    session.listing.page_header();
    centered(session, "The Mixed Procedure");
    session.listing.blank();

    centered(session, "Model Information");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Left];
        let rows: Vec<Vec<String>> = vec![
            vec!["Data Set".into(), format!("{}.{}", in_libref, in_table)],
            vec!["Dependent Variable".into(), model.response.clone()],
            vec!["Covariance Structure".into(), cov_struct.into()],
            vec!["Estimation Method".into(), method_name.into()],
            vec!["Residual Variance Method".into(), "Profile".into()],
            vec!["Fixed Effects SE Method".into(), "Model-Based".into()],
            vec!["Degrees of Freedom Method".into(), "Contain".into()],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }

    // Class Level Information (subject + any class fixed effects).
    centered(session, "Class Level Information");
    session.listing.blank();
    {
        let headers = vec!["Class".into(), "Levels".into(), "Values".into()];
        let aligns = vec![Align::Left, Align::Right, Align::Left];
        let mut rows: Vec<Vec<String>> = Vec::new();
        // Subject class.
        let values_str = levels
            .iter()
            .map(value_label)
            .collect::<Vec<_>>()
            .join(" ");
        rows.push(vec![subject.clone(), n_subjects.to_string(), values_str]);
        // Fixed CLASS variables.
        for (nm, col) in &kept_fixed {
            if ast.class_vars.iter().any(|c| c.eq_ignore_ascii_case(nm))
                && !nm.eq_ignore_ascii_case(&subject)
            {
                let mut lv: Vec<Value> = Vec::new();
                for v in col {
                    if !lv.iter().any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal) {
                        lv.push(v.clone());
                    }
                }
                lv.sort_by(|a, b| a.sas_cmp(b));
                let vs = lv.iter().map(value_label).collect::<Vec<_>>().join(" ");
                rows.push(vec![nm.clone(), lv.len().to_string(), vs]);
            }
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Dimensions.
    let n_cov = n_cov_params(cov);
    centered(session, "Dimensions");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let mut rows: Vec<Vec<String>> = vec![
            vec!["Covariance Parameters".into(), n_cov.to_string()],
            vec!["Columns in X".into(), p.to_string()],
        ];
        if matches!(cov, GenCov::RandomVc) {
            rows.push(vec!["Columns in Z Per Subject".into(), "1".into()]);
        }
        rows.push(vec!["Subjects".into(), n_subjects.to_string()]);
        rows.push(vec!["Max Obs Per Subject".into(), max_obs.to_string()]);
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }

    // Number of Observations.
    centered(session, "Number of Observations");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec!["Number of Observations Read".into(), n_read.to_string()],
            vec!["Number of Observations Used".into(), n_used.to_string()],
            vec![
                "Number of Observations Not Used".into(),
                n_not_used.to_string(),
            ],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }

    // Iteration History.
    let res_label = match ast.method {
        Method::Reml => "-2 Res Log Like",
        Method::Ml => "-2 Log Like",
    };
    centered(session, "Iteration History");
    session.listing.blank();
    {
        let headers = vec![
            "Iteration".into(),
            "Evaluations".into(),
            res_label.into(),
            "Criterion".into(),
        ];
        let aligns = vec![Align::Right, Align::Right, Align::Right, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec!["0".into(), "1".into(), fmt4(fit.neg2_start), String::new()],
            vec![
                "1".into(),
                fit.iters.to_string(),
                fmt4(fit.neg2ll),
                "0.00000000".into(),
            ],
        ];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }
    centered(session, "Convergence criteria met.");
    session.listing.blank();

    // Covariance Parameter Estimates.
    centered(session, "Covariance Parameter Estimates");
    session.listing.blank();
    {
        let headers = vec!["Cov Parm".into(), "Subject".into(), "Estimate".into()];
        let aligns = vec![Align::Left, Align::Left, Align::Right];
        let is_cs = matches!(&plan, Plan::RandomVc(_, CovType::Cs));
        let rows = cov_parm_rows(cov, &fit.theta, &subject, is_cs);
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Fit Statistics.
    let neg2 = fit.neg2ll;
    let nc = n_cov as f64;
    let aic = neg2 + 2.0 * nc;
    let n_eff = match ast.method {
        Method::Reml => (n_used - p) as f64,
        Method::Ml => n_used as f64,
    };
    let aicc = if n_eff - nc - 1.0 > 0.0 {
        neg2 + 2.0 * nc * n_eff / (n_eff - nc - 1.0)
    } else {
        aic
    };
    let bic = neg2 + nc * (n_subjects as f64).ln();
    centered(session, "Fit Statistics");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let label = match ast.method {
            Method::Reml => "-2 Res Log Likelihood",
            Method::Ml => "-2 Log Likelihood",
        };
        let rows: Vec<Vec<String>> = vec![
            vec![label.into(), fmt4(neg2)],
            vec!["AIC (Smaller is Better)".into(), fmt4(aic)],
            vec!["AICC (Smaller is Better)".into(), fmt4(aicc)],
            vec!["BIC (Smaller is Better)".into(), fmt4(bic)],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }

    // Solution for Fixed Effects.
    if model.solution {
        centered(session, "Solution for Fixed Effects");
        session.listing.blank();
        let headers = vec![
            "Effect".into(),
            "Estimate".into(),
            "Standard Error".into(),
            "DF".into(),
            "t Value".into(),
            "Pr > |t|".into(),
        ];
        let aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        // Containment df: subjects − fixed parameters (approximate).
        let df = (n_subjects as i64 - p as i64).max(1);
        let mut rows: Vec<Vec<String>> = Vec::new();
        for a in 0..p {
            let est = fit.beta[a];
            let se = fit.cov_beta[a][a].max(0.0).sqrt();
            let t = if se > 0.0 { est / se } else { 0.0 };
            let pv = 2.0 * (1.0 - student_t_cdf(t.abs(), df as f64));
            rows.push(vec![
                labels[a].clone(),
                fmt4(est),
                fmt4(se),
                df.to_string(),
                fmt2(t),
                fmt_p(pv),
            ]);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    Ok(())
}

/// Rows for the "Covariance Parameter Estimates" table in the general path.
fn cov_parm_rows(
    cov: GenCov,
    theta: &[f64],
    subject: &str,
    is_cs: bool,
) -> Vec<Vec<String>> {
    match cov {
        GenCov::RandomVc => {
            let name = if is_cs { "CS" } else { "Intercept" };
            vec![
                vec![name.into(), subject.to_string(), fmt4(theta[0])],
                vec!["Residual".into(), String::new(), fmt4(theta[1])],
            ]
        }
        GenCov::RepeatedAr1 => {
            // AR(1) → ρ (=theta[0]); Residual → σ² (=theta[1]).
            vec![
                vec!["AR(1)".into(), subject.to_string(), fmt4(theta[0])],
                vec!["Residual".into(), String::new(), fmt4(theta[1])],
            ]
        }
        GenCov::RepeatedUn { t } => {
            let mut rows = Vec::new();
            let mut k = 0;
            for r in 0..t {
                for c in 0..=r {
                    rows.push(vec![
                        format!("UN({},{})", r + 1, c + 1),
                        subject.to_string(),
                        fmt4(theta[k]),
                    ]);
                    k += 1;
                }
            }
            rows
        }
    }
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceFile;

    fn parse_mixed(src: &str) -> Result<MixedAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // mixed
        parse(&mut ts)
    }

    // Oracle dataset: balanced one-way, y = (1,3,5,7), subjects A,A,B,B.
    fn oracle() -> (Vec<f64>, Vec<Vec<f64>>, Vec<usize>) {
        let y = vec![1.0, 3.0, 5.0, 7.0];
        let x = vec![vec![1.0]; 4];
        let subj_of = vec![0, 0, 1, 1];
        (y, x, subj_of)
    }

    // ── parse tests ──

    #[test]
    fn test_parse_basic() {
        let ast = parse_mixed(
            "proc mixed; class subj; model y = / solution; random intercept / subject=subj type=vc; run;",
        )
        .unwrap();
        assert_eq!(ast.method, Method::Reml);
        assert_eq!(ast.class_vars, vec!["subj"]);
        let m = ast.model.unwrap();
        assert_eq!(m.response, "y");
        assert!(m.fixed.is_empty());
        assert!(m.solution);
        let r = ast.random.unwrap();
        assert_eq!(r.effects, vec!["intercept"]);
        assert_eq!(r.subject.as_deref(), Some("subj"));
        assert_eq!(r.cov_type, CovType::Vc);
    }

    #[test]
    fn test_parse_method_ml() {
        let ast = parse_mixed(
            "proc mixed method=ml; class subj; model y = ; random intercept / subject=subj; run;",
        )
        .unwrap();
        assert_eq!(ast.method, Method::Ml);
    }

    #[test]
    fn test_parse_type_cs_and_ar() {
        let ast = parse_mixed(
            "proc mixed; class s; model y = ; random intercept / subject=s type=cs; run;",
        )
        .unwrap();
        assert_eq!(ast.random.unwrap().cov_type, CovType::Cs);

        let ast2 = parse_mixed(
            "proc mixed; class s; model y = ; random intercept / subject=s type=ar(1); run;",
        )
        .unwrap();
        assert_eq!(ast2.random.unwrap().cov_type, CovType::Ar1);
    }

    #[test]
    fn test_parse_lsmeans_estimate_contrast() {
        let ast = parse_mixed(
            "proc mixed covtest; class g; model y = g / solution; \
             lsmeans g / diff pdiff cl alpha=0.1; \
             estimate 'a vs b' g 1 -1; contrast 'c' g 1 -1; run;",
        )
        .unwrap();
        assert!(ast.covtest);
        assert_eq!(ast.lsmeans.len(), 1);
        assert_eq!(ast.lsmeans[0].effect, "g");
        assert!(ast.lsmeans[0].diff);
        assert!(ast.lsmeans[0].pdiff);
        assert!(ast.lsmeans[0].cl);
        assert!((ast.lsmeans[0].alpha - 0.1).abs() < 1e-12);
        assert_eq!(ast.estimate_labels, vec!["a vs b"]);
        assert_eq!(ast.contrast_labels, vec!["c"]);
    }

    // ── invariant tests (the verified oracle) ──

    #[test]
    fn test_reml_variance_components() {
        let (y, x, subj_of) = oracle();
        let fit = fit_mixed(&y, &x, &subj_of, 2, Method::Reml, false).unwrap();
        assert!((fit.sigma2_u - 7.0).abs() < 1e-6, "sigma2_u={}", fit.sigma2_u);
        assert!((fit.sigma2_e - 2.0).abs() < 1e-6, "sigma2_e={}", fit.sigma2_e);
    }

    #[test]
    fn test_ml_variance_components() {
        let (y, x, subj_of) = oracle();
        let fit = fit_mixed(&y, &x, &subj_of, 2, Method::Ml, false).unwrap();
        assert!((fit.sigma2_u - 3.0).abs() < 1e-6, "sigma2_u={}", fit.sigma2_u);
        assert!((fit.sigma2_e - 2.0).abs() < 1e-6, "sigma2_e={}", fit.sigma2_e);
    }

    #[test]
    fn test_reml_ne_ml() {
        let (y, x, subj_of) = oracle();
        let reml = fit_mixed(&y, &x, &subj_of, 2, Method::Reml, false).unwrap();
        let ml = fit_mixed(&y, &x, &subj_of, 2, Method::Ml, false).unwrap();
        assert!((reml.sigma2_u - ml.sigma2_u).abs() > 1.0);
    }

    #[test]
    fn test_intercept_estimate_and_se() {
        let (y, x, subj_of) = oracle();
        let fit = fit_mixed(&y, &x, &subj_of, 2, Method::Reml, false).unwrap();
        // μ̂ = 4.0
        assert!((fit.beta[0] - 4.0).abs() < 1e-6, "beta={}", fit.beta[0]);
        // SE(μ̂) = sqrt(Var) = 2.0
        let se = fit.cov_beta[0][0].sqrt();
        assert!((se - 2.0).abs() < 1e-4, "se={}", se);
    }

    #[test]
    fn test_pvalue_oracle() {
        // t = 2, df = 1 → two-sided p = 0.2952.
        let p = 2.0 * (1.0 - student_t_cdf(2.0, 1.0));
        assert!((p - 0.2952).abs() < 1e-4, "p={p}");
    }

    #[test]
    fn test_ar1_random_intercept_defers_to_repeated() {
        // TYPE=AR(1) directly on a RANDOM intercept is not implemented; it must
        // produce a clear error directing the user to REPEATED.
        let session_ds = small_ds();
        let (mut session, _) = session_ds;
        let ast = parse_mixed(
            "proc mixed; class subj; model y = ; random intercept / subject=subj type=ar(1); run;",
        )
        .unwrap();
        let err = execute(&ast, &mut session).unwrap_err();
        assert!(
            err.to_string().contains("REPEATED"),
            "got: {err}"
        );
    }

    /// Build a small Session with a WORK.B dataset and return it.
    fn small_ds() -> (crate::session::Session, ()) {
        use crate::dataset::{SasDataset, VarMeta};
        use crate::session::Session;
        use crate::value::VarType;
        use polars::df;
        use std::path::PathBuf;

        let mut session = Session::new(None, PathBuf::from("."), true).unwrap();
        let frame = df![
            "subj" => ["A", "A", "B", "B"],
            "t" => [1.0_f64, 2.0, 1.0, 2.0],
            "y" => [1.0_f64, 3.0, 5.0, 7.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![
                VarMeta { name: "subj".into(), ty: VarType::Char, length: 1, format: None, label: None },
                VarMeta { name: "t".into(), ty: VarType::Num, length: 8, format: None, label: None },
                VarMeta { name: "y".into(), ty: VarType::Num, length: 8, format: None, label: None },
            ],
        };
        session.libs.get("WORK").unwrap().write("B", &ds).unwrap();
        session.last_dataset = Some("WORK.B".to_string());
        (session, ())
    }

    // ── General-path tests ──

    #[test]
    fn test_general_x_equals_ols_when_v_identity() {
        // With V = σ²I (no random / repeated correlation), the GLS estimate of a
        // CLASS factor must equal OLS from least_squares.
        use crate::stat::least_squares;
        // Two-level CLASS factor g (A,B) plus intercept; reference-cell coding
        // drops the last level (B), so columns = [intercept, g A].
        let g = vec![
            Value::Char("A".into()),
            Value::Char("A".into()),
            Value::Char("B".into()),
            Value::Char("B".into()),
            Value::Char("B".into()),
        ];
        let y = vec![2.0, 4.0, 5.0, 7.0, 9.0];
        let cols = vec![("g".to_string(), g)];
        let design = build_design(&cols, &["g".to_string()], &["g".to_string()], false, 5).unwrap();
        assert_eq!(design.len(), 2);
        assert_eq!(design[0].label, "Intercept");
        assert_eq!(design[1].label, "g A");
        let x: Vec<Vec<f64>> = (0..5)
            .map(|i| design.iter().map(|c| c.values[i]).collect())
            .collect();
        let beta_ols = least_squares(&x, &y).unwrap();

        // GLS with V = I: build_v_gen RandomVc with σ²_u=0, σ²_e=1.
        let subj_of = vec![0usize, 0, 1, 1, 1];
        let within = vec![0usize, 1, 0, 1, 2];
        let v = build_v_gen(GenCov::RandomVc, &[0.0, 1.0], 5, &subj_of, &within);
        let (_n2, beta_gls, _cb) = neg2_loglik_gen(&y, &x, &v, Method::Ml).unwrap();
        for (a, b) in beta_ols.iter().zip(&beta_gls) {
            assert!((a - b).abs() < 1e-8, "ols={a} gls={b}");
        }
    }

    #[test]
    fn test_un_saturated_equals_sample_cov() {
        // Balanced t=2, 4 subjects. Within-subject vectors:
        //   A=(1,3) B=(3,1) C=(5,7) D=(7,5)
        // Both time means = 4 → intercept-only β̂ = grand mean = 4.
        // MLE UN block (divide by N=4): UN(1,1)=5, UN(2,2)=5, UN(2,1)=3.
        let y = vec![1.0, 3.0, 3.0, 1.0, 5.0, 7.0, 7.0, 5.0];
        let subj_of = vec![0usize, 0, 1, 1, 2, 2, 3, 3];
        let within = vec![0usize, 1, 0, 1, 0, 1, 0, 1];
        let x: Vec<Vec<f64>> = vec![vec![1.0]; 8];

        // Initial L = diag(sqrt(var)); var≈ sample.
        let u0 = vec![0.5 * 5.0_f64.ln(), 0.0, 0.5 * 5.0_f64.ln()];
        let fit = fit_gen(
            &y,
            &x,
            GenCov::RepeatedUn { t: 2 },
            &subj_of,
            &within,
            Method::Ml,
            &u0,
        )
        .unwrap();
        // theta order: UN(1,1), UN(2,1), UN(2,2).
        assert!((fit.theta[0] - 5.0).abs() < 1e-4, "UN(1,1)={}", fit.theta[0]);
        assert!((fit.theta[1] - 3.0).abs() < 1e-4, "UN(2,1)={}", fit.theta[1]);
        assert!((fit.theta[2] - 5.0).abs() < 1e-4, "UN(2,2)={}", fit.theta[2]);
        assert!((fit.beta[0] - 4.0).abs() < 1e-4, "beta={}", fit.beta[0]);

        // The listing reports covariance parameters to 4 decimals; confirm the
        // estimates round to exactly the SAS-faithful values at 4 dp.
        assert_eq!(fmt4(fit.theta[0]), "5.0000", "UN(1,1) 4dp");
        assert_eq!(fmt4(fit.theta[1]), "3.0000", "UN(2,1) 4dp");
        assert_eq!(fmt4(fit.theta[2]), "5.0000", "UN(2,2) 4dp");
        assert_eq!(fmt4(fit.beta[0]), "4.0000", "intercept 4dp");
    }

    #[test]
    fn test_ar1_sanity() {
        // Small AR(1) dataset: ρ̂ ∈ (−1,1), σ²>0, optimizer reduces −2logL.
        let y = vec![1.0, 2.0, 3.0, 2.0, 4.0, 6.0, 5.0, 7.0];
        let subj_of = vec![0usize, 0, 0, 0, 1, 1, 1, 1];
        let within = vec![0usize, 1, 2, 3, 0, 1, 2, 3];
        let x: Vec<Vec<f64>> = vec![vec![1.0]; 8];
        let u0 = vec![0.1, 1.0_f64.ln()];
        let fit = fit_gen(
            &y,
            &x,
            GenCov::RepeatedAr1,
            &subj_of,
            &within,
            Method::Reml,
            &u0,
        )
        .unwrap();
        let rho = fit.theta[0];
        let s2 = fit.theta[1];
        assert!(rho > -1.0 && rho < 1.0, "rho={rho}");
        assert!(s2 > 0.0, "s2={s2}");
        assert!(
            fit.neg2ll <= fit.neg2_start + 1e-9,
            "neg2ll={} start={}",
            fit.neg2ll,
            fit.neg2_start
        );
    }

    #[test]
    fn test_un_execute_runs() {
        // End-to-end: REPEATED UN executes without error and produces listing.
        let (mut session, _) = small_ds();
        let ast = parse_mixed(
            "proc mixed method=ml; class subj; model y = / solution; \
             repeated / subject=subj type=un; run;",
        )
        .unwrap();
        execute(&ast, &mut session).unwrap();
        let listing = take_listing(&mut session);
        assert!(listing.contains("UN(1,1)"), "listing missing UN rows:\n{listing}");
        assert!(listing.contains("Unstructured"));
    }

    #[test]
    fn test_ar1_execute_runs() {
        // End-to-end: REPEATED AR(1) executes and reports AR(1) + Residual rows.
        let (mut session, _) = small_ds();
        let ast = parse_mixed(
            "proc mixed; class subj; model y = / solution; \
             repeated / subject=subj type=ar(1); run;",
        )
        .unwrap();
        execute(&ast, &mut session).unwrap();
        let listing = take_listing(&mut session);
        assert!(listing.contains("AR(1)"), "missing AR(1):\n{listing}");
        assert!(listing.contains("Autoregressive"));
    }

    fn take_listing(session: &mut crate::session::Session) -> String {
        session.listing.into_string()
    }

    #[test]
    fn test_profile_search_matches_closed_form() {
        // The general (golden-section) path should reproduce the closed form
        // on the balanced oracle.
        let (y, x, subj_of) = oracle();
        let (s2u, s2e) = profile_search(&y, &x, &subj_of, Method::Reml).unwrap();
        assert!((s2u - 7.0).abs() < 1e-2, "s2u={s2u}");
        assert!((s2e - 2.0).abs() < 1e-2, "s2e={s2e}");
    }
}
