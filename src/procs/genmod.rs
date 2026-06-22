//! PROC GENMOD — Generalized Linear Models via Newton-Raphson / IRLS (M26.2).
//!
//! Supports:
//! - DIST=POISSON (link=LOG, canonical)
//! - DIST=BINOMIAL (link=LOGIT, canonical)
//! - DIST=NORMAL (link=IDENTITY, canonical)
//! - DIST=GAMMA — deferred, parse OK but execute returns error
//! - FREQ statement (weighted observations).
//! - MODEL statement with EVENT= and DESCENDING options (Binomial).
//! - Produces: Model Information, Response Profile (Binomial only),
//!   Model Convergence Status, Criteria For Assessing Goodness Of Fit
//!   (Deviance/Pearson/LL/AIC/AICC/BIC), Analysis Of Maximum Likelihood
//!   Parameter Estimates (β/SE/Wald CI/Wald χ²/p), Scale parameter row.

use std::f64::consts::PI;

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::{chisq_sf, decode_column};
use crate::session::Session;
use crate::stat::invert_matrix;
use crate::token::TokenKind;
use crate::value::{format_best, Value};

// ───────────────────────── AST ─────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Distribution {
    Poisson,
    Binomial,
    Normal,
    Gamma, // deferred — parse OK, execute errors
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkFunction {
    Log,
    Logit,
    Identity,
    /// Reciprocal (inverse) link: η = 1/μ, μ = 1/η — Gamma canonical link.
    Reciprocal,
}

/// Canonical link for each distribution (SAS 9.4 defaults).
fn canonical_link(dist: &Distribution) -> LinkFunction {
    match dist {
        Distribution::Poisson => LinkFunction::Log,
        Distribution::Binomial => LinkFunction::Logit,
        Distribution::Normal => LinkFunction::Identity,
        // SAS GENMOD canonical link for Gamma is the reciprocal (power(-1)).
        Distribution::Gamma => LinkFunction::Reciprocal,
    }
}

#[derive(Debug, Clone)]
pub struct GenmodAst {
    pub data_options: GenmodDataOptions,
    pub model: Option<GenmodModel>,
    pub freq_var: Option<String>,
    pub class_vars: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GenmodDataOptions {
    pub input: Option<DatasetRef>,
}

#[derive(Debug, Clone)]
pub struct GenmodModel {
    pub response: String,
    pub event: Option<String>,
    pub descending: bool,
    pub predictors: Vec<String>,
    pub dist: Distribution,
    pub link: LinkFunction,
    pub noprint: bool,
    /// MODEL ... / SCALE=value — fix the dispersion at this value instead of
    /// estimating it (Normal/Gamma). `None` → estimate.
    pub scale: Option<f64>,
    /// MODEL ... / NOSCALE — hold the scale fixed at 1 (Normal/Gamma) rather
    /// than estimating it. Combined with SCALE= it fixes at the given value.
    pub noscale: bool,
}

// ───────────────────────── Link / variance functions ─────────────────────────

/// Apply inverse link: η → μ (mean on natural scale).
///
/// For the reciprocal link μ = 1/η; the IRLS step can drive η through 0 and
/// make μ negative, which is invalid for Gamma (μ > 0). We clamp μ to a small
/// positive floor here; the IRLS loop additionally step-halves on invalid μ.
fn inv_link(eta: f64, lf: &LinkFunction) -> f64 {
    match lf {
        LinkFunction::Log => eta.exp().max(1e-10),
        LinkFunction::Logit => {
            let e = (-eta).exp();
            1.0 / (1.0 + e)
        }
        LinkFunction::Identity => eta,
        LinkFunction::Reciprocal => {
            if eta.abs() < 1e-12 {
                1e12
            } else {
                let mu = 1.0 / eta;
                if mu > 0.0 {
                    mu
                } else {
                    1e-10
                }
            }
        }
    }
}

/// Variance function V(μ) for the family.
fn variance(mu: f64, dist: &Distribution) -> f64 {
    match dist {
        Distribution::Poisson => mu,
        Distribution::Binomial => {
            let v = mu * (1.0 - mu);
            v.max(1e-15)
        }
        Distribution::Normal => 1.0,
        // Gamma: V(μ) = μ².
        Distribution::Gamma => (mu * mu).max(1e-15),
    }
}

/// dη/dμ = g'(μ) where g is the link function.
fn deta_dmu(mu: f64, lf: &LinkFunction) -> f64 {
    match lf {
        LinkFunction::Log => 1.0 / mu,
        LinkFunction::Logit => {
            let v = mu * (1.0 - mu);
            1.0 / v.max(1e-15)
        }
        LinkFunction::Identity => 1.0,
        // η = 1/μ ⇒ dη/dμ = −1/μ².
        LinkFunction::Reciprocal => -1.0 / (mu * mu).max(1e-15),
    }
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC GENMOD. Called AFTER `proc genmod` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<GenmodAst> {
    let mut input: Option<DatasetRef> = None;

    // PROC GENMOD statement options until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            input = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else {
            ts.next();
        }
    }

    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<GenmodModel> = None;
    let mut freq_var: Option<String> = None;

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "class" {
            ts.next();
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    class_vars.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "model" {
            ts.next(); // consume "model"

            // Response variable
            let response = ts
                .peek()
                .ident()
                .map(str::to_string)
                .ok_or_else(|| SasError::parse("expected response variable", ts.peek().span))?;
            ts.next();

            // Optional response options: (event='val' descending ...)
            let mut event: Option<String> = None;
            let mut descending = false;

            if ts.peek().kind == TokenKind::LParen {
                ts.next(); // consume '('
                loop {
                    if ts.peek().kind == TokenKind::RParen || ts.peek().kind == TokenKind::Eof {
                        break;
                    }
                    if ts.peek().kind == TokenKind::Semi {
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

            // Expect '='
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after response variable in MODEL",
                    ts.peek().span,
                ));
            }
            ts.next();

            // Predictors until '/' or ';'
            let mut predictors: Vec<String> = Vec::new();
            let mut dist_opt: Option<Distribution> = None;
            let mut link_opt: Option<LinkFunction> = None;
            let mut noprint = false;
            let mut scale_opt: Option<f64> = None;
            let mut noscale = false;

            loop {
                if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                    break;
                }
                if ts.peek().kind == TokenKind::Slash {
                    ts.next(); // consume '/'
                    // Parse options
                    while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                        if ts.peek().is_kw("dist") {
                            ts.next();
                            if ts.peek().kind == TokenKind::Eq {
                                ts.next();
                            }
                            if let Some(name) = ts.peek().ident().map(str::to_string) {
                                ts.next();
                                match name.to_ascii_lowercase().as_str() {
                                    "poisson" => dist_opt = Some(Distribution::Poisson),
                                    "binomial" => dist_opt = Some(Distribution::Binomial),
                                    "normal" => dist_opt = Some(Distribution::Normal),
                                    "gamma" => dist_opt = Some(Distribution::Gamma),
                                    _ => {} // ignore unknown
                                }
                            }
                        } else if ts.peek().is_kw("link") {
                            ts.next();
                            if ts.peek().kind == TokenKind::Eq {
                                ts.next();
                            }
                            if let Some(name) = ts.peek().ident().map(str::to_string) {
                                ts.next();
                                match name.to_ascii_lowercase().as_str() {
                                    "log" => link_opt = Some(LinkFunction::Log),
                                    "logit" => link_opt = Some(LinkFunction::Logit),
                                    "identity" => link_opt = Some(LinkFunction::Identity),
                                    "reciprocal" | "inverse" | "power" => {
                                        // POWER(-1) ≈ reciprocal; treat POWER as
                                        // reciprocal here (full power family deferred).
                                        link_opt = Some(LinkFunction::Reciprocal)
                                    }
                                    _ => {} // ignore unknown
                                }
                            }
                        } else if ts.peek().is_kw("noprint") {
                            noprint = true;
                            ts.next();
                        } else if ts.peek().is_kw("noscale") {
                            noscale = true;
                            ts.next();
                        } else if ts.peek().is_kw("scale") {
                            ts.next();
                            if ts.peek().kind == TokenKind::Eq {
                                ts.next();
                            }
                            // SCALE=<number>; accept numeric literal.
                            if let TokenKind::Num(v) = ts.peek().kind {
                                scale_opt = Some(v);
                                ts.next();
                            } else if let Some(s) = ts.peek().ident().map(str::to_string) {
                                if let Ok(v) = s.parse::<f64>() {
                                    scale_opt = Some(v);
                                }
                                ts.next();
                            } else {
                                ts.next();
                            }
                        } else {
                            ts.next();
                        }
                    }
                    break;
                }
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    predictors.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;

            // Determine distribution (default Poisson if only link given)
            let dist = dist_opt.unwrap_or(Distribution::Poisson);
            // If LINK not given, use canonical link for the distribution
            let link = link_opt.unwrap_or_else(|| canonical_link(&dist));

            model = Some(GenmodModel {
                response,
                event,
                descending,
                predictors,
                dist,
                link,
                noprint,
                scale: scale_opt,
                noscale,
            });
            Ok(true)
        } else if kw == "freq" {
            ts.next();
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                freq_var = Some(name);
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(GenmodAst {
        data_options: GenmodDataOptions { input },
        model,
        freq_var,
        class_vars,
    })
}

// ───────────────────────── Formatting helpers ─────────────────────────

fn fmt4(v: f64) -> String {
    format!("{v:.4}")
}

fn fmt_p_opt(p: f64) -> String {
    if p < 0.0001 {
        "<.0001".to_string()
    } else {
        format!("{p:.4}")
    }
}

fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(text.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), text));
}

