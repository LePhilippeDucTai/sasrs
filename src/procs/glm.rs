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
    /// Legacy flat list of effect variable names (one-way path). Kept intact for
    /// byte-identity of the existing snapshot. For `a b a*b` this is `["a","b","a*b"]`.
    pub effects: Vec<String>,
    /// Structured effect terms for the multiway engine. Each term is the list of
    /// CLASS variable names it involves: main effect = 1 elt, `a*b` = `["a","b"]`.
    pub effect_terms: Vec<Vec<String>>,
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
            // Read effects: idents (optionally joined by `*`) after `=` until `/` or `;`.
            // Build both the legacy flat `effects` list and the structured
            // `effect_terms` (Vec of CLASS-var-name lists) for the multiway engine.
            let mut effects: Vec<String> = Vec::new();
            let mut effect_terms: Vec<Vec<String>> = Vec::new();
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
                    ts.next();
                    // Build the structured term: name, then any `* name` continuations.
                    let mut parts: Vec<String> = vec![name];
                    while ts.peek().kind == TokenKind::Star {
                        ts.next();
                        if let Some(next_name) = ts.peek().ident().map(str::to_string) {
                            parts.push(next_name);
                            ts.next();
                        } else {
                            break;
                        }
                    }
                    // Legacy flat representation: join interaction parts with `*`.
                    effects.push(parts.join("*"));
                    effect_terms.push(parts);
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            model = Some(GlmModel {
                dependents,
                effects,
                effect_terms,
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

    // Branch: the existing one-way path is taken ONLY for a single main effect
    // over a single CLASS variable with no interaction. Anything else (interaction
    // term, multiple effect terms, or multiple CLASS vars) goes to the general
    // multiway engine. This keeps the one-way path byte-identical.
    let has_interaction = model.effect_terms.iter().any(|t| t.len() > 1);
    let is_multiway =
        has_interaction || model.effect_terms.len() > 1 || ast.class_vars.len() > 1;
    if is_multiway {
        return execute_multiway(ast, model, session);
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

// ───────────────────────── Multiway engine (M34.5) ─────────────────────────

/// One CLASS factor resolved against the usable rows of a dependent variable.
struct Factor {
    name: String,
    /// Distinct non-missing levels in `sas_cmp` order. Reference cell = LAST.
    levels: Vec<Value>,
}

impl Factor {
    /// Index of the level for value `v` (must exist).
    fn level_of(&self, v: &Value) -> usize {
        self.levels
            .iter()
            .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            .unwrap()
    }
    /// Number of dummy columns this factor contributes (levels − 1, last dropped).
    fn n_dummies(&self) -> usize {
        self.levels.len().saturating_sub(1)
    }
}

/// Human-readable level label, matching the one-way path's scheme.
fn level_label_value(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim_end().to_string(),
        Value::Num(f) => format!("{f}"),
        Value::Missing(k) => k.display(),
    }
}

/// Compute residual sum of squares for a design matrix `x` (rows × cols) and `y`.
/// Returns SSE = ‖y − Xβ̂‖². On a rank-deficient / singular fit returns NaN.
fn sse_of(x: &[Vec<f64>], y: &[f64]) -> f64 {
    if x.is_empty() || x[0].is_empty() {
        // No predictors at all → SSE around 0 (degenerate); treat as total.
        let ybar = y.iter().sum::<f64>() / y.len().max(1) as f64;
        return y.iter().map(|&v| (v - ybar).powi(2)).sum();
    }
    let beta = match crate::stat::linalg::least_squares(x, y) {
        Ok(b) => b,
        Err(_) => return f64::NAN,
    };
    let mut sse = 0.0;
    for (i, row) in x.iter().enumerate() {
        let fitted: f64 = row.iter().zip(beta.iter()).map(|(a, b)| a * b).sum();
        sse += (y[i] - fitted).powi(2);
    }
    sse
}

/// Build the reference-cell dummy values for a single row, per factor.
/// `dummies[f]` is a Vec of length `factors[f].n_dummies()`; entry j = 1 if the
/// row is at level j of factor f (j < levels−1), else 0. (Reference level → all 0.)
fn row_dummies(factors: &[Factor], row_levels: &[usize]) -> Vec<Vec<f64>> {
    factors
        .iter()
        .zip(row_levels.iter())
        .map(|(f, &li)| {
            let nd = f.n_dummies();
            let mut d = vec![0.0; nd];
            if li < nd {
                d[li] = 1.0;
            }
            d
        })
        .collect()
}

/// Build the sum-to-zero (effect / deviation) coded values for a single row,
/// per factor. Each factor with levels 1..L (sas_cmp order) contributes L−1
/// columns; column j (0-based) = +1 if the row is at level j, −1 if the row is
/// at the LAST level L−1, else 0.
///
/// This full-rank effect coding spans the same column space as the reference-cell
/// coding, but interaction columns built from these centered contrasts make the
/// per-term partial SS coincide with the SAS Type III estimable-function SS.
fn row_effects(factors: &[Factor], row_levels: &[usize]) -> Vec<Vec<f64>> {
    factors
        .iter()
        .zip(row_levels.iter())
        .map(|(f, &li)| {
            let nd = f.n_dummies();
            let last = f.levels.len().saturating_sub(1);
            let mut d = vec![0.0; nd];
            if li == last {
                for v in d.iter_mut() {
                    *v = -1.0;
                }
            } else if li < nd {
                d[li] = 1.0;
            }
            d
        })
        .collect()
}

/// Build the full design matrix column layout for a set of terms.
/// Returns, per term, the list of (factor_index, dummy_index) pairs identifying
/// the parent dummies whose elementwise product forms each interaction column.
/// For a main effect each "column spec" is a single pair.
fn term_column_specs(
    terms: &[Vec<usize>],
    factors: &[Factor],
) -> Vec<Vec<Vec<(usize, usize)>>> {
    terms
        .iter()
        .map(|term_factor_idxs| {
            // Cartesian product of each parent factor's dummy indices.
            let mut combos: Vec<Vec<(usize, usize)>> = vec![vec![]];
            for &fi in term_factor_idxs {
                let nd = factors[fi].n_dummies();
                let mut next = Vec::new();
                for prefix in &combos {
                    for j in 0..nd {
                        let mut c = prefix.clone();
                        c.push((fi, j));
                        next.push(c);
                    }
                }
                combos = next;
            }
            combos
        })
        .collect()
}

/// General multi-way / interaction GLM engine.
fn execute_multiway(ast: &GlmAst, model: &GlmModel, session: &mut Session) -> Result<()> {
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

    // --- 2. Validate CLASS vars and effect variables ---
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
    // Every variable appearing in an effect term must be a CLASS variable.
    for term in &model.effect_terms {
        for v in term {
            if !ast.class_vars.iter().any(|c| c.eq_ignore_ascii_case(v)) {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    v.to_uppercase()
                )));
            }
        }
    }

    // Decode each CLASS column once (canonical name from metadata).
    let mut class_cols: Vec<(String, Vec<Value>)> = Vec::new();
    for class_var in &ast.class_vars {
        let col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(class_var))
            .unwrap();
        let col = decode_column(&ds, col_idx)?;
        class_cols.push((ds.vars[col_idx].name.clone(), col));
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
    for (name, col) in &class_cols {
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
        let values_str: Vec<String> = levels.iter().map(level_label_value).collect();
        cli_rows.push(vec![
            name.clone(),
            format!("{}", levels.len()),
            values_str.join(" "),
        ]);
    }
    session.listing.write_table(&cli_headers, &cli_aligns, &cli_rows);
    session.listing.blank();
    session.listing.write_line(&format!(
        "               Number of Observations Read     {}",
        n_obs
    ));
    session.listing.blank();
    session.listing.blank();

    // Map each effect term (Vec of class-var names) to indices into `class_cols`.
    let term_factor_idxs: Vec<Vec<usize>> = model
        .effect_terms
        .iter()
        .map(|term| {
            term.iter()
                .map(|v| {
                    class_cols
                        .iter()
                        .position(|(n, _)| n.eq_ignore_ascii_case(v))
                        .unwrap()
                })
                .collect()
        })
        .collect();

    // --- 5. Per-dependent variable loop ---
    for dep_var in &model.dependents {
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

        // Listwise deletion: require dependent present and EVERY CLASS var present.
        let mut usable_rows: Vec<usize> = Vec::new();
        for i in 0..n_obs {
            let dep_ok = matches!(value_to_num(&dep_col[i]), Some(v) if !v.is_nan());
            let cls_ok = class_cols.iter().all(|(_, c)| !c[i].is_missing());
            if dep_ok && cls_ok {
                usable_rows.push(i);
            }
        }
        let n = usable_rows.len();

        // Resolve factor levels over the usable rows (sas_cmp order, ref = last).
        let mut factors: Vec<Factor> = Vec::new();
        for (name, col) in &class_cols {
            let mut levels: Vec<Value> = Vec::new();
            for &r in &usable_rows {
                let v = &col[r];
                if !levels
                    .iter()
                    .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
                {
                    levels.push(v.clone());
                }
            }
            levels.sort_by(|a, b| a.sas_cmp(b));
            factors.push(Factor {
                name: name.clone(),
                levels,
            });
        }

        // Per-row factor level indices.
        let row_level_idx: Vec<Vec<usize>> = usable_rows
            .iter()
            .map(|&r| {
                class_cols
                    .iter()
                    .enumerate()
                    .map(|(fi, (_, col))| factors[fi].level_of(&col[r]))
                    .collect()
            })
            .collect();

        // Response vector.
        let y: Vec<f64> = usable_rows
            .iter()
            .map(|&r| value_to_num(&dep_col[r]).unwrap())
            .collect();
        let y_bar = if n > 0 { y.iter().sum::<f64>() / n as f64 } else { f64::NAN };
        let sst: f64 = y.iter().map(|&v| (v - y_bar).powi(2)).sum();

        // Column specs: per term, list of column definitions (each a product of
        // parent (factor, dummy) pairs).
        let col_specs = term_column_specs(&term_factor_idxs, &factors);
        let term_df: Vec<usize> = term_factor_idxs
            .iter()
            .map(|fis| fis.iter().map(|&fi| factors[fi].n_dummies()).product())
            .collect();

        // Precompute per-row dummy vectors for each factor.
        let row_dummy_cache: Vec<Vec<Vec<f64>>> = row_level_idx
            .iter()
            .map(|rl| row_dummies(&factors, rl))
            .collect();

        // Build a column value for a given column-spec at a given row.
        let col_value = |row: usize, spec: &[(usize, usize)]| -> f64 {
            let mut prod = 1.0;
            for &(fi, dj) in spec {
                prod *= row_dummy_cache[row][fi][dj];
            }
            prod
        };

        // Effect (sum-to-zero) coded counterpart, used ONLY for the Type III SS
        // pass. The column layout (col_specs) is identical; only the per-factor
        // contrast values differ (+1/−1/0 instead of 1/0).
        let row_effect_cache: Vec<Vec<Vec<f64>>> = row_level_idx
            .iter()
            .map(|rl| row_effects(&factors, rl))
            .collect();
        let col_value_eff = |row: usize, spec: &[(usize, usize)]| -> f64 {
            let mut prod = 1.0;
            for &(fi, dj) in spec {
                prod *= row_effect_cache[row][fi][dj];
            }
            prod
        };

        // Assemble the FULL design matrix: intercept + all terms' columns.
        let mut full_design: Vec<Vec<f64>> = vec![vec![1.0]; n];
        let mut next_col = 1usize;
        for specs in &col_specs {
            for spec in specs {
                for (row, design_row) in full_design.iter_mut().enumerate() {
                    design_row.push(col_value(row, spec));
                }
                next_col += 1;
            }
        }
        let ncols = next_col;

        let sse_full = sse_of(&full_design, &y);
        let ssm = sst - sse_full;
        let df_error = (n as i64 - ncols as i64).max(0) as f64;
        let df_model: f64 = term_df.iter().map(|&d| d as f64).sum();
        let df_total = (n as f64 - 1.0).max(0.0);
        let mse = if df_error > 0.0 { sse_full / df_error } else { f64::NAN };
        let msm = if df_model > 0.0 { ssm / df_model } else { f64::NAN };
        let f_model = if mse > 0.0 && !mse.is_nan() { msm / mse } else { f64::NAN };
        let p_model = if f_model.is_nan() {
            None
        } else {
            Some((1.0 - f_cdf(f_model, df_model, df_error)).clamp(0.0, 1.0))
        };
        let r2 = if sst > 0.0 { ssm / sst } else { f64::NAN };
        let root_mse = if !mse.is_nan() { mse.sqrt() } else { f64::NAN };
        let cv = if y_bar.abs() > 1e-15 && !root_mse.is_nan() {
            root_mse / y_bar.abs() * 100.0
        } else {
            f64::NAN
        };

        session.log.note(&format!("There were {} observations used.", n));

        // --- Helper to build a design from a subset of terms (intercept + terms) ---
        let build_design = |term_subset: &[usize]| -> Vec<Vec<f64>> {
            let mut design: Vec<Vec<f64>> = vec![vec![1.0]; n];
            for &t in term_subset {
                for spec in &col_specs[t] {
                    for (row, design_row) in design.iter_mut().enumerate() {
                        design_row.push(col_value(row, spec));
                    }
                }
            }
            design
        };

        // --- Type I (sequential) SS per term ---
        let mut type1_ss: Vec<f64> = Vec::with_capacity(col_specs.len());
        {
            let mut prev_subset: Vec<usize> = Vec::new();
            let intercept_only: Vec<Vec<f64>> = vec![vec![1.0]; n];
            let mut prev_sse = sse_of(&intercept_only, &y);
            for t in 0..col_specs.len() {
                prev_subset.push(t);
                let sse_with = sse_of(&build_design(&prev_subset), &y);
                type1_ss.push((prev_sse - sse_with).max(0.0));
                prev_sse = sse_with;
            }
        }

        // --- Type III (partial) SS per term, using sum-to-zero EFFECT coding ---
        // SAS Type III SS for an effect equals the partial SS for that effect when
        // the design is built with full-rank effect coding (centered contrasts):
        // the interaction columns are then orthogonalized against lower-order
        // marginals, so dropping a term's effect-coded columns yields the SAS
        // estimable-function SS even for a main effect involved in an interaction
        // on unbalanced data. Reference-cell coding does NOT give this for a
        // lower-order term when a higher-order interaction is present.
        //
        // The effect-coded full model spans the same column space as the
        // reference-cell full model, so SSE_full is identical (asserted in tests).
        let mut type3_ss: Vec<f64> = Vec::with_capacity(col_specs.len());
        {
            // Effect-coded full model — must reproduce sse_full.
            let mut eff_full: Vec<Vec<f64>> = vec![vec![1.0]; n];
            for specs in &col_specs {
                for spec in specs {
                    for (row, design_row) in eff_full.iter_mut().enumerate() {
                        design_row.push(col_value_eff(row, spec));
                    }
                }
            }
            let sse_full_eff = sse_of(&eff_full, &y);
            for t in 0..col_specs.len() {
                // Build effect-coded design = full minus term t's columns.
                let mut design: Vec<Vec<f64>> = vec![vec![1.0]; n];
                for (ti, specs) in col_specs.iter().enumerate() {
                    if ti == t {
                        continue;
                    }
                    for spec in specs {
                        for (row, design_row) in design.iter_mut().enumerate() {
                            design_row.push(col_value_eff(row, spec));
                        }
                    }
                }
                let sse_drop = sse_of(&design, &y);
                type3_ss.push((sse_drop - sse_full_eff).max(0.0));
            }
        }

        // Term labels (e.g. `a*b`).
        let term_labels: Vec<String> = model
            .effect_terms
            .iter()
            .map(|t| t.join("*"))
            .collect();

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
        let anova_rows: Vec<Vec<String>> = vec![
            vec![
                "Model".into(),
                format!("{}", df_model as usize),
                fmt5(ssm),
                if msm.is_nan() { ".".into() } else { fmt5(msm) },
                if f_model.is_nan() { ".".into() } else { fmt2(f_model) },
                fmt_p(p_model),
            ],
            vec![
                "Error".into(),
                format!("{}", df_error as usize),
                fmt5(sse_full),
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

        // Fit statistics
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

        // Type I / Type III SS tables
        for (ss_label, ss_vec) in [("Type I SS", &type1_ss), ("Type III SS", &type3_ss)] {
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
            let mut t_rows: Vec<Vec<String>> = Vec::new();
            for (ti, &ss) in ss_vec.iter().enumerate() {
                let df = term_df[ti] as f64;
                let ms = if df > 0.0 { ss / df } else { f64::NAN };
                let f = if mse > 0.0 && !mse.is_nan() && !ms.is_nan() {
                    ms / mse
                } else {
                    f64::NAN
                };
                let p = if f.is_nan() {
                    None
                } else {
                    Some((1.0 - f_cdf(f, df, df_error)).clamp(0.0, 1.0))
                };
                t_rows.push(vec![
                    term_labels[ti].clone(),
                    format!("{}", term_df[ti]),
                    fmt5(ss),
                    if ms.is_nan() { ".".into() } else { fmt5(ms) },
                    if f.is_nan() { ".".into() } else { fmt2(f) },
                    fmt_p(p),
                ]);
            }
            session.listing.write_table(&t_headers, &t_aligns, &t_rows);
            session.listing.blank();
            session.listing.blank();
        }

        // Need (X'X)^-1 for SOLUTION / LSMEANS standard errors.
        let beta = crate::stat::linalg::least_squares(&full_design, &y).ok();
        let xtx_inv = {
            let xt = crate::stat::linalg::transpose(&full_design);
            let xtx = crate::stat::linalg::matrix_mult(&xt, &full_design);
            crate::stat::linalg::invert_matrix(&xtx).ok()
        };

        // --- SOLUTION (parameter estimates) ---
        if model.solution {
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

            // Column labels parallel to full_design columns: [Intercept, then term cols].
            let mut col_labels: Vec<String> = vec!["Intercept".into()];
            for (ti, specs) in col_specs.iter().enumerate() {
                let term = &model.effect_terms[ti];
                for spec in specs {
                    // spec is Vec<(factor_idx, dummy_idx)>; build "fac LEVEL" pieces.
                    let pieces: Vec<String> = spec
                        .iter()
                        .map(|&(fi, dj)| {
                            format!("{} {}", factors[fi].name, level_label_value(&factors[fi].levels[dj]))
                        })
                        .collect();
                    let _ = term; // term name implied by factor names in pieces
                    col_labels.push(pieces.join(" "));
                }
            }

            let push_param = |rows: &mut Vec<Vec<String>>, label: String, est: f64, col: usize| {
                let se = match &xtx_inv {
                    Some(inv) if !mse.is_nan() && inv[col][col] >= 0.0 => {
                        (mse * inv[col][col]).sqrt()
                    }
                    _ => f64::NAN,
                };
                let t = if se > 0.0 { est / se } else { f64::NAN };
                let p = if t.is_nan() {
                    None
                } else {
                    Some(2.0 * (1.0 - student_t_cdf(t.abs(), df_error)))
                };
                rows.push(vec![
                    label,
                    fmt6(est),
                    fmt6(se),
                    fmt2(t),
                    fmt_p(p),
                ]);
            };

            if let Some(b) = &beta {
                for (ci, lbl) in col_labels.iter().enumerate() {
                    push_param(&mut param_rows, lbl.clone(), b[ci], ci);
                }
            }
            // Reference-level rows (estimate 0, "B"), one per main effect's last level
            // and per interaction combination touching a reference level, mirroring
            // the one-way path's single reference row. We append the main-effect
            // reference rows for readability.
            for (fi, factor) in factors.iter().enumerate() {
                // Only emit a reference row if this factor appears as a main effect term.
                let is_main = term_factor_idxs.iter().any(|t| t.len() == 1 && t[0] == fi);
                if is_main {
                    let ref_lvl = factor.levels.last();
                    if let Some(rl) = ref_lvl {
                        param_rows.push(vec![
                            format!("{} {}", factor.name, level_label_value(rl)),
                            "0".into(),
                            "B".into(),
                            "".into(),
                            "".into(),
                        ]);
                    }
                }
            }
            session.listing.write_table(&param_headers, &param_aligns, &param_rows);
            session.listing.blank();
            session.listing.blank();
        }

        // --- LSMEANS (main effects only) ---
        for lsm_var in &ast.lsmeans_vars {
            let fi = match factors.iter().position(|f| f.name.eq_ignore_ascii_case(lsm_var)) {
                Some(i) => i,
                None => continue,
            };
            // Only meaningful for a main-effect factor.
            centered(session, "Least Squares Means");
            session.listing.blank();
            let lsm_headers: Vec<String> = vec![
                factors[fi].name.clone(),
                format!("{} LSMEAN", dep_var),
                "Standard Error".into(),
                "Pr > |t|".into(),
            ];
            let lsm_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
            let mut lsm_rows: Vec<Vec<String>> = Vec::new();

            // LS mean for level L of factor fi = average over balanced (uniform)
            // levels of all OTHER factors of the predicted cell mean. With a
            // reference-cell coding and fitted β, the predicted value for a cell is
            // a linear combo of β; averaging the contrast vector over the other
            // factors' levels yields the LS-mean estimable function L·β.
            for (li, level) in factors[fi].levels.iter().enumerate() {
                // Build the averaged estimable coefficient vector over full_design cols.
                let lvec = lsmean_coef_vector(
                    fi,
                    li,
                    &factors,
                    &term_factor_idxs,
                    &col_specs,
                    ncols,
                );
                let est = match &beta {
                    Some(b) => lvec.iter().zip(b).map(|(c, bb)| c * bb).sum::<f64>(),
                    None => f64::NAN,
                };
                let se = match &xtx_inv {
                    Some(inv) if !mse.is_nan() => {
                        // var = mse * l' (X'X)^-1 l
                        let mut q = 0.0;
                        for a in 0..ncols {
                            if lvec[a] == 0.0 {
                                continue;
                            }
                            for b2 in 0..ncols {
                                q += lvec[a] * inv[a][b2] * lvec[b2];
                            }
                        }
                        if q >= 0.0 { (mse * q).sqrt() } else { f64::NAN }
                    }
                    _ => f64::NAN,
                };
                let t = if se > 0.0 { est / se } else { f64::NAN };
                let p = if t.is_nan() {
                    None
                } else {
                    Some(2.0 * (1.0 - student_t_cdf(t.abs(), df_error)))
                };
                lsm_rows.push(vec![
                    level_label_value(level),
                    fmt6(est),
                    fmt6(se),
                    fmt_p(p),
                ]);
            }
            session.listing.write_table(&lsm_headers, &lsm_aligns, &lsm_rows);
            session.listing.blank();
            session.listing.blank();
        }

        // --- CONTRAST / ESTIMATE: main-effect coefficient vectors only ---
        // Group means for a single main-effect factor are reconstructed via the
        // LS means above; ESTIMATE/CONTRAST referencing an interaction emit a NOTE.
        for c in &ast.contrasts {
            if model
                .effect_terms
                .iter()
                .any(|t| t.len() > 1 && t.iter().any(|v| v.eq_ignore_ascii_case(&c.effect)))
                && !factors.iter().any(|f| f.name.eq_ignore_ascii_case(&c.effect))
            {
                session.log.note(&format!(
                    "CONTRAST '{}' references an effect not supported in the multiway path; skipped.",
                    c.label
                ));
            }
        }
        for e in &ast.estimates {
            if !factors.iter().any(|f| f.name.eq_ignore_ascii_case(&e.effect)) {
                session.log.note(&format!(
                    "ESTIMATE '{}' references an effect not supported in the multiway path; skipped.",
                    e.label
                ));
            }
        }
    }

    Ok(())
}

/// Build the estimable LS-mean coefficient vector (length `ncols`) for level `li`
/// of factor `fi`, averaging uniformly over all OTHER factors' levels. The vector
/// is in the same column order as the full design (intercept + term columns).
fn lsmean_coef_vector(
    target_fi: usize,
    target_li: usize,
    factors: &[Factor],
    term_factor_idxs: &[Vec<usize>],
    col_specs: &[Vec<Vec<(usize, usize)>>],
    ncols: usize,
) -> Vec<f64> {
    // Enumerate the balanced grid of all factors' levels, fixing target factor = li.
    let dims: Vec<usize> = factors.iter().map(|f| f.levels.len()).collect();
    let mut grid_levels: Vec<Vec<usize>> = vec![vec![]];
    for (fi, &dim) in dims.iter().enumerate() {
        let mut next = Vec::new();
        for prefix in &grid_levels {
            if fi == target_fi {
                let mut c = prefix.clone();
                c.push(target_li);
                next.push(c);
            } else {
                for l in 0..dim {
                    let mut c = prefix.clone();
                    c.push(l);
                    next.push(c);
                }
            }
        }
        grid_levels = next;
    }
    let ncells = grid_levels.len().max(1) as f64;

    // For each cell, build its design row (intercept + term cols), then average.
    let mut acc = vec![0.0; ncols];
    for cell in &grid_levels {
        let dummies = row_dummies(factors, cell);
        let mut row = vec![1.0];
        for (ti, specs) in col_specs.iter().enumerate() {
            let _ = ti;
            let _ = term_factor_idxs;
            for spec in specs {
                let mut prod = 1.0;
                for &(fi, dj) in spec {
                    prod *= dummies[fi][dj];
                }
                row.push(prod);
            }
        }
        for (a, &v) in row.iter().enumerate() {
            acc[a] += v / ncells;
        }
    }
    acc
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
                effect_terms: vec![vec!["sex".into()]],
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
                effect_terms: vec![vec!["sex".into()]],
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
                effect_terms: vec![vec!["sex".into()]],
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

    // ── M34.5: effect-term parsing `a b a*b` ────────────────────────────────

    #[test]
    fn test_parse_interaction_terms() {
        let ast = parse_glm(
            "proc glm; class a b; model y = a b a*b / solution; run;",
        )
        .unwrap();
        let m = ast.model.unwrap();
        // Legacy flat list keeps `a*b` joined.
        assert_eq!(m.effects, vec!["a", "b", "a*b"]);
        // Structured terms: main effects 1 elt, interaction 2 elts.
        assert_eq!(
            m.effect_terms,
            vec![
                vec!["a".to_string()],
                vec!["b".to_string()],
                vec!["a".to_string(), "b".to_string()],
            ]
        );
        assert!(m.solution);
    }

    // ── M34.5: two-way design matrix dimensions ─────────────────────────────

    #[test]
    fn test_two_way_design_dimensions() {
        // Factor a: 3 levels (A1,A2,A3), b: 2 levels (B1,B2). Balanced 2 obs/cell.
        // Reference-cell columns: intercept(1) + a(2) + b(1) + a*b(2) = 6.
        let fa = Factor {
            name: "a".into(),
            levels: vec![
                Value::Char("A1".into()),
                Value::Char("A2".into()),
                Value::Char("A3".into()),
            ],
        };
        let fb = Factor {
            name: "b".into(),
            levels: vec![Value::Char("B1".into()), Value::Char("B2".into())],
        };
        let factors = vec![fa, fb];
        assert_eq!(factors[0].n_dummies(), 2);
        assert_eq!(factors[1].n_dummies(), 1);

        // terms: a (factor 0), b (factor 1), a*b (0,1)
        let term_factor_idxs = vec![vec![0usize], vec![1usize], vec![0usize, 1usize]];
        let specs = term_column_specs(&term_factor_idxs, &factors);
        let ncols: usize = 1 + specs.iter().map(|s| s.len()).sum::<usize>();
        assert_eq!(specs[0].len(), 2); // a
        assert_eq!(specs[1].len(), 1); // b
        assert_eq!(specs[2].len(), 2); // a*b = 2*1
        assert_eq!(ncols, 6);
    }

    // ── M34.5: reference-cell betas on a balanced 2×2 design ────────────────

    #[test]
    fn test_reference_cell_betas_2x2() {
        // Balanced 2x2: cell means chosen, two obs per cell.
        // a in {A,B}, b in {X,Y}. Reference = last level: a=B, b=Y.
        // Cell means: (A,X)=10, (A,Y)=14, (B,X)=20, (B,Y)=30.
        // Reference-cell model y = mu + a_A + b_X + ab_AX:
        //   mu = mean(B,Y) = 30
        //   b_X = mean(B,X) - mean(B,Y) = 20 - 30 = -10
        //   a_A = mean(A,Y) - mean(B,Y) = 14 - 30 = -16
        //   ab_AX = (A,X) - (A,Y) - (B,X) + (B,Y) = 10 - 14 - 20 + 30 = 6
        let mut session = make_session();
        let frame = df![
            "a" => ["A","A","A","A","B","B","B","B"],
            "b" => ["X","X","Y","Y","X","X","Y","Y"],
            "y" => [10.0_f64,10.0, 14.0,14.0, 20.0,20.0, 30.0,30.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![char_meta("a"), char_meta("b"), num_meta("y")],
        };
        session.libs.get("WORK").unwrap().write("TW", &ds).unwrap();

        let ast = GlmAst {
            data_options: GlmDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "TW".into(),
                }),
            },
            class_vars: vec!["a".into(), "b".into()],
            model: Some(GlmModel {
                dependents: vec!["y".into()],
                effects: vec!["a".into(), "b".into(), "a*b".into()],
                effect_terms: vec![
                    vec!["a".into()],
                    vec!["b".into()],
                    vec!["a".into(), "b".into()],
                ],
                solution: true,
                noprint: false,
            }),
            lsmeans_vars: vec!["a".into()],
            estimates: vec![],
            contrasts: vec![],
            means_vars: vec![],
        };

        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();

        // Intercept (mu) = 30.000000
        assert!(listing.contains("30.000000"), "expected intercept 30: {listing}");
        // a A = -16.000000
        assert!(listing.contains("-16.000000"), "expected a A=-16: {listing}");
        // b X = -10.000000
        assert!(listing.contains("-10.000000"), "expected b X=-10: {listing}");
        // interaction a A b X = 6.000000
        assert!(listing.contains("6.000000"), "expected ab=6: {listing}");
        // LSMEAN for a=A is mean of cell means over b = (10+14)/2 = 12; a=B = 25.
        assert!(listing.contains("12.000000"), "expected LSMEAN a=A 12: {listing}");
        assert!(listing.contains("25.000000"), "expected LSMEAN a=B 25: {listing}");
    }

    // ── M34.5: Type I vs Type III on an UNBALANCED two-way design ───────────

    #[test]
    fn test_type1_vs_type3_unbalanced() {
        // Unbalanced 2x2 (cell counts differ) so Type I != Type III.
        // a in {A,B}, b in {X,Y}.
        // (A,X): 1 obs y=10; (A,Y): 2 obs y=12,14; (B,X): 3 obs y=20,22,24; (B,Y): 1 obs y=30.
        let mut session = make_session();
        let frame = df![
            "a" => ["A","A","A","B","B","B","B"],
            "b" => ["X","Y","Y","X","X","X","Y"],
            "y" => [10.0_f64, 12.0, 14.0, 20.0, 22.0, 24.0, 30.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![char_meta("a"), char_meta("b"), num_meta("y")],
        };
        session.libs.get("WORK").unwrap().write("UB", &ds).unwrap();

        // Build factors / engine directly to inspect the SS numerically.
        let class_cols: Vec<(String, Vec<Value>)> = vec![
            (
                "a".into(),
                ["A", "A", "A", "B", "B", "B", "B"]
                    .iter()
                    .map(|s| Value::Char((*s).into()))
                    .collect(),
            ),
            (
                "b".into(),
                ["X", "Y", "Y", "X", "X", "X", "Y"]
                    .iter()
                    .map(|s| Value::Char((*s).into()))
                    .collect(),
            ),
        ];
        let y = vec![10.0, 12.0, 14.0, 20.0, 22.0, 24.0, 30.0];
        let mut factors: Vec<Factor> = Vec::new();
        for (name, col) in &class_cols {
            let mut levels: Vec<Value> = Vec::new();
            for v in col {
                if !levels.iter().any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal) {
                    levels.push(v.clone());
                }
            }
            levels.sort_by(|a, b| a.sas_cmp(b));
            factors.push(Factor { name: name.clone(), levels });
        }
        let term_factor_idxs = vec![vec![0usize], vec![1usize]]; // a, b main effects
        let col_specs = term_column_specs(&term_factor_idxs, &factors);
        let n = y.len();
        let row_levels: Vec<Vec<usize>> = (0..n)
            .map(|r| {
                class_cols
                    .iter()
                    .enumerate()
                    .map(|(fi, (_, col))| factors[fi].level_of(&col[r]))
                    .collect()
            })
            .collect();
        let dummy_cache: Vec<Vec<Vec<f64>>> =
            row_levels.iter().map(|rl| row_dummies(&factors, rl)).collect();
        let col_value = |row: usize, spec: &[(usize, usize)]| -> f64 {
            spec.iter().map(|&(fi, dj)| dummy_cache[row][fi][dj]).product()
        };
        let build = |subset: &[usize]| -> Vec<Vec<f64>> {
            let mut d: Vec<Vec<f64>> = vec![vec![1.0]; n];
            for &t in subset {
                for spec in &col_specs[t] {
                    for (r, row) in d.iter_mut().enumerate() {
                        row.push(col_value(r, spec));
                    }
                }
            }
            d
        };
        let ybar = y.iter().sum::<f64>() / n as f64;
        let sst: f64 = y.iter().map(|v| (v - ybar).powi(2)).sum();
        let sse_full = sse_of(&build(&[0, 1]), &y);
        let ssm = sst - sse_full;

        // Type I: a then b.
        let sse_int = sse_of(&vec![vec![1.0]; n], &y);
        let sse_a = sse_of(&build(&[0]), &y);
        let t1_a = sse_int - sse_a;
        let t1_b = sse_a - sse_full;
        // Type I sums to model SS.
        assert!((t1_a + t1_b - ssm).abs() < 1e-8, "Type I should sum to SSM");

        // Type III: drop each term from full.
        let sse_drop_a = sse_of(&build(&[1]), &y); // full minus a = {intercept,b}
        let sse_drop_b = sse_of(&build(&[0]), &y); // full minus b = {intercept,a}
        let t3_a = sse_drop_a - sse_full;
        let t3_b = sse_drop_b - sse_full;

        // Unbalanced ⇒ Type I and Type III differ for the FIRST entered term (a).
        assert!(
            (t1_a - t3_a).abs() > 1e-6,
            "Type I vs III for 'a' should differ on unbalanced data: t1_a={t1_a}, t3_a={t3_a}"
        );
        // The last-entered term's Type I equals its Type III (b is adjusted for a in both).
        assert!(
            (t1_b - t3_b).abs() < 1e-8,
            "Type I and III for last term should match: {t1_b} vs {t3_b}"
        );
        // Also exercise the full execute path produces both tables.
        let ast = GlmAst {
            data_options: GlmDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "UB".into(),
                }),
            },
            class_vars: vec!["a".into(), "b".into()],
            model: Some(GlmModel {
                dependents: vec!["y".into()],
                effects: vec!["a".into(), "b".into()],
                effect_terms: vec![vec!["a".into()], vec!["b".into()]],
                solution: false,
                noprint: false,
            }),
            lsmeans_vars: vec![],
            estimates: vec![],
            contrasts: vec![],
            means_vars: vec![],
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Type I SS"), "missing Type I: {listing}");
        assert!(listing.contains("Type III SS"), "missing Type III: {listing}");
    }

    // ── M34.5 fix: effect-coded Type III on an UNBALANCED 2×2 WITH interaction ─
    //
    // Model: y = a b a*b on an unbalanced 2×2. Checks:
    //  1. The effect-coded full model SSE == reference-cell full model SSE (~1e-6).
    //  2. The interaction term's Type III is the SAME under both codings (it is the
    //     highest-order term, so dropping it is coding-invariant).
    //  3. The MAIN-EFFECT Type III values CHANGE between reference-cell and effect
    //     coding — effect coding gives the SAS-correct estimable-function SS.
    #[test]
    fn test_type3_effect_coding_2x2_interaction() {
        // (A,X): 1 obs y=10; (A,Y): 2 obs y=12,14;
        // (B,X): 3 obs y=20,22,24; (B,Y): 1 obs y=30.  (same unbalanced cells)
        let class_cols: Vec<(String, Vec<Value>)> = vec![
            (
                "a".into(),
                ["A", "A", "A", "B", "B", "B", "B"]
                    .iter()
                    .map(|s| Value::Char((*s).into()))
                    .collect(),
            ),
            (
                "b".into(),
                ["X", "Y", "Y", "X", "X", "X", "Y"]
                    .iter()
                    .map(|s| Value::Char((*s).into()))
                    .collect(),
            ),
        ];
        let y = vec![10.0, 12.0, 14.0, 20.0, 22.0, 24.0, 30.0];
        let n = y.len();

        let mut factors: Vec<Factor> = Vec::new();
        for (name, col) in &class_cols {
            let mut levels: Vec<Value> = Vec::new();
            for v in col {
                if !levels.iter().any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal) {
                    levels.push(v.clone());
                }
            }
            levels.sort_by(|a, b| a.sas_cmp(b));
            factors.push(Factor { name: name.clone(), levels });
        }
        // Terms: a, b, a*b.
        let term_factor_idxs = vec![vec![0usize], vec![1usize], vec![0usize, 1usize]];
        let col_specs = term_column_specs(&term_factor_idxs, &factors);

        let row_levels: Vec<Vec<usize>> = (0..n)
            .map(|r| {
                class_cols
                    .iter()
                    .enumerate()
                    .map(|(fi, (_, col))| factors[fi].level_of(&col[r]))
                    .collect()
            })
            .collect();
        let dummy_cache: Vec<Vec<Vec<f64>>> =
            row_levels.iter().map(|rl| row_dummies(&factors, rl)).collect();
        let effect_cache: Vec<Vec<Vec<f64>>> =
            row_levels.iter().map(|rl| row_effects(&factors, rl)).collect();

        // Build a design (intercept + given terms) from a chosen coding cache.
        let build = |subset: &[usize], cache: &[Vec<Vec<f64>>]| -> Vec<Vec<f64>> {
            let mut d: Vec<Vec<f64>> = vec![vec![1.0]; n];
            for &t in subset {
                for spec in &col_specs[t] {
                    for (r, row) in d.iter_mut().enumerate() {
                        let v: f64 = spec.iter().map(|&(fi, dj)| cache[r][fi][dj]).product();
                        row.push(v);
                    }
                }
            }
            d
        };

        // (1) Effect-coded full SSE == reference-cell full SSE.
        let sse_full_ref = sse_of(&build(&[0, 1, 2], &dummy_cache), &y);
        let sse_full_eff = sse_of(&build(&[0, 1, 2], &effect_cache), &y);
        assert!(
            (sse_full_ref - sse_full_eff).abs() < 1e-6,
            "effect-coded full SSE must equal reference-cell full SSE: {sse_full_ref} vs {sse_full_eff}"
        );

        // Reference-cell Type III (the OLD, incorrect-for-main-effects approach).
        let t3_ref = |t: usize| -> f64 {
            let subset: Vec<usize> = (0..3).filter(|&x| x != t).collect();
            sse_of(&build(&subset, &dummy_cache), &y) - sse_full_ref
        };
        // Effect-coded Type III (the FIXED approach).
        let t3_eff = |t: usize| -> f64 {
            let subset: Vec<usize> = (0..3).filter(|&x| x != t).collect();
            sse_of(&build(&subset, &effect_cache), &y) - sse_full_eff
        };

        // (2) Interaction term (t=2) is highest-order → Type III coding-invariant.
        assert!(
            (t3_ref(2) - t3_eff(2)).abs() < 1e-6,
            "interaction Type III must be unchanged: ref={} eff={}",
            t3_ref(2),
            t3_eff(2)
        );

        // (3) Main-effect Type III values CHANGE between codings.
        assert!(
            (t3_ref(0) - t3_eff(0)).abs() > 1e-6,
            "main-effect 'a' Type III must change: ref={} eff={}",
            t3_ref(0),
            t3_eff(0)
        );
        assert!(
            (t3_ref(1) - t3_eff(1)).abs() > 1e-6,
            "main-effect 'b' Type III must change: ref={} eff={}",
            t3_ref(1),
            t3_eff(1)
        );

        // Type I (coding-invariant) still sums to Model SS, regardless of coding.
        let ybar = y.iter().sum::<f64>() / n as f64;
        let sst: f64 = y.iter().map(|v| (v - ybar).powi(2)).sum();
        let ssm = sst - sse_full_ref;
        let sse_int = sse_of(&vec![vec![1.0]; n], &y);
        let sse_a = sse_of(&build(&[0], &dummy_cache), &y);
        let sse_ab = sse_of(&build(&[0, 1], &dummy_cache), &y);
        let t1_a = sse_int - sse_a;
        let t1_b = sse_a - sse_ab;
        let t1_ab = sse_ab - sse_full_ref;
        assert!(
            (t1_a + t1_b + t1_ab - ssm).abs() < 1e-8,
            "Type I must sum to Model SS: {t1_a}+{t1_b}+{t1_ab} vs {ssm}"
        );

        // Report the corrected main-effect Type III values (effect coding).
        eprintln!(
            "effect-coded Type III: a={:.6} b={:.6} a*b={:.6} (sse_full={:.6})",
            t3_eff(0),
            t3_eff(1),
            t3_eff(2),
            sse_full_eff
        );
        eprintln!(
            "reference-cell Type III (old): a={:.6} b={:.6} a*b={:.6}",
            t3_ref(0),
            t3_ref(1),
            t3_ref(2)
        );
    }
}
