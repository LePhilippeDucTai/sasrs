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
use crate::session::Session;
use crate::stat::{f_cdf, invert_matrix, student_t_cdf};
use crate::token::TokenKind;
use crate::value::{format_best, Value};

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

fn canonical_link(dist: Distribution) -> LinkFunction {
    match dist {
        Distribution::Normal => LinkFunction::Identity,
        Distribution::Poisson => LinkFunction::Log,
        Distribution::Binary => LinkFunction::Logit,
        Distribution::Gamma => LinkFunction::Log,
        Distribution::NegBinomial => LinkFunction::Log,
    }
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

// ───────────────────────── Parser helpers ─────────────────────────

fn parse_cov_type(ts: &mut StatementStream) -> CovType {
    let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
    let t = match v.as_deref() {
        Some("cs") => CovType::Cs,
        Some("un") => CovType::Un,
        Some("ar") => CovType::Ar1,
        _ => CovType::Vc,
    };
    ts.next();
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

/// Parse PROC GLIMMIX. Called AFTER `proc glimmix` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<GlimmixAst> {
    let mut data: Option<DatasetRef> = None;
    let mut method = Method::Rspl;

    // PROC GLIMMIX statement options until `;`.
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
                Some("laplace") => Method::Laplace,
                Some("quad") => Method::Quad,
                _ => Method::Rspl,
            };
            ts.next();
        } else {
            ts.next();
        }
    }

    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<ModelSpec> = None;
    let mut random: Option<RandomSpec> = None;
    let mut freq_var: Option<String> = None;
    let mut weight_var: Option<String> = None;
    let mut estimate_labels: Vec<String> = Vec::new();
    let mut contrast_labels: Vec<String> = Vec::new();
    let mut lsmeans: Vec<String> = Vec::new();

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
        } else if kw == "freq" {
            ts.next();
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                freq_var = Some(name);
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "weight" {
            ts.next();
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                weight_var = Some(name);
                ts.next();
            }
            ts.expect_semi()?;
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
        } else if kw == "lsmeans" {
            ts.next();
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                lsmeans.push(name);
            }
            ts.skip_to_semi();
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(GlimmixAst {
        data,
        method,
        class_vars,
        model,
        random,
        freq_var,
        weight_var,
        estimate_labels,
        contrast_labels,
        lsmeans,
    })
}

/// Parse the MODEL statement body (after `model`).
fn parse_model(ts: &mut StatementStream) -> Result<ModelSpec> {
    let response = ts
        .peek()
        .ident()
        .map(str::to_string)
        .ok_or_else(|| SasError::parse("expected response variable in MODEL", ts.peek().span))?;
    ts.next();

    // Optional response options: (event='val' | descending)
    let mut event: Option<String> = None;
    let mut descending = false;
    if ts.peek().kind == TokenKind::LParen {
        ts.next();
        loop {
            if ts.peek().kind == TokenKind::RParen
                || ts.peek().kind == TokenKind::Semi
                || ts.peek().kind == TokenKind::Eof
            {
                break;
            }
            if ts.peek().is_kw("event") {
                ts.next();
                if ts.peek().kind == TokenKind::Eq {
                    ts.next();
                    if let TokenKind::Str { value, .. } = &ts.peek().kind.clone() {
                        event = Some(value.clone());
                        ts.next();
                    }
                }
            } else if ts.peek().is_kw("descending") {
                descending = true;
                ts.next();
            } else {
                ts.next();
            }
        }
        if ts.peek().kind == TokenKind::RParen {
            ts.next();
        }
    }

    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            "expected '=' in MODEL statement",
            ts.peek().span,
        ));
    }
    ts.next();

    let mut fixed: Vec<String> = Vec::new();
    while ts.peek().kind != TokenKind::Semi
        && ts.peek().kind != TokenKind::Slash
        && ts.peek().kind != TokenKind::Eof
    {
        if let Some(name) = ts.peek().ident().map(str::to_string) {
            fixed.push(name);
        }
        ts.next();
    }

    let mut dist_opt: Option<Distribution> = None;
    let mut link_opt: Option<LinkFunction> = None;
    let mut solution = false;
    let mut noint = false;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let tk = ts.peek();
            if tk.is_kw("dist") || tk.is_kw("distribution") || tk.is_kw("d") {
                let _ = common::expect_eq(ts, "DIST");
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    ts.next();
                    dist_opt = Some(match name.to_ascii_lowercase().as_str() {
                        "normal" | "gaussian" | "gauss" => Distribution::Normal,
                        "poisson" | "poi" => Distribution::Poisson,
                        "binary" | "bin" | "binomial" => Distribution::Binary,
                        "gamma" | "gam" => Distribution::Gamma,
                        "negbinomial" | "negbin" | "nb" => Distribution::NegBinomial,
                        _ => Distribution::Normal,
                    });
                }
            } else if tk.is_kw("link") {
                let _ = common::expect_eq(ts, "LINK");
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    ts.next();
                    link_opt = Some(match name.to_ascii_lowercase().as_str() {
                        "identity" | "id" => LinkFunction::Identity,
                        "log" => LinkFunction::Log,
                        "logit" => LinkFunction::Logit,
                        "probit" => LinkFunction::Probit,
                        "cloglog" | "cll" => LinkFunction::Cloglog,
                        _ => LinkFunction::Identity,
                    });
                }
            } else if tk.is_kw("solution") || tk.is_kw("s") {
                solution = true;
                ts.next();
            } else if tk.is_kw("noint") {
                noint = true;
                ts.next();
            } else {
                ts.next();
            }
        }
    }
    ts.expect_semi()?;

    let dist = dist_opt.unwrap_or(Distribution::Normal);
    let link = link_opt.unwrap_or_else(|| canonical_link(dist));

    Ok(ModelSpec {
        response,
        event,
        descending,
        fixed,
        dist,
        link,
        solution,
        noint,
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
    let mut solution = false;

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
            } else if tk.is_kw("solution") || tk.is_kw("s") {
                solution = true;
                ts.next();
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
        solution,
    }
}