// ───────────────────────── Value helpers ─────────────────────────

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

// ───────────────────────── Matrix helpers ─────────────────────────

fn mat_vec(mat: &[Vec<f64>], vec: &[f64]) -> Vec<f64> {
    mat.iter()
        .map(|row| row.iter().zip(vec.iter()).map(|(a, b)| a * b).sum())
        .collect()
}

// ───────────────────────── Deviance contribution ─────────────────────────

fn dev_contribution_binom(y: f64, mu: f64) -> f64 {
    let t1 = if y > 0.0 { y * (y / mu).ln() } else { 0.0 };
    let t2 = if (1.0 - y) > 0.0 {
        (1.0 - y) * ((1.0 - y) / (1.0 - mu)).ln()
    } else {
        0.0
    };
    2.0 * (t1 + t2)
}

// ───────────────────────── CLASS design terms ─────────────────────────

/// One term on the MODEL right-hand side, expanded into design columns.
///
/// PARAM = reference coding with ref = LAST level (matches SAS GENMOD default
/// `PARAM=GLM`-style ordering with the last level as the reference cell). A
/// CLASS factor with L distinct non-missing levels (in `Value::sas_cmp` order)
/// contributes L−1 design columns; the last level is the dropped reference.
enum DesignTerm {
    /// Continuous predictor → a single design column carrying the numeric value.
    Continuous { name: String, col: usize },
    /// CLASS predictor → L−1 indicator columns (reference = last level).
    Class {
        name: String,
        col: usize,
        /// Distinct non-missing levels, `sas_cmp` order. Reference = last.
        levels: Vec<Value>,
    },
}

impl DesignTerm {
    /// Number of design columns this term contributes.
    fn n_cols(&self) -> usize {
        match self {
            DesignTerm::Continuous { .. } => 1,
            DesignTerm::Class { levels, .. } => levels.len().saturating_sub(1),
        }
    }
}

