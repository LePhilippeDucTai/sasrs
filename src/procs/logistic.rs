//! PROC LOGISTIC — logistic regression via Newton-Raphson MLE (M26.1, M34.6).
//!
//! Supports:
//! - Binary response (two levels) and ordinal response (>2 ordered levels,
//!   proportional-odds cumulative-logit model) (M34.6).
//! - FREQ statement (weighted observations).
//! - MODEL statement with DESCENDING option and EVENT= option.
//! - CLASS variables: reference-cell (PARAM=REF, ref=last level) coding (M34.6).
//!   NOTE: SAS's default CLASS parameterization is EFFECT coding; we implement
//!   reference-cell coding (the GLM/`PARAM=REF` convention) instead. This is a
//!   documented deviation — the design columns and the resulting parameter
//!   estimates therefore match `PROC LOGISTIC ... (PARAM=REF REF=LAST)`.
//! - LINK= option: LOGIT (default), CLOGLOG, PROBIT (M34.6).
//! - OUTPUT OUT= PREDICTED=/P= XBETA= statement (M34.6).
//! - Produces: Model Information, Class Level Information (when CLASS present),
//!   Response Profile, Model Fit Statistics, Global Null tests (LR, Score, Wald),
//!   Analysis of ML Estimates, and (logit only) Odds Ratio Estimates.

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::{chisq_sf, decode_column, probnorm};
use crate::session::Session;
use crate::stat::invert_matrix;
use crate::token::TokenKind;
use crate::value::{format_best, VarType, Value};
use polars::prelude::{Column, DataFrame, NamedFrom, Series};

/// Link function for the (binary) logistic model. `Logit` is the default and
/// reproduces the canonical-link IRLS exactly; the other two branch only where
/// the mean/derivative differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Logit,
    Cloglog,
    Probit,
}

impl Link {
    /// Mean μ = g⁻¹(η).
    fn mean(self, eta: f64) -> f64 {
        match self {
            Link::Logit => 1.0 / (1.0 + (-eta).exp()),
            Link::Probit => probnorm(eta),
            Link::Cloglog => 1.0 - (-(eta.exp())).exp(),
        }
    }

    /// dμ/dη.
    fn dmu_deta(self, eta: f64) -> f64 {
        match self {
            Link::Logit => {
                let p = 1.0 / (1.0 + (-eta).exp());
                p * (1.0 - p)
            }
            // φ(η): standard normal pdf.
            Link::Probit => (-0.5 * eta * eta).exp() / (2.0 * std::f64::consts::PI).sqrt(),
            // exp(η − exp(η)).
            Link::Cloglog => (eta - eta.exp()).exp(),
        }
    }

}

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct LogisticAst {
    pub data_options: LogisticDataOptions,
    pub class_vars: Vec<String>,
    pub model: Option<LogisticModel>,
    pub freq_var: Option<String>,
    pub outputs: Vec<LogisticOutput>,
}

#[derive(Debug, Clone)]
pub struct LogisticDataOptions {
    pub input: Option<DatasetRef>,
}

#[derive(Debug, Clone)]
pub struct LogisticModel {
    pub response: String,
    pub event: Option<String>,
    pub descending: bool,
    pub predictors: Vec<String>,
    pub noprint: bool,
    pub link: Link,
}