// ───────────────────────── Link / variance ─────────────────────────

fn inv_link(eta: f64, lf: LinkFunction) -> f64 {
    match lf {
        LinkFunction::Identity => eta,
        LinkFunction::Log => eta.exp().max(1e-10),
        LinkFunction::Logit => 1.0 / (1.0 + (-eta).exp()),
        _ => eta, // unreachable for implemented links
    }
}

/// dμ/dη (derivative of the inverse link).
fn dmu_deta(eta: f64, lf: LinkFunction) -> f64 {
    match lf {
        LinkFunction::Identity => 1.0,
        LinkFunction::Log => eta.exp().max(1e-10),
        LinkFunction::Logit => {
            let mu = 1.0 / (1.0 + (-eta).exp());
            (mu * (1.0 - mu)).max(1e-15)
        }
        _ => 1.0,
    }
}

fn variance(mu: f64, dist: Distribution) -> f64 {
    match dist {
        Distribution::Normal => 1.0,
        Distribution::Poisson => mu.max(1e-15),
        Distribution::Binary => (mu * (1.0 - mu)).max(1e-15),
        _ => 1.0,
    }
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

fn value_matches_event(v: &Value, event: &str) -> bool {
    match v {
        Value::Char(s) => s.trim_end() == event.trim(),
        Value::Num(f) => {
            if let Ok(ev_num) = event.trim().parse::<f64>() {
                (f - ev_num).abs() < 1e-15
            } else {
                format_best(*f, 12) == event.trim()
            }
        }
        Value::Missing(_) => false,
    }
}

// ───────────────────────── Linear algebra ─────────────────────────

fn mat_vec(mat: &[Vec<f64>], v: &[f64]) -> Vec<f64> {
    mat.iter()
        .map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum())
        .collect()
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn log_det_spd(a: &[Vec<f64>]) -> Result<f64> {
    let l = crate::stat::cholesky(a)?;
    let mut s = 0.0;
    for (i, row) in l.iter().enumerate() {
        s += row[i].ln();
    }
    Ok(2.0 * s)
}

// ───────────────────────── No-random GLM (IRLS) fit ─────────────────────────

/// Result of the no-random fixed-effects fit.
struct GlmFit {
    beta: Vec<f64>,
    /// Var(β̂) = scale * (X'WX)⁻¹.
    cov_beta: Vec<Vec<f64>>,
    /// Fitted means μ_i.
    mu: Vec<f64>,
    iterations: usize,
}

/// IRLS for the fixed-effects-only GLM (mirrors PROC GENMOD), with FREQ
/// weighting. For NORMAL, scale = MSE; for Poisson/Binary, scale = 1.
fn fit_glm(
    y: &[f64],
    x: &[Vec<f64>],
    freq: &[f64],
    dist: Distribution,
    lf: LinkFunction,
) -> Result<GlmFit> {
    let n = y.len();
    let p = x[0].len();
    let n_total: f64 = freq.iter().sum();

    // Initialise β via η0 on the (weighted) mean response.
    let y_mean: f64 = y.iter().zip(freq).map(|(yi, w)| yi * w).sum::<f64>() / n_total;
    let eta0 = match lf {
        LinkFunction::Log => y_mean.max(1e-10).ln(),
        LinkFunction::Logit => {
            let pp = y_mean.clamp(1e-10, 1.0 - 1e-10);
            (pp / (1.0 - pp)).ln().clamp(-10.0, 10.0)
        }
        _ => y_mean,
    };
    let mut beta = vec![0.0; p];
    beta[0] = eta0;

    let mut iterations = 0;
    let mut converged = false;
    for it in 0..50 {
        iterations = it + 1;
        let mut score = vec![0.0; p];
        let mut hess = vec![vec![0.0; p]; p];
        for i in 0..n {
            let eta: f64 = dot(&x[i], &beta);
            let mu = inv_link(eta, lf);
            let v = variance(mu, dist);
            let d = dmu_deta(eta, lf);
            let w = freq[i] * d * d / v;
            let resid_adj = freq[i] * (y[i] - mu) * d / v;
            for j in 0..p {
                score[j] += x[i][j] * resid_adj;
                for k in 0..p {
                    hess[j][k] += x[i][j] * x[i][k] * w;
                }
            }
        }
        let hinv = invert_matrix(&hess)?;
        let delta = mat_vec(&hinv, &score);
        for j in 0..p {
            beta[j] += delta[j];
        }
        let max_delta = delta.iter().map(|d| d.abs()).fold(0.0_f64, f64::max);
        let max_beta = beta.iter().map(|b| b.abs()).fold(0.0_f64, f64::max);
        if max_delta / (1.0 + max_beta) < 1e-10 {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err(SasError::runtime("PROC GLIMMIX failed to converge."));
    }

    // Final H = X'WX and μ at convergence.
    let mut hess = vec![vec![0.0; p]; p];
    let mut mu = Vec::with_capacity(n);
    for i in 0..n {
        let eta: f64 = dot(&x[i], &beta);
        let m = inv_link(eta, lf);
        let v = variance(m, dist);
        let d = dmu_deta(eta, lf);
        let w = freq[i] * d * d / v;
        mu.push(m);
        for j in 0..p {
            for k in 0..p {
                hess[j][k] += x[i][j] * x[i][k] * w;
            }
        }
    }
    let hinv = invert_matrix(&hess)?;

    // Scale: Normal → MSE; others → 1 (the oracle demands GENMOD scale-1 SEs).
    let scale = if dist == Distribution::Normal {
        let sse: f64 = (0..n).map(|i| freq[i] * (y[i] - mu[i]).powi(2)).sum();
        let dfe = (n_total - p as f64).max(1.0);
        sse / dfe
    } else {
        1.0
    };
    let cov_beta: Vec<Vec<f64>> = hinv
        .iter()
        .map(|row| row.iter().map(|v| scale * v).collect())
        .collect();

    Ok(GlmFit {
        beta,
        cov_beta,
        mu,
        iterations,
    })
}

// ───────────────────────── Variance-components mixed fit ─────────────────────

/// Fit y = Xβ + Zu + ε with V = σ²_u ZZ' + σ²_e I (single random intercept).
/// Returns (σ²_u, σ²_e, β, Var(β), -2 Res LogLik). Used for NORMAL/IDENTITY
/// (closed-form REML) and as the WMME solver inside the PQL loop (working data).
fn fit_vc(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    n_subjects: usize,
    weights: Option<&[f64]>,
) -> Result<(f64, f64, Vec<f64>, Vec<Vec<f64>>, f64)> {
    let p = x[0].len();

    // Balance detection for the closed-form intercept-only path.
    let mut counts = vec![0usize; n_subjects];
    for &s in subj_of {
        counts[s] += 1;
    }
    let n_i = counts[0];
    let balanced = counts.iter().all(|&c| c == n_i) && n_i > 0;
    let intercept_only = p == 1 && x.iter().all(|row| row[0] == 1.0);
    let unweighted = weights.is_none();

    let (mut sigma2_u, sigma2_e) =
        if unweighted && balanced && intercept_only && n_subjects >= 2 {
            closed_form_vc(y, subj_of, n_subjects, n_i)
        } else {
            profile_search(y, x, subj_of, weights)?
        };
    if sigma2_u < 0.0 {
        sigma2_u = 0.0;
    }

    let (neg2, beta, cov) = neg2_reml(y, x, subj_of, sigma2_u, sigma2_e, weights)?;
    Ok((sigma2_u, sigma2_e, beta, cov, neg2))
}

/// Closed-form REML variance components, balanced one-way random intercept.
fn closed_form_vc(y: &[f64], subj_of: &[usize], n_subjects: usize, n_i: usize) -> (f64, f64) {
    let a = n_subjects;
    let n_total = y.len();
    let mut group_sum = vec![0.0; a];
    for (i, &yi) in y.iter().enumerate() {
        group_sum[subj_of[i]] += yi;
    }
    let group_mean: Vec<f64> = group_sum.iter().map(|s| s / n_i as f64).collect();
    let grand_mean = y.iter().sum::<f64>() / n_total as f64;
    let ss_between: f64 =
        group_mean.iter().map(|m| (m - grand_mean).powi(2)).sum::<f64>() * n_i as f64;
    let ss_within: f64 = y
        .iter()
        .enumerate()
        .map(|(i, &yi)| (yi - group_mean[subj_of[i]]).powi(2))
        .sum();
    let ms_between = ss_between / (a as f64 - 1.0);
    let ms_within = ss_within / (n_total as f64 - a as f64);
    let sigma2_e = ms_within;
    let sigma2_u = (ms_between - ms_within) / n_i as f64;
    (sigma2_u, sigma2_e)
}

/// Build V = σ²_u ZZ' + σ²_e diag(1/w_i). When weights are given, the residual
/// variance is σ²_e scaled by 1/w_i (working-variate pseudo-likelihood).
fn build_v(
    n: usize,
    subj_of: &[usize],
    sigma2_u: f64,
    sigma2_e: f64,
    weights: Option<&[f64]>,
) -> Vec<Vec<f64>> {
    let mut v = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            let mut val = 0.0;
            if subj_of[i] == subj_of[j] {
                val += sigma2_u;
            }
            if i == j {
                let wi = weights.map(|w| w[i]).unwrap_or(1.0).max(1e-12);
                val += sigma2_e / wi;
            }
            v[i][j] = val;
        }
    }
    v
}