/// Level label for a CLASS value, matching the Class Level Information scheme.
fn class_level_label(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim_end().to_string(),
        Value::Num(f) => format!("{f}"),
        Value::Missing(k) => k.display(),
    }
}

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &GenmodAst, session: &mut Session) -> Result<()> {
    // ── 1. Guards ──────────────────────────────────────────────────────────
    let model = ast.model.as_ref().ok_or_else(|| {
        SasError::runtime("MODEL statement required for PROC GENMOD")
    })?;

    // ── 2. Read dataset ────────────────────────────────────────────────────
    let in_ref = common::resolve_last_dataset(&ast.data_options.input, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_read = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_read, in_libref, in_table
    ));

    let resp_name = &model.response;
    let predictors = &model.predictors;
    let nb_preds = predictors.len();
    let dist = &model.dist;
    let lf = &model.link;

    // ── Find column indices ────────────────────────────────────────────────
    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", nm.to_uppercase()))
            })
    };

    let resp_idx = find_col(resp_name)?;
    let mut pred_idxs: Vec<usize> = Vec::with_capacity(nb_preds);
    for nm in predictors {
        pred_idxs.push(find_col(nm)?);
    }
    let freq_idx: Option<usize> = if let Some(fv) = &ast.freq_var {
        Some(find_col(fv)?)
    } else {
        None
    };

    // ── Decode columns ─────────────────────────────────────────────────────
    let resp_col = decode_column(&ds, resp_idx)?;
    let mut pred_cols: Vec<Vec<Value>> = Vec::with_capacity(nb_preds);
    for &idx in &pred_idxs {
        pred_cols.push(decode_column(&ds, idx)?);
    }
    let freq_col: Option<Vec<Value>> = if let Some(fi) = freq_idx {
        Some(decode_column(&ds, fi)?)
    } else {
        None
    };

    // ── Build design terms (CLASS reference-cell coding, ref = last level) ──
    // Each predictor is either continuous (1 column) or a CLASS factor (L−1
    // indicator columns). CLASS levels are the distinct non-missing values in
    // `Value::sas_cmp` order; the last level is the reference cell.
    let mut design_terms: Vec<DesignTerm> = Vec::with_capacity(nb_preds);
    for (pi, nm) in predictors.iter().enumerate() {
        let is_class = ast
            .class_vars
            .iter()
            .any(|c| c.eq_ignore_ascii_case(nm));
        if is_class {
            let col = &pred_cols[pi];
            let mut levels: Vec<Value> = Vec::new();
            for v in col.iter() {
                if v.is_missing() {
                    continue;
                }
                if !levels
                    .iter()
                    .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
                {
                    levels.push(v.clone());
                }
            }
            levels.sort_by(|a, b| a.sas_cmp(b));
            if levels.len() < 2 {
                return Err(SasError::runtime(format!(
                    "CLASS variable {} must have at least 2 levels.",
                    nm.to_uppercase()
                )));
            }
            design_terms.push(DesignTerm::Class {
                name: ds.vars[pred_idxs[pi]].name.clone(),
                col: pi,
                levels,
            });
        } else {
            design_terms.push(DesignTerm::Continuous {
                name: ds.vars[pred_idxs[pi]].name.clone(),
                col: pi,
            });
        }
    }
    let n_design: usize = design_terms.iter().map(|t| t.n_cols()).sum();

    // ── 3. Prepare response for Binomial (determine event level) ──────────
    // For non-binomial, collect distinct levels for reference but encode y
    // numerically. For Binomial, reproduce LOGISTIC's level-determination.
    let binomial_event_level: Option<Value>;
    let binomial_event_label: Option<String>;
    let binomial_nonevent_label: Option<String>;
    let binomial_n_event_total: f64;
    let binomial_n_nonevent_total: f64;

    if *dist == Distribution::Binomial {
        // Collect distinct non-missing levels
        let mut levels: Vec<Value> = Vec::new();
        for i in 0..n_read {
            let v = &resp_col[i];
            if v.is_missing() {
                continue;
            }
            if !levels.iter().any(|lv| lv.sas_cmp(v) == std::cmp::Ordering::Equal) {
                levels.push(v.clone());
            }
        }
        levels.sort_by(|a, b| a.sas_cmp(b));

        if levels.len() != 2 {
            return Err(SasError::runtime(format!(
                "Response variable {} must have exactly 2 non-missing levels for DIST=BINOMIAL (found {}).",
                resp_name.to_uppercase(),
                levels.len()
            )));
        }

        // Determine event level
        let event_level_ref: &Value = if let Some(ev_str) = &model.event {
            levels
                .iter()
                .find(|lv| value_matches_event(lv, ev_str))
                .ok_or_else(|| {
                    SasError::runtime(format!(
                        "Event value '{}' not found in response variable {}.",
                        ev_str,
                        resp_name.to_uppercase()
                    ))
                })?
        } else if model.descending {
            &levels[1] // max level
        } else {
            &levels[0] // min level (default)
        };

        let el = value_label(event_level_ref);
        let nel = value_label(if std::ptr::eq(event_level_ref, &levels[0]) {
            &levels[1]
        } else {
            &levels[0]
        });

        // Pre-compute event/nonevent totals for Response Profile
        let mut ev_total = 0.0_f64;
        let mut nev_total = 0.0_f64;
        for i in 0..n_read {
            if resp_col[i].is_missing() {
                continue;
            }
            let w = if let Some(fc) = &freq_col {
                match value_to_num(&fc[i]) {
                    Some(f) if !f.is_nan() && f > 0.0 => f,
                    _ => continue,
                }
            } else {
                1.0
            };
            if resp_col[i].sas_cmp(event_level_ref) == std::cmp::Ordering::Equal {
                ev_total += w;
            } else {
                nev_total += w;
            }
        }

        binomial_event_level = Some(event_level_ref.clone());
        binomial_event_label = Some(el);
        binomial_nonevent_label = Some(nel);
        binomial_n_event_total = ev_total;
        binomial_n_nonevent_total = nev_total;
    } else {
        binomial_event_level = None;
        binomial_event_label = None;
        binomial_nonevent_label = None;
        binomial_n_event_total = 0.0;
        binomial_n_nonevent_total = 0.0;
    }

    // ── 4. Listwise deletion + encoding ───────────────────────────────────
    let mut y_vec: Vec<f64> = Vec::new();
    let mut x_mat: Vec<Vec<f64>> = Vec::new();
    let mut freq_vec: Vec<f64> = Vec::new();

    for i in 0..n_read {
        if resp_col[i].is_missing() {
            continue;
        }

        let w = if let Some(fc) = &freq_col {
            match value_to_num(&fc[i]) {
                Some(f) if !f.is_nan() && f > 0.0 => f,
                _ => continue,
            }
        } else {
            1.0
        };

        let mut row = vec![1.0_f64]; // intercept
        let mut ok = true;
        for term in &design_terms {
            match term {
                DesignTerm::Continuous { col, .. } => {
                    match value_to_num(&pred_cols[*col][i]) {
                        Some(v) if !v.is_nan() => row.push(v),
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                DesignTerm::Class { col, levels, .. } => {
                    let v = &pred_cols[*col][i];
                    if v.is_missing() {
                        ok = false;
                        break;
                    }
                    // Reference-cell dummies: 1 for the matching non-reference
                    // level, 0 elsewhere (reference = last level → all zeros).
                    let nref = levels.len() - 1;
                    let pos = levels
                        .iter()
                        .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal);
                    match pos {
                        Some(li) => {
                            for k in 0..nref {
                                row.push(if k == li { 1.0 } else { 0.0 });
                            }
                        }
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
            }
        }
        if !ok {
            continue;
        }

        // Encode response
        let yi: f64 = if *dist == Distribution::Binomial {
            let ev_level = binomial_event_level.as_ref().unwrap();
            if resp_col[i].sas_cmp(ev_level) == std::cmp::Ordering::Equal {
                1.0
            } else {
                0.0
            }
        } else {
            // Numeric response for Poisson and Normal
            match value_to_num(&resp_col[i]) {
                Some(v) if !v.is_nan() => v,
                _ => continue,
            }
        };

        y_vec.push(yi);
        x_mat.push(row);
        freq_vec.push(w);
    }

    let n_total: f64 = freq_vec.iter().sum();
    let n_obs = y_vec.len();

    session.log.note(&format!(
        "There were {} observations used.",
        n_total as i64
    ));

    let p_param = 1 + n_design; // intercept + design columns

    if n_obs <= n_design {
        return Err(SasError::runtime(
            "Not enough observations for PROC GENMOD",
        ));
    }

    // ── 5. Listing header ─────────────────────────────────────────────────
    if !model.noprint {
        session.listing.page_header();
        centered(session, "The GENMOD Procedure");
        session.listing.blank();

        // ── 6. Model Information ──────────────────────────────────────────
        centered(session, "Model Information");
        session.listing.blank();

        let ds_display = format!("{}.{}", in_libref, in_table);
        let dist_name = match dist {
            Distribution::Poisson => "Poisson",
            Distribution::Binomial => "Binomial",
            Distribution::Normal => "Normal",
            Distribution::Gamma => "Gamma",
        };
        let link_name = match lf {
            LinkFunction::Log => "Log",
            LinkFunction::Logit => "Logit",
            LinkFunction::Identity => "Identity",
            LinkFunction::Reciprocal => "Reciprocal",
        };

        let info_headers: Vec<String> = vec!["".into(), "".into()];
        let info_aligns = vec![Align::Left, Align::Left];
        let info_rows: Vec<Vec<String>> = vec![
            vec!["Data Set".into(), ds_display],
            vec!["Response Variable".into(), resp_name.clone()],
            vec!["Distribution".into(), dist_name.into()],
            vec!["Link Function".into(), link_name.into()],
            vec!["Dependent Variable".into(), resp_name.clone()],
            vec!["Observations Used".into(), (n_total as i64).to_string()],
        ];
        session
            .listing
            .write_table(&info_headers, &info_aligns, &info_rows);
        session.listing.blank();

        // ── Class Level Information (only when CLASS variables present) ────
        if !ast.class_vars.is_empty() {
            centered(session, "Class Level Information");
            session.listing.blank();

            let cli_headers: Vec<String> =
                vec!["Class".into(), "Levels".into(), "Values".into()];
            let cli_aligns = vec![Align::Left, Align::Right, Align::Left];
            let mut cli_rows: Vec<Vec<String>> = Vec::new();
            for term in &design_terms {
                if let DesignTerm::Class { name, levels, .. } = term {
                    let values_str: Vec<String> =
                        levels.iter().map(class_level_label).collect();
                    cli_rows.push(vec![
                        name.clone(),
                        format!("{}", levels.len()),
                        values_str.join(" "),
                    ]);
                }
            }
            session
                .listing
                .write_table(&cli_headers, &cli_aligns, &cli_rows);
            session.listing.blank();
        }

        // ── 7. Response Profile (Binomial only) ───────────────────────────
        if *dist == Distribution::Binomial {
            centered(session, "Response Profile");
            session.listing.blank();

            let el = binomial_event_label.as_deref().unwrap_or("");
            let nel = binomial_nonevent_label.as_deref().unwrap_or("");

            let rp_headers: Vec<String> = vec![
                "Ordered Value".into(),
                resp_name.clone(),
                "Total Frequency".into(),
            ];
            let rp_aligns = vec![Align::Right, Align::Left, Align::Right];
            let rp_rows: Vec<Vec<String>> = vec![
                vec![
                    "1".into(),
                    el.to_string(),
                    (binomial_n_event_total as i64).to_string(),
                ],
                vec![
                    "2".into(),
                    nel.to_string(),
                    (binomial_n_nonevent_total as i64).to_string(),
                ],
            ];
            session
                .listing
                .write_table(&rp_headers, &rp_aligns, &rp_rows);
            session.listing.blank();
            session.listing.write_line(&format!(
                "PROC GENMOD is modeling the probability that {}={}.",
                resp_name, el
            ));
            session.listing.blank();
        }

        // ── 8. Convergence status ─────────────────────────────────────────
        centered(session, "Model Convergence Status");
        session.listing.blank();
        session
            .listing
            .write_line("     Convergence criterion (GCONV=1E-8) satisfied.");
        session.listing.blank();
    }

    // ── 9. IRLS / Newton-Raphson ──────────────────────────────────────────
    // Initialize β
    let y_mean: f64 = {
        let sum_wy: f64 = y_vec.iter().zip(freq_vec.iter()).map(|(y, w)| y * w).sum();
        sum_wy / n_total
    };

    let eta0 = match lf {
        LinkFunction::Log => y_mean.max(1e-10).ln(),
        LinkFunction::Logit => {
            let p = y_mean.clamp(1e-10, 1.0 - 1e-10);
            (p / (1.0 - p)).ln().clamp(-10.0, 10.0)
        }
        LinkFunction::Identity => y_mean,
        // η = 1/μ; guard ȳ > 0 (Gamma response is positive).
        LinkFunction::Reciprocal => 1.0 / y_mean.max(1e-10),
    };

    let mut beta: Vec<f64> = vec![0.0; p_param];
    beta[0] = eta0;

    // IRLS iterations (max 50)
    let mut converged = false;
    for _iter in 0..50 {
        // Compute H = X'WX and score = X'W(z - Xβ) = X'(y - μ)/V * deta_dmu⁻¹
        // Actually: score_j = Σ freq_i * x_ij * (y_i - mu_i) / (V(mu_i) * deta_dmu(mu_i))
        // H_jk = Σ freq_i * x_ij * x_ik / (V(mu_i) * deta_dmu(mu_i)^2)
        let mut score: Vec<f64> = vec![0.0; p_param];
        let mut hessian: Vec<Vec<f64>> = vec![vec![0.0; p_param]; p_param];

        for i in 0..n_obs {
            let xi = &x_mat[i];
            let eta: f64 = xi.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
            let mu = inv_link(eta, lf);
            let v = variance(mu, dist);
            let dg = deta_dmu(mu, lf);
            let w_irls = freq_vec[i] / (v * dg * dg);

            // Score: X' w_irls * (z_i - eta_i) where z_i = eta + (y-mu)*dg
            // = X' freq_i * (y_i - mu_i) / (V * dg)
            let resid_adj = freq_vec[i] * (y_vec[i] - mu) / (v * dg);
            for j in 0..p_param {
                score[j] += xi[j] * resid_adj;
            }

            // Hessian H = X'WX (positive definite)
            for j in 0..p_param {
                for k in 0..p_param {
                    hessian[j][k] += xi[j] * xi[k] * w_irls;
                }
            }
        }

        // Solve H·δ = score
        let h_inv = invert_matrix(&hessian)?;
        let delta = mat_vec(&h_inv, &score);

        // Update β with step-halving to keep μ valid (μ>0 for Gamma/Poisson,
        // 0<μ<1 for Binomial). The reciprocal link in particular can drive μ
        // negative; halve the step until every fitted μ is in range.
        let mu_in_range = |b: &[f64]| -> bool {
            for xi in x_mat.iter() {
                let eta: f64 = xi.iter().zip(b.iter()).map(|(x, c)| x * c).sum();
                let mu = inv_link(eta, lf);
                let ok = match dist {
                    Distribution::Binomial => mu > 0.0 && mu < 1.0,
                    _ => mu > 0.0,
                };
                if !ok || !mu.is_finite() {
                    return false;
                }
            }
            true
        };

        let mut step = 1.0_f64;
        let mut trial = beta.clone();
        for _ in 0..40 {
            for j in 0..p_param {
                trial[j] = beta[j] + step * delta[j];
            }
            if mu_in_range(&trial) {
                break;
            }
            step *= 0.5;
        }
        beta = trial;

        // GCONV convergence check (scaled by the realized step length)
        let max_delta = delta
            .iter()
            .map(|d| (step * d).abs())
            .fold(0.0_f64, f64::max);
        let max_beta = beta.iter().map(|b| b.abs()).fold(0.0_f64, f64::max);
        let gconv = max_delta / (1.0 + max_beta);
        if gconv < 1e-8 {
            converged = true;
            break;
        }
    }

    if !converged {
        // NOTE (not a panic): report non-convergence and stop this PROC cleanly.
        session
            .log
            .note("PROC GENMOD failed to converge within the iteration limit.");
        return Err(SasError::runtime("PROC GENMOD failed to converge"));
    }

    // ── Final H = X'WX at convergence ────────────────────────────────────
    let mut final_hessian: Vec<Vec<f64>> = vec![vec![0.0; p_param]; p_param];
    let mut final_mu: Vec<f64> = Vec::with_capacity(n_obs);

    for i in 0..n_obs {
        let xi = &x_mat[i];
        let eta: f64 = xi.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
        let mu = inv_link(eta, lf);
        let v = variance(mu, dist);
        let dg = deta_dmu(mu, lf);
        let w_irls = freq_vec[i] / (v * dg * dg);
        final_mu.push(mu);
        for j in 0..p_param {
            for k in 0..p_param {
                final_hessian[j][k] += xi[j] * xi[k] * w_irls;
            }
        }
    }

    let h_inv = invert_matrix(&final_hessian)?;

    // ── 10. Scale / Dispersion ────────────────────────────────────────────
    // Poisson, Binomial: scale=1 (fixed, DF=0).
    // Normal: φ̂ = MSE = SSE/(n−p); reported Scale = √φ̂ = σ̂, DF=n−p.
    // Gamma:  Pearson dispersion φ̂ = (1/(n−p)) Σ (y−μ)²/μ²; reported Scale is
    //         the estimate of 1/φ (SAS GENMOD reports the Gamma "Scale" in the
    //         1/φ form). The exact SAS default uses the ML (digamma) scale; we
    //         approximate it by the moment-based Pearson dispersion — the two
    //         agree asymptotically but differ in finite samples. DF=n−p.
    //
    // NOSCALE / SCALE=: when NOSCALE, the dispersion is held FIXED rather than
    // estimated (φ=1 by default, or derived from SCALE=); when SCALE=v is given
    // the reported Scale is fixed at v (for Normal v=σ ⇒ φ=v²; for Gamma the
    // value is the 1/φ form ⇒ φ=1/v). A fixed scale carries DF=0.
    //
    // `disp_phi` is the dispersion φ used to scale Var(β̂)=φ·H⁻¹ and the scaled
    // criteria. `scale_est` is the value printed on the Scale row.
    let scale_est: f64;
    let scale_df: i64;
    let disp_phi: f64;
    let var_beta: Vec<Vec<f64>>;

    let df_err = (n_total as i64) - (p_param as i64);
    let fixed_scale = model.noscale || model.scale.is_some();

    match dist {
        Distribution::Normal => {
            if fixed_scale {
                // SCALE= is σ for Normal (default 1 under NOSCALE alone).
                let sigma = model.scale.unwrap_or(1.0);
                scale_est = sigma;
                disp_phi = sigma * sigma;
                scale_df = 0;
            } else {
                let sse: f64 = y_vec
                    .iter()
                    .zip(final_mu.iter())
                    .zip(freq_vec.iter())
                    .map(|((y, mu), w)| w * (y - mu) * (y - mu))
                    .sum();
                let mse = sse / (df_err as f64);
                scale_est = mse.sqrt();
                disp_phi = mse;
                scale_df = df_err;
            }
            var_beta = h_inv
                .iter()
                .map(|row| row.iter().map(|v| disp_phi * v).collect())
                .collect();
        }
        Distribution::Gamma => {
            // Pearson dispersion φ̂ = (1/(n−p)) Σ w (y−μ)²/μ².
            let pearson_disp: f64 = {
                let s: f64 = (0..n_obs)
                    .map(|i| {
                        let y = y_vec[i];
                        let mu = final_mu[i];
                        freq_vec[i] * (y - mu) * (y - mu) / (mu * mu)
                    })
                    .sum();
                s / (df_err as f64)
            };
            if fixed_scale {
                // For Gamma the printed Scale is the 1/φ form; SCALE=v ⇒ φ=1/v.
                let s = model.scale.unwrap_or(1.0);
                scale_est = s;
                disp_phi = if s != 0.0 { 1.0 / s } else { 1.0 };
                scale_df = 0;
            } else {
                disp_phi = pearson_disp;
                scale_est = if pearson_disp != 0.0 {
                    1.0 / pearson_disp
                } else {
                    0.0
                };
                scale_df = df_err;
            }
            var_beta = h_inv
                .iter()
                .map(|row| row.iter().map(|v| disp_phi * v).collect())
                .collect();
        }
        _ => {
            // Poisson / Binomial: dispersion fixed at 1 (φ=1), Scale row = 1.
            // SCALE= is accepted but only rescales the scaled criteria below; the
            // SEs are left at the φ=1 model (overdispersion adjustment of SEs is
            // deferred).
            scale_est = 1.0;
            scale_df = 0;
            disp_phi = 1.0;
            var_beta = h_inv;
        }
    }

    // ── 11. SE, Wald chi², CI ─────────────────────────────────────────────
    let se_beta: Vec<f64> = (0..p_param).map(|j| var_beta[j][j].sqrt()).collect();
    let wald_chi2: Vec<f64> = (0..p_param)
        .map(|j| (beta[j] / se_beta[j]).powi(2))
        .collect();
    let wald_p: Vec<f64> = wald_chi2.iter().map(|&w| chisq_sf(w, 1.0)).collect();

    // ── 12. Log-likelihood, GOF ───────────────────────────────────────────
    let log_lik: f64 = match dist {
        Distribution::Poisson => (0..n_obs)
            .map(|i| {
                let mu = final_mu[i];
                let y = y_vec[i];
                let fi = freq_vec[i];
                fi * (y * mu.ln() - mu)
            })
            .sum(),
        Distribution::Binomial => (0..n_obs)
            .map(|i| {
                let mu = final_mu[i].clamp(1e-15, 1.0 - 1e-15);
                let y = y_vec[i];
                let fi = freq_vec[i];
                fi * (y * mu.ln() + (1.0 - y) * (1.0 - mu).ln())
            })
            .sum(),
        Distribution::Normal => {
            let sigma2 = scale_est * scale_est;
            (0..n_obs)
                .map(|i| {
                    let y = y_vec[i];
                    let mu = final_mu[i];
                    let fi = freq_vec[i];
                    -0.5 * fi * ((y - mu) * (y - mu) / sigma2 + (2.0 * PI * sigma2).ln())
                })
                .sum()
        }
        Distribution::Gamma => {
            // Full Gamma log-likelihood with dispersion φ (shape ν = 1/φ):
            //   Σ fi·[ (ν−1)·ln y − ν·y/μ − ν·ln μ + ν·ln ν − ln Γ(ν) ].
            let nu = if disp_phi > 0.0 { 1.0 / disp_phi } else { 1.0 };
            (0..n_obs)
                .map(|i| {
                    let y = y_vec[i].max(1e-300);
                    let mu = final_mu[i].max(1e-300);
                    let fi = freq_vec[i];
                    fi * ((nu - 1.0) * y.ln() - nu * y / mu - nu * mu.ln()
                        + nu * nu.ln()
                        - crate::stat::ln_gamma(nu))
                })
                .sum()
        }
    };

    let deviance: f64 = match dist {
        Distribution::Poisson => (0..n_obs)
            .map(|i| {
                let y = y_vec[i];
                let mu = final_mu[i];
                let fi = freq_vec[i];
                let t1 = if y > 0.0 { y * (y / mu).ln() } else { 0.0 };
                fi * 2.0 * (t1 - (y - mu))
            })
            .sum(),
        Distribution::Binomial => (0..n_obs)
            .map(|i| {
                let y = y_vec[i];
                let mu = final_mu[i].clamp(1e-15, 1.0 - 1e-15);
                let fi = freq_vec[i];
                fi * dev_contribution_binom(y, mu)
            })
            .sum(),
        Distribution::Normal => (0..n_obs)
            .map(|i| {
                let y = y_vec[i];
                let mu = final_mu[i];
                let fi = freq_vec[i];
                fi * (y - mu) * (y - mu)
            })
            .sum(),
        // Gamma deviance = 2·Σ fi·[ −ln(y/μ) + (y−μ)/μ ].
        Distribution::Gamma => (0..n_obs)
            .map(|i| {
                let y = y_vec[i].max(1e-300);
                let mu = final_mu[i].max(1e-300);
                let fi = freq_vec[i];
                fi * 2.0 * (-(y / mu).ln() + (y - mu) / mu)
            })
            .sum(),
    };

    let pearson: f64 = (0..n_obs)
        .map(|i| {
            let y = y_vec[i];
            let mu = final_mu[i];
            let fi = freq_vec[i];
            let v = variance(mu, dist);
            fi * (y - mu) * (y - mu) / v
        })
        .sum();

    let df_gof = (n_total as i64) - (p_param as i64);

    // Scaled deviance and scaled Pearson: divide by the dispersion φ. For Normal
    // φ=σ²=scale_est² and for Poisson/Binomial φ=1, so the values match the
    // previous scale_est² form byte-for-byte; for Gamma φ is the dispersion
    // (scale_est=1/φ), so dividing by φ (not scale_est²) is the correct scaling.
    let scaled_deviance = deviance / disp_phi;
    let scaled_pearson = pearson / disp_phi;

    // Information criteria — n_params excludes Scale for Poisson/Binomial
    // (scale fixed, not estimated), but for Normal Scale is estimated via
    // a moment estimator (not MLE parameter in the LL); SAS GENMOD uses p_param
    // (just regression params) for AIC/BIC of all three distributions.
    let n_params = p_param; // intercept + predictors (no scale term)
    let aic = -2.0 * log_lik + 2.0 * (n_params as f64);
    let aicc = aic
        + 2.0 * (n_params as f64) * (n_params as f64 + 1.0)
            / (n_total - n_params as f64 - 1.0);
    let bic = -2.0 * log_lik + (n_params as f64) * n_total.ln();

    // Scale SE (Normal/Gamma, when estimated): SE(scale) ≈ scale / sqrt(2·df).
    let se_scale: f64 = if (*dist == Distribution::Normal
        || *dist == Distribution::Gamma)
        && scale_df > 0
    {
        scale_est / (2.0 * scale_df as f64).sqrt()
    } else {
        0.0
    };

    // ── 13. Listing — GOF table ───────────────────────────────────────────
    if !model.noprint {
        centered(session, "Criteria For Assessing Goodness Of Fit");
        session.listing.blank();

        let gof_headers: Vec<String> = vec![
            "Criterion".into(),
            "DF".into(),
            "Value".into(),
            "Value/DF".into(),
        ];
        let gof_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];

        let df_str = df_gof.to_string();
        let blank = "".to_string();

        let gof_rows: Vec<Vec<String>> = vec![
            vec![
                "Deviance".into(),
                df_str.clone(),
                fmt4(deviance),
                fmt4(deviance / df_gof as f64),
            ],
            vec![
                "Scaled Deviance".into(),
                df_str.clone(),
                fmt4(scaled_deviance),
                fmt4(scaled_deviance / df_gof as f64),
            ],
            vec![
                "Pearson Chi-Square".into(),
                df_str.clone(),
                fmt4(pearson),
                fmt4(pearson / df_gof as f64),
            ],
            vec![
                "Scaled Pearson X2".into(),
                df_str.clone(),
                fmt4(scaled_pearson),
                fmt4(scaled_pearson / df_gof as f64),
            ],
            vec!["Log Likelihood".into(), blank.clone(), fmt4(log_lik), blank.clone()],
            vec!["Full Log Likelihood".into(), blank.clone(), fmt4(log_lik), blank.clone()],
            vec!["AIC (smaller is better)".into(), blank.clone(), fmt4(aic), blank.clone()],
            vec!["AICC (smaller is better)".into(), blank.clone(), fmt4(aicc), blank.clone()],
            vec!["BIC (smaller is better)".into(), blank.clone(), fmt4(bic), blank.clone()],
        ];

        session
            .listing
            .write_table(&gof_headers, &gof_aligns, &gof_rows);
        session.listing.blank();

        // ── 14. Analysis of ML Parameter Estimates ────────────────────────
        centered(session, "Analysis Of Maximum Likelihood Parameter Estimates");
        session.listing.blank();

        let amle_headers: Vec<String> = vec![
            "Parameter".into(),
            "DF".into(),
            "Estimate".into(),
            "Standard Error".into(),
            "Wald 95% Confidence Limits Lower".into(),
            "Wald 95% Confidence Limits Upper".into(),
            "Wald Chi-Square".into(),
            "Pr > ChiSq".into(),
        ];
        let amle_aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];

        let mut amle_rows: Vec<Vec<String>> = Vec::with_capacity(p_param + 1);

        // Intercept row.
        {
            let ci_lower = beta[0] - 1.96 * se_beta[0];
            let ci_upper = beta[0] + 1.96 * se_beta[0];
            amle_rows.push(vec![
                "Intercept".to_string(),
                "1".into(),
                fmt4(beta[0]),
                fmt4(se_beta[0]),
                fmt4(ci_lower),
                fmt4(ci_upper),
                fmt4(wald_chi2[0]),
                fmt_p_opt(wald_p[0]),
            ]);
        }

        // Predictor / CLASS rows. `col` walks the β vector starting after the
        // intercept; CLASS factors emit one row per non-reference level (label
        // "classvar level") followed by a reference-level row (estimate 0, DF 0,
        // ref = last level). Continuous predictors emit a single row labelled by
        // the variable name (byte-identical to the pre-CLASS layout).
        let mut col = 1usize;
        for term in &design_terms {
            match term {
                DesignTerm::Continuous { name, .. } => {
                    let j = col;
                    let ci_lower = beta[j] - 1.96 * se_beta[j];
                    let ci_upper = beta[j] + 1.96 * se_beta[j];
                    amle_rows.push(vec![
                        name.clone(),
                        "1".into(),
                        fmt4(beta[j]),
                        fmt4(se_beta[j]),
                        fmt4(ci_lower),
                        fmt4(ci_upper),
                        fmt4(wald_chi2[j]),
                        fmt_p_opt(wald_p[j]),
                    ]);
                    col += 1;
                }
                DesignTerm::Class { name, levels, .. } => {
                    let nref = levels.len() - 1;
                    for li in 0..nref {
                        let j = col + li;
                        let lbl = format!("{} {}", name, class_level_label(&levels[li]));
                        let ci_lower = beta[j] - 1.96 * se_beta[j];
                        let ci_upper = beta[j] + 1.96 * se_beta[j];
                        amle_rows.push(vec![
                            lbl,
                            "1".into(),
                            fmt4(beta[j]),
                            fmt4(se_beta[j]),
                            fmt4(ci_lower),
                            fmt4(ci_upper),
                            fmt4(wald_chi2[j]),
                            fmt_p_opt(wald_p[j]),
                        ]);
                    }
                    // Reference level row (last level): estimate 0, DF 0.
                    let ref_lbl =
                        format!("{} {}", name, class_level_label(&levels[nref]));
                    amle_rows.push(vec![
                        ref_lbl,
                        "0".into(),
                        fmt4(0.0),
                        fmt4(0.0),
                        fmt4(0.0),
                        fmt4(0.0),
                        ".".into(),
                        ".".into(),
                    ]);
                    col += nref;
                }
            }
        }

        // Scale row. Normal: Scale=σ̂ with a Wald CI from se_scale. Gamma: Scale
        // is the 1/φ estimate (Pearson-based); we report the estimate and its CI
        // from se_scale (≈ Scale/√(2·df)). Poisson/Binomial: fixed at 1.
        let (scale_ci_lower, scale_ci_upper) =
            if *dist == Distribution::Normal || *dist == Distribution::Gamma {
                (
                    fmt4((scale_est - 1.96 * se_scale).max(0.0)),
                    fmt4(scale_est + 1.96 * se_scale),
                )
            } else {
                (fmt4(1.0), fmt4(1.0))
            };

        amle_rows.push(vec![
            "Scale".into(),
            scale_df.to_string(),
            fmt4(scale_est),
            fmt4(se_scale),
            scale_ci_lower,
            scale_ci_upper,
            ".".into(), // no Wald for scale row
            ".".into(),
        ]);

        session
            .listing
            .write_table(&amle_headers, &amle_aligns, &amle_rows);
    }

    Ok(())
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

    fn parse_genmod(src: &str) -> Result<GenmodAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // genmod
        parse(&mut ts)
    }

    /// Create the Poisson oracle session: y ∈ {1,2,3,4,5,6}, x ∈ {0,0,0,1,1,1}
    fn make_poisson_session() -> (Session, GenmodAst) {
        let session = make_session();
        let frame = df![
            "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
            "x" => [0.0_f64, 0.0, 0.0, 1.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("POIS", &ds).unwrap();

        let ast = GenmodAst {
            data_options: GenmodDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "POIS".into(),
                }),
            },
            class_vars: vec![],
            model: Some(GenmodModel {
                response: "y".into(),
                event: None,
                descending: false,
                predictors: vec!["x".into()],
                dist: Distribution::Poisson,
                link: LinkFunction::Log,
                noprint: false,
                scale: None,
                noscale: false,
            }),
            freq_var: None,
        };
        (session, ast)
    }

    // ── Parse tests ──────────────────────────────────────────────────────

    #[test]
    fn test_parse_poisson_log() {
        let ast = parse_genmod("proc genmod; model y = x / dist=poisson link=log; run;").unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.dist, Distribution::Poisson);
        assert_eq!(m.link, LinkFunction::Log);
    }

    #[test]
    fn test_parse_binomial_logit() {
        // dist=binomial without explicit link → canonical Logit
        let ast = parse_genmod("proc genmod; model y = x / dist=binomial; run;").unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.dist, Distribution::Binomial);
        assert_eq!(m.link, LinkFunction::Logit);
    }

    #[test]
    fn test_parse_normal_identity() {
        // dist=normal without explicit link → canonical Identity
        let ast = parse_genmod("proc genmod; model y = x / dist=normal; run;").unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.dist, Distribution::Normal);
        assert_eq!(m.link, LinkFunction::Identity);
    }

    #[test]
    fn test_parse_descending() {
        let ast = parse_genmod("proc genmod; model y(descending) = x / dist=binomial; run;").unwrap();
        assert!(ast.model.unwrap().descending);
    }

    #[test]
    fn test_parse_event() {
        let ast = parse_genmod("proc genmod; model y(event='1') = x / dist=binomial; run;").unwrap();
        assert_eq!(ast.model.unwrap().event, Some("1".to_string()));
    }

    #[test]
    fn test_parse_gamma_ok() {
        // Parse should succeed (error deferred to execute)
        let ast = parse_genmod("proc genmod; model y = x / dist=gamma; run;");
        assert!(ast.is_ok(), "DIST=GAMMA parse should succeed");
        assert_eq!(ast.unwrap().model.unwrap().dist, Distribution::Gamma);
    }

    #[test]
    fn test_parse_gamma_default_link_reciprocal() {
        // DIST=GAMMA without an explicit LINK= → canonical reciprocal.
        let ast = parse_genmod("proc genmod; model y = x / dist=gamma; run;").unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.dist, Distribution::Gamma);
        assert_eq!(m.link, LinkFunction::Reciprocal);
    }

    #[test]
    fn test_parse_gamma_link_log() {
        let ast =
            parse_genmod("proc genmod; model y = x / dist=gamma link=log; run;").unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.dist, Distribution::Gamma);
        assert_eq!(m.link, LinkFunction::Log);
    }

    #[test]
    fn test_parse_scale_noscale() {
        let ast =
            parse_genmod("proc genmod; model y = x / dist=normal noscale; run;").unwrap();
        assert!(ast.model.unwrap().noscale);
        let ast2 =
            parse_genmod("proc genmod; model y = x / dist=normal scale=2.5; run;").unwrap();
        assert_eq!(ast2.model.unwrap().scale, Some(2.5));
    }

    // ── Execute tests — Poisson oracle ───────────────────────────────────

    fn run_poisson() -> String {
        let (mut session, ast) = make_poisson_session();
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    }

    #[test]
    fn test_execute_poisson_beta0() {
        let (mut session, ast) = make_poisson_session();
        let mut ast2 = ast.clone();
        ast2.model.as_mut().unwrap().noprint = true;

        // Run directly and check beta via log — use execute with noprint off
        // to exercise the path, but check values through listing
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // β₀ ≈ ln(2) = 0.6931
        assert!(
            listing.contains("0.6931") || listing.contains("0.693"),
            "β₀ not found: {listing}"
        );
    }

    #[test]
    fn test_execute_poisson_beta1() {
        let listing = run_poisson();
        // β₁ ≈ ln(5/2) = 0.9163
        assert!(
            listing.contains("0.9163") || listing.contains("0.916"),
            "β₁ not found: {listing}"
        );
    }

    #[test]
    fn test_execute_poisson_se() {
        let listing = run_poisson();
        // SE(β₁) ≈ 0.4830
        assert!(
            listing.contains("0.4830") || listing.contains("0.483"),
            "SE(β₁) not found: {listing}"
        );
    }

    // ── Execute tests — Normal oracle ────────────────────────────────────

    fn make_normal_session() -> (Session, GenmodAst) {
        let session = make_session();
        let frame = df![
            "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
            "x" => [0.0_f64, 0.0, 0.0, 1.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("POIS", &ds).unwrap();

        let ast = GenmodAst {
            data_options: GenmodDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "POIS".into(),
                }),
            },
            class_vars: vec![],
            model: Some(GenmodModel {
                response: "y".into(),
                event: None,
                descending: false,
                predictors: vec!["x".into()],
                dist: Distribution::Normal,
                link: LinkFunction::Identity,
                noprint: false,
                scale: None,
                noscale: false,
            }),
            freq_var: None,
        };
        (session, ast)
    }

    #[test]
    fn test_execute_normal_beta() {
        let (mut session, ast) = make_normal_session();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // β₀ = 2.0000, β₁ = 3.0000
        assert!(
            listing.contains("2.0000") || listing.contains("2.000"),
            "β₀ not found: {listing}"
        );
        assert!(
            listing.contains("3.0000") || listing.contains("3.000"),
            "β₁ not found: {listing}"
        );
    }

    #[test]
    fn test_execute_normal_scale() {
        let (mut session, ast) = make_normal_session();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // Scale = sqrt(MSE) = sqrt(1.0) = 1.0000
        assert!(
            listing.contains("1.0000") || listing.contains("1.000"),
            "Scale not found: {listing}"
        );
    }

    // ── Gamma + CLASS + SCALE tests (M34.7) ──────────────────────────────

    fn char_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.into(),
            ty: VarType::Char,
            length: 8,
            format: None,
            label: None,
        }
    }

    /// Intercept-only Gamma; y has mean ȳ. `link` selects LOG or RECIPROCAL.
    fn make_gamma_intercept_session(link: LinkFunction) -> (Session, GenmodAst) {
        let session = make_session();
        let frame = df![
            "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y")],
        };
        session.libs.get("WORK").unwrap().write("GAM", &ds).unwrap();
        let ast = GenmodAst {
            data_options: GenmodDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "GAM".into(),
                }),
            },
            class_vars: vec![],
            model: Some(GenmodModel {
                response: "y".into(),
                event: None,
                descending: false,
                predictors: vec![],
                dist: Distribution::Gamma,
                link,
                noprint: true,
                scale: None,
                noscale: false,
            }),
            freq_var: None,
        };
        (session, ast)
    }

    /// Pull β̂₀ from the listing of an intercept-only model by parsing the
    /// Intercept row's Estimate column. Easier: run with noprint=false and grep.
    fn gamma_intercept_estimate(link: LinkFunction) -> f64 {
        let (mut session, mut ast) = make_gamma_intercept_session(link);
        ast.model.as_mut().unwrap().noprint = false;
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // Find the "Intercept" line and take its first numeric token.
        let line = listing
            .lines()
            .find(|l| l.trim_start().starts_with("Intercept"))
            .expect("Intercept row");
        // tokens after "Intercept": DF Estimate ...
        let toks: Vec<&str> = line.split_whitespace().collect();
        // toks[0]="Intercept", toks[1]=DF("1"), toks[2]=Estimate
        toks[2].parse::<f64>().expect("estimate parse")
    }

    #[test]
    fn test_gamma_intercept_only_log_link() {
        // LINK=LOG intercept-only Gamma ⇒ β̂₀ = ln(ȳ); ȳ = 3.5.
        let est = gamma_intercept_estimate(LinkFunction::Log);
        let expected = (3.5_f64).ln();
        assert!(
            (est - expected).abs() < 1e-3,
            "log-link intercept {est} vs ln(3.5)={expected}"
        );
    }

    #[test]
    fn test_gamma_intercept_only_reciprocal_link() {
        // Canonical reciprocal intercept-only Gamma ⇒ β̂₀ = 1/ȳ; ȳ = 3.5.
        let est = gamma_intercept_estimate(LinkFunction::Reciprocal);
        let expected = 1.0 / 3.5;
        assert!(
            (est - expected).abs() < 1e-3,
            "reciprocal-link intercept {est} vs 1/3.5={expected}"
        );
    }

    #[test]
    fn test_gamma_pearson_dispersion() {
        // Independently verify the Pearson dispersion φ̂ for an intercept-only
        // reciprocal-link Gamma: μ̂ = ȳ for every obs, so
        //   φ̂ = (1/(n−1)) Σ (y−ȳ)²/ȳ².
        let (mut session, mut ast) =
            make_gamma_intercept_session(LinkFunction::Reciprocal);
        ast.model.as_mut().unwrap().noprint = false;
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();

        let y = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
        let ybar = 3.5;
        let phi: f64 =
            y.iter().map(|v| (v - ybar).powi(2) / (ybar * ybar)).sum::<f64>() / 5.0;
        // Reported Scale = 1/φ.
        let scale = 1.0 / phi;
        let scale_str = format!("{scale:.4}");
        assert!(
            listing.contains(&scale_str),
            "expected Scale={scale_str} (1/φ) in listing:\n{listing}"
        );
    }

    /// 2-level CLASS predictor must equal manual 0/1 dummy coding of the same
    /// predictor (reference = last level → "b", design column flags level "a").
    #[test]
    fn test_class_two_level_equals_manual_dummy() {
        // CLASS version: group ∈ {a,a,a,b,b,b}, y Poisson counts.
        let session = make_session();
        let frame_c = df![
            "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
            "g" => ["a", "a", "a", "b", "b", "b"]
        ]
        .unwrap();
        let ds_c = SasDataset {
            df: frame_c,
            vars: vec![num_meta("y"), char_meta("g")],
        };
        session.libs.get("WORK").unwrap().write("CLS", &ds_c).unwrap();
        let ast_c = GenmodAst {
            data_options: GenmodDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "CLS".into(),
                }),
            },
            class_vars: vec!["g".into()],
            model: Some(GenmodModel {
                response: "y".into(),
                event: None,
                descending: false,
                predictors: vec!["g".into()],
                dist: Distribution::Poisson,
                link: LinkFunction::Log,
                noprint: false,
                scale: None,
                noscale: false,
            }),
            freq_var: None,
        };
        let mut s_c = session;
        execute(&ast_c, &mut s_c).unwrap();
        let listing_c = s_c.listing.into_string();

        // Manual dummy: d = 1 if g=="a" else 0 (ref = last level "b").
        let session2 = make_session();
        let frame_d = df![
            "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
            "d" => [1.0_f64, 1.0, 1.0, 0.0, 0.0, 0.0]
        ]
        .unwrap();
        let ds_d = SasDataset {
            df: frame_d,
            vars: vec![num_meta("y"), num_meta("d")],
        };
        session2.libs.get("WORK").unwrap().write("DUM", &ds_d).unwrap();
        let ast_d = GenmodAst {
            data_options: GenmodDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "DUM".into(),
                }),
            },
            class_vars: vec![],
            model: Some(GenmodModel {
                response: "y".into(),
                event: None,
                descending: false,
                predictors: vec!["d".into()],
                dist: Distribution::Poisson,
                link: LinkFunction::Log,
                noprint: false,
                scale: None,
                noscale: false,
            }),
            freq_var: None,
        };
        let mut s_d = session2;
        execute(&ast_d, &mut s_d).unwrap();
        let listing_d = s_d.listing.into_string();

        // Both fit β̂ for the "a vs b" contrast = ln(2) − ln(5) = ln(0.4).
        let contrast = (2.0_f64).ln() - (5.0_f64).ln();
        let contrast_str = format!("{contrast:.4}");
        assert!(
            listing_c.contains(&contrast_str),
            "CLASS contrast {contrast_str} missing:\n{listing_c}"
        );
        assert!(
            listing_d.contains(&contrast_str),
            "manual-dummy contrast {contrast_str} missing:\n{listing_d}"
        );
        // The Class Level Information table must appear for the CLASS run.
        assert!(listing_c.contains("Class Level Information"));
        // Reference level row "g b" with DF 0 must be present.
        assert!(
            listing_c.contains("g b"),
            "reference-level row 'g b' missing:\n{listing_c}"
        );
    }

    /// Design-matrix dimensionality: a 3-level CLASS contributes L−1=2 columns,
    /// plus a continuous predictor and the intercept ⇒ p = 4.
    #[test]
    fn test_design_matrix_dimensions() {
        let three = DesignTerm::Class {
            name: "g".into(),
            col: 0,
            levels: vec![
                Value::Char("a".into()),
                Value::Char("b".into()),
                Value::Char("c".into()),
            ],
        };
        assert_eq!(three.n_cols(), 2);
        let cont = DesignTerm::Continuous {
            name: "x".into(),
            col: 1,
        };
        assert_eq!(cont.n_cols(), 1);
        // intercept + 2 (class) + 1 (continuous) = 4 parameters.
        let p_param = 1 + three.n_cols() + cont.n_cols();
        assert_eq!(p_param, 4);
    }

    #[test]
    fn test_scale_fixed_normal_noscale() {
        // NOSCALE on Normal fixes σ at 1 ⇒ Scale row = 1.0000, DF 0.
        let (mut session, mut ast) = make_normal_session();
        ast.model.as_mut().unwrap().noscale = true;
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        let scale_line = listing
            .lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>())
            .find(|toks| toks.first() == Some(&"Scale"))
            .expect("Scale row");
        // toks: ["Scale", DF, Estimate, ...]
        assert_eq!(scale_line[1], "0", "NOSCALE ⇒ DF 0: {scale_line:?}");
        assert_eq!(scale_line[2], "1.0000", "NOSCALE ⇒ Scale 1: {scale_line:?}");
    }
}
