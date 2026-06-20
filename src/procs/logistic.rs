//! PROC LOGISTIC — binary logistic regression via Newton-Raphson MLE (M26.1).
//!
//! Supports:
//! - Binary response (two levels only).
//! - FREQ statement (weighted observations).
//! - MODEL statement with DESCENDING option and EVENT= option.
//! - Produces: Model Information, Response Profile, Model Fit Statistics,
//!   Global Null tests (LR, Score, Wald), Analysis of ML Estimates,
//!   and Odds Ratio Estimates.

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

#[derive(Debug, Clone)]
pub struct LogisticAst {
    pub data_options: LogisticDataOptions,
    pub class_vars: Vec<String>,
    pub model: Option<LogisticModel>,
    pub freq_var: Option<String>,
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
        } else {
            Ok(false)
        }
    })?;

    Ok(LogisticAst {
        data_options: LogisticDataOptions { input },
        class_vars,
        model,
        freq_var,
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

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &LogisticAst, session: &mut Session) -> Result<()> {
    // ── 1. Guards ──────────────────────────────────────────────────────────
    let model = ast.model.as_ref().ok_or_else(|| {
        SasError::runtime("MODEL statement required")
    })?;

    if !ast.class_vars.is_empty() {
        return Err(SasError::runtime(
            "CLASS variables not yet implemented in PROC LOGISTIC",
        ));
    }

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

    // ── 3. Determine event level ───────────────────────────────────────────
    // Collect distinct non-missing levels of the response, sorted by sas_cmp.
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
            "Response variable {} must have exactly 2 non-missing levels for binary logistic regression (found {}).",
            resp_name.to_uppercase(),
            levels.len()
        )));
    }

    // Determine event label (level for which P(Y=event) is modeled)
    let event_level: &Value = if let Some(ev_str) = &model.event {
        // Find level matching event string
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
        // DESCENDING: model last level (sas_cmp max)
        &levels[1]
    } else {
        // Default SAS: first level (sas_cmp min)
        &levels[0]
    };

    let event_label = value_label(event_level);
    let nonevent_level: &Value = if std::ptr::eq(event_level, &levels[0]) {
        &levels[1]
    } else {
        &levels[0]
    };
    let nonevent_label = value_label(nonevent_level);

    // ── 4. Listwise deletion + encoding ───────────────────────────────────
    let mut y_vec: Vec<f64> = Vec::new();
    let mut x_mat: Vec<Vec<f64>> = Vec::new();
    let mut freq_vec: Vec<f64> = Vec::new();

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

        // Check all predictors
        let mut row = vec![1.0_f64]; // intercept
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

    if n_obs <= nb_preds + 1 {
        return Err(SasError::runtime(
            "Not enough observations for logistic regression",
        ));
    }

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
            vec!["Model".into(), "binary logit".into()],
            vec!["Optimization Technique".into(), "Newton-Raphson".into()],
        ];
        session
            .listing
            .write_table(&info_headers, &info_aligns, &info_rows);
        session.listing.blank();

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
    let p_param = 1 + nb_preds; // number of parameters (with intercept)
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
            let pi = (1.0 / (1.0 + (-eta).exp())).clamp(1e-10, 1.0 - 1e-10);
            let wi = pi * (1.0 - pi);
            let fi = freq_vec[i];

            // Score (gradient)
            for j in 0..p_param {
                score[j] += fi * xi[j] * (y_vec[i] - pi);
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
        return Err(SasError::runtime("PROC LOGISTIC failed to converge"));
    }

    // ── 9b. Final Hessian and variance-covariance matrix ─────────────────
    let mut final_hessian: Vec<Vec<f64>> = vec![vec![0.0; p_param]; p_param];
    let mut final_p: Vec<f64> = Vec::with_capacity(n_obs);

    for i in 0..n_obs {
        let xi = &x_mat[i];
        let eta: f64 = xi.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
        let pi = (1.0 / (1.0 + (-eta).exp())).clamp(1e-10, 1.0 - 1e-10);
        let wi = pi * (1.0 - pi);
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
    let lr_p = chisq_sf(lr_chi2, nb_preds as f64);

    // Wald global: β_c' * Σ_c^{-1} * β_c where Σ_c = submatrix for predictors
    let wald_chi2_global = if nb_preds > 0 {
        let sigma_c = submatrix_predictors(&var_beta, nb_preds);
        let sigma_c_inv = invert_matrix(&sigma_c)?;
        let beta_c: Vec<f64> = (1..=nb_preds).map(|j| beta[j]).collect();
        let tmp = mat_vec(&sigma_c_inv, &beta_c);
        dot(&beta_c, &tmp)
    } else {
        0.0
    };
    let wald_global_p = chisq_sf(wald_chi2_global, nb_preds as f64);

    // Score test (under H₀: β_c=0, β₀=logit(p̄))
    let score_chi2 = if nb_preds > 0 {
        // Score_c_j = Σ freq_i * x_ij * (y_i - p̄) for j = predictors
        let mut score_c: Vec<f64> = vec![0.0; nb_preds];
        // Score_0 = Σ freq_i * (y_i - p̄)
        let mut score_0: f64 = 0.0;
        // I_cc_jk = p̄*(1-p̄) * Σ freq_i * x_ij * x_ik  (j,k predictors)
        let mut i_cc: Vec<Vec<f64>> = vec![vec![0.0; nb_preds]; nb_preds];
        // I_00 = p̄*(1-p̄) * n_total
        let i_00 = p_bar * (1.0 - p_bar) * n_total;
        // I_0c_j = p̄*(1-p̄) * Σ freq_i * x_ij  (j predictor)
        let mut i_0c: Vec<f64> = vec![0.0; nb_preds];

        for i in 0..n_obs {
            let fi = freq_vec[i];
            let xi = &x_mat[i];
            let resid = y_vec[i] - p_bar;
            score_0 += fi * resid;
            for j in 0..nb_preds {
                score_c[j] += fi * xi[j + 1] * resid;
                i_0c[j] += fi * xi[j + 1];
                for k in 0..nb_preds {
                    i_cc[j][k] += fi * xi[j + 1] * xi[k + 1];
                }
            }
        }
        // Apply p̄*(1-p̄) to I matrices
        let pb_var = p_bar * (1.0 - p_bar);
        for j in 0..nb_preds {
            i_0c[j] *= pb_var;
            for k in 0..nb_preds {
                i_cc[j][k] *= pb_var;
            }
        }

        // Schur complement: I_cc|0 = I_cc - I_c0 * I_00^{-1} * I_0c
        // (I_c0 = I_0c^T for scalar I_00)
        let i_00_inv = 1.0 / i_00;
        let mut i_cc_schur = i_cc.clone();
        for j in 0..nb_preds {
            for k in 0..nb_preds {
                i_cc_schur[j][k] -= i_0c[j] * i_00_inv * i_0c[k];
            }
        }

        // Score_c|0 = Score_c - (I_c0 / I_00) * Score_0
        let mut score_c_schur = score_c.clone();
        for j in 0..nb_preds {
            score_c_schur[j] -= (i_0c[j] / i_00) * score_0;
        }

        // χ²_Score = Score_c|0' * I_cc|0^{-1} * Score_c|0
        let i_cc_schur_inv = invert_matrix(&i_cc_schur)?;
        let tmp = mat_vec(&i_cc_schur_inv, &score_c_schur);
        dot(&score_c_schur, &tmp)
    } else {
        0.0
    };
    let score_p = chisq_sf(score_chi2, nb_preds as f64);

    // ── Listing (remaining sections) ─────────────────────────────────────
    if !model.noprint {
        // ── 9c. Convergence status ────────────────────────────────────────
        session.listing.blank();
        centered(session, "Model Convergence Status");
        session.listing.blank();
        session
            .listing
            .write_line("     Convergence criterion (GCONV=1E-8) satisfied.");
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
                nb_preds.to_string(),
                fmt_p_opt(lr_p),
            ],
            vec![
                "Score".into(),
                fmt4(score_chi2),
                nb_preds.to_string(),
                fmt_p_opt(score_p),
            ],
            vec![
                "Wald".into(),
                fmt4(wald_chi2_global),
                nb_preds.to_string(),
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
                predictors[j - 1].clone()
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
        if nb_preds > 0 {
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
            let mut ore_rows: Vec<Vec<String>> = Vec::with_capacity(nb_preds);
            for j in 1..=nb_preds {
                let or_j = beta[j].exp();
                let ci_lower = (beta[j] - 1.96 * se_beta[j]).exp();
                let ci_upper = (beta[j] + 1.96 * se_beta[j]).exp();
                ore_rows.push(vec![
                    predictors[j - 1].clone(),
                    fmt4(or_j),
                    fmt4(ci_lower),
                    fmt4(ci_upper),
                ]);
            }
            session
                .listing
                .write_table(&ore_headers, &ore_aligns, &ore_rows);
        }
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
            }),
            freq_var: Some("count".into()),
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
}