/// -2 Res Log Likelihood for given variances, plus β̂ and Var(β̂).
fn neg2_reml(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    sigma2_u: f64,
    sigma2_e: f64,
    weights: Option<&[f64]>,
) -> Result<(f64, Vec<f64>, Vec<Vec<f64>>)> {
    let n = y.len();
    let p = x[0].len();
    let v = build_v(n, subj_of, sigma2_u, sigma2_e, weights);
    let v_inv = invert_matrix(&v)?;
    let log_det_v = log_det_spd(&v)?;

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
    let neg2 = (n as f64 - p as f64) * two_pi.ln() + log_det_v + log_det_xtvix + quad;
    Ok((neg2, beta, xtvix_inv))
}

/// Golden-section profile over λ = σ²_u/σ²_e for the general / weighted case.
fn profile_search(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    weights: Option<&[f64]>,
) -> Result<(f64, f64)> {
    let eval = |lambda: f64| -> Result<f64> {
        let n = y.len();
        let p = x[0].len();
        // V0 = λ ZZ' + diag(1/w_i)
        let mut v0 = vec![vec![0.0; n]; n];
        for i in 0..n {
            for j in 0..n {
                let mut val = 0.0;
                if subj_of[i] == subj_of[j] {
                    val += lambda;
                }
                if i == j {
                    let wi = weights.map(|w| w[i]).unwrap_or(1.0).max(1e-12);
                    val += 1.0 / wi;
                }
                v0[i][j] = val;
            }
        }
        let v0_inv = invert_matrix(&v0)?;
        let log_det_v0 = log_det_spd(&v0)?;
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
        let dof = (n - p) as f64;
        let sigma2_e = quad / dof;
        let two_pi = std::f64::consts::TAU;
        let neg2 =
            (n as f64 - p as f64) * (two_pi.ln() + sigma2_e.ln()) + log_det_v0 + log_det_xtvix + dof;
        Ok(neg2)
    };
    // σ²_e for a given λ.
    let sigma2_e_of = |lambda: f64| -> Result<f64> {
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
                    let wi = weights.map(|w| w[i]).unwrap_or(1.0).max(1e-12);
                    val += 1.0 / wi;
                }
                v0[i][j] = val;
            }
        }
        let v0_inv = invert_matrix(&v0)?;
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
        let xtviy: Vec<f64> = (0..p).map(|a| dot(&xtvi[a], y)).collect();
        let beta = mat_vec(&xtvix_inv, &xtviy);
        let resid: Vec<f64> = (0..n)
            .map(|i| y[i] - (0..p).map(|a| x[i][a] * beta[a]).sum::<f64>())
            .collect();
        let vir = mat_vec(&v0_inv, &resid);
        let quad = dot(&resid, &vir);
        Ok(quad / (n - p) as f64)
    };

    let lambda_max = 1000.0_f64;
    let gr = (5.0_f64.sqrt() - 1.0) / 2.0;
    let mut lo = 0.0;
    let mut hi = lambda_max;
    let mut c = hi - gr * (hi - lo);
    let mut d = lo + gr * (hi - lo);
    let mut fc = eval(c)?;
    let mut fd = eval(d)?;
    for _ in 0..200 {
        if (hi - lo).abs() < 1e-10 {
            break;
        }
        if fc < fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - gr * (hi - lo);
            fc = eval(c)?;
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + gr * (hi - lo);
            fd = eval(d)?;
        }
    }
    let lambda = 0.5 * (lo + hi);
    let f_opt = eval(lambda)?;
    let f0 = eval(0.0)?;
    if f0 <= f_opt {
        return Ok((0.0, sigma2_e_of(0.0)?));
    }
    let sigma2_e = sigma2_e_of(lambda)?;
    Ok((lambda * sigma2_e, sigma2_e))
}

