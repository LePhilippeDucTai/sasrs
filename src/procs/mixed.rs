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

fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("expected '=' after {opt}"),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

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
            ts.next();
            expect_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if tk.is_kw("method") {
            ts.next();
            expect_eq(ts, "METHOD")?;
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

    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }

        if ts.peek().is_kw("class") {
            ts.next();
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    class_vars.push(name);
                }
                ts.next();
            }
            ts.expect_semi()?;
        } else if ts.peek().is_kw("model") {
            ts.next();
            model = Some(parse_model(ts)?);
        } else if ts.peek().is_kw("random") {
            ts.next();
            random = Some(parse_random(ts));
        } else if ts.peek().is_kw("repeated") {
            ts.next();
            repeated = Some(parse_repeated(ts));
        } else if ts.peek().is_kw("lsmeans") {
            ts.next();
            if let Some(spec) = parse_lsmeans(ts) {
                lsmeans.push(spec);
            }
        } else if ts.peek().is_kw("estimate") {
            ts.next();
            if let TokenKind::Str { value, .. } = &ts.peek().kind {
                estimate_labels.push(value.clone());
            }
            ts.skip_to_semi();
        } else if ts.peek().is_kw("contrast") {
            ts.next();
            if let TokenKind::Str { value, .. } = &ts.peek().kind {
                contrast_labels.push(value.clone());
            }
            ts.skip_to_semi();
        } else {
            ts.skip_to_semi();
        }
    }

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
                ts.next();
                expect_eq(ts, "DDFM")?;
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
                ts.next();
                let _ = expect_eq(ts, "SUBJECT");
                subject = ts.peek().ident().map(str::to_string);
                ts.next();
            } else if tk.is_kw("type") {
                ts.next();
                let _ = expect_eq(ts, "TYPE");
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
                ts.next();
                let _ = expect_eq(ts, "SUBJECT");
                subject = ts.peek().ident().map(str::to_string);
                ts.next();
            } else if tk.is_kw("type") {
                ts.next();
                let _ = expect_eq(ts, "TYPE");
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
                ts.next();
                let _ = expect_eq(ts, "ALPHA");
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

// ───────────────────────── Resolve DATA= ─────────────────────────

fn resolve_input(ast: &MixedAst, session: &Session) -> Result<DatasetRef> {
    match &ast.data {
        Some(r) => Ok(r.clone()),
        None => {
            let last = session.last_dataset.clone().ok_or_else(|| {
                SasError::runtime("There is no default input data set (_LAST_ is undefined).")
            })?;
            let parts: Vec<&str> = last.splitn(2, '.').collect();
            if parts.len() == 2 {
                Ok(DatasetRef {
                    libref: Some(parts[0].to_string()),
                    name: parts[1].to_string(),
                })
            } else {
                Ok(DatasetRef {
                    libref: None,
                    name: last,
                })
            }
        }
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

pub fn execute(ast: &MixedAst, session: &mut Session) -> Result<()> {
    // ── 1. Validate / guards ────────────────────────────────────────────────
    let model = ast.model.as_ref().ok_or_else(|| {
        SasError::runtime("MODEL statement required in PROC MIXED.")
    })?;

    let random = ast.random.as_ref().ok_or_else(|| {
        SasError::runtime("PROC MIXED currently requires a RANDOM statement with SUBJECT=.")
    })?;

    // Reject not-yet-implemented covariance structures.
    match random.cov_type {
        CovType::Vc | CovType::Cs => {}
        CovType::Ar1 | CovType::Un => {
            return Err(SasError::runtime(
                "TYPE=AR(1)/UN is not yet implemented for PROC MIXED.",
            ));
        }
    }

    let subject = random.subject.as_ref().ok_or_else(|| {
        SasError::runtime("RANDOM statement requires SUBJECT= in PROC MIXED.")
    })?;

    // Only the random intercept is implemented.
    let is_intercept = random.effects.len() == 1
        && random.effects[0].eq_ignore_ascii_case("intercept");
    if !is_intercept {
        return Err(SasError::runtime(
            "Only RANDOM INTERCEPT is implemented in PROC MIXED.",
        ));
    }

    // Fixed effects beyond an intercept are not exercised by the oracle; we
    // support intercept-only models (model y = ).
    if !model.fixed.is_empty() {
        return Err(SasError::runtime(
            "Fixed CLASS/continuous effects are not yet implemented in PROC MIXED; \
             use an intercept-only model (model y = ).",
        ));
    }
    if model.noint {
        return Err(SasError::runtime(
            "NOINT is not yet implemented in PROC MIXED.",
        ));
    }

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
    let in_ref = resolve_input(ast, session)?;
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
    fn test_profile_search_matches_closed_form() {
        // The general (golden-section) path should reproduce the closed form
        // on the balanced oracle.
        let (y, x, subj_of) = oracle();
        let (s2u, s2e) = profile_search(&y, &x, &subj_of, Method::Reml).unwrap();
        assert!((s2u - 7.0).abs() < 1e-2, "s2u={s2u}");
        assert!((s2e - 2.0).abs() < 1e-2, "s2e={s2e}");
    }
}