/// `OUTPUT OUT=ds PREDICTED=name | P=name [XBETA=name]`.
#[derive(Debug, Clone)]
pub struct LogisticOutput {
    pub out: DatasetRef,
    pub predicted: Option<String>,
    pub xbeta: Option<String>,
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC LOGISTIC. Called AFTER `proc logistic` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<LogisticAst> {
    let mut input: Option<DatasetRef> = None;

    // PROC LOGISTIC statement options, until `;`
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
            // Skip unknown proc-level options (DESCENDING as proc option: ignored)
            ts.next();
        }
    }

    // Sub-statements until run;/quit;
    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<LogisticModel> = None;
    let mut freq_var: Option<String> = None;
    let mut outputs: Vec<LogisticOutput> = Vec::new();

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "class" {
            ts.next(); // consume "class"
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
            // Parse response variable name
            let response = ts
                .peek()
                .ident()
                .map(str::to_string)
                .ok_or_else(|| SasError::parse("expected response variable", ts.peek().span))?;
            ts.next();

            // Parse optional response options: (event='val' descending ...)
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
                            ts.next(); // consume '='
                            // Accept string literal (single or double quoted)
                            if let TokenKind::Str { value, .. } = &ts.peek().kind.clone() {
                                event = Some(value.clone());
                                ts.next();
                            }
                        }
                    } else if ts.peek().is_kw("descending") {
                        descending = true;
                        ts.next();
                    } else {
                        ts.next(); // skip unknown options
                    }
                }
                if ts.peek().kind == TokenKind::RParen {
                    ts.next(); // consume ')'
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

            // Parse predictors until '/' or ';'
            let mut predictors: Vec<String> = Vec::new();
            let mut noprint = false;
            let mut link = Link::Logit;

            loop {
                if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                    break;
                }
                if ts.peek().kind == TokenKind::Slash {
                    ts.next(); // consume '/'
                    // Parse options until ';'
                    while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                        if ts.peek().is_kw("noprint") {
                            noprint = true;
                            ts.next();
                        } else if ts.peek().is_kw("link") {
                            ts.next(); // consume "link"
                            if ts.peek().kind == TokenKind::Eq {
                                ts.next(); // consume '='
                                if let Some(name) = ts.peek().ident().map(str::to_string) {
                                    link = match name.to_lowercase().as_str() {
                                        "cloglog" | "ccll" => Link::Cloglog,
                                        "probit" | "normit" => Link::Probit,
                                        _ => Link::Logit,
                                    };
                                    ts.next();
                                }
                            }
                        } else {
                            ts.next(); // skip unknown options
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
            model = Some(LogisticModel {
                response,
                event,
                descending,
                predictors,
                noprint,
                link,
            });
            Ok(true)
        } else if kw == "freq" {
            ts.next(); // consume "freq"
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                freq_var = Some(name);
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "output" {
            ts.next(); // consume "output"
            let mut out: Option<DatasetRef> = None;
            let mut predicted: Option<String> = None;
            let mut xbeta: Option<String> = None;
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if ts.peek().is_kw("out") {
                    out = Some(common::parse_out_opt(ts)?);
                } else if ts.peek().is_kw("predicted")
                    || ts.peek().is_kw("pred")
                    || ts.peek().is_kw("prob")
                    || ts.peek().is_kw("p")
                {
                    common::expect_eq(ts, "PREDICTED")?;
                    predicted = ts.peek().ident().map(str::to_string);
                    if predicted.is_some() {
                        ts.next();
                    }
                } else if ts.peek().is_kw("xbeta") {
                    common::expect_eq(ts, "XBETA")?;
                    xbeta = ts.peek().ident().map(str::to_string);
                    if xbeta.is_some() {
                        ts.next();
                    }
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            if let Some(out_ref) = out {
                outputs.push(LogisticOutput {
                    out: out_ref,
                    predicted,
                    xbeta,
                });
            }
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(LogisticAst {
        data_options: LogisticDataOptions { input },
        class_vars,
        model,
        freq_var,
        outputs,
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

// ───────────────────────── Value → display string ─────────────────────────

/// Format a Value for display in the Response Profile (matches SAS best.)
fn value_label(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.trim_end().to_string(),
    }
}

/// Compare a Value against an event string. Returns true if they match.
/// For numeric values, try parsing the event string as f64.
/// For char values, compare trimmed strings.
fn value_matches_event(v: &Value, event: &str) -> bool {
    match v {
        Value::Char(s) => s.trim_end() == event.trim(),
        Value::Num(f) => {
            // Try numeric parse
            if let Ok(ev_num) = event.trim().parse::<f64>() {
                (f - ev_num).abs() < 1e-15
            } else {
                // Fallback: compare string representations
                format_best(*f, 12) == event.trim()
            }
        }
        Value::Missing(_) => false,
    }
}

// ───────────────────────── Matrix helpers ─────────────────────────

/// Multiply matrix (m×k) by vector (k) → vector (m).
fn mat_vec(mat: &[Vec<f64>], vec: &[f64]) -> Vec<f64> {
    mat.iter()
        .map(|row| row.iter().zip(vec.iter()).map(|(a, b)| a * b).sum())
        .collect()
}

/// Inner product of two vectors.
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Extract a sub-matrix (rows `r`, cols `c` from index 1 onwards).
fn submatrix_predictors(mat: &[Vec<f64>], p: usize) -> Vec<Vec<f64>> {
    (1..=p)
        .map(|i| (1..=p).map(|j| mat[i][j]).collect())
        .collect()
}

// ───────────────────────── Design matrix ─────────────────────────

/// Metadata for one MODEL effect (a continuous predictor or a CLASS variable).
/// CLASS variables expand to `levels.len() - 1` reference-cell design columns,
/// with the LAST level (in `sas_cmp` order) as the reference (PARAM=REF, REF=LAST).
struct Effect {
    /// Predictor name as written in MODEL.
    name: String,
    /// Index into the decoded `pred_cols` (== predictor position in MODEL).
    pred_col_idx: usize,
    /// `true` if this effect is a CLASS variable.
    is_class: bool,
    /// Non-reference levels (one design column each), in `sas_cmp` order.
    /// Empty for continuous effects.
    levels: Vec<Value>,
    /// Reference level label (CLASS only).
    ref_label: String,
}

/// The expanded design: parameter labels (one per non-intercept column, index 0
/// = first non-intercept column) and the list of effects in MODEL order.
struct Design {
    effects: Vec<Effect>,
    /// Label for each non-intercept design column.
    col_labels: Vec<String>,
}

impl Design {
    fn n_cols(&self) -> usize {
        self.col_labels.len()
    }
}

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &LogisticAst, session: &mut Session) -> Result<()> {
    // ── 1. Guards ──────────────────────────────────────────────────────────
    let model = ast.model.as_ref().ok_or_else(|| {
        SasError::runtime("MODEL statement required")
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

    // ── Build design (CLASS expansion via reference-cell coding) ───────────
    // PARAM=REF, REF=LAST: a CLASS var with L levels (sas_cmp order) adds L−1
    // design columns, one per non-reference level; the LAST level is reference.
    // (SAS default is EFFECT coding — documented deviation; matches PARAM=REF.)
    let class_set: Vec<String> = ast.class_vars.clone();
    let is_class_var =
        |nm: &str| class_set.iter().any(|c| c.eq_ignore_ascii_case(nm));

    let mut effects: Vec<Effect> = Vec::with_capacity(nb_preds);
    let mut col_labels: Vec<String> = Vec::new();
    for (pi, nm) in predictors.iter().enumerate() {
        if is_class_var(nm) {
            // Collect distinct non-missing levels of this CLASS column.
            let col = &pred_cols[pi];
            let mut levs: Vec<Value> = Vec::new();
            for v in col.iter().take(n_read) {
                if v.is_missing() {
                    continue;
                }
                if !levs.iter().any(|lv| lv.sas_cmp(v) == std::cmp::Ordering::Equal) {
                    levs.push(v.clone());
                }
            }
            levs.sort_by(|a, b| a.sas_cmp(b));
            if levs.len() < 2 {
                return Err(SasError::runtime(format!(
                    "CLASS variable {} must have at least 2 levels.",
                    nm.to_uppercase()
                )));
            }
            let ref_label = value_label(&levs[levs.len() - 1]);
            let non_ref: Vec<Value> = levs[..levs.len() - 1].to_vec();
            for lv in &non_ref {
                col_labels.push(format!("{} {}", nm, value_label(lv)));
            }
            effects.push(Effect {
                name: nm.clone(),
                pred_col_idx: pi,
                is_class: true,
                levels: non_ref,
                ref_label,
            });
        } else {
            col_labels.push(nm.clone());
            effects.push(Effect {
                name: nm.clone(),
                pred_col_idx: pi,
                is_class: false,
                levels: Vec::new(),
                ref_label: String::new(),
            });
        }
    }
    let design = Design {
        effects,
        col_labels,
    };

    // ── 3. Determine response levels (sorted by sas_cmp) ───────────────────
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

    if levels.len() < 2 {
        return Err(SasError::runtime(format!(
            "Response variable {} must have at least 2 non-missing levels (found {}).",
            resp_name.to_uppercase(),
            levels.len()
        )));
    }

    // ── Ordinal branch: >2 ordered response levels → proportional-odds. ────
    if levels.len() > 2 {
        if model.link != Link::Logit {
            return Err(SasError::runtime(
                "Ordinal (multi-level) response is only supported with LINK=LOGIT.",
            ));
        }
        return execute_ordinal(
            ast, session, model, &ds, &in_libref, &in_table, resp_name, &resp_col,
            &pred_cols, &freq_col, &levels, &design, n_read,
        );
    }

    // ── 3b. Binary: determine event level (P(Y=event) is modeled) ──────────
    let event_level: &Value = if let Some(ev_str) = &model.event {
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
        &levels[1]
    } else {
        &levels[0]
    };

    let event_label = value_label(event_level);
    let nonevent_level: &Value = if std::ptr::eq(event_level, &levels[0]) {
        &levels[1]
    } else {
        &levels[0]
    };
    let nonevent_label = value_label(nonevent_level);

    // Number of non-intercept design columns (CLASS-expanded).
    let nb_cols = design.n_cols();

    // ── 4. Listwise deletion + encoding ───────────────────────────────────
    // `complete_mask[i]` marks rows used in the fit (for OUTPUT OUT=).
    let mut y_vec: Vec<f64> = Vec::new();
    let mut x_mat: Vec<Vec<f64>> = Vec::new();
    let mut freq_vec: Vec<f64> = Vec::new();
    let mut complete_mask: Vec<bool> = vec![false; n_read];

    for i in 0..n_read {
        // Skip if response is missing
        if resp_col[i].is_missing() {
            continue;
        }

        // Check freq
        let w = if let Some(fc) = &freq_col {
            match value_to_num(&fc[i]) {
                Some(f) if !f.is_nan() && f > 0.0 => f,
                _ => continue,
            }
        } else {
            1.0
        };

        // Build design row: intercept then expanded effect columns.
        let mut row = vec![1.0_f64]; // intercept
        let mut ok = true;
        for eff in &design.effects {
            let col = &pred_cols[eff.pred_col_idx];
            if eff.is_class {
                // Reference-cell dummies for the current level.
                let v = &col[i];
                if v.is_missing() {
                    ok = false;
                    break;
                }
                for lv in &eff.levels {
                    row.push(
                        if v.sas_cmp(lv) == std::cmp::Ordering::Equal {
                            1.0
                        } else {
                            0.0
                        },
                    );
                }
            } else {
                match value_to_num(&col[i]) {
                    Some(v) if !v.is_nan() => row.push(v),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if !ok {
            continue;
        }

        // Encode response: 1.0 if event, 0.0 otherwise
        let yi = if resp_col[i].sas_cmp(event_level) == std::cmp::Ordering::Equal {
            1.0
        } else {
            0.0
        };

        y_vec.push(yi);
        x_mat.push(row);
        freq_vec.push(w);
        complete_mask[i] = true;
    }

    let n_total: f64 = freq_vec.iter().sum();
    let n_event_total: f64 = y_vec.iter().zip(freq_vec.iter()).map(|(y, w)| y * w).sum();
    let n_nonevent_total = n_total - n_event_total;
    let n_obs = y_vec.len();

    // ── 5. Manual NOTE ────────────────────────────────────────────────────
    session.log.note(&format!(
        "There were {} observations used.",
        n_total as i64
    ));

    if n_obs <= nb_cols + 1 {
        return Err(SasError::runtime(
            "Not enough observations for logistic regression",
        ));
    }

    // Model-description string: "binary logit" stays byte-identical for the
    // default link; other links read "binary <link-name>".
    let model_desc = match model.link {
        Link::Logit => "binary logit".to_string(),
        Link::Probit => "binary probit".to_string(),
        Link::Cloglog => "binary cloglog".to_string(),
    };

    // ── 6. Listing header ─────────────────────────────────────────────────
    if !model.noprint {
        session.listing.page_header();
        centered(session, "The LOGISTIC Procedure");
        session.listing.blank();

        // ── 7. Model Information ──────────────────────────────────────────
        centered(session, "Model Information");
        session.listing.blank();

        let ds_display = format!("{}.{}", in_libref, in_table);
        let info_headers: Vec<String> = vec!["".into(), "".into()];
        let info_aligns = vec![Align::Left, Align::Left];
        let info_rows: Vec<Vec<String>> = vec![
            vec!["Data Set".into(), ds_display],
            vec!["Response Variable".into(), resp_name.clone()],
            vec!["Number of Response Levels".into(), "2".into()],
            vec!["Model".into(), model_desc.clone()],
            vec!["Optimization Technique".into(), "Newton-Raphson".into()],
        ];
        session
            .listing
            .write_table(&info_headers, &info_aligns, &info_rows);
        session.listing.blank();

        // ── 7b. Class Level Information (when CLASS present) ──────────────
        write_class_level_info(session, &design);

        // ── 8. Response Profile ───────────────────────────────────────────
        centered(session, "Response Profile");
        session.listing.blank();

        let rp_headers: Vec<String> = vec![
            "Ordered Value".into(),
            resp_name.clone(),
            "Total Frequency".into(),
        ];
        let rp_aligns = vec![Align::Right, Align::Left, Align::Right];
        let rp_rows: Vec<Vec<String>> = vec![
            vec![
                "1".into(),
                event_label.clone(),
                (n_event_total as i64).to_string(),
            ],
            vec![
                "2".into(),
                nonevent_label.clone(),
                (n_nonevent_total as i64).to_string(),
            ],
        ];
        session
            .listing
            .write_table(&rp_headers, &rp_aligns, &rp_rows);
        session.listing.blank();
        session.listing.write_line(&format!(
            "PROC LOGISTIC is modeling the probability that {}={}.",
            resp_name, event_label
        ));
    }

    // ── 9. NR/IRLS Algorithm ─────────────────────────────────────────────
    let p_bar = n_event_total / n_total;
    let p_param = 1 + nb_cols; // number of parameters (with intercept)
    let link = model.link;
    let mut beta: Vec<f64> = vec![0.0; p_param];
    beta[0] = (p_bar / (1.0 - p_bar)).ln();

    let mut converged = false;
    for _iter in 0..50 {
        // Compute predictions
        let mut score: Vec<f64> = vec![0.0; p_param];
        let mut hessian: Vec<Vec<f64>> = vec![vec![0.0; p_param]; p_param];

        for i in 0..n_obs {
            let xi = &x_mat[i];
            let eta: f64 = xi.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
            let fi = freq_vec[i];

            // `s_resid` is the per-obs score multiplier for x[j]; `wi` the
            // Fisher weight for the Hessian. For LINK=LOGIT these reduce to the
            // canonical (y−μ) and μ(1−μ), reproducing the original code exactly.
            let (s_resid, wi) = if link == Link::Logit {
                let pi = (1.0 / (1.0 + (-eta).exp())).clamp(1e-10, 1.0 - 1e-10);
                (y_vec[i] - pi, pi * (1.0 - pi))
            } else {
                let mu = link.mean(eta).clamp(1e-10, 1.0 - 1e-10);
                let dmu = link.dmu_deta(eta).max(1e-12);
                let var = mu * (1.0 - mu);
                (dmu * (y_vec[i] - mu) / var, dmu * dmu / var)
            };

            // Score (gradient)
            for j in 0..p_param {
                score[j] += fi * xi[j] * s_resid;
            }

            // Hessian = -X'WX (negative information)
            for j in 0..p_param {
                for k in 0..p_param {
                    hessian[j][k] -= fi * xi[j] * xi[k] * wi;
                }
            }
        }

        // Newton step: delta = -H^{-1} * score = solve(-H, score)
        // Negate hessian to get positive definite matrix
        let neg_hessian: Vec<Vec<f64>> = hessian
            .iter()
            .map(|row| row.iter().map(|v| -v).collect())
            .collect();

        let neg_h_inv = invert_matrix(&neg_hessian)?;
        let delta = mat_vec(&neg_h_inv, &score);

        // Update beta
        for j in 0..p_param {
            beta[j] += delta[j];
        }

        // Convergence check (GCONV)
        let max_delta = delta.iter().map(|d| d.abs()).fold(0.0_f64, f64::max);
        let max_beta = beta.iter().map(|b| b.abs()).fold(0.0_f64, f64::max);
        let gconv = max_delta / (1.0 + max_beta);
        if gconv < 1e-8 {
            converged = true;
            break;
        }
    }

    if !converged {
        // Quasi-complete or complete separation, or slow convergence: warn but
        // proceed with the last iterate rather than panicking.
        session.log.note(
            "PROC LOGISTIC: the maximum likelihood estimate may not exist \
             (possible separation); iteration limit reached.",
        );
    }

    // ── 9b. Final Hessian and variance-covariance matrix ─────────────────
    let mut final_hessian: Vec<Vec<f64>> = vec![vec![0.0; p_param]; p_param];
    let mut final_p: Vec<f64> = Vec::with_capacity(n_obs);

    for i in 0..n_obs {
        let xi = &x_mat[i];
        let eta: f64 = xi.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
        let (pi, wi) = if link == Link::Logit {
            let pi = (1.0 / (1.0 + (-eta).exp())).clamp(1e-10, 1.0 - 1e-10);
            (pi, pi * (1.0 - pi))
        } else {
            let mu = link.mean(eta).clamp(1e-10, 1.0 - 1e-10);
            let dmu = link.dmu_deta(eta).max(1e-12);
            (mu, dmu * dmu / (mu * (1.0 - mu)))
        };
        let fi = freq_vec[i];
        final_p.push(pi);
        for j in 0..p_param {
            for k in 0..p_param {
                final_hessian[j][k] -= fi * xi[j] * xi[k] * wi;
            }
        }
    }

    let neg_final_hessian: Vec<Vec<f64>> = final_hessian
        .iter()
        .map(|row| row.iter().map(|v| -v).collect())
        .collect();
    let var_beta = invert_matrix(&neg_final_hessian)?;

    // Standard errors, Wald chi-squares, p-values for each parameter
    let se_beta: Vec<f64> = (0..p_param).map(|j| var_beta[j][j].sqrt()).collect();
    let wald_chi2: Vec<f64> = (0..p_param)
        .map(|j| (beta[j] / se_beta[j]).powi(2))
        .collect();
    let wald_p: Vec<f64> = wald_chi2
        .iter()
        .map(|&w| chisq_sf(w, 1.0))
        .collect();

    // ── 11. Log-likelihoods ───────────────────────────────────────────────
    let log_l: f64 = (0..n_obs)
        .map(|i| {
            let pi = final_p[i];
            let fi = freq_vec[i];
            fi * (y_vec[i] * pi.ln() + (1.0 - y_vec[i]) * (1.0 - pi).ln())
        })
        .sum();

    let log_l_null: f64 = (0..n_obs)
        .map(|i| {
            let fi = freq_vec[i];
            fi * (y_vec[i] * p_bar.ln() + (1.0 - y_vec[i]) * (1.0 - p_bar).ln())
        })
        .sum();

    let neg2log_l = -2.0 * log_l;
    let neg2log_l_null = -2.0 * log_l_null;

    // ── 12. AIC / SC ─────────────────────────────────────────────────────
    let aic = neg2log_l + 2.0 * p_param as f64;
    let sc = neg2log_l + p_param as f64 * n_total.ln();
    let aic_null = neg2log_l_null + 2.0; // 1 parameter (intercept only)
    let sc_null = neg2log_l_null + n_total.ln();

    // ── 13. Global tests under H₀: BETA=0 ────────────────────────────────
    // LR chi2
    let lr_chi2 = neg2log_l_null - neg2log_l;
    let lr_p = chisq_sf(lr_chi2, nb_cols as f64);

    // Wald global: β_c' * Σ_c^{-1} * β_c where Σ_c = submatrix for predictors
    let wald_chi2_global = if nb_cols > 0 {
        let sigma_c = submatrix_predictors(&var_beta, nb_cols);
        let sigma_c_inv = invert_matrix(&sigma_c)?;
        let beta_c: Vec<f64> = (1..=nb_cols).map(|j| beta[j]).collect();
        let tmp = mat_vec(&sigma_c_inv, &beta_c);
        dot(&beta_c, &tmp)
    } else {
        0.0
    };
    let wald_global_p = chisq_sf(wald_chi2_global, nb_cols as f64);

    // Score test (under H₀: β_c=0, β₀=logit(p̄)). Computed with the canonical
    // (logit) Fisher information; for non-logit links it is a close Rao-score
    // approximation to the global association test.
    let score_chi2 = if nb_cols > 0 {
        // Score_c_j = Σ freq_i * x_ij * (y_i - p̄) for j = predictors
        let mut score_c: Vec<f64> = vec![0.0; nb_cols];
        // Score_0 = Σ freq_i * (y_i - p̄)
        let mut score_0: f64 = 0.0;
        // I_cc_jk = p̄*(1-p̄) * Σ freq_i * x_ij * x_ik  (j,k predictors)
        let mut i_cc: Vec<Vec<f64>> = vec![vec![0.0; nb_cols]; nb_cols];
        // I_00 = p̄*(1-p̄) * n_total
        let i_00 = p_bar * (1.0 - p_bar) * n_total;
        // I_0c_j = p̄*(1-p̄) * Σ freq_i * x_ij  (j predictor)
        let mut i_0c: Vec<f64> = vec![0.0; nb_cols];

        for i in 0..n_obs {
            let fi = freq_vec[i];
            let xi = &x_mat[i];
            let resid = y_vec[i] - p_bar;
            score_0 += fi * resid;
            for j in 0..nb_cols {
                score_c[j] += fi * xi[j + 1] * resid;
                i_0c[j] += fi * xi[j + 1];
                for k in 0..nb_cols {
                    i_cc[j][k] += fi * xi[j + 1] * xi[k + 1];
                }
            }
        }
        // Apply p̄*(1-p̄) to I matrices
        let pb_var = p_bar * (1.0 - p_bar);
        for j in 0..nb_cols {
            i_0c[j] *= pb_var;
            for k in 0..nb_cols {
                i_cc[j][k] *= pb_var;
            }
        }

        // Schur complement: I_cc|0 = I_cc - I_c0 * I_00^{-1} * I_0c
        // (I_c0 = I_0c^T for scalar I_00)
        let i_00_inv = 1.0 / i_00;
        let mut i_cc_schur = i_cc.clone();
        for j in 0..nb_cols {
            for k in 0..nb_cols {
                i_cc_schur[j][k] -= i_0c[j] * i_00_inv * i_0c[k];
            }
        }

        // Score_c|0 = Score_c - (I_c0 / I_00) * Score_0
        let mut score_c_schur = score_c.clone();
        for j in 0..nb_cols {
            score_c_schur[j] -= (i_0c[j] / i_00) * score_0;
        }

        // χ²_Score = Score_c|0' * I_cc|0^{-1} * Score_c|0
        let i_cc_schur_inv = invert_matrix(&i_cc_schur)?;
        let tmp = mat_vec(&i_cc_schur_inv, &score_c_schur);
        dot(&score_c_schur, &tmp)
    } else {
        0.0
    };
    let score_p = chisq_sf(score_chi2, nb_cols as f64);

    // ── Listing (remaining sections) ─────────────────────────────────────
    if !model.noprint {
        // ── 9c. Convergence status ────────────────────────────────────────
        session.listing.blank();
        centered(session, "Model Convergence Status");
        session.listing.blank();
        if converged {
            session
                .listing
                .write_line("     Convergence criterion (GCONV=1E-8) satisfied.");
        } else {
            session
                .listing
                .write_line("     Iteration limit reached without convergence.");
        }
        session.listing.blank();

        // ── 14. Model Fit Statistics ──────────────────────────────────────
        centered(session, "Model Fit Statistics");
        session.listing.blank();

        let mfs_headers: Vec<String> = vec![
            "Criterion".into(),
            "Intercept Only".into(),
            "Intercept and Covariates".into(),
        ];
        let mfs_aligns = vec![Align::Left, Align::Right, Align::Right];
        let mfs_rows: Vec<Vec<String>> = vec![
            vec!["AIC".into(), fmt4(aic_null), fmt4(aic)],
            vec!["SC".into(), fmt4(sc_null), fmt4(sc)],
            vec!["-2 Log L".into(), fmt4(neg2log_l_null), fmt4(neg2log_l)],
        ];
        session
            .listing
            .write_table(&mfs_headers, &mfs_aligns, &mfs_rows);
        session.listing.blank();

        // ── 15. Testing Global Null Hypothesis: BETA=0 ───────────────────
        centered(session, "Testing Global Null Hypothesis: BETA=0");
        session.listing.blank();

        let gnh_headers: Vec<String> = vec![
            "Test".into(),
            "Chi-Square".into(),
            "DF".into(),
            "Pr > ChiSq".into(),
        ];
        let gnh_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
        let gnh_rows: Vec<Vec<String>> = vec![
            vec![
                "Likelihood Ratio".into(),
                fmt4(lr_chi2),
                nb_cols.to_string(),
                fmt_p_opt(lr_p),
            ],
            vec![
                "Score".into(),
                fmt4(score_chi2),
                nb_cols.to_string(),
                fmt_p_opt(score_p),
            ],
            vec![
                "Wald".into(),
                fmt4(wald_chi2_global),
                nb_cols.to_string(),
                fmt_p_opt(wald_global_p),
            ],
        ];
        session
            .listing
            .write_table(&gnh_headers, &gnh_aligns, &gnh_rows);
        session.listing.blank();

        // ── 16. Analysis of Maximum Likelihood Estimates ──────────────────
        centered(session, "Analysis of Maximum Likelihood Estimates");
        session.listing.blank();

        let amle_headers: Vec<String> = vec![
            "Parameter".into(),
            "DF".into(),
            "Estimate".into(),
            "Standard Error".into(),
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
        ];
        let mut amle_rows: Vec<Vec<String>> = Vec::with_capacity(p_param);
        for j in 0..p_param {
            let param_name = if j == 0 {
                "Intercept".to_string()
            } else {
                design.col_labels[j - 1].clone()
            };
            amle_rows.push(vec![
                param_name,
                "1".into(),
                fmt4(beta[j]),
                fmt4(se_beta[j]),
                fmt4(wald_chi2[j]),
                fmt_p_opt(wald_p[j]),
            ]);
        }
        session
            .listing
            .write_table(&amle_headers, &amle_aligns, &amle_rows);
        session.listing.blank();

        // ── 17. Odds Ratio Estimates ──────────────────────────────────────
        // Odds ratios are only meaningful (exp(β) = odds multiplier) for the
        // logit link, so the table is omitted for PROBIT/CLOGLOG (documented).
        if nb_cols > 0 && model.link == Link::Logit {
            centered(session, "Odds Ratio Estimates");
            session.listing.blank();

            let ore_headers: Vec<String> = vec![
                "Effect".into(),
                "Point Estimate".into(),
                "Lower".into(),
                "Upper".into(),
            ];
            let ore_aligns = vec![
                Align::Left,
                Align::Right,
                Align::Right,
                Align::Right,
            ];
            let mut ore_rows: Vec<Vec<String>> = Vec::with_capacity(nb_cols);
            // Walk the design columns effect-by-effect so CLASS rows can be
            // labelled "var level vs reflevel".
            let mut col = 1usize; // beta index (skip intercept)
            for eff in &design.effects {
                if eff.is_class {
                    for lv in &eff.levels {
                        let or_j = beta[col].exp();
                        let ci_lower = (beta[col] - 1.96 * se_beta[col]).exp();
                        let ci_upper = (beta[col] + 1.96 * se_beta[col]).exp();
                        ore_rows.push(vec![
                            format!(
                                "{} {} vs {}",
                                eff.name,
                                value_label(lv),
                                eff.ref_label
                            ),
                            fmt4(or_j),
                            fmt4(ci_lower),
                            fmt4(ci_upper),
                        ]);
                        col += 1;
                    }
                } else {
                    let or_j = beta[col].exp();
                    let ci_lower = (beta[col] - 1.96 * se_beta[col]).exp();
                    let ci_upper = (beta[col] + 1.96 * se_beta[col]).exp();
                    ore_rows.push(vec![
                        eff.name.clone(),
                        fmt4(or_j),
                        fmt4(ci_lower),
                        fmt4(ci_upper),
                    ]);
                    col += 1;
                }
            }
            session
                .listing
                .write_table(&ore_headers, &ore_aligns, &ore_rows);
        }
    }

    // ── OUTPUT OUT= ────────────────────────────────────────────────────────
    // Binary: predicted = P(Y = event). xbeta = linear predictor η.
    let preds_out: Vec<f64> = (0..n_obs).map(|i| final_p[i]).collect();
    let xbeta_out: Vec<f64> = x_mat
        .iter()
        .map(|xi| xi.iter().zip(beta.iter()).map(|(x, b)| x * b).sum())
        .collect();
    write_outputs(
        &ast.outputs,
        &ds,
        &complete_mask,
        &preds_out,
        &xbeta_out,
        session,
    )?;

    Ok(())
}

// ───────────────────────── CLASS / OUTPUT helpers ─────────────────────────

fn num_var_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

/// Print "Class Level Information" (one row per CLASS variable, listing levels
/// in `sas_cmp` order). No-op when no CLASS variable is present, so the binary
/// no-CLASS listing is byte-identical to the pre-M34.6 output.
fn write_class_level_info(session: &mut Session, design: &Design) {
    let has_class = design.effects.iter().any(|e| e.is_class);
    if !has_class {
        return;
    }
    centered(session, "Class Level Information");
    session.listing.blank();

    let headers: Vec<String> = vec!["Class".into(), "Value".into()];
    let aligns = vec![Align::Left, Align::Left];
    let mut rows: Vec<Vec<String>> = Vec::new();
    for eff in &design.effects {
        if !eff.is_class {
            continue;
        }
        // Reconstruct the full ordered level list: non-reference levels then ref.
        let mut labels: Vec<String> =
            eff.levels.iter().map(value_label).collect();
        labels.push(eff.ref_label.clone());
        rows.push(vec![eff.name.clone(), labels.join(" ")]);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// Write OUTPUT OUT= datasets: complete-case input rows plus the requested
/// predicted-probability and/or XBETA columns. Mirrors `reg.rs::write_outputs`
/// (creation NOTE + `_LAST_`).
fn write_outputs(
    outputs: &[LogisticOutput],
    ds: &SasDataset,
    complete_mask: &[bool],
    predicted: &[f64],
    xbeta: &[f64],
    session: &mut Session,
) -> Result<()> {
    if outputs.is_empty() {
        return Ok(());
    }

    let complete_indices: Vec<usize> = complete_mask
        .iter()
        .enumerate()
        .filter_map(|(i, &c)| if c { Some(i) } else { None })
        .collect();

    for out_spec in outputs {
        let n_cols = ds.vars.len();
        let mut columns: Vec<Column> = Vec::with_capacity(n_cols + 2);
        let mut out_vars: Vec<VarMeta> = Vec::with_capacity(n_cols + 2);

        for col_idx in 0..n_cols {
            let col_vals = decode_column(ds, col_idx)?;
            match ds.vars[col_idx].ty {
                VarType::Num => {
                    let data: Vec<Option<f64>> = complete_indices
                        .iter()
                        .map(|&i| value_to_num(&col_vals[i]))
                        .collect();
                    columns.push(Series::new(ds.vars[col_idx].name.as_str().into(), data).into());
                }
                VarType::Char => {
                    let data: Vec<Option<String>> = complete_indices
                        .iter()
                        .map(|&i| match &col_vals[i] {
                            Value::Char(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                    columns.push(Series::new(ds.vars[col_idx].name.as_str().into(), data).into());
                }
            }
            out_vars.push(ds.vars[col_idx].clone());
        }

        if let Some(pred_name) = &out_spec.predicted {
            let data: Vec<Option<f64>> = predicted.iter().map(|&v| Some(v)).collect();
            columns.push(Series::new(pred_name.as_str().into(), data).into());
            out_vars.push(num_var_meta(pred_name));
        }
        if let Some(xb_name) = &out_spec.xbeta {
            let data: Vec<Option<f64>> = xbeta.iter().map(|&v| Some(v)).collect();
            columns.push(Series::new(xb_name.as_str().into(), data).into());
            out_vars.push(num_var_meta(xb_name));
        }

        let out_df = DataFrame::new(columns)?;
        let out_ds = SasDataset {
            df: out_df,
            vars: out_vars,
        };

        let out_libref = out_spec.out.libref_or_work();
        let out_table = out_spec.out.name.to_uppercase();
        let display = format!("{out_libref}.{out_table}");
        let n_rows = out_ds.n_obs();
        let n_vars_out = out_ds.vars.len();
        session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
        session.last_dataset = Some(display.clone());
        session.log.note(&format!(
            "The data set {} has {} observations and {} variables.",
            display, n_rows, n_vars_out
        ));
    }

    Ok(())
}

// ───────────────────────── Ordinal (cumulative-logit) ─────────────────────────

/// Proportional-odds cumulative-logit model for an ordered response with k>2
/// levels: intercepts α_1 < … < α_{k−1} plus a SHARED slope vector β, fit by
/// Newton-Raphson on the cumulative-logit log-likelihood.
///
/// Parameter layout in `theta`: [α_1, …, α_{k−1}, β_1, …, β_m].
/// Cumulative model: P(Y ≤ j) = logit⁻¹(α_j + x'β) using ordered categories
/// 1..k (category 1 = lowest in `sas_cmp` order). SAS by default orders
/// FORMATTED values ascending and models P(Y ≤ j); DESCENDING reverses.
///
/// Deferrals (documented):
/// - The "Score Test for the Proportional Odds Assumption" is NOT computed; a
///   deferral NOTE is emitted instead.
/// - OUTPUT predicted = P(Y in lowest modeled cumulative category) = P(Y = 1).
#[allow(clippy::too_many_arguments)]
fn execute_ordinal(
    ast: &LogisticAst,
    session: &mut Session,
    model: &LogisticModel,
    ds: &SasDataset,
    in_libref: &str,
    in_table: &str,
    resp_name: &str,
    resp_col: &[Value],
    pred_cols: &[Vec<Value>],
    freq_col: &Option<Vec<Value>>,
    levels: &[Value],
    design: &Design,
    n_read: usize,
) -> Result<()> {
    let k = levels.len(); // number of response levels (>2)
    let n_int = k - 1; // number of intercepts
    let nb_cols = design.n_cols();
    let n_par = n_int + nb_cols;

    // Order of categories: DESCENDING reverses the sas_cmp ascending order.
    // `cat_of[i]` = ordered category index (1..=k) for row i's response.
    let ordered_levels: Vec<&Value> = if model.descending {
        levels.iter().rev().collect()
    } else {
        levels.iter().collect()
    };

    // ── Listwise deletion + design build ──────────────────────────────────
    let mut cat_vec: Vec<usize> = Vec::new(); // 1..=k
    let mut x_mat: Vec<Vec<f64>> = Vec::new(); // design columns (NO intercept)
    let mut freq_vec: Vec<f64> = Vec::new();
    let mut complete_mask: Vec<bool> = vec![false; n_read];

    for i in 0..n_read {
        if resp_col[i].is_missing() {
            continue;
        }
        let w = if let Some(fc) = freq_col {
            match value_to_num(&fc[i]) {
                Some(f) if !f.is_nan() && f > 0.0 => f,
                _ => continue,
            }
        } else {
            1.0
        };
        let mut row: Vec<f64> = Vec::with_capacity(nb_cols);
        let mut ok = true;
        for eff in &design.effects {
            let col = &pred_cols[eff.pred_col_idx];
            if eff.is_class {
                let v = &col[i];
                if v.is_missing() {
                    ok = false;
                    break;
                }
                for lv in &eff.levels {
                    row.push(if v.sas_cmp(lv) == std::cmp::Ordering::Equal {
                        1.0
                    } else {
                        0.0
                    });
                }
            } else {
                match value_to_num(&col[i]) {
                    Some(v) if !v.is_nan() => row.push(v),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if !ok {
            continue;
        }
        // Category index (1..=k) in the ordered scheme.
        let cat = ordered_levels
            .iter()
            .position(|lv| lv.sas_cmp(&resp_col[i]) == std::cmp::Ordering::Equal)
            .map(|p| p + 1);
        let cat = match cat {
            Some(c) => c,
            None => continue,
        };
        cat_vec.push(cat);
        x_mat.push(row);
        freq_vec.push(w);
        complete_mask[i] = true;
    }

    let n_obs = cat_vec.len();
    let n_total: f64 = freq_vec.iter().sum();

    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_read, in_libref, in_table
    ));
    session
        .log
        .note(&format!("There were {} observations used.", n_total as i64));

    if n_obs <= n_par {
        return Err(SasError::runtime(
            "Not enough observations for ordinal logistic regression",
        ));
    }

    // ── Newton-Raphson on the cumulative-logit log-likelihood ─────────────
    // P(Y ≤ j) = σ(α_j + x'β). Initialise α at the empirical cumulative
    // logits and β = 0.
    let mut theta = vec![0.0_f64; n_par];
    {
        // Empirical cumulative proportions (weighted).
        let mut cum = vec![0.0_f64; k];
        for i in 0..n_obs {
            cum[cat_vec[i] - 1] += freq_vec[i];
        }
        let mut running = 0.0;
        for j in 0..n_int {
            running += cum[j];
            let p = (running / n_total).clamp(1e-6, 1.0 - 1e-6);
            theta[j] = (p / (1.0 - p)).ln();
        }
    }

    let sigma = |z: f64| 1.0 / (1.0 + (-z).exp());
    let mut converged = false;
    for _iter in 0..25 {
        let mut grad = vec![0.0_f64; n_par];
        let mut hess = vec![vec![0.0_f64; n_par]; n_par];

        for i in 0..n_obs {
            let xi = &x_mat[i];
            let fi = freq_vec[i];
            let c = cat_vec[i]; // 1..=k
            let xb: f64 = xi.iter().zip(theta[n_int..].iter()).map(|(x, b)| x * b).sum();

            // Cumulative probs γ_j = P(Y ≤ j) = σ(α_j + xβ), j = 1..k-1
            // with γ_0 = 0, γ_k = 1. Probability of category c is γ_c − γ_{c-1}.
            let gamma = |j: usize| -> f64 {
                if j == 0 {
                    0.0
                } else if j >= k {
                    1.0
                } else {
                    sigma(theta[j - 1] + xb)
                }
            };
            let g_c = gamma(c);
            let g_cm1 = gamma(c - 1);
            let prob = (g_c - g_cm1).max(1e-12);

            // d σ(η)/dη = σ(1−σ).
            let dsig = |j: usize| -> f64 {
                if j == 0 || j >= k {
                    0.0
                } else {
                    let s = sigma(theta[j - 1] + xb);
                    s * (1.0 - s)
                }
            };
            let d_c = dsig(c);
            let d_cm1 = dsig(c - 1);

            // ∂logL/∂α_j: only α_{c} and α_{c-1} contribute.
            // ∂prob/∂α_{c} = d_c ; ∂prob/∂α_{c-1} = −d_{c-1}.
            let inv = fi / prob;
            if c <= n_int {
                grad[c - 1] += inv * d_c;
            }
            if c - 1 >= 1 {
                grad[c - 2] += inv * (-d_cm1);
            }
            // ∂prob/∂β = (d_c − d_{c-1}) * x.
            let dprob_db = d_c - d_cm1;
            for (m, &xm) in xi.iter().enumerate() {
                grad[n_int + m] += inv * dprob_db * xm;
            }

            // Gauss-Newton / Fisher-style Hessian approximation: −(1/prob²)
            // outer product of ∂prob (expected-information style), summed.
            // Build the gradient-of-prob vector once.
            let mut dp = vec![0.0_f64; n_par];
            if c <= n_int {
                dp[c - 1] += d_c;
            }
            if c - 1 >= 1 {
                dp[c - 2] += -d_cm1;
            }
            for (m, &xm) in xi.iter().enumerate() {
                dp[n_int + m] += dprob_db * xm;
            }
            let coef = fi / (prob * prob);
            for a in 0..n_par {
                if dp[a] == 0.0 {
                    continue;
                }
                for b in 0..n_par {
                    hess[a][b] -= coef * dp[a] * dp[b];
                }
            }
        }

        let neg_hess: Vec<Vec<f64>> =
            hess.iter().map(|r| r.iter().map(|v| -v).collect()).collect();
        let inv = match invert_matrix(&neg_hess) {
            Ok(m) => m,
            Err(_) => break,
        };
        let delta = mat_vec(&inv, &grad);
        for j in 0..n_par {
            theta[j] += delta[j];
        }
        let max_delta = delta.iter().map(|d| d.abs()).fold(0.0_f64, f64::max);
        let max_t = theta.iter().map(|b| b.abs()).fold(0.0_f64, f64::max);
        if max_delta / (1.0 + max_t) < 1e-8 {
            converged = true;
            break;
        }
    }

    if !converged {
        session.log.note(
            "PROC LOGISTIC (ordinal): iteration limit reached without convergence \
             (possible separation).",
        );
    }

    // Variance-covariance for standard errors (final information).
    let var = ordinal_varcov(&x_mat, &cat_vec, &freq_vec, &theta, n_int, nb_cols, k);
    let se: Vec<f64> = (0..n_par)
        .map(|j| var.get(j).map(|r| r[j]).unwrap_or(f64::NAN).max(0.0).sqrt())
        .collect();
    let wald: Vec<f64> = (0..n_par).map(|j| (theta[j] / se[j]).powi(2)).collect();
    let wald_p: Vec<f64> = wald.iter().map(|&w| chisq_sf(w, 1.0)).collect();

    // ── Listing ───────────────────────────────────────────────────────────
    if !model.noprint {
        session.listing.page_header();
        centered(session, "The LOGISTIC Procedure");
        session.listing.blank();

        centered(session, "Model Information");
        session.listing.blank();
        let ds_display = format!("{}.{}", in_libref, in_table);
        let info_rows: Vec<Vec<String>> = vec![
            vec!["Data Set".into(), ds_display],
            vec!["Response Variable".into(), resp_name.to_string()],
            vec!["Number of Response Levels".into(), k.to_string()],
            vec!["Model".into(), "cumulative logit".into()],
            vec!["Optimization Technique".into(), "Newton-Raphson".into()],
        ];
        session.listing.write_table(
            &["".into(), "".into()],
            &[Align::Left, Align::Left],
            &info_rows,
        );
        session.listing.blank();

        write_class_level_info(session, design);

        // Response Profile
        centered(session, "Response Profile");
        session.listing.blank();
        let mut freq_by_cat = vec![0.0_f64; k];
        for i in 0..n_obs {
            freq_by_cat[cat_vec[i] - 1] += freq_vec[i];
        }
        let rp_headers = vec![
            "Ordered Value".into(),
            resp_name.to_string(),
            "Total Frequency".into(),
        ];
        let rp_aligns = vec![Align::Right, Align::Left, Align::Right];
        let rp_rows: Vec<Vec<String>> = (0..k)
            .map(|j| {
                vec![
                    (j + 1).to_string(),
                    value_label(ordered_levels[j]),
                    (freq_by_cat[j] as i64).to_string(),
                ]
            })
            .collect();
        session.listing.write_table(&rp_headers, &rp_aligns, &rp_rows);
        session.listing.blank();

        session
            .log
            .note("PROC LOGISTIC: Score Test for the Proportional Odds Assumption is deferred.");

        centered(session, "Model Convergence Status");
        session.listing.blank();
        if converged {
            session
                .listing
                .write_line("     Convergence criterion (GCONV=1E-8) satisfied.");
        } else {
            session
                .listing
                .write_line("     Iteration limit reached without convergence.");
        }
        session.listing.blank();

        // Analysis of Maximum Likelihood Estimates
        centered(session, "Analysis of Maximum Likelihood Estimates");
        session.listing.blank();
        let amle_headers: Vec<String> = vec![
            "Parameter".into(),
            "DF".into(),
            "Estimate".into(),
            "Standard Error".into(),
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
        ];
        let mut amle_rows: Vec<Vec<String>> = Vec::with_capacity(n_par);
        for j in 0..n_int {
            amle_rows.push(vec![
                format!("Intercept {}", j + 1),
                "1".into(),
                fmt4(theta[j]),
                fmt4(se[j]),
                fmt4(wald[j]),
                fmt_p_opt(wald_p[j]),
            ]);
        }
        for (m, label) in design.col_labels.iter().enumerate() {
            let j = n_int + m;
            amle_rows.push(vec![
                label.clone(),
                "1".into(),
                fmt4(theta[j]),
                fmt4(se[j]),
                fmt4(wald[j]),
                fmt_p_opt(wald_p[j]),
            ]);
        }
        session
            .listing
            .write_table(&amle_headers, &amle_aligns, &amle_rows);
        session.listing.blank();

        // Odds Ratio Estimates (shared slopes; logit link).
        if nb_cols > 0 {
            centered(session, "Odds Ratio Estimates");
            session.listing.blank();
            let ore_headers: Vec<String> = vec![
                "Effect".into(),
                "Point Estimate".into(),
                "Lower".into(),
                "Upper".into(),
            ];
            let ore_aligns =
                vec![Align::Left, Align::Right, Align::Right, Align::Right];
            let mut col = n_int;
            let mut ore_rows: Vec<Vec<String>> = Vec::new();
            for eff in &design.effects {
                if eff.is_class {
                    for lv in &eff.levels {
                        ore_rows.push(vec![
                            format!("{} {} vs {}", eff.name, value_label(lv), eff.ref_label),
                            fmt4(theta[col].exp()),
                            fmt4((theta[col] - 1.96 * se[col]).exp()),
                            fmt4((theta[col] + 1.96 * se[col]).exp()),
                        ]);
                        col += 1;
                    }
                } else {
                    ore_rows.push(vec![
                        eff.name.clone(),
                        fmt4(theta[col].exp()),
                        fmt4((theta[col] - 1.96 * se[col]).exp()),
                        fmt4((theta[col] + 1.96 * se[col]).exp()),
                    ]);
                    col += 1;
                }
            }
            session
                .listing
                .write_table(&ore_headers, &ore_aligns, &ore_rows);
        }
    }

    // ── OUTPUT: predicted = P(Y = lowest ordered category) = P(Y ≤ 1) ──────
    let predicted: Vec<f64> = x_mat
        .iter()
        .map(|xi| {
            let xb: f64 = xi.iter().zip(theta[n_int..].iter()).map(|(x, b)| x * b).sum();
            sigma(theta[0] + xb)
        })
        .collect();
    let xbeta: Vec<f64> = x_mat
        .iter()
        .map(|xi| xi.iter().zip(theta[n_int..].iter()).map(|(x, b)| x * b).sum())
        .collect();
    write_outputs(
        &ast.outputs,
        ds,
        &complete_mask,
        &predicted,
        &xbeta,
        session,
    )?;

    Ok(())
}

/// Final-iterate variance-covariance for the ordinal model (inverse of the
/// observed information). Returns an `n_par × n_par` matrix; on inversion
/// failure returns NaNs so SEs degrade gracefully rather than panicking.
#[allow(clippy::too_many_arguments)]
fn ordinal_varcov(
    x_mat: &[Vec<f64>],
    cat_vec: &[usize],
    freq_vec: &[f64],
    theta: &[f64],
    n_int: usize,
    nb_cols: usize,
    k: usize,
) -> Vec<Vec<f64>> {
    let n_par = n_int + nb_cols;
    let sigma = |z: f64| 1.0 / (1.0 + (-z).exp());
    let mut hess = vec![vec![0.0_f64; n_par]; n_par];
    for i in 0..x_mat.len() {
        let xi = &x_mat[i];
        let fi = freq_vec[i];
        let c = cat_vec[i];
        let xb: f64 = xi.iter().zip(theta[n_int..].iter()).map(|(x, b)| x * b).sum();
        let gamma = |j: usize| -> f64 {
            if j == 0 {
                0.0
            } else if j >= k {
                1.0
            } else {
                sigma(theta[j - 1] + xb)
            }
        };
        let dsig = |j: usize| -> f64 {
            if j == 0 || j >= k {
                0.0
            } else {
                let s = sigma(theta[j - 1] + xb);
                s * (1.0 - s)
            }
        };
        let prob = (gamma(c) - gamma(c - 1)).max(1e-12);
        let d_c = dsig(c);
        let d_cm1 = dsig(c - 1);
        let mut dp = vec![0.0_f64; n_par];
        if c <= n_int {
            dp[c - 1] += d_c;
        }
        if c - 1 >= 1 {
            dp[c - 2] += -d_cm1;
        }
        let dprob_db = d_c - d_cm1;
        for (m, &xm) in xi.iter().enumerate() {
            dp[n_int + m] += dprob_db * xm;
        }
        let coef = fi / (prob * prob);
        for a in 0..n_par {
            if dp[a] == 0.0 {
                continue;
            }
            for b in 0..n_par {
                hess[a][b] += coef * dp[a] * dp[b];
            }
        }
    }
    invert_matrix(&hess).unwrap_or_else(|_| vec![vec![f64::NAN; n_par]; n_par])
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

    fn parse_logistic(src: &str) -> Result<LogisticAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // logistic
        parse(&mut ts)
    }

    // Helper: create the 2x2 oracle dataset
    fn make_oracle_session() -> (Session, LogisticAst) {
        let session = make_session();
        // 4 rows (we use freq_var instead of repeated rows)
        let frame = df![
            "y" => [1.0_f64, 1.0, 0.0, 0.0],
            "x" => [1.0_f64, 0.0, 1.0, 0.0],
            "count" => [20.0_f64, 10.0, 5.0, 25.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x"), num_meta("count")],
        };
        session.libs.get("WORK").unwrap().write("COUNTS", &ds).unwrap();

        let ast = LogisticAst {
            data_options: LogisticDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "COUNTS".into(),
                }),
            },
            class_vars: vec![],
            model: Some(LogisticModel {
                response: "y".into(),
                event: None,
                descending: true,
                predictors: vec!["x".into()],
                noprint: false,
                link: Link::Logit,
            }),
            freq_var: Some("count".into()),
            outputs: vec![],
        };
        (session, ast)
    }

    #[test]
    fn test_parse_basic() {
        let ast = parse_logistic("proc logistic; model y = x; run;").unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.response, "y");
        assert_eq!(m.predictors, vec!["x"]);
        assert!(!m.descending);
        assert!(m.event.is_none());
    }

    #[test]
    fn test_parse_descending() {
        let ast = parse_logistic("proc logistic; model y(descending) = x; run;").unwrap();
        assert!(ast.model.unwrap().descending);
    }

    #[test]
    fn test_parse_event() {
        let ast = parse_logistic("proc logistic; model y(event='1') = x; run;").unwrap();
        assert_eq!(ast.model.unwrap().event, Some("1".to_string()));
    }

    #[test]
    fn test_parse_freq() {
        let ast = parse_logistic("proc logistic; model y = x; freq cnt; run;").unwrap();
        assert_eq!(ast.freq_var, Some("cnt".to_string()));
    }

    #[test]
    fn test_parse_class_allowed() {
        // CLASS declaration is valid at parse time (error only at execution)
        let ast = parse_logistic("proc logistic; class z; model y = x; run;").unwrap();
        assert_eq!(ast.class_vars, vec!["z"]);
    }

    #[test]
    fn test_execute_beta_oracle() {
        let (mut session, ast) = make_oracle_session();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // β₁ ≈ 2.3026
        assert!(
            listing.contains("2.3026") || listing.contains("2.302"),
            "β₁ not found in listing: {listing}"
        );
    }

    #[test]
    fn test_execute_or_oracle() {
        let (mut session, ast) = make_oracle_session();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // OR = exp(β₁) ≈ 10.000
        assert!(
            listing.contains("10.000") || listing.contains("10.0000"),
            "OR not found in listing: {listing}"
        );
    }

    #[test]
    fn test_execute_se_oracle() {
        let (mut session, ast) = make_oracle_session();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // SE(β₁) ≈ 0.6245
        assert!(
            listing.contains("0.6245") || listing.contains("0.624"),
            "SE not found in listing: {listing}"
        );
    }

    #[test]
    fn test_execute_neg2logl() {
        let (mut session, ast) = make_oracle_session();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // -2LogL ≈ 66.8990
        assert!(
            listing.contains("66.899") || listing.contains("66.8990"),
            "-2LogL not found in listing: {listing}"
        );
    }

    #[test]
    fn test_execute_lr_test() {
        let (mut session, ast) = make_oracle_session();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // LR χ² ≈ 16.279
        assert!(
            listing.contains("16.27") || listing.contains("16.279"),
            "LR test not found in listing: {listing}"
        );
    }

    fn char_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.into(),
            ty: VarType::Char,
            length: 8,
            format: None,
            label: None,
        }
    }

    // CLASS x (numeric 0/1) must reproduce the continuous-x fit: OR = 10,
    // log-odds-ratio ln(10) ≈ 2.3026 (ref = last level = 1, so the modeled
    // dummy is for level 0). With ref=last, the non-reference dummy is level 0,
    // giving β for "0 vs 1" = −ln(10); SAS prints "x 0 vs 1" OR = 0.1. To get
    // the same OR=10 as continuous x, we instead use a CLASS with the level
    // ordering matching x. Verify the magnitude of the slope is ln(10).
    #[test]
    fn test_execute_class_reproduces_binary_or() {
        let session = make_session();
        let frame = df![
            "y" => [1.0_f64, 1.0, 0.0, 0.0],
            "x" => ["1", "0", "1", "0"],
            "count" => [20.0_f64, 10.0, 5.0, 25.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), char_meta("x"), num_meta("count")],
        };
        session.libs.get("WORK").unwrap().write("CCLASS", &ds).unwrap();

        let ast = LogisticAst {
            data_options: LogisticDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "CCLASS".into(),
                }),
            },
            class_vars: vec!["x".into()],
            model: Some(LogisticModel {
                response: "y".into(),
                event: None,
                descending: true,
                predictors: vec!["x".into()],
                noprint: false,
                link: Link::Logit,
            }),
            freq_var: Some("count".into()),
            outputs: vec![],
        };
        let mut session = session;
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // ref = last level "1"; non-ref dummy is "0". β = ln(odds(0)/odds(1)) =
        // ln( (10/25)/(20/5) ) = ln(0.1) = −2.3026 → OR 0.1000. Magnitude ln(10).
        assert!(
            listing.contains("Class Level Information"),
            "missing Class Level Information: {listing}"
        );
        assert!(
            listing.contains("-2.3026") || listing.contains("2.3026"),
            "CLASS slope magnitude ln(10) not found: {listing}"
        );
        assert!(
            listing.contains("0.1000") || listing.contains("x 0 vs 1"),
            "CLASS odds ratio row not found: {listing}"
        );
    }

    fn tiny_link_session(link: Link) -> (Session, LogisticAst) {
        let session = make_session();
        let frame = df![
            "y" => [0.0_f64, 0.0, 1.0, 1.0, 1.0, 0.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 2.5]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("TINY", &ds).unwrap();
        let ast = LogisticAst {
            data_options: LogisticDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "TINY".into(),
                }),
            },
            class_vars: vec![],
            model: Some(LogisticModel {
                response: "y".into(),
                event: Some("1".into()),
                descending: false,
                predictors: vec!["x".into()],
                noprint: false,
                link,
            }),
            freq_var: None,
            outputs: vec![],
        };
        (session, ast)
    }

    #[test]
    fn test_execute_probit_converges() {
        let (mut session, ast) = tiny_link_session(Link::Probit);
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("binary probit"), "model line: {listing}");
        // Estimates must be finite (no NaN/inf printed).
        assert!(!listing.contains("NaN") && !listing.contains("inf"), "{listing}");
        // No odds-ratio table for non-logit links.
        assert!(
            !listing.contains("Odds Ratio Estimates"),
            "probit must omit odds ratios: {listing}"
        );
    }

    #[test]
    fn test_execute_cloglog_converges() {
        let (mut session, ast) = tiny_link_session(Link::Cloglog);
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("binary cloglog"), "model line: {listing}");
        assert!(!listing.contains("NaN") && !listing.contains("inf"), "{listing}");
    }

    #[test]
    fn test_execute_ordinal_monotone_intercepts() {
        let session = make_session();
        // Ordered response with 3 levels, x increasing with category.
        let frame = df![
            "y" => [1.0_f64, 1.0, 2.0, 2.0, 3.0, 3.0, 1.0, 2.0, 3.0, 2.0],
            "x" => [1.0_f64, 1.5, 2.0, 2.5, 3.0, 3.5, 1.2, 2.2, 3.2, 2.4]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("ORD", &ds).unwrap();
        let ast = LogisticAst {
            data_options: LogisticDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "ORD".into(),
                }),
            },
            class_vars: vec![],
            model: Some(LogisticModel {
                response: "y".into(),
                event: None,
                descending: false,
                predictors: vec!["x".into()],
                noprint: false,
                link: Link::Logit,
            }),
            freq_var: None,
            outputs: vec![],
        };
        let mut session = session;
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(
            listing.contains("cumulative logit"),
            "ordinal model not used: {listing}"
        );
        assert!(
            listing.contains("Intercept 1") && listing.contains("Intercept 2"),
            "multiple intercepts not printed: {listing}"
        );
        // Monotone intercepts α_1 < α_2: parse the Estimate column from the
        // "Intercept j" rows and assert ordering.
        let parse_intercept = |label: &str| -> f64 {
            let line = listing
                .lines()
                .find(|l| l.trim_start().starts_with(label))
                .unwrap_or_else(|| panic!("no line for {label}: {listing}"));
            // Columns: Parameter DF Estimate ... → 3rd whitespace field.
            let fields: Vec<&str> = line.split_whitespace().collect();
            // "Intercept" "1" "1" "<est>" ...  → index 3.
            fields[3].parse::<f64>().expect("estimate parse")
        };
        let a1 = parse_intercept("Intercept 1");
        let a2 = parse_intercept("Intercept 2");
        assert!(a1 < a2, "intercepts not monotone: a1={a1} a2={a2}");
    }

    #[test]
    fn test_output_predicted_in_unit_interval() {
        let (mut session, mut ast) = make_oracle_session();
        ast.outputs = vec![LogisticOutput {
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "PRED".into(),
            },
            predicted: Some("phat".into()),
            xbeta: Some("eta".into()),
        }];
        execute(&ast, &mut session).unwrap();
        let (out, _notes) = session.libs.get("WORK").unwrap().read("PRED").unwrap();
        let pcol = out
            .vars
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case("phat"))
            .expect("phat column");
        let vals = decode_column(&out, pcol).unwrap();
        for v in &vals {
            if let Some(p) = value_to_num(v) {
                assert!((0.0..=1.0).contains(&p), "predicted prob out of range: {p}");
            }
        }
        let log = session.log.into_string();
        assert!(
            log.contains("WORK.PRED has"),
            "creation NOTE missing: {log}"
        );
    }
}