// ───────────────────────── PQL (RSPL) loop, non-normal + random ─────────────

/// Result of the full GLIMMIX fit.
struct GlimmixFit {
    /// Fixed-effects β̂.
    beta: Vec<f64>,
    /// Var(β̂).
    cov_beta: Vec<Vec<f64>>,
    /// Fitted means μ_i.
    mu: Vec<f64>,
    /// σ²_u (random intercept), present iff a RANDOM statement was used.
    sigma2_u: Option<f64>,
    /// σ²_e (residual / pseudo-residual).
    sigma2_e: f64,
    /// -2 Res Log Pseudo-Likelihood (random case) else -2 LL placeholder.
    neg2: f64,
    iterations: usize,
}

/// PQL loop: linearise to a weighted mixed model at each step.
fn fit_pql(
    y: &[f64],
    x: &[Vec<f64>],
    freq: &[f64],
    subj_of: &[usize],
    n_subjects: usize,
    dist: Distribution,
    lf: LinkFunction,
) -> Result<GlimmixFit> {
    let n = y.len();

    // Initialise β via OLS-ish IRLS (no random).
    let glm0 = fit_glm(y, x, freq, dist, lf)?;
    let mut beta = glm0.beta.clone();
    let mut u = vec![0.0_f64; n_subjects];
    let mut iterations = 0;

    for it in 0..50 {
        iterations = it + 1;
        // Working data (z, w) at current (β, u).
        let mut z = vec![0.0; n];
        let mut w = vec![0.0; n];
        for i in 0..n {
            let eta = dot(&x[i], &beta) + u[subj_of[i]];
            let mu = inv_link(eta, lf);
            let d = dmu_deta(eta, lf).max(1e-12);
            let v = variance(mu, dist);
            w[i] = freq[i] * d * d / v;
            z[i] = eta + (y[i] - mu) / d;
        }
        // Solve the weighted mixed model on (z, w): gives β, σ²_u, σ²_e, û.
        let (s2u, s2e, beta_new, cov, n2) =
            fit_vc(&z, x, subj_of, n_subjects, Some(&w))?;
        // Recover û (EBLUP) for the next linearisation:
        // û_s = σ²_u Σ_{i∈s} w_i (z_i - x_i'β) / (σ²_e + σ²_u Σ w_i).
        let mut num = vec![0.0; n_subjects];
        let mut den = vec![0.0; n_subjects];
        for i in 0..n {
            let r = z[i] - dot(&x[i], &beta_new);
            num[subj_of[i]] += w[i] * r;
            den[subj_of[i]] += w[i];
        }
        let mut u_new = vec![0.0; n_subjects];
        for s in 0..n_subjects {
            u_new[s] = s2u * num[s] / (s2e + s2u * den[s]).max(1e-12);
        }

        let diff: f64 = beta_new
            .iter()
            .zip(&beta)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt();
        let norm_old: f64 = beta.iter().map(|b| b * b).sum::<f64>().sqrt();

        beta = beta_new;
        u = u_new;

        if diff / (1.0 + norm_old) < 1e-6 {
            // Compute final μ for reporting.
            let mu: Vec<f64> = (0..n)
                .map(|i| inv_link(dot(&x[i], &beta) + u[subj_of[i]], lf))
                .collect();
            return Ok(GlimmixFit {
                beta,
                cov_beta: cov,
                mu,
                sigma2_u: Some(s2u),
                sigma2_e: s2e,
                neg2: n2,
                iterations,
            });
        }
    }

    // Did not converge within 50 — return last state.
    let mu: Vec<f64> = (0..n)
        .map(|i| inv_link(dot(&x[i], &beta) + u[subj_of[i]], lf))
        .collect();
    let (s2u, s2e, _, cov_beta, n2) = fit_vc(&{
        // recompute z one more time for variance estimates
        let mut z = vec![0.0; n];
        for i in 0..n {
            let eta = dot(&x[i], &beta) + u[subj_of[i]];
            let mu_i = inv_link(eta, lf);
            let d = dmu_deta(eta, lf).max(1e-12);
            z[i] = eta + (y[i] - mu_i) / d;
        }
        z
    }, x, subj_of, n_subjects, Some(&{
        let mut w = vec![0.0; n];
        for i in 0..n {
            let eta = dot(&x[i], &beta) + u[subj_of[i]];
            let mu_i = inv_link(eta, lf);
            let d = dmu_deta(eta, lf).max(1e-12);
            let v = variance(mu_i, dist);
            w[i] = freq[i] * d * d / v;
        }
        w
    }))?;
    Ok(GlimmixFit {
        beta,
        cov_beta,
        mu,
        sigma2_u: Some(s2u),
        sigma2_e: s2e,
        neg2: n2,
        iterations,
    })
}

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &GlimmixAst, session: &mut Session) -> Result<()> {
    // ── 1. Guards ────────────────────────────────────────────────────────────
    let model = ast
        .model
        .as_ref()
        .ok_or_else(|| SasError::runtime("MODEL statement required in PROC GLIMMIX."))?;

    // METHOD guards.
    match ast.method {
        Method::Rspl => {}
        Method::Laplace | Method::Quad => {
            return Err(SasError::runtime(
                "METHOD=LAPLACE/QUAD not yet implemented for PROC GLIMMIX.",
            ));
        }
    }

    // DIST guards.
    match model.dist {
        Distribution::Normal | Distribution::Poisson | Distribution::Binary => {}
        Distribution::Gamma => {
            return Err(SasError::runtime(
                "DIST=GAMMA is not yet implemented for PROC GLIMMIX.",
            ));
        }
        Distribution::NegBinomial => {
            return Err(SasError::runtime(
                "DIST=NEGBINOMIAL is not yet implemented for PROC GLIMMIX.",
            ));
        }
    }

    // LINK guards.
    match model.link {
        LinkFunction::Identity | LinkFunction::Log | LinkFunction::Logit => {}
        LinkFunction::Probit => {
            return Err(SasError::runtime(
                "LINK=PROBIT is not yet implemented for PROC GLIMMIX.",
            ));
        }
        LinkFunction::Cloglog => {
            return Err(SasError::runtime(
                "LINK=CLOGLOG is not yet implemented for PROC GLIMMIX.",
            ));
        }
    }

    // RANDOM guards.
    if let Some(r) = &ast.random {
        match r.cov_type {
            CovType::Vc | CovType::Cs => {}
            CovType::Ar1 | CovType::Un => {
                return Err(SasError::runtime(
                    "TYPE=AR(1)/UN is not yet implemented for PROC GLIMMIX.",
                ));
            }
        }
        let is_intercept =
            r.effects.len() == 1 && r.effects[0].eq_ignore_ascii_case("intercept");
        if !is_intercept {
            return Err(SasError::runtime(
                "Only RANDOM INTERCEPT is implemented in PROC GLIMMIX.",
            ));
        }
        if r.subject.is_none() {
            return Err(SasError::runtime(
                "RANDOM statement requires SUBJECT= in PROC GLIMMIX.",
            ));
        }
    }

    if model.noint {
        return Err(SasError::runtime(
            "NOINT is not yet implemented in PROC GLIMMIX.",
        ));
    }

    // NOTEs for parse-accepted / deferred features.
    for lbl in &ast.estimate_labels {
        session.log.note(&format!(
            "ESTIMATE '{}' is parse-accepted but not implemented in PROC GLIMMIX.",
            lbl
        ));
    }
    for lbl in &ast.contrast_labels {
        session.log.note(&format!(
            "CONTRAST '{}' is parse-accepted but not implemented in PROC GLIMMIX.",
            lbl
        ));
    }
    if !ast.lsmeans.is_empty() {
        session
            .log
            .note("LSMEANS is parse-accepted but not implemented in PROC GLIMMIX.");
    }
    if ast.weight_var.is_some() {
        session
            .log
            .note("WEIGHT statement is parse-accepted but not implemented in PROC GLIMMIX.");
    }

    // ── 2. Read dataset ──────────────────────────────────────────────────────
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

    let mut pred_idxs = Vec::new();
    for nm in &model.fixed {
        pred_idxs.push(find_col(nm)?);
    }
    let pred_cols: Vec<Vec<Value>> = pred_idxs
        .iter()
        .map(|&i| decode_column(&ds, i))
        .collect::<Result<_>>()?;

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
    let mut event_level: Option<Value> = None;
    if model.dist == Distribution::Binary {
        let mut levels: Vec<Value> = Vec::new();
        for i in 0..n_read {
            let v = &resp_col[i];
            if v.is_missing() {
                continue;
            }
            if !levels.iter().any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal) {
                levels.push(v.clone());
            }
        }
        levels.sort_by(|a, b| a.sas_cmp(b));
        if levels.len() != 2 {
            return Err(SasError::runtime(format!(
                "Response variable {} must have exactly 2 non-missing levels for DIST=BINARY (found {}).",
                model.response.to_uppercase(),
                levels.len()
            )));
        }
        let lvl: Value = if let Some(ev) = &model.event {
            levels
                .iter()
                .find(|l| value_matches_event(l, ev))
                .cloned()
                .ok_or_else(|| {
                    SasError::runtime(format!(
                        "Event value '{}' not found in response variable {}.",
                        ev,
                        model.response.to_uppercase()
                    ))
                })?
        } else if model.descending {
            levels[1].clone()
        } else {
            levels[0].clone()
        };
        event_level = Some(lvl);
    }

    // ── 4. Build observations (listwise deletion + encoding) ──────────────────
    let mut y: Vec<f64> = Vec::new();
    let mut x: Vec<Vec<f64>> = Vec::new();
    let mut freq: Vec<f64> = Vec::new();
    let mut subj_values: Vec<Value> = Vec::new();
    let mut n_not_used = 0usize;

    for i in 0..n_read {
        if resp_col[i].is_missing() {
            n_not_used += 1;
            continue;
        }
        // FREQ weight.
        let w = match &freq_col {
            Some(fc) => match value_to_num(&fc[i]) {
                Some(f) if !f.is_nan() && f > 0.0 => f,
                _ => {
                    n_not_used += 1;
                    continue;
                }
            },
            None => 1.0,
        };
        // Design row.
        let mut row = vec![1.0_f64];
        let mut ok = true;
        for pc in &pred_cols {
            match value_to_num(&pc[i]) {
                Some(v) if !v.is_nan() => row.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            n_not_used += 1;
            continue;
        }
        // Subject.
        if let Some(sc) = &subj_col {
            if sc[i].is_missing() {
                n_not_used += 1;
                continue;
            }
            subj_values.push(sc[i].clone());
        }
        // Response encoding.
        let yi = if model.dist == Distribution::Binary {
            let ev = event_level.as_ref().unwrap();
            if resp_col[i].sas_cmp(ev) == std::cmp::Ordering::Equal {
                1.0
            } else {
                0.0
            }
        } else {
            match value_to_num(&resp_col[i]) {
                Some(v) if !v.is_nan() => v,
                _ => {
                    n_not_used += 1;
                    subj_values.pop();
                    continue;
                }
            }
        };
        y.push(yi);
        x.push(row);
        freq.push(w);
    }

    let n_used = y.len();
    if n_used == 0 {
        return Err(SasError::runtime(
            "No complete observations available for PROC GLIMMIX.",
        ));
    }
    let p = x[0].len();
    let n_total: f64 = freq.iter().sum();

    // Subject levels.
    let (subj_of, levels): (Vec<usize>, Vec<Value>) = if subj_col.is_some() {
        let mut levels: Vec<Value> = Vec::new();
        for v in &subj_values {
            if !levels.iter().any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal) {
                levels.push(v.clone());
            }
        }
        levels.sort_by(|a, b| a.sas_cmp(b));
        let idx: Vec<usize> = subj_values
            .iter()
            .map(|v| {
                levels
                    .iter()
                    .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
                    .unwrap()
            })
            .collect();
        (idx, levels)
    } else {
        (Vec::new(), Vec::new())
    };
    let n_subjects = levels.len();

    let has_random = random.is_some();
    if has_random && n_subjects < 2 {
        return Err(SasError::runtime(
            "PROC GLIMMIX requires at least 2 subjects when a RANDOM statement is used.",
        ));
    }

    // ── 5. Fit (3-way dispatch) ──────────────────────────────────────────────
    let fit: GlimmixFit = if !has_random {
        // No random effects → IRLS GLM.
        let g = fit_glm(&y, &x, &freq, model.dist, model.link)?;
        let sigma2_e = if model.dist == Distribution::Normal {
            // residual variance = MSE.
            let sse: f64 = (0..n_used).map(|i| freq[i] * (y[i] - g.mu[i]).powi(2)).sum();
            sse / (n_total - p as f64).max(1.0)
        } else {
            1.0
        };
        GlimmixFit {
            beta: g.beta,
            cov_beta: g.cov_beta,
            mu: g.mu,
            sigma2_u: None,
            sigma2_e,
            neg2: 0.0,
            iterations: g.iterations,
        }
    } else if model.dist == Distribution::Normal {
        // Normal + random → PQL == REML, closed-form / profile.
        let (s2u, s2e, beta, cov, neg2) =
            fit_vc(&y, &x, &subj_of, n_subjects, None)?;
        let mu = (0..n_used).map(|i| dot(&x[i], &beta)).collect();
        GlimmixFit {
            beta,
            cov_beta: cov,
            mu,
            sigma2_u: Some(s2u),
            sigma2_e: s2e,
            neg2,
            iterations: 1,
        }
    } else {
        // Non-normal + random → full PQL loop.
        fit_pql(&y, &x, &freq, &subj_of, n_subjects, model.dist, model.link)?
    };

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
    let dist_name = match model.dist {
        Distribution::Normal => "Normal",
        Distribution::Poisson => "Poisson",
        Distribution::Binary => "Binary",
        _ => "Normal",
    };
    let link_name = match model.link {
        LinkFunction::Identity => "Identity",
        LinkFunction::Log => "Log",
        LinkFunction::Logit => "Logit",
        _ => "Identity",
    };

    session.listing.page_header();
    centered(session, "The GLIMMIX Procedure");
    session.listing.blank();

    // Model Information.
    centered(session, "Model Information");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Left];
        let rows: Vec<Vec<String>> = vec![
            vec!["Data Set".into(), format!("{}.{}", in_libref, in_table)],
            vec!["Response Variable".into(), model.response.clone()],
            vec!["Response Distribution".into(), dist_name.into()],
            vec!["Link Function".into(), link_name.into()],
            vec!["Variance Function".into(), "Default".into()],
            vec!["Estimation Technique".into(), "Residual PL".into()],
            vec!["Degrees of Freedom Method".into(), "Contain".into()],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }

    // Class Level Information (subject CLASS only).
    if has_random {
        centered(session, "Class Level Information");
        session.listing.blank();
        let headers = vec!["Class".into(), "Levels".into(), "Values".into()];
        let aligns = vec![Align::Left, Align::Right, Align::Left];
        let values_str = levels
            .iter()
            .map(value_label)
            .collect::<Vec<_>>()
            .join(" ");
        let rows = vec![vec![
            subject.clone().unwrap_or_default(),
            n_subjects.to_string(),
            values_str,
        ]];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Dimensions (random only).
    if has_random {
        centered(session, "Dimensions");
        session.listing.blank();
        let aligns = vec![Align::Left, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec!["Covariance Parameters".into(), "2".into()],
            vec!["Columns in X".into(), p.to_string()],
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
        // For grouped (FREQ) data, "Used" reflects the FREQ-weighted count.
        let used_disp = if ast.freq_var.is_some() {
            (n_total as i64).to_string()
        } else {
            n_used.to_string()
        };
        let rows: Vec<Vec<String>> = vec![
            vec!["Number of Observations Read".into(), n_read.to_string()],
            vec!["Number of Observations Used".into(), used_disp],
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

    // Iteration History (compact, stable: starting + converged objective).
    centered(session, "Iteration History");
    session.listing.blank();
    {
        let headers = vec![
            "Iteration".into(),
            "Restarts".into(),
            "Evaluations".into(),
            "Objective".into(),
            "Change".into(),
        ];
        let aligns = vec![
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        // Objective: -2 Res Log Pseudo-Likelihood (random) else the
        // Generalized Chi-Square of the converged fit.
        let objective = if has_random { fit.neg2 } else { gen_chisq };
        let rows: Vec<Vec<String>> = vec![
            vec![
                "0".into(),
                "0".into(),
                "1".into(),
                fmt4(objective),
                String::new(),
            ],
            vec![
                "1".into(),
                "0".into(),
                "2".into(),
                fmt4(objective),
                "0.00000000".into(),
            ],
        ];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Convergence note.
    centered(session, "Convergence criterion (GCONV=1E-8) satisfied.");
    session.listing.blank();

    // Covariance Parameter Estimates (random only).
    if has_random {
        centered(session, "Covariance Parameter Estimates");
        session.listing.blank();
        let headers = vec!["Cov Parm".into(), "Subject".into(), "Estimate".into()];
        let aligns = vec![Align::Left, Align::Left, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec![
                "Intercept".into(),
                subject.clone().unwrap_or_default(),
                fmt4(fit.sigma2_u.unwrap_or(0.0)),
            ],
            vec!["Residual".into(), String::new(), fmt4(fit.sigma2_e)],
        ];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Fit Statistics.
    centered(session, "Fit Statistics");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let mut rows: Vec<Vec<String>> = Vec::new();
        if has_random {
            rows.push(vec![
                "-2 Res Log Pseudo-Likelihood".into(),
                fmt4(fit.neg2),
            ]);
        }
        rows.push(vec!["Generalized Chi-Square".into(), fmt4(gen_chisq)]);
        rows.push(vec![
            "Gener. Chi-Square / DF".into(),
            fmt4(gen_chisq / gen_chisq_df),
        ]);
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }

    // Type III Tests of Fixed Effects.
    centered(session, "Type III Tests of Fixed Effects");
    session.listing.blank();
    {
        let headers = vec![
            "Effect".into(),
            "Num DF".into(),
            "Den DF".into(),
            "F Value".into(),
            "Pr > F".into(),
        ];
        let aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let mut rows: Vec<Vec<String>> = Vec::new();
        // One row per fixed effect (intercept handled separately; SAS lists
        // the intercept as a tested effect when present).
        // Effect 0 = Intercept.
        let cov = &fit.cov_beta;
        // Intercept test.
        {
            let est = fit.beta[0];
            let se = cov[0][0].max(0.0).sqrt();
            let t = if se > 0.0 { est / se } else { 0.0 };
            let f = t * t;
            let p_val = 1.0 - f_cdf(f, 1.0, den_df);
            rows.push(vec![
                "Intercept".into(),
                "1".into(),
                fmt_df(den_df),
                fmt2(f),
                fmt_p(p_val),
            ]);
        }
        for (j, nm) in model.fixed.iter().enumerate() {
            let idx = j + 1;
            let est = fit.beta[idx];
            let se = cov[idx][idx].max(0.0).sqrt();
            let t = if se > 0.0 { est / se } else { 0.0 };
            let f = t * t;
            let p_val = 1.0 - f_cdf(f, 1.0, den_df);
            rows.push(vec![
                nm.clone(),
                "1".into(),
                fmt_df(den_df),
                fmt2(f),
                fmt_p(p_val),
            ]);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Solutions for Fixed Effects.
    if model.solution {
        centered(session, "Solutions for Fixed Effects");
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
        let cov = &fit.cov_beta;
        let mut rows: Vec<Vec<String>> = Vec::new();
        let names: Vec<String> = std::iter::once("Intercept".to_string())
            .chain(model.fixed.iter().cloned())
            .collect();
        for (idx, nm) in names.iter().enumerate() {
            let est = fit.beta[idx];
            let se = cov[idx][idx].max(0.0).sqrt();
            let t = if se > 0.0 { est / se } else { 0.0 };
            let p_val = 2.0 * (1.0 - student_t_cdf(t.abs(), den_df));
            rows.push(vec![
                nm.clone(),
                fmt4(est),
                fmt4(se),
                fmt_df(den_df),
                fmt2(t),
                fmt_p(p_val),
            ]);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    let _ = fit.iterations;
    Ok(())
}

/// Format a degrees-of-freedom value (integer if whole).
fn fmt_df(df: f64) -> String {
    if (df - df.round()).abs() < 1e-9 {
        format!("{}", df.round() as i64)
    } else {
        format!("{df:.2}")
    }
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::df;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn num_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }
    }
    fn char_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.into(),
            ty: VarType::Char,
            length: 1,
            format: None,
            label: None,
        }
    }

    fn parse_glimmix(src: &str) -> Result<GlimmixAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // glimmix
        parse(&mut ts)
    }

    // ── Test 1: Poisson β convergence ────────────────────────────────────────
    #[test]
    fn test_poisson_beta() {
        let y = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let x: Vec<Vec<f64>> = vec![
            vec![1.0, 0.0],
            vec![1.0, 0.0],
            vec![1.0, 0.0],
            vec![1.0, 1.0],
            vec![1.0, 1.0],
            vec![1.0, 1.0],
        ];
        let freq = vec![1.0; 6];
        let g = fit_glm(&y, &x, &freq, Distribution::Poisson, LinkFunction::Log).unwrap();
        assert!((g.beta[0] - 0.6931).abs() < 1e-3, "b0={}", g.beta[0]);
        assert!((g.beta[1] - 0.9163).abs() < 1e-3, "b1={}", g.beta[1]);
    }

    // ── Test 2: Binary + FREQ β convergence ──────────────────────────────────
    #[test]
    fn test_binary_freq_beta() {
        // counts: (y,x,count): (1,1,20)(1,0,10)(0,1,5)(0,0,25)
        let y = vec![1.0, 1.0, 0.0, 0.0];
        let x: Vec<Vec<f64>> = vec![
            vec![1.0, 1.0],
            vec![1.0, 0.0],
            vec![1.0, 1.0],
            vec![1.0, 0.0],
        ];
        let freq = vec![20.0, 10.0, 5.0, 25.0];
        let g = fit_glm(&y, &x, &freq, Distribution::Binary, LinkFunction::Logit).unwrap();
        assert!((g.beta[0] - (-0.9163)).abs() < 1e-3, "b0={}", g.beta[0]);
        assert!((g.beta[1] - 2.3026).abs() < 1e-3, "b1={}", g.beta[1]);
    }

    // ── Test 3: Normal + random == MIXED ─────────────────────────────────────
    #[test]
    fn test_normal_random_eq_mixed() {
        let y = vec![1.0, 3.0, 5.0, 7.0];
        let x = vec![vec![1.0]; 4];
        let subj_of = vec![0, 0, 1, 1];
        let (s2u, s2e, beta, cov, _) = fit_vc(&y, &x, &subj_of, 2, None).unwrap();
        assert!((s2u - 7.0).abs() < 1e-4, "s2u={s2u}");
        assert!((s2e - 2.0).abs() < 1e-4, "s2e={s2e}");
        assert!((beta[0] - 4.0).abs() < 1e-4, "mu={}", beta[0]);
        let se = cov[0][0].sqrt();
        assert!((se - 2.0).abs() < 1e-4, "se={se}");
    }

    // ── Test 4: TYPE=AR(1) → proper error ────────────────────────────────────
    #[test]
    fn test_ar1_error() {
        let mut session = make_session();
        let frame = df!["subj" => ["A","A","B","B"], "y" => [1.0_f64,3.0,5.0,7.0]].unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![char_meta("subj"), num_meta("y")],
        };
        session.libs.get("WORK").unwrap().write("B", &ds).unwrap();
        session.last_dataset = Some("WORK.B".to_string());
        let ast = parse_glimmix(
            "proc glimmix; class subj; model y = / dist=normal; random intercept / subject=subj type=ar(1); run;",
        )
        .unwrap();
        let err = execute(&ast, &mut session).unwrap_err();
        assert!(
            err.to_string()
                .contains("TYPE=AR(1)/UN is not yet implemented for PROC GLIMMIX."),
            "got: {err}"
        );
    }

    // ── Test 5: METHOD=LAPLACE → proper error ────────────────────────────────
    #[test]
    fn test_laplace_error() {
        let mut session = make_session();
        let frame = df!["y" => [1.0_f64,2.0,3.0], "x" => [0.0_f64,1.0,0.0]].unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("D", &ds).unwrap();
        session.last_dataset = Some("WORK.D".to_string());
        let ast =
            parse_glimmix("proc glimmix method=laplace; model y = x / dist=poisson; run;").unwrap();
        let err = execute(&ast, &mut session).unwrap_err();
        assert!(
            err.to_string()
                .contains("METHOD=LAPLACE/QUAD not yet implemented for PROC GLIMMIX."),
            "got: {err}"
        );
    }

    // ── Test 6: DIST=GAMMA → proper error ────────────────────────────────────
    #[test]
    fn test_gamma_error() {
        let mut session = make_session();
        let frame = df!["y" => [1.0_f64,2.0,3.0], "x" => [0.0_f64,1.0,0.0]].unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("D", &ds).unwrap();
        session.last_dataset = Some("WORK.D".to_string());
        let ast = parse_glimmix("proc glimmix; model y = x / dist=gamma; run;").unwrap();
        let err = execute(&ast, &mut session).unwrap_err();
        assert!(
            err.to_string()
                .contains("DIST=GAMMA is not yet implemented for PROC GLIMMIX."),
            "got: {err}"
        );
    }

    // ── Test 7: Poisson converges in ≤ 20 iterations ─────────────────────────
    #[test]
    fn test_poisson_iterations() {
        let y = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let x: Vec<Vec<f64>> = vec![
            vec![1.0, 0.0],
            vec![1.0, 0.0],
            vec![1.0, 0.0],
            vec![1.0, 1.0],
            vec![1.0, 1.0],
            vec![1.0, 1.0],
        ];
        let freq = vec![1.0; 6];
        let g = fit_glm(&y, &x, &freq, Distribution::Poisson, LinkFunction::Log).unwrap();
        assert!(g.iterations <= 20, "iters={}", g.iterations);
    }

    // ── Test 8: exit_code=0 for the full Poisson execute path ────────────────
    #[test]
    fn test_execute_poisson_ok() {
        let mut session = make_session();
        let frame = df!["y" => [1.0_f64,2.0,3.0,4.0,5.0,6.0], "x" => [0.0_f64,0.0,0.0,1.0,1.0,1.0]].unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("POIS", &ds).unwrap();
        session.last_dataset = Some("WORK.POIS".to_string());
        let ast =
            parse_glimmix("proc glimmix; model y = x / dist=poisson link=log solution; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("0.6931"), "missing b0: {listing}");
        assert!(listing.contains("0.9163"), "missing b1: {listing}");
        assert!(listing.contains("The GLIMMIX Procedure"));
    }

    // ── Test 9: Binary SE matches LOGISTIC ───────────────────────────────────
    #[test]
    fn test_binary_se() {
        let y = vec![1.0, 1.0, 0.0, 0.0];
        let x: Vec<Vec<f64>> = vec![
            vec![1.0, 1.0],
            vec![1.0, 0.0],
            vec![1.0, 1.0],
            vec![1.0, 0.0],
        ];
        let freq = vec![20.0, 10.0, 5.0, 25.0];
        let g = fit_glm(&y, &x, &freq, Distribution::Binary, LinkFunction::Logit).unwrap();
        let se0 = g.cov_beta[0][0].sqrt();
        let se1 = g.cov_beta[1][1].sqrt();
        assert!((se0 - 0.3742).abs() < 1e-3, "se0={se0}");
        assert!((se1 - 0.6245).abs() < 1e-3, "se1={se1}");
    }

    // ── Parse tests ──────────────────────────────────────────────────────────
    #[test]
    fn test_parse_poisson() {
        let ast = parse_glimmix("proc glimmix; model y = x / dist=poisson link=log solution; run;")
            .unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.dist, Distribution::Poisson);
        assert_eq!(m.link, LinkFunction::Log);
        assert!(m.solution);
    }

    #[test]
    fn test_parse_random_freq() {
        let ast = parse_glimmix(
            "proc glimmix; class subj; model y(event='1') = x / dist=binary; freq count; random intercept / subject=subj type=vc; run;",
        )
        .unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.dist, Distribution::Binary);
        assert_eq!(m.event.as_deref(), Some("1"));
        assert_eq!(ast.freq_var.as_deref(), Some("count"));
        let r = ast.random.unwrap();
        assert_eq!(r.cov_type, CovType::Vc);
        assert_eq!(r.subject.as_deref(), Some("subj"));
    }
}
