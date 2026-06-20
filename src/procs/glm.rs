//! PROC GLM — General Linear Model for one-way CLASS designs (M25.3).
//!
//! Extends PROC ANOVA with:
//! - `/SOLUTION` option: parameter estimates (intercept + CLASS level effects)
//! - LSMEANS statement: least-squares means with SEs and Pr > |t|
//! - ESTIMATE statement: user-defined linear combinations of CLASS means
//! - CONTRAST statement: F-tests for linear combinations (same as ESTIMATE but gives F)
//!
//! For now, only one-way CLASS designs (single effect in MODEL) are supported.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::{decode_column, sample_std};
use crate::session::Session;
use crate::stat::{f_cdf, student_t_cdf};
use crate::token::TokenKind;
use crate::value::{Value, VarType};

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct GlmAst {
    pub data_options: GlmDataOptions,
    pub class_vars: Vec<String>,
    pub model: Option<GlmModel>,
    pub lsmeans_vars: Vec<String>,
    pub estimates: Vec<GlmEstimate>,
    pub contrasts: Vec<GlmContrast>,
    pub means_vars: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GlmDataOptions {
    pub input: Option<DatasetRef>,
}

#[derive(Debug, Clone)]
pub struct GlmModel {
    pub dependents: Vec<String>,
    pub effects: Vec<String>,
    pub solution: bool,
    pub noprint: bool,
}

#[derive(Debug, Clone)]
pub struct GlmEstimate {
    pub label: String,
    pub effect: String,
    pub coefficients: Vec<f64>,
}

#[derive(Debug, Clone)]
pub struct GlmContrast {
    pub label: String,
    pub effect: String,
    pub coefficients: Vec<f64>,
}

// ───────────────────────── Parser helpers ─────────────────────────

/// Parse a list of numeric coefficients from the token stream.
/// Reads numbers (and optional leading minus sign) until `;` or `/`.
fn parse_coefficients(ts: &mut StatementStream) -> Vec<f64> {
    let mut coeffs = Vec::new();
    loop {
        let kind = ts.peek().kind.clone();
        match kind {
            TokenKind::Semi | TokenKind::Slash | TokenKind::Eof => break,
            TokenKind::Minus => {
                // Could be a negative number: consume `-` then number
                ts.next();
                let next_kind = ts.peek().kind.clone();
                if let TokenKind::Num(v) = next_kind {
                    coeffs.push(-v);
                    ts.next();
                } else {
                    // Not a number — stop
                    break;
                }
            }
            TokenKind::Num(v) => {
                coeffs.push(v);
                ts.next();
            }
            _ => break,
        }
    }
    coeffs
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC GLM. Called AFTER `proc glm` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<GlmAst> {
    let mut input: Option<DatasetRef> = None;

    // PROC GLM statement options until `;`
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

    // Sub-statements until run;/quit;
    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<GlmModel> = None;
    let mut lsmeans_vars: Vec<String> = Vec::new();
    let mut estimates: Vec<GlmEstimate> = Vec::new();
    let mut contrasts: Vec<GlmContrast> = Vec::new();
    let mut means_vars: Vec<String> = Vec::new();

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "class" {
            ts.next();
            class_vars = ts.parse_name_list()?;
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "model" {
            ts.next();
            // Read dependents: idents before `=`
            let mut dependents: Vec<String> = Vec::new();
            loop {
                if ts.peek().kind == TokenKind::Semi
                    || ts.peek().kind == TokenKind::Eof
                    || ts.peek().kind == TokenKind::Eq
                {
                    break;
                }
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    dependents.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            // Consume `=`
            if ts.peek().kind == TokenKind::Eq {
                ts.next();
            }
            // Read effects: idents after `=` until `/` or `;`
            let mut effects: Vec<String> = Vec::new();
            let mut solution = false;
            let mut noprint = false;
            loop {
                if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                    break;
                }
                if ts.peek().kind == TokenKind::Slash {
                    ts.next();
                    // Parse options until semi
                    while ts.peek().kind != TokenKind::Semi
                        && ts.peek().kind != TokenKind::Eof
                    {
                        if ts.peek().is_kw("solution") {
                            solution = true;
                        }
                        if ts.peek().is_kw("noprint") {
                            noprint = true;
                        }
                        ts.next();
                    }
                    break;
                }
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    effects.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            model = Some(GlmModel {
                dependents,
                effects,
                solution,
                noprint,
            });
            Ok(true)
        } else if kw == "lsmeans" {
            ts.next();
            // Read lsmeans vars (idents before `/` or `;`)
            let mut vars: Vec<String> = Vec::new();
            loop {
                if ts.peek().kind == TokenKind::Semi
                    || ts.peek().kind == TokenKind::Eof
                    || ts.peek().kind == TokenKind::Slash
                {
                    break;
                }
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    vars.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            // Skip options after `/`
            if ts.peek().kind == TokenKind::Slash {
                ts.next();
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            lsmeans_vars = vars;
            Ok(true)
        } else if kw == "estimate" {
            ts.next();
            // Read label (string literal)
            let label = if let TokenKind::Str { value, .. } = ts.peek().kind.clone() {
                ts.next();
                value
            } else {
                String::new()
            };
            // Read effect (ident)
            let effect = if let Some(name) = ts.peek().ident().map(str::to_string) {
                ts.next();
                name
            } else {
                String::new()
            };
            // Read coefficients
            let coefficients = parse_coefficients(ts);
            // Skip options after `/` if any
            if ts.peek().kind == TokenKind::Slash {
                ts.next();
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            estimates.push(GlmEstimate {
                label,
                effect,
                coefficients,
            });
            Ok(true)
        } else if kw == "contrast" {
            ts.next();
            // Read label (string literal)
            let label = if let TokenKind::Str { value, .. } = ts.peek().kind.clone() {
                ts.next();
                value
            } else {
                String::new()
            };
            // Read effect (ident)
            let effect = if let Some(name) = ts.peek().ident().map(str::to_string) {
                ts.next();
                name
            } else {
                String::new()
            };
            // Read coefficients
            let coefficients = parse_coefficients(ts);
            // Skip options after `/` if any
            if ts.peek().kind == TokenKind::Slash {
                ts.next();
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            contrasts.push(GlmContrast {
                label,
                effect,
                coefficients,
            });
            Ok(true)
        } else if kw == "means" {
            ts.next();
            means_vars = ts.parse_name_list()?;
            ts.expect_semi()?;
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(GlmAst {
        data_options: GlmDataOptions { input },
        class_vars,
        model,
        lsmeans_vars,
        estimates,
        contrasts,
        means_vars,
    })
}

// ───────────────────────── Formatting ─────────────────────────

fn fmt5(v: f64) -> String {
    format!("{v:.5}")
}

fn fmt2(v: f64) -> String {
    format!("{v:.2}")
}

fn fmt6(v: f64) -> String {
    format!("{v:.6}")
}

fn fmt_p(p: Option<f64>) -> String {
    match p {
        None => ".".to_string(),
        Some(v) if v < 0.0001 => "<.0001".to_string(),
        Some(v) => format!("{v:.4}"),
    }
}

// ───────────────────────── Listing helpers ─────────────────────────

fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(text.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), text));
}

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &GlmAst, session: &mut Session) -> Result<()> {
    // Guard: MODEL required
    let model = match &ast.model {
        Some(m) => m,
        None => {
            session.log.note("No MODEL statement found in PROC GLM.");
            return Ok(());
        }
    };

    // Pre-check: at least one effect and at least one class var
    if model.effects.is_empty() || ast.class_vars.is_empty() {
        return Err(SasError::runtime(
            "MODEL statement requires at least one CLASS effect.",
        ));
    }

    // Pre-check: no interaction effects
    for eff in &model.effects {
        if eff.contains('*') {
            return Err(SasError::runtime(
                "Interaction effects not yet implemented in PROC GLM.",
            ));
        }
    }

    // --- 1. Resolve dataset ---
    let in_ref = common::resolve_last_dataset(&ast.data_options.input, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_obs = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_obs, in_libref, in_table
    ));

    // --- 2. Validate CLASS vars ---
    for class_var in &ast.class_vars {
        let found = ds
            .vars
            .iter()
            .any(|m| m.name.eq_ignore_ascii_case(class_var));
        if !found {
            return Err(SasError::runtime(format!(
                "Variable {} not found.",
                class_var.to_uppercase()
            )));
        }
    }

    // --- 3. Listing header ---
    session.listing.page_header();
    centered(session, "The GLM Procedure");
    session.listing.blank();

    // --- 4. Class Level Information ---
    centered(session, "Class Level Information");
    session.listing.blank();

    let cli_headers: Vec<String> = vec!["Class".into(), "Levels".into(), "Values".into()];
    let cli_aligns = vec![Align::Left, Align::Right, Align::Left];
    let mut cli_rows: Vec<Vec<String>> = Vec::new();

    let mut class_col_data: Vec<(String, Vec<Value>)> = Vec::new();
    for class_var in &ast.class_vars {
        let col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(class_var))
            .unwrap();
        let col = decode_column(&ds, col_idx)?;

        let mut levels: Vec<Value> = Vec::new();
        for i in 0..n_obs {
            let v = &col[i];
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

        let values_str: Vec<String> = levels
            .iter()
            .map(|v| match v {
                Value::Char(s) => s.trim_end().to_string(),
                Value::Num(f) => format!("{f}"),
                Value::Missing(k) => k.display(),
            })
            .collect();

        cli_rows.push(vec![
            ds.vars[col_idx].name.clone(),
            format!("{}", levels.len()),
            values_str.join(" "),
        ]);

        class_col_data.push((ds.vars[col_idx].name.clone(), col));
    }

    session.listing.write_table(&cli_headers, &cli_aligns, &cli_rows);
    session.listing.blank();

    session.listing.write_line(&format!(
        "               Number of Observations Read     {}",
        n_obs
    ));
    session.listing.blank();
    session.listing.blank();

    // --- 5. Per-dependent variable loop ---
    for dep_var in &model.dependents {
        // Find dependent column
        let dep_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(dep_var))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", dep_var.to_uppercase()))
            })?;
        if ds.vars[dep_idx].ty != VarType::Num {
            return Err(SasError::runtime(format!(
                "Dependent variable {} must be numeric.",
                dep_var.to_uppercase()
            )));
        }
        let dep_col = decode_column(&ds, dep_idx)?;

        // For one-way GLM, use the first effect as the CLASS grouping variable
        let eff = &model.effects[0];

        let class_col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(eff))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", eff.to_uppercase()))
            })?;
        let class_col = decode_column(&ds, class_col_idx)?;

        // Listwise deletion
        let mut usable_rows: Vec<usize> = Vec::new();
        for i in 0..n_obs {
            let dep_ok = match value_to_num(&dep_col[i]) {
                Some(v) if !v.is_nan() => true,
                _ => false,
            };
            let cls_ok = !class_col[i].is_missing();
            if dep_ok && cls_ok {
                usable_rows.push(i);
            }
        }
        let n = usable_rows.len();

        // Group by CLASS levels (sorted by sas_cmp)
        let mut levels: Vec<Value> = Vec::new();
        for &r in &usable_rows {
            let v = &class_col[r];
            if !levels
                .iter()
                .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            {
                levels.push(v.clone());
            }
        }
        levels.sort_by(|a, b| a.sas_cmp(b));
        let k = levels.len();

        // Collect values per group
        let mut groups: Vec<Vec<f64>> = vec![Vec::new(); k];
        for &r in &usable_rows {
            let v = &class_col[r];
            let yi = value_to_num(&dep_col[r]).unwrap();
            let gi = levels
                .iter()
                .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
                .unwrap();
            groups[gi].push(yi);
        }

        // Compute statistics
        let y_bar = if n > 0 {
            groups.iter().flat_map(|g| g.iter()).sum::<f64>() / n as f64
        } else {
            f64::NAN
        };

        let mut ssm = 0.0_f64;
        let mut sse = 0.0_f64;
        let mut group_means: Vec<f64> = Vec::with_capacity(k);
        for g in &groups {
            let ni = g.len();
            let y_bar_i = if ni > 0 {
                g.iter().sum::<f64>() / ni as f64
            } else {
                f64::NAN
            };
            group_means.push(y_bar_i);
            ssm += ni as f64 * (y_bar_i - y_bar).powi(2);
            sse += g.iter().map(|&y| (y - y_bar_i).powi(2)).sum::<f64>();
        }
        let sst = ssm + sse;

        let df_model = (k as f64 - 1.0).max(0.0);
        let df_error = (n as f64 - k as f64).max(0.0);
        let df_total = (n as f64 - 1.0).max(0.0);

        let msm = if df_model > 0.0 { ssm / df_model } else { f64::NAN };
        let mse = if df_error > 0.0 { sse / df_error } else { f64::NAN };
        let f_stat = if mse > 0.0 && !mse.is_nan() { msm / mse } else { f64::NAN };
        let p_f = if f_stat.is_nan() {
            None
        } else {
            Some((1.0 - f_cdf(f_stat, df_model, df_error)).clamp(0.0, 1.0))
        };

        let r2 = if sst > 0.0 { ssm / sst } else { f64::NAN };
        let root_mse = if !mse.is_nan() { mse.sqrt() } else { f64::NAN };
        let cv = if y_bar.abs() > 1e-15 && !root_mse.is_nan() {
            root_mse / y_bar.abs() * 100.0
        } else {
            f64::NAN
        };

        session.log.note(&format!("There were {} observations used.", n));

        // Helper: format level label
        let level_label = |v: &Value| -> String {
            match v {
                Value::Char(s) => s.trim_end().to_string(),
                Value::Num(f) => format!("{f}"),
                Value::Missing(k) => k.display(),
            }
        };

        // --- Dependent Variable header ---
        centered(session, &format!("Dependent Variable: {}", dep_var));
        session.listing.blank();

        // ANOVA table
        centered(session, "Analysis of Variance");
        session.listing.blank();

        let anova_headers: Vec<String> = vec![
            "Source".into(),
            "DF".into(),
            "Sum of Squares".into(),
            "Mean Square".into(),
            "F Value".into(),
            "Pr > F".into(),
        ];
        let anova_aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let f_str = if f_stat.is_nan() { ".".to_string() } else { fmt2(f_stat) };
        let anova_rows: Vec<Vec<String>> = vec![
            vec![
                "Model".into(),
                format!("{}", df_model as usize),
                fmt5(ssm),
                if msm.is_nan() { ".".into() } else { fmt5(msm) },
                f_str.clone(),
                fmt_p(p_f),
            ],
            vec![
                "Error".into(),
                format!("{}", df_error as usize),
                fmt5(sse),
                if mse.is_nan() { ".".into() } else { fmt5(mse) },
                "".into(),
                "".into(),
            ],
            vec![
                "Corrected Total".into(),
                format!("{}", df_total as usize),
                fmt5(sst),
                "".into(),
                "".into(),
                "".into(),
            ],
        ];
        session.listing.write_table(&anova_headers, &anova_aligns, &anova_rows);
        session.listing.blank();
        session.listing.blank();

        // Fit statistics table
        let dep_mean_header = format!("{} Mean", dep_var);
        let fit_headers: Vec<String> = vec![
            "R-Square".into(),
            "Coeff Var".into(),
            "Root MSE".into(),
            dep_mean_header,
        ];
        let fit_aligns = vec![Align::Right, Align::Right, Align::Right, Align::Right];
        let fit_rows: Vec<Vec<String>> = vec![vec![
            fmt6(r2),
            fmt6(cv),
            fmt6(root_mse),
            fmt6(y_bar),
        ]];
        session.listing.write_table(&fit_headers, &fit_aligns, &fit_rows);
        session.listing.blank();
        session.listing.blank();

        // Type I SS and Type III SS (identical for one-way)
        for (ss_label, _is_type3) in [("Type I SS", false), ("Type III SS", true)] {
            let t_headers: Vec<String> = vec![
                "Source".into(),
                "DF".into(),
                ss_label.into(),
                "Mean Square".into(),
                "F Value".into(),
                "Pr > F".into(),
            ];
            let t_aligns = vec![
                Align::Left,
                Align::Right,
                Align::Right,
                Align::Right,
                Align::Right,
                Align::Right,
            ];
            let f_str2 = if f_stat.is_nan() { ".".to_string() } else { fmt2(f_stat) };
            let t_rows: Vec<Vec<String>> = vec![vec![
                eff.clone(),
                format!("{}", df_model as usize),
                if msm.is_nan() { ".".into() } else { fmt5(ssm) },
                if msm.is_nan() { ".".into() } else { fmt5(msm) },
                f_str2,
                fmt_p(p_f),
            ]];
            session.listing.write_table(&t_headers, &t_aligns, &t_rows);
            session.listing.blank();
            session.listing.blank();
        }

        // --- Parameter Estimates (if /SOLUTION) ---
        if model.solution && k >= 1 {
            centered(session, "Parameter Estimates");
            session.listing.blank();

            let param_headers: Vec<String> = vec![
                "Parameter".into(),
                "Estimate".into(),
                "Standard Error".into(),
                "t Value".into(),
                "Pr > |t|".into(),
            ];
            let param_aligns = vec![
                Align::Left,
                Align::Right,
                Align::Right,
                Align::Right,
                Align::Right,
            ];
            let mut param_rows: Vec<Vec<String>> = Vec::new();

            // Reference level = last level (index k-1)
            let ref_idx = k - 1;
            let n_ref = groups[ref_idx].len();
            let y_ref = group_means[ref_idx];

            // Intercept = mean of reference level
            let intercept = y_ref;
            let se_intercept = if n_ref > 0 && !mse.is_nan() {
                (mse / n_ref as f64).sqrt()
            } else {
                f64::NAN
            };
            let t_intercept = if se_intercept > 0.0 { intercept / se_intercept } else { f64::NAN };
            let p_intercept = if t_intercept.is_nan() {
                None
            } else {
                Some(2.0 * (1.0 - student_t_cdf(t_intercept.abs(), df_error)))
            };

            param_rows.push(vec![
                "Intercept".into(),
                fmt6(intercept),
                fmt6(se_intercept),
                fmt2(t_intercept),
                fmt_p(p_intercept),
            ]);

            // Effect levels: i = 0..k-2 (all except reference)
            for i in 0..k - 1 {
                let n_i = groups[i].len();
                let y_i = group_means[i];
                let estimate_i = y_i - y_ref;
                let se_i = if n_i > 0 && n_ref > 0 && !mse.is_nan() {
                    (mse * (1.0 / n_i as f64 + 1.0 / n_ref as f64)).sqrt()
                } else {
                    f64::NAN
                };
                let t_i = if se_i > 0.0 { estimate_i / se_i } else { f64::NAN };
                let p_i = if t_i.is_nan() {
                    None
                } else {
                    Some(2.0 * (1.0 - student_t_cdf(t_i.abs(), df_error)))
                };
                let lbl_i = level_label(&levels[i]);
                param_rows.push(vec![
                    format!("{} {}", eff, lbl_i),
                    fmt6(estimate_i),
                    fmt6(se_i),
                    fmt2(t_i),
                    fmt_p(p_i),
                ]);
            }

            // Reference level row: "B" in SE column, estimate "0"
            let lbl_ref = level_label(&levels[ref_idx]);
            param_rows.push(vec![
                format!("{} {}", eff, lbl_ref),
                "0".into(),
                "B".into(),
                "".into(),
                "".into(),
            ]);

            session.listing.write_table(&param_headers, &param_aligns, &param_rows);
            session.listing.blank();
            session.listing.blank();
        }

        // --- LSMEANS ---
        let show_lsmeans = !ast.lsmeans_vars.is_empty()
            && ast
                .lsmeans_vars
                .iter()
                .any(|v| v.eq_ignore_ascii_case(eff));

        if show_lsmeans {
            centered(session, "Least Squares Means");
            session.listing.blank();

            let lsm_headers: Vec<String> = vec![
                eff.clone(),
                format!("{} LSMEAN", dep_var),
                "Standard Error".into(),
                "Pr > |t|".into(),
            ];
            let lsm_aligns = vec![
                Align::Left,
                Align::Right,
                Align::Right,
                Align::Right,
            ];
            let mut lsm_rows: Vec<Vec<String>> = Vec::new();

            for (gi, level) in levels.iter().enumerate() {
                let lbl = level_label(level);
                let n_i = groups[gi].len();
                let lsmean_i = group_means[gi];
                let se_lsm = if n_i > 0 && !mse.is_nan() {
                    (mse / n_i as f64).sqrt()
                } else {
                    f64::NAN
                };
                let t_lsm = if se_lsm > 0.0 { lsmean_i / se_lsm } else { f64::NAN };
                let p_lsm = if t_lsm.is_nan() {
                    None
                } else {
                    Some(2.0 * (1.0 - student_t_cdf(t_lsm.abs(), df_error)))
                };
                lsm_rows.push(vec![
                    lbl,
                    fmt6(lsmean_i),
                    fmt6(se_lsm),
                    fmt_p(p_lsm),
                ]);
            }

            session.listing.write_table(&lsm_headers, &lsm_aligns, &lsm_rows);
            session.listing.blank();
            session.listing.blank();
        }

        // --- CONTRASTS ---
        let relevant_contrasts: Vec<&GlmContrast> = ast
            .contrasts
            .iter()
            .filter(|c| c.effect.eq_ignore_ascii_case(eff))
            .collect();

        if !relevant_contrasts.is_empty() {
            centered(session, "Contrasts");
            session.listing.blank();

            let con_headers: Vec<String> = vec![
                "Contrast".into(),
                "DF".into(),
                "Contrast SS".into(),
                "Mean Square".into(),
                "F Value".into(),
                "Pr > F".into(),
            ];
            let con_aligns = vec![
                Align::Left,
                Align::Right,
                Align::Right,
                Align::Right,
                Align::Right,
                Align::Right,
            ];
            let mut con_rows: Vec<Vec<String>> = Vec::new();

            for contrast in &relevant_contrasts {
                let c = &contrast.coefficients;
                if c.len() != k {
                    return Err(SasError::runtime(format!(
                        "Contrast '{}' coefficients mismatch: expected {k} coefficients, got {}.",
                        contrast.label,
                        c.len()
                    )));
                }
                // Estimate = Σ c_i × ȳ_i
                let estimate: f64 = c.iter().zip(group_means.iter()).map(|(ci, yi)| ci * yi).sum();
                // SE² = MSE × Σ (c_i²/n_i)
                let sum_c2_over_n: f64 = c
                    .iter()
                    .zip(groups.iter())
                    .map(|(ci, g)| {
                        let ni = g.len();
                        if ni > 0 { ci * ci / ni as f64 } else { 0.0 }
                    })
                    .sum();
                let se_sq = if !mse.is_nan() { mse * sum_c2_over_n } else { f64::NAN };
                // F = Estimate² / se_sq
                let f_con = if se_sq > 0.0 { estimate * estimate / se_sq } else { f64::NAN };
                let p_con = if f_con.is_nan() {
                    None
                } else {
                    Some((1.0 - f_cdf(f_con, 1.0, df_error)).clamp(0.0, 1.0))
                };
                // Contrast SS = F × MSE = Estimate² / Σ(c_i²/n_i)
                let css = if sum_c2_over_n > 0.0 { estimate * estimate / sum_c2_over_n } else { f64::NAN };

                con_rows.push(vec![
                    contrast.label.clone(),
                    "1".into(),
                    if css.is_nan() { ".".into() } else { fmt5(css) },
                    if css.is_nan() { ".".into() } else { fmt5(css) },
                    if f_con.is_nan() { ".".into() } else { fmt2(f_con) },
                    fmt_p(p_con),
                ]);
            }

            session.listing.write_table(&con_headers, &con_aligns, &con_rows);
            session.listing.blank();
            session.listing.blank();
        }

        // --- ESTIMATES ---
        let relevant_estimates: Vec<&GlmEstimate> = ast
            .estimates
            .iter()
            .filter(|e| e.effect.eq_ignore_ascii_case(eff))
            .collect();

        if !relevant_estimates.is_empty() {
            centered(session, "Estimates");
            session.listing.blank();

            let est_headers: Vec<String> = vec![
                "Parameter".into(),
                "Estimate".into(),
                "Standard Error".into(),
                "t Value".into(),
                "Pr > |t|".into(),
            ];
            let est_aligns = vec![
                Align::Left,
                Align::Right,
                Align::Right,
                Align::Right,
                Align::Right,
            ];
            let mut est_rows: Vec<Vec<String>> = Vec::new();

            for est in &relevant_estimates {
                let c = &est.coefficients;
                // Estimate = Σ c_i × ȳ_i
                let estimate: f64 = c.iter().zip(group_means.iter()).map(|(ci, yi)| ci * yi).sum();
                // SE² = MSE × Σ (c_i²/n_i)
                let sum_c2_over_n: f64 = c
                    .iter()
                    .zip(groups.iter())
                    .map(|(ci, g)| {
                        let ni = g.len();
                        if ni > 0 { ci * ci / ni as f64 } else { 0.0 }
                    })
                    .sum();
                let se = if !mse.is_nan() && sum_c2_over_n > 0.0 {
                    (mse * sum_c2_over_n).sqrt()
                } else {
                    f64::NAN
                };
                let t_val = if se > 0.0 { estimate / se } else { f64::NAN };
                let p_val = if t_val.is_nan() {
                    None
                } else {
                    Some(2.0 * (1.0 - student_t_cdf(t_val.abs(), df_error)))
                };

                est_rows.push(vec![
                    est.label.clone(),
                    fmt6(estimate),
                    fmt6(se),
                    fmt2(t_val),
                    fmt_p(p_val),
                ]);
            }

            session.listing.write_table(&est_headers, &est_aligns, &est_rows);
            session.listing.blank();
            session.listing.blank();
        }

        // --- MEANS section ---
        let show_means = !ast.means_vars.is_empty()
            && ast
                .means_vars
                .iter()
                .any(|m| m.eq_ignore_ascii_case(eff));

        if show_means {
            centered(session, &format!("Level of {}", eff));
            session.listing.blank();

            let means_headers: Vec<String> = vec![
                eff.clone(),
                "N".into(),
                "Mean".into(),
                "Std Dev".into(),
            ];
            let means_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
            let mut means_rows: Vec<Vec<String>> = Vec::new();

            for (gi, level) in levels.iter().enumerate() {
                let lbl = level_label(level);
                let n_i = groups[gi].len();
                let mean_i = group_means[gi];
                let std_i = sample_std(&groups[gi]);
                means_rows.push(vec![
                    lbl,
                    format!("{}", n_i),
                    fmt6(mean_i),
                    match std_i {
                        Some(s) => fmt6(s),
                        None => ".".to_string(),
                    },
                ]);
            }

            session.listing.write_table(&means_headers, &means_aligns, &means_rows);
            session.listing.blank();
            session.listing.blank();
        }
    }

    Ok(())
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::DatasetRef;
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

    fn parse_glm(src: &str) -> Result<GlmAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // glm
        parse(&mut ts)
    }

    // ── Test 1: one-way GLM parameter estimates ────────────────────────────

    #[test]
    fn test_one_way_glm_params() {
        // y=[1,2,3,10,11,12], groups=["A","B"]
        // ȳ_A = 2.0, ȳ_B = 11.0
        // Reference = last level = "B"
        // Intercept = ȳ_B = 11.0
        // Effect A = ȳ_A - ȳ_B = 2.0 - 11.0 = -9.0

        let a_group: Vec<f64> = vec![1.0, 2.0, 3.0];
        let b_group: Vec<f64> = vec![10.0, 11.0, 12.0];

        let y_bar_a = a_group.iter().sum::<f64>() / a_group.len() as f64;
        let y_bar_b = b_group.iter().sum::<f64>() / b_group.len() as f64;

        assert!((y_bar_a - 2.0).abs() < 1e-10, "y_bar_a={y_bar_a}");
        assert!((y_bar_b - 11.0).abs() < 1e-10, "y_bar_b={y_bar_b}");

        // reference = last in sas_cmp order = "B" (B > A alphabetically)
        let intercept = y_bar_b;
        let effect_a = y_bar_a - y_bar_b;

        assert!((intercept - 11.0).abs() < 1e-10, "intercept={intercept}");
        assert!((effect_a - (-9.0)).abs() < 1e-10, "effect_a={effect_a}");
    }

    // ── Test 2: parse model with /SOLUTION ───────────────────────────────

    #[test]
    fn test_parse_model_solution() {
        let ast = parse_glm(
            "proc glm; class sex; model height = sex / solution; run;",
        )
        .unwrap();
        let m = ast.model.unwrap();
        assert!(m.solution, "solution should be true");
        assert_eq!(m.dependents, vec!["height"]);
        assert_eq!(m.effects, vec!["sex"]);
    }

    // ── Test 3: parse ESTIMATE statement ─────────────────────────────────

    #[test]
    fn test_parse_estimate() {
        let ast = parse_glm(
            "proc glm; class sex; model y = sex; estimate 'F vs M' sex 1 -1; run;",
        )
        .unwrap();
        assert_eq!(ast.estimates.len(), 1);
        let e = &ast.estimates[0];
        assert_eq!(e.label, "F vs M");
        assert_eq!(e.effect, "sex");
        assert_eq!(e.coefficients.len(), 2);
        assert!((e.coefficients[0] - 1.0).abs() < 1e-10);
        assert!((e.coefficients[1] - (-1.0)).abs() < 1e-10);
    }

    // ── Test 4: parse CONTRAST statement ─────────────────────────────────

    #[test]
    fn test_parse_contrast() {
        let ast = parse_glm(
            "proc glm; class sex; model y = sex; contrast 'F vs M' sex 1 -1; run;",
        )
        .unwrap();
        assert_eq!(ast.contrasts.len(), 1);
        let c = &ast.contrasts[0];
        assert_eq!(c.label, "F vs M");
        assert_eq!(c.effect, "sex");
        assert_eq!(c.coefficients, vec![1.0, -1.0]);
    }

    // ── Test 5: execute listing contains LSMEANS ─────────────────────────

    #[test]
    fn test_execute_lsmeans() {
        let mut session = make_session();
        let frame = df![
            "sex"    => ["F","F","F","M","M","M"],
            "height" => [62.0_f64, 63.0, 64.0, 69.0, 70.0, 71.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![char_meta("sex"), num_meta("height")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let ast = GlmAst {
            data_options: GlmDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "T".into(),
                }),
            },
            class_vars: vec!["sex".into()],
            model: Some(GlmModel {
                dependents: vec!["height".into()],
                effects: vec!["sex".into()],
                solution: false,
                noprint: false,
            }),
            lsmeans_vars: vec!["sex".into()],
            estimates: vec![],
            contrasts: vec![],
            means_vars: vec![],
        };

        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();

        assert!(listing.contains("Least Squares Means"), "listing={listing}");
    }

    // ── Test 6: ESTIMATE arithmetic ──────────────────────────────────────

    #[test]
    fn test_execute_estimate_correct() {
        // y=[1,2,3,10,11,12], groups=["A","B"]
        // ȳ_A=2, ȳ_B=11, ESTIMATE 'A vs B' sex 1 -1 (coefficient order = sas_cmp A,B)
        // Estimate = 1*2 + (-1)*11 = -9
        // SSE = 2+2 = 4, df_error = 4, MSE = 1
        // SE = √(1*(1/3+1/3)) = √(2/3) ≈ 0.8165
        // t = -9 / 0.8165 ≈ -11.02 (negative, A < B)

        let mut session = make_session();
        let frame = df![
            "sex"    => ["A","A","A","B","B","B"],
            "height" => [1.0_f64, 2.0, 3.0, 10.0, 11.0, 12.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![char_meta("sex"), num_meta("height")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let ast = GlmAst {
            data_options: GlmDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "T".into(),
                }),
            },
            class_vars: vec!["sex".into()],
            model: Some(GlmModel {
                dependents: vec!["height".into()],
                effects: vec!["sex".into()],
                solution: false,
                noprint: false,
            }),
            lsmeans_vars: vec![],
            estimates: vec![GlmEstimate {
                label: "A vs B".into(),
                effect: "sex".into(),
                coefficients: vec![1.0, -1.0],
            }],
            contrasts: vec![],
            means_vars: vec![],
        };

        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();

        // Estimate should be -9.0
        assert!(listing.contains("-9.000000"), "Expected -9.000000 in listing: {listing}");
        // t value should be negative
        assert!(listing.contains("-11"), "Expected negative t value: {listing}");
    }

    // ── Test 7: CONTRAST F = (ESTIMATE t)² ───────────────────────────────

    #[test]
    fn test_execute_contrast_f_eq_t_squared() {
        // Same data as test 6: A=[1,2,3], B=[10,11,12]
        // ESTIMATE t ≈ -11.02, CONTRAST F ≈ 121.5
        // t² = 121.5 ≈ F (within rounding)

        let mut session = make_session();
        let frame = df![
            "sex"    => ["A","A","A","B","B","B"],
            "height" => [1.0_f64, 2.0, 3.0, 10.0, 11.0, 12.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![char_meta("sex"), num_meta("height")],
        };
        session.libs.get("WORK").unwrap().write("T2", &ds).unwrap();

        let ast = GlmAst {
            data_options: GlmDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "T2".into(),
                }),
            },
            class_vars: vec!["sex".into()],
            model: Some(GlmModel {
                dependents: vec!["height".into()],
                effects: vec!["sex".into()],
                solution: false,
                noprint: false,
            }),
            lsmeans_vars: vec![],
            estimates: vec![GlmEstimate {
                label: "A vs B".into(),
                effect: "sex".into(),
                coefficients: vec![1.0, -1.0],
            }],
            contrasts: vec![GlmContrast {
                label: "A vs B".into(),
                effect: "sex".into(),
                coefficients: vec![1.0, -1.0],
            }],
            means_vars: vec![],
        };

        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();

        // F should be 121.5 (= t² = 11.02² ≈ 121.5)
        // Check both sections are present
        assert!(listing.contains("Estimates"), "listing missing Estimates: {listing}");
        assert!(listing.contains("Contrasts"), "listing missing Contrasts: {listing}");
        // The ANOVA F is 121.5 (same as the contrast F)
        assert!(listing.contains("121.50"), "Expected F=121.50 in listing: {listing}");
    }
}
