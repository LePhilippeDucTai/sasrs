//! PROC ANOVA — One-way ANOVA for balanced designs (M25.2).
//!
//! Implements CLASS statement, MODEL statement (multiple dependents, one CLASS
//! effect), MEANS statement. Produces Class Level Information, ANOVA table,
//! fit statistics, Type I SS, Type III SS, and optional MEANS table.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::{decode_column, sample_std};
use crate::session::Session;
use crate::stat::f_cdf;
use crate::token::TokenKind;
use crate::value::{Value, VarType};

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct AnovaAst {
    pub data_options: AnovaDataOptions,
    pub class_vars: Vec<String>,
    pub model: Option<AnovaModel>,
    pub means_vars: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AnovaDataOptions {
    pub input: Option<DatasetRef>,
}

#[derive(Debug, Clone)]
pub struct AnovaModel {
    pub dependents: Vec<String>,
    /// Raw effect tokens as written (e.g. `["a", "b", "a*b"]`). The one-way path
    /// only ever inspects `effects[0]`.
    pub effects: Vec<String>,
    /// Structured effect terms: each term is the list of CLASS var names it
    /// references. A main effect is a 1-element vec; `a*b` is `["a","b"]`.
    pub terms: Vec<Vec<String>>,
    pub noprint: bool,
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC ANOVA. Called AFTER `proc anova` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<AnovaAst> {
    let mut input: Option<DatasetRef> = None;

    // PROC ANOVA statement options until `;`
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
            // Skip unknown proc-level options
            ts.next();
        }
    }

    // Sub-statements until run;/quit;
    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<AnovaModel> = None;
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
            // Read effects: idents after `=` until `/` or `;`. Idents joined by
            // `*` form a single interaction term (e.g. `a*b`).
            let mut effects: Vec<String> = Vec::new();
            let mut terms: Vec<Vec<String>> = Vec::new();
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
                        if ts.peek().is_kw("noprint") {
                            noprint = true;
                        }
                        ts.next();
                    }
                    break;
                }
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    ts.next();
                    // Collect the rest of an interaction chain `name*x*y...`.
                    let mut parts = vec![name];
                    while ts.peek().kind == TokenKind::Star {
                        ts.next();
                        if let Some(next_name) = ts.peek().ident().map(str::to_string) {
                            parts.push(next_name);
                            ts.next();
                        } else {
                            break;
                        }
                    }
                    effects.push(parts.join("*"));
                    terms.push(parts);
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            model = Some(AnovaModel {
                dependents,
                effects,
                terms,
                noprint,
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

    Ok(AnovaAst {
        data_options: AnovaDataOptions { input },
        class_vars,
        model,
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

pub fn execute(ast: &AnovaAst, session: &mut Session) -> Result<()> {
    // Guard: MODEL required
    let model = match &ast.model {
        Some(m) => m,
        None => {
            session.log.note("No MODEL statement found in PROC ANOVA.");
            return Ok(());
        }
    };

    // Pre-check: at least one effect and at least one class var
    if model.effects.is_empty() || ast.class_vars.is_empty() {
        return Err(SasError::runtime(
            "MODEL statement requires at least one CLASS effect.",
        ));
    }

    // Validate every effect term references only declared CLASS variables.
    for term in &model.terms {
        for part in term {
            let is_class = ast
                .class_vars
                .iter()
                .any(|c| c.eq_ignore_ascii_case(part));
            if !is_class {
                return Err(SasError::runtime(format!(
                    "Variable {} not found in CLASS list.",
                    part.to_uppercase()
                )));
            }
        }
    }

    // Decide one-way vs multi-way. The one-way path (byte-identical to the
    // existing snapshot) is taken ONLY when the model is a single main effect
    // referencing exactly one CLASS variable and no interaction is present.
    let distinct_class_used: std::collections::BTreeSet<String> = model
        .terms
        .iter()
        .flatten()
        .map(|s| s.to_uppercase())
        .collect();
    let is_multiway = model.terms.len() > 1
        || model.terms.iter().any(|t| t.len() > 1)
        || distinct_class_used.len() > 1;
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
    centered(session, "The ANOVA Procedure");
    session.listing.blank();

    // --- 4. Class Level Information (once, before per-dep loop) ---
    centered(session, "Class Level Information");
    session.listing.blank();

    let cli_headers: Vec<String> = vec!["Class".into(), "Levels".into(), "Values".into()];
    let cli_aligns = vec![Align::Left, Align::Right, Align::Left];
    let mut cli_rows: Vec<Vec<String>> = Vec::new();

    // Decode CLASS columns and collect distinct values
    let mut class_col_data: Vec<(String, Vec<Value>)> = Vec::new();
    for class_var in &ast.class_vars {
        let col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(class_var))
            .unwrap(); // already validated above
        let col = decode_column(&ds, col_idx)?;

        // Collect distinct non-missing values, sorted by sas_cmp
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

    // Number of Observations
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

        // For one-way ANOVA, use the first effect as the CLASS grouping variable
        let eff = &model.effects[0];

        // Find the CLASS column for this effect
        let class_col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(eff))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", eff.to_uppercase()))
            })?;
        let class_col = decode_column(&ds, class_col_idx)?;

        // Listwise deletion: keep rows where both dep_var and class_var are non-missing
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

        // Group by CLASS levels
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

        session.log.note(&format!(
            "There were {} observations used.",
            n
        ));

        // Listing — Dependent Variable header
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
                f_str,
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

        // MEANS section — only if means_vars contains `eff`
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
                let level_label = match level {
                    Value::Char(s) => s.trim_end().to_string(),
                    Value::Num(f) => format!("{f}"),
                    Value::Missing(k) => k.display(),
                };
                let n_i = groups[gi].len();
                let mean_i = if n_i > 0 {
                    groups[gi].iter().sum::<f64>() / n_i as f64
                } else {
                    f64::NAN
                };
                let std_i = sample_std(&groups[gi]);
                means_rows.push(vec![
                    level_label,
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

// ───────────────────────── Multi-way engine ─────────────────────────

/// Build a reference-cell (last-level-dropped) design matrix and run the model.
/// Returns the SSE of the ordinary least-squares fit `min ‖Xβ − y‖²`.
fn fit_sse(x: &[Vec<f64>], y: &[f64]) -> f64 {
    if x.is_empty() || x[0].is_empty() {
        // Intercept-free / empty model: SSE around 0 is Σ y².
        return y.iter().map(|&v| v * v).sum();
    }
    let beta = match crate::stat::linalg::least_squares(x, y) {
        Ok(b) => b,
        Err(_) => return f64::NAN,
    };
    let mut sse = 0.0;
    for (i, row) in x.iter().enumerate() {
        let yhat: f64 = row.iter().zip(&beta).map(|(&xij, &bj)| xij * bj).sum();
        let r = y[i] - yhat;
        sse += r * r;
    }
    sse
}

/// Compute the per-level dummy columns (reference-cell, last level dropped) for
/// one CLASS variable. `codes[i]` is the level index of observation i.
/// Returns a Vec of columns, one per non-reference level (L−1 columns).
fn main_effect_dummies(codes: &[usize], n_levels: usize, n: usize) -> Vec<Vec<f64>> {
    let mut cols: Vec<Vec<f64>> = Vec::new();
    // Drop the LAST level as the reference cell.
    for lvl in 0..n_levels.saturating_sub(1) {
        let mut col = vec![0.0; n];
        for i in 0..n {
            if codes[i] == lvl {
                col[i] = 1.0;
            }
        }
        cols.push(col);
    }
    cols
}

/// Compute the sum-to-zero (effect / deviation) coded columns for one CLASS
/// variable. `codes[i]` is the level index of observation i. Levels are ordered
/// 1..L by sas_cmp; column j (j=0..L−2) is +1 at level j, −1 at the LAST level
/// L−1, else 0. Returns L−1 columns. Building interaction terms from elementwise
/// products of these centered contrasts yields the SAS Type III estimable
/// function, so the partial SS matches SAS Type III even on unbalanced data.
fn main_effect_effect_coded(codes: &[usize], n_levels: usize, n: usize) -> Vec<Vec<f64>> {
    let mut cols: Vec<Vec<f64>> = Vec::new();
    let last = n_levels.saturating_sub(1);
    for lvl in 0..n_levels.saturating_sub(1) {
        let mut col = vec![0.0; n];
        for i in 0..n {
            if codes[i] == lvl {
                col[i] = 1.0;
            } else if codes[i] == last {
                col[i] = -1.0;
            }
        }
        cols.push(col);
    }
    cols
}

/// General multi-way ANOVA (interactions, multiple CLASS vars), reference-cell
/// coding with Type I (sequential) and Type III (partial) sums of squares.
fn execute_multiway(ast: &AnovaAst, model: &AnovaModel, session: &mut Session) -> Result<()> {
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

    // --- 2. Validate CLASS vars exist ---
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

    // Decode all CLASS columns once, keyed by uppercase name.
    let mut class_cols: std::collections::HashMap<String, (String, Vec<Value>)> =
        std::collections::HashMap::new();
    for class_var in &ast.class_vars {
        let col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(class_var))
            .unwrap();
        let col = decode_column(&ds, col_idx)?;
        class_cols.insert(
            class_var.to_uppercase(),
            (ds.vars[col_idx].name.clone(), col),
        );
    }

    // --- 3. Listing header ---
    session.listing.page_header();
    centered(session, "The ANOVA Procedure");
    session.listing.blank();

    // --- 4. Class Level Information ---
    centered(session, "Class Level Information");
    session.listing.blank();

    let cli_headers: Vec<String> = vec!["Class".into(), "Levels".into(), "Values".into()];
    let cli_aligns = vec![Align::Left, Align::Right, Align::Left];
    let mut cli_rows: Vec<Vec<String>> = Vec::new();

    for class_var in &ast.class_vars {
        let (disp_name, col) = &class_cols[&class_var.to_uppercase()];
        let mut levels: Vec<Value> = Vec::new();
        for v in col.iter().take(n_obs) {
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
        let values_str: Vec<String> = levels.iter().map(value_label).collect();
        cli_rows.push(vec![
            disp_name.clone(),
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

    // Distinct CLASS vars referenced by the model, uppercased.
    let used_classes: Vec<String> = {
        let mut seen: Vec<String> = Vec::new();
        for part in model.terms.iter().flatten() {
            let up = part.to_uppercase();
            if !seen.contains(&up) {
                seen.push(up);
            }
        }
        seen
    };

    // --- 5. Per-dependent loop ---
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

        // Listwise deletion: drop rows where dep or ANY used CLASS var missing.
        let mut usable: Vec<usize> = Vec::new();
        for i in 0..n_obs {
            let dep_ok = matches!(value_to_num(&dep_col[i]), Some(v) if !v.is_nan());
            if !dep_ok {
                continue;
            }
            let cls_ok = used_classes
                .iter()
                .all(|c| !class_cols[c].1[i].is_missing());
            if cls_ok {
                usable.push(i);
            }
        }
        let n = usable.len();

        // y vector and corrected total.
        let y: Vec<f64> = usable.iter().map(|&r| value_to_num(&dep_col[r]).unwrap()).collect();
        let y_bar = if n > 0 { y.iter().sum::<f64>() / n as f64 } else { f64::NAN };
        let sst: f64 = y.iter().map(|&v| (v - y_bar).powi(2)).sum();

        // Levels per used CLASS var (sas_cmp order over usable rows).
        let mut var_levels: std::collections::HashMap<String, Vec<Value>> =
            std::collections::HashMap::new();
        for c in &used_classes {
            let col = &class_cols[c].1;
            let mut levels: Vec<Value> = Vec::new();
            for &r in &usable {
                let v = &col[r];
                if !levels.iter().any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal) {
                    levels.push(v.clone());
                }
            }
            levels.sort_by(|a, b| a.sas_cmp(b));
            var_levels.insert(c.clone(), levels);
        }

        // Per-CLASS-var level codes for each usable observation.
        let mut var_codes: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for c in &used_classes {
            let col = &class_cols[c].1;
            let levels = &var_levels[c];
            let codes: Vec<usize> = usable
                .iter()
                .map(|&r| {
                    levels
                        .iter()
                        .position(|l| l.sas_cmp(&col[r]) == std::cmp::Ordering::Equal)
                        .unwrap()
                })
                .collect();
            var_codes.insert(c.clone(), codes);
        }

        // Per-CLASS-var main-effect dummy columns (reference-cell), used for the
        // Type I sequential pass.
        let mut var_dummies: std::collections::HashMap<String, Vec<Vec<f64>>> =
            std::collections::HashMap::new();
        // Per-CLASS-var sum-to-zero (effect) coded columns, used for Type III.
        let mut var_effect: std::collections::HashMap<String, Vec<Vec<f64>>> =
            std::collections::HashMap::new();
        for c in &used_classes {
            let l = var_levels[c].len();
            var_dummies.insert(c.clone(), main_effect_dummies(&var_codes[c], l, n));
            var_effect.insert(c.clone(), main_effect_effect_coded(&var_codes[c], l, n));
        }

        // Build the column block for each model term from a per-var coding map.
        // A main effect contributes its columns; an interaction term contributes
        // the elementwise products (Cartesian) across its parents' columns.
        let build_term_blocks =
            |coding: &std::collections::HashMap<String, Vec<Vec<f64>>>| -> Vec<Vec<Vec<f64>>> {
                let mut blocks: Vec<Vec<Vec<f64>>> = Vec::new();
                for term in &model.terms {
                    let parents: Vec<&Vec<Vec<f64>>> =
                        term.iter().map(|p| &coding[&p.to_uppercase()]).collect();
                    let mut block: Vec<Vec<f64>> = vec![vec![1.0; n]];
                    for parent in &parents {
                        let mut next: Vec<Vec<f64>> = Vec::new();
                        for existing in &block {
                            for pcol in parent.iter() {
                                let prod: Vec<f64> =
                                    existing.iter().zip(pcol).map(|(&a, &b)| a * b).collect();
                                next.push(prod);
                            }
                        }
                        block = next;
                    }
                    blocks.push(block);
                }
                blocks
            };

        // Reference-cell term blocks (Type I + Model SS) and effect-coded term
        // blocks (Type III). Both span the same column space, so SSE_full is
        // identical between them.
        let term_blocks = build_term_blocks(&var_dummies);
        let term_blocks_eff = build_term_blocks(&var_effect);

        // Per-term DF = product of (levels − 1).
        let term_dfs: Vec<usize> = model
            .terms
            .iter()
            .map(|term| {
                term.iter()
                    .map(|p| var_levels[&p.to_uppercase()].len().saturating_sub(1))
                    .product()
            })
            .collect();

        // Assemble a design matrix from a set of term blocks: intercept + the
        // included term blocks.
        let build_from = |blocks: &[Vec<Vec<f64>>], include: &[bool]| -> Vec<Vec<f64>> {
            let mut x = vec![vec![1.0]; n]; // intercept column
            for (t, block) in blocks.iter().enumerate() {
                if include[t] {
                    for (i, row) in x.iter_mut().enumerate() {
                        for col in block {
                            row.push(col[i]);
                        }
                    }
                }
            }
            x
        };
        let build_design = |include: &[bool]| -> Vec<Vec<f64>> {
            build_from(&term_blocks, include)
        };

        let n_terms = model.terms.len();
        let all_true = vec![true; n_terms];
        let full_x = build_design(&all_true);
        let full_cols = full_x[0].len();
        let sse_full = fit_sse(&full_x, &y);
        let ssm = sst - sse_full;
        let df_model = (full_cols - 1) as f64;
        let df_error = (n as f64 - full_cols as f64).max(0.0);
        let df_total = (n as f64 - 1.0).max(0.0);
        let msm = if df_model > 0.0 { ssm / df_model } else { f64::NAN };
        let mse = if df_error > 0.0 { sse_full / df_error } else { f64::NAN };
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

        // Type I (sequential): SS_t = SSE(0..t) − SSE(0..=t).
        let mut type1: Vec<f64> = Vec::with_capacity(n_terms);
        {
            let mut prev_sse = sst; // model with intercept only
            for t in 0..n_terms {
                let mut include = vec![false; n_terms];
                for inc in include.iter_mut().take(t + 1) {
                    *inc = true;
                }
                let x = build_design(&include);
                let sse = fit_sse(&x, &y);
                type1.push(prev_sse - sse);
                prev_sse = sse;
            }
        }

        // Type III (partial), computed with sum-to-zero EFFECT coding so the
        // partial SS matches SAS Type III on unbalanced data. The effect-coded
        // full model spans the same column space as the reference-cell full
        // model, so its SSE equals `sse_full` (asserted in debug builds).
        let full_x_eff = build_from(&term_blocks_eff, &all_true);
        let sse_full_eff = fit_sse(&full_x_eff, &y);
        debug_assert!(
            (sse_full_eff - sse_full).abs() <= 1e-6 * (1.0 + sse_full.abs()),
            "effect-coded SSE_full {sse_full_eff} != reference-cell SSE_full {sse_full}"
        );
        let mut type3: Vec<f64> = Vec::with_capacity(n_terms);
        for t in 0..n_terms {
            let mut include = vec![true; n_terms];
            include[t] = false;
            let x = build_from(&term_blocks_eff, &include);
            let sse = fit_sse(&x, &y);
            type3.push(sse - sse_full_eff);
        }

        session.log.note(&format!("There were {} observations used.", n));

        // --- Listing: Dependent Variable header ---
        centered(session, &format!("Dependent Variable: {}", dep_var));
        session.listing.blank();

        // ANOVA table (Model / Error / Corrected Total).
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
        let f_str = if f_model.is_nan() { ".".to_string() } else { fmt2(f_model) };
        let anova_rows: Vec<Vec<String>> = vec![
            vec![
                "Model".into(),
                format!("{}", df_model as usize),
                fmt5(ssm),
                if msm.is_nan() { ".".into() } else { fmt5(msm) },
                f_str,
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

        // Fit statistics.
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

        // Term labels with `*` join.
        let term_labels: Vec<String> = model.terms.iter().map(|t| t.join("*")).collect();

        // Type I SS and Type III SS tables, one row per term.
        for (ss_label, ss_vals) in
            [("Type I SS", &type1), ("Type III SS", &type3)]
        {
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
            for t in 0..n_terms {
                let df = term_dfs[t] as f64;
                let ss = ss_vals[t];
                let ms = if df > 0.0 { ss / df } else { f64::NAN };
                let f = if mse > 0.0 && !mse.is_nan() && !ms.is_nan() {
                    ms / mse
                } else {
                    f64::NAN
                };
                let p = if f.is_nan() || df <= 0.0 || df_error <= 0.0 {
                    None
                } else {
                    Some((1.0 - f_cdf(f, df, df_error)).clamp(0.0, 1.0))
                };
                t_rows.push(vec![
                    term_labels[t].clone(),
                    format!("{}", term_dfs[t]),
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

        // MEANS: main-effect marginal cell means for each requested CLASS var.
        for mvar in &ast.means_vars {
            let up = mvar.to_uppercase();
            if !used_classes.contains(&up) {
                continue;
            }
            let levels = &var_levels[&up];
            let codes = &var_codes[&up];
            let disp = &class_cols[&up].0;

            centered(session, &format!("Level of {}", disp));
            session.listing.blank();

            let means_headers: Vec<String> = vec![
                disp.clone(),
                "N".into(),
                "Mean".into(),
                "Std Dev".into(),
            ];
            let means_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
            let mut means_rows: Vec<Vec<String>> = Vec::new();
            for (li, level) in levels.iter().enumerate() {
                let vals: Vec<f64> = (0..n)
                    .filter(|&i| codes[i] == li)
                    .map(|i| y[i])
                    .collect();
                let ni = vals.len();
                let mean_i = if ni > 0 { vals.iter().sum::<f64>() / ni as f64 } else { f64::NAN };
                let std_i = sample_std(&vals);
                means_rows.push(vec![
                    value_label(level),
                    format!("{}", ni),
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

/// Render a CLASS level value the way the existing one-way path does.
fn value_label(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim_end().to_string(),
        Value::Num(f) => format!("{f}"),
        Value::Missing(k) => k.display(),
    }
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

    fn parse_anova(src: &str) -> Result<AnovaAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // anova
        parse(&mut ts)
    }

    // ── Test 1: one-way ANOVA arithmetic ──────────────────────────────────

    #[test]
    fn test_one_way_anova_simple() {
        // y=[1,2,3,10,11,12], groups=["A","A","A","B","B","B"]
        // k=2, n=6, ȳ_A=2, ȳ_B=11, ȳ=6.5
        // SSModel = 3*(2-6.5)² + 3*(11-6.5)² = 60.75 + 60.75 = 121.5
        // SSE = (1-2)²+(2-2)²+(3-2)² + (10-11)²+(11-11)²+(12-11)² = 2 + 2 = 4
        // df_model=1, df_error=4, MSModel=121.5, MSE=1.0
        // F = 121.5

        let a_group: Vec<f64> = vec![1.0, 2.0, 3.0];
        let b_group: Vec<f64> = vec![10.0, 11.0, 12.0];
        let all_vals: Vec<f64> = a_group.iter().chain(b_group.iter()).cloned().collect();
        let n = 6usize;
        let k = 2usize;

        let y_bar = all_vals.iter().sum::<f64>() / n as f64;
        assert!((y_bar - 6.5).abs() < 1e-10, "y_bar={y_bar}");

        let y_bar_a = 2.0_f64;
        let y_bar_b = 11.0_f64;
        let ssm = 3.0 * (y_bar_a - y_bar).powi(2) + 3.0 * (y_bar_b - y_bar).powi(2);
        assert!((ssm - 121.5).abs() < 1e-9, "ssm={ssm}");

        let sse_a: f64 = a_group.iter().map(|&y| (y - y_bar_a).powi(2)).sum();
        let sse_b: f64 = b_group.iter().map(|&y| (y - y_bar_b).powi(2)).sum();
        let sse = sse_a + sse_b;
        assert!((sse - 4.0).abs() < 1e-9, "sse={sse}");

        let df_model = (k - 1) as f64;
        let df_error = (n - k) as f64;
        let msm = ssm / df_model;
        let mse = sse / df_error;
        let f_stat = msm / mse;

        assert!((f_stat - 121.5).abs() < 1e-9, "F={f_stat}");

        let p = (1.0 - f_cdf(f_stat, df_model, df_error)).clamp(0.0, 1.0);
        assert!(f_stat > 100.0, "F should be > 100, got {f_stat}");
        assert!(p < 0.001, "p should be very small, got {p}");
    }

    // ── Test 2: parse model with multiple dependents ───────────────────────

    #[test]
    fn test_parse_model_multi_dep() {
        let ast = parse_anova(
            "proc anova; class sex; model height weight = sex; means sex; run;",
        )
        .unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.dependents, vec!["height", "weight"]);
        assert_eq!(m.effects, vec!["sex"]);
        assert_eq!(ast.means_vars, vec!["sex"]);
    }

    // ── Test 3: parse class with multiple vars ────────────────────────────

    #[test]
    fn test_parse_class() {
        let ast = parse_anova("proc anova data=x; class a b; model y = a; run;").unwrap();
        assert_eq!(ast.class_vars, vec!["a", "b"]);
    }

    // ── Test 4: execute listing checks ───────────────────────────────────

    #[test]
    fn test_execute_listing() {
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

        let ast = AnovaAst {
            data_options: AnovaDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "T".into(),
                }),
            },
            class_vars: vec!["sex".into()],
            model: Some(AnovaModel {
                dependents: vec!["height".into()],
                effects: vec!["sex".into()],
                terms: vec![vec!["sex".into()]],
                noprint: false,
            }),
            means_vars: vec![],
        };

        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();

        assert!(listing.contains("The ANOVA Procedure"), "{listing}");
        assert!(listing.contains("Class Level Information"), "{listing}");
        assert!(listing.contains("Dependent Variable"), "{listing}");
        assert!(listing.contains("Corrected Total"), "{listing}");
        assert!(listing.contains("Type I SS"), "{listing}");
        assert!(listing.contains("Type III SS"), "{listing}");
    }

    // ── Test 5: execute means section ────────────────────────────────────

    #[test]
    fn test_execute_means() {
        let mut session = make_session();
        let frame = df![
            "sex"    => ["F","F","F","M","M","M"],
            "weight" => [100.0_f64, 110.0, 120.0, 150.0, 160.0, 170.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![char_meta("sex"), num_meta("weight")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let ast = AnovaAst {
            data_options: AnovaDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "T".into(),
                }),
            },
            class_vars: vec!["sex".into()],
            model: Some(AnovaModel {
                dependents: vec!["weight".into()],
                effects: vec!["sex".into()],
                terms: vec![vec!["sex".into()]],
                noprint: false,
            }),
            means_vars: vec!["sex".into()],
        };

        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();

        // Should show class level in the means table
        assert!(listing.contains("Level of sex") || listing.contains("sex"), "{listing}");
        assert!(listing.contains('F'), "{listing}");
        assert!(listing.contains('M'), "{listing}");
    }

    // ── Test 6: effect-term parsing of `a b a*b` ──────────────────────────

    #[test]
    fn test_parse_interaction_terms() {
        let ast = parse_anova(
            "proc anova; class a b; model y = a b a*b; run;",
        )
        .unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.effects, vec!["a", "b", "a*b"]);
        assert_eq!(
            m.terms,
            vec![
                vec!["a".to_string()],
                vec!["b".to_string()],
                vec!["a".to_string(), "b".to_string()],
            ]
        );
    }

    #[test]
    fn test_parse_three_way_interaction() {
        let ast = parse_anova("proc anova; class a b c; model y = a*b*c; run;").unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.effects, vec!["a*b*c"]);
        assert_eq!(m.terms, vec![vec!["a".to_string(), "b".to_string(), "c".to_string()]]);
    }

    // ── Test 7: two-way design-matrix dimension check ─────────────────────

    #[test]
    fn test_two_way_design_dims() {
        // a has 2 levels, b has 3 levels, n observations.
        // Main effect a -> 1 dummy col; b -> 2 dummy cols; a*b -> 1*2 = 2 cols.
        // Full design = intercept(1) + 1 + 2 + 2 = 6 columns.
        let n = 6;
        let a_codes = vec![0usize, 0, 0, 1, 1, 1];
        let b_codes = vec![0usize, 1, 2, 0, 1, 2];
        let a_d = main_effect_dummies(&a_codes, 2, n);
        let b_d = main_effect_dummies(&b_codes, 3, n);
        assert_eq!(a_d.len(), 1, "a should give 1 dummy col");
        assert_eq!(b_d.len(), 2, "b should give 2 dummy cols");

        // Interaction columns: elementwise products of a's dummies × b's dummies.
        let mut inter: Vec<Vec<f64>> = Vec::new();
        for ac in &a_d {
            for bc in &b_d {
                inter.push(ac.iter().zip(bc).map(|(&x, &y)| x * y).collect());
            }
        }
        assert_eq!(inter.len(), 2, "a*b should give 2 interaction cols");
        // Full design columns count.
        let full_cols = 1 + a_d.len() + b_d.len() + inter.len();
        assert_eq!(full_cols, 6);
    }

    // ── Test 8: balanced two-way SS — Type I == Type III ──────────────────

    /// Build the reference-cell design (intercept + main effects a,b +
    /// optional a*b) for the test, returning the column-major matrix as rows.
    fn build_two_way(
        a_codes: &[usize],
        a_lvls: usize,
        b_codes: &[usize],
        b_lvls: usize,
        include_inter: bool,
    ) -> Vec<Vec<f64>> {
        let n = a_codes.len();
        let a_d = main_effect_dummies(a_codes, a_lvls, n);
        let b_d = main_effect_dummies(b_codes, b_lvls, n);
        let mut rows = vec![vec![1.0]; n];
        for (i, row) in rows.iter_mut().enumerate() {
            for c in &a_d {
                row.push(c[i]);
            }
            for c in &b_d {
                row.push(c[i]);
            }
            if include_inter {
                for ac in &a_d {
                    for bc in &b_d {
                        row.push(ac[i] * bc[i]);
                    }
                }
            }
        }
        rows
    }

    /// Sequential (Type I, reference-cell) and partial (Type III, effect-coded)
    /// SS for a 2-term model (a, b) (no interaction), using the engine's
    /// `fit_sse`. Returns `(t1_a, t1_b, t3_a, t3_b, sst, sse_full)`.
    fn two_way_ss(
        a_codes: &[usize],
        a_lvls: usize,
        b_codes: &[usize],
        b_lvls: usize,
        y: &[f64],
    ) -> (f64, f64, f64, f64, f64, f64) {
        let n = y.len();
        let y_bar = y.iter().sum::<f64>() / n as f64;
        let sst: f64 = y.iter().map(|&v| (v - y_bar).powi(2)).sum();

        let only_a = {
            let a_d = main_effect_dummies(a_codes, a_lvls, n);
            let mut rows = vec![vec![1.0]; n];
            for (i, row) in rows.iter_mut().enumerate() {
                for c in &a_d {
                    row.push(c[i]);
                }
            }
            rows
        };
        let a_then_b = build_two_way(a_codes, a_lvls, b_codes, b_lvls, false);
        let full = a_then_b.clone();
        let sse_full = fit_sse(&full, y);

        // Type I (reference-cell): a = SST - SSE(a); b = SSE(a) - SSE(a,b).
        let sse_a = fit_sse(&only_a, y);
        let t1_a = sst - sse_a;
        let t1_b = sse_a - sse_full;

        // Type III with sum-to-zero EFFECT coding. Build the effect-coded full
        // model and the two reduced models (intercept + the OTHER main effect).
        let a_e = main_effect_effect_coded(a_codes, a_lvls, n);
        let b_e = main_effect_effect_coded(b_codes, b_lvls, n);
        let full_eff = {
            let mut rows = vec![vec![1.0]; n];
            for (i, row) in rows.iter_mut().enumerate() {
                for c in &a_e {
                    row.push(c[i]);
                }
                for c in &b_e {
                    row.push(c[i]);
                }
            }
            rows
        };
        let sse_full_eff = fit_sse(&full_eff, y);
        // Same column space as reference-cell full -> identical fit.
        assert!(
            (sse_full_eff - sse_full).abs() < 1e-6,
            "effect-coded SSE_full {sse_full_eff} != reference-cell {sse_full}"
        );
        let intercept_plus = |cols: &[Vec<f64>]| -> Vec<Vec<f64>> {
            let mut rows = vec![vec![1.0]; n];
            for (i, row) in rows.iter_mut().enumerate() {
                for c in cols {
                    row.push(c[i]);
                }
            }
            rows
        };
        let sse_minus_a = fit_sse(&intercept_plus(&b_e), y); // full minus a
        let sse_minus_b = fit_sse(&intercept_plus(&a_e), y); // full minus b
        let t3_a = sse_minus_a - sse_full_eff;
        let t3_b = sse_minus_b - sse_full_eff;

        (t1_a, t1_b, t3_a, t3_b, sst, sse_full)
    }

    #[test]
    fn test_two_way_balanced_type1_eq_type3() {
        // Balanced 2x2 with 2 reps per cell (n=8). Type I == Type III.
        // a: 0,0,0,0,1,1,1,1   b: 0,0,1,1,0,0,1,1
        let a = vec![0usize, 0, 0, 0, 1, 1, 1, 1];
        let b = vec![0usize, 0, 1, 1, 0, 0, 1, 1];
        let y = vec![10.0, 12.0, 20.0, 22.0, 30.0, 28.0, 44.0, 46.0];
        let (t1a, t1b, t3a, t3b, sst, sse_full) = two_way_ss(&a, 2, &b, 2, &y);

        assert!((t1a - t3a).abs() < 1e-7, "balanced: a t1={t1a} t3={t3a}");
        assert!((t1b - t3b).abs() < 1e-7, "balanced: b t1={t1b} t3={t3b}");
        // Type I should sum (with interaction omitted) to the model SS of the
        // two-main-effect model: SSM = SST - SSE_full.
        let ssm = sst - sse_full;
        assert!((t1a + t1b - ssm).abs() < 1e-7, "t1 sum {} != ssm {}", t1a + t1b, ssm);
    }

    #[test]
    fn test_two_way_unbalanced_type1_ne_type3() {
        // UNBALANCED 2x2: cell counts differ so Type I != Type III for at least
        // one term, and the F/p use the fitted MSE.
        // a: 0,0,0,1,1   b: 0,1,1,0,1   (cell (0,0):1, (0,1):2, (1,0):1, (1,1):1)
        let a = vec![0usize, 0, 0, 1, 1];
        let b = vec![0usize, 1, 1, 0, 1];
        let y = vec![5.0, 9.0, 11.0, 20.0, 30.0];
        let (t1a, t1b, t3a, t3b, sst, sse_full) = two_way_ss(&a, 2, &b, 2, &y);

        // Type I of the LAST term equals its Type III (last sequential == partial
        // when it is the final term), but the FIRST term differs.
        assert!(
            (t1a - t3a).abs() > 1e-6,
            "unbalanced: expected a's TypeI({t1a}) != TypeIII({t3a})"
        );
        // Type I sums to model SS (2-main-effect model).
        let ssm = sst - sse_full;
        assert!(
            (t1a + t1b - ssm).abs() < 1e-7,
            "t1 sum {} != ssm {}",
            t1a + t1b,
            ssm
        );

        // F/p for term b against fitted MSE (df_error = n - cols = 5 - 3 = 2).
        let n = y.len();
        let full = build_two_way(&a, 2, &b, 2, false);
        let cols = full[0].len();
        let df_error = (n - cols) as f64;
        let mse = sse_full / df_error;
        let df_b = 1.0; // b has 2 levels -> 1 df
        let ms_b = t3b / df_b;
        let f_b = ms_b / mse;
        let p_b = (1.0 - f_cdf(f_b, df_b, df_error)).clamp(0.0, 1.0);
        assert!(f_b > 0.0 && f_b.is_finite(), "F_b={f_b}");
        assert!((0.0..=1.0).contains(&p_b), "p_b={p_b}");
    }

    /// Effect-coded (sum-to-zero) Type III SS for a full 3-term 2x2 model
    /// (a, b, a*b), plus the reference-cell SSE_full for the same model, using
    /// the engine's coding builders. Returns
    /// `(t3_a, t3_b, t3_ab, sse_full_ref, sse_full_eff)`.
    fn two_way_full_type3_effect(
        a_codes: &[usize],
        a_lvls: usize,
        b_codes: &[usize],
        b_lvls: usize,
        y: &[f64],
    ) -> (f64, f64, f64, f64, f64) {
        let n = y.len();
        let a_d = main_effect_dummies(a_codes, a_lvls, n);
        let b_d = main_effect_dummies(b_codes, b_lvls, n);
        let a_e = main_effect_effect_coded(a_codes, a_lvls, n);
        let b_e = main_effect_effect_coded(b_codes, b_lvls, n);

        // Build a design from a list of main-effect column groups, where an
        // interaction group is the elementwise product of two main-effect groups.
        let assemble = |groups: &[&Vec<Vec<f64>>]| -> Vec<Vec<f64>> {
            let mut rows = vec![vec![1.0]; n];
            for g in groups {
                for (i, row) in rows.iter_mut().enumerate() {
                    for c in g.iter() {
                        row.push(c[i]);
                    }
                }
            }
            rows
        };
        let inter = |x: &[Vec<f64>], z: &[Vec<f64>]| -> Vec<Vec<f64>> {
            let mut out = Vec::new();
            for xc in x {
                for zc in z {
                    out.push(xc.iter().zip(zc).map(|(&p, &q)| p * q).collect());
                }
            }
            out
        };

        // Reference-cell full (a, b, a*b).
        let ab_d = inter(&a_d, &b_d);
        let full_ref = assemble(&[&a_d, &b_d, &ab_d]);
        let sse_full_ref = fit_sse(&full_ref, y);

        // Effect-coded full (a, b, a*b).
        let ab_e = inter(&a_e, &b_e);
        let full_eff = assemble(&[&a_e, &b_e, &ab_e]);
        let sse_full_eff = fit_sse(&full_eff, y);

        // Type III = SSE(full minus term) - SSE(full), effect-coded.
        let sse_minus_a = fit_sse(&assemble(&[&b_e, &ab_e]), y);
        let sse_minus_b = fit_sse(&assemble(&[&a_e, &ab_e]), y);
        let sse_minus_ab = fit_sse(&assemble(&[&a_e, &b_e]), y);
        let t3_a = sse_minus_a - sse_full_eff;
        let t3_b = sse_minus_b - sse_full_eff;
        let t3_ab = sse_minus_ab - sse_full_eff;

        (t3_a, t3_b, t3_ab, sse_full_ref, sse_full_eff)
    }

    #[test]
    fn test_unbalanced_2x2_type3_effect_coding() {
        // UNBALANCED 2x2 with interaction. With reference-cell coding the
        // main-effect Type III SS are coding-dependent and wrong; sum-to-zero
        // effect coding gives the SAS Type III values. The highest-order
        // (interaction) Type III is already correct and must be unchanged.
        let a = vec![0usize, 0, 0, 1, 1, 1, 1];
        let b = vec![0usize, 1, 1, 0, 0, 1, 1];
        let y = vec![5.0, 9.0, 11.0, 20.0, 22.0, 30.0, 34.0];

        let (t3_a, t3_b, t3_ab, sse_ref, sse_eff) =
            two_way_full_type3_effect(&a, 2, &b, 2, &y);

        // SSE_full invariance: same fit, different basis.
        assert!(
            (sse_ref - sse_eff).abs() < 1e-6,
            "SSE_full not invariant: ref={sse_ref} eff={sse_eff}"
        );

        // Reference-cell Type III for the interaction (highest-order) term: this
        // is coding-invariant and must equal the effect-coded value.
        let n = y.len();
        let a_d = main_effect_dummies(&a, 2, n);
        let b_d = main_effect_dummies(&b, 2, n);
        let assemble_ref = |inc_ab: bool| -> Vec<Vec<f64>> {
            let mut rows = vec![vec![1.0]; n];
            for (i, row) in rows.iter_mut().enumerate() {
                for c in &a_d {
                    row.push(c[i]);
                }
                for c in &b_d {
                    row.push(c[i]);
                }
                if inc_ab {
                    for ac in &a_d {
                        for bc in &b_d {
                            row.push(ac[i] * bc[i]);
                        }
                    }
                }
            }
            rows
        };
        let sse_full_ref = fit_sse(&assemble_ref(true), &y);
        let sse_no_ab_ref = fit_sse(&assemble_ref(false), &y);
        let t3_ab_ref = sse_no_ab_ref - sse_full_ref;
        assert!(
            (t3_ab - t3_ab_ref).abs() < 1e-6,
            "interaction Type III must be coding-invariant: eff={t3_ab} ref={t3_ab_ref}"
        );

        // Main-effect Type III values are finite, positive, and (the point of
        // the fix) differ from the wrong reference-cell partial values.
        assert!(t3_a > 0.0 && t3_a.is_finite(), "t3_a={t3_a}");
        assert!(t3_b > 0.0 && t3_b.is_finite(), "t3_b={t3_b}");

        // Reference-cell "full minus a's dummy cols" partial (the OLD, wrong way).
        let assemble_ref_minus = |skip_a: bool, skip_b: bool| -> Vec<Vec<f64>> {
            let mut rows = vec![vec![1.0]; n];
            for (i, row) in rows.iter_mut().enumerate() {
                if !skip_a {
                    for c in &a_d {
                        row.push(c[i]);
                    }
                }
                if !skip_b {
                    for c in &b_d {
                        row.push(c[i]);
                    }
                }
                for ac in &a_d {
                    for bc in &b_d {
                        row.push(ac[i] * bc[i]);
                    }
                }
            }
            rows
        };
        let t3_a_ref_old = fit_sse(&assemble_ref_minus(true, false), &y) - sse_full_ref;
        // The main-effect Type III genuinely changed with the fix.
        assert!(
            (t3_a - t3_a_ref_old).abs() > 1e-6,
            "main-effect Type III should change: eff={t3_a} old-ref={t3_a_ref_old}"
        );

        eprintln!("unbalanced 2x2 Type III: a={t3_a} b={t3_b} a*b={t3_ab}");
    }

    // ── Test 9: end-to-end multiway execute path ──────────────────────────

    #[test]
    fn test_execute_multiway_listing() {
        let mut session = make_session();
        // 2x2 design, two CLASS vars + interaction.
        let frame = df![
            "a" => ["L","L","L","L","H","H","H","H"],
            "b" => ["X","X","Y","Y","X","X","Y","Y"],
            "y" => [10.0_f64, 12.0, 20.0, 22.0, 30.0, 28.0, 44.0, 46.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![char_meta("a"), char_meta("b"), num_meta("y")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let ast = AnovaAst {
            data_options: AnovaDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "T".into(),
                }),
            },
            class_vars: vec!["a".into(), "b".into()],
            model: Some(AnovaModel {
                dependents: vec!["y".into()],
                effects: vec!["a".into(), "b".into(), "a*b".into()],
                terms: vec![
                    vec!["a".into()],
                    vec!["b".into()],
                    vec!["a".into(), "b".into()],
                ],
                noprint: false,
            }),
            means_vars: vec!["a".into()],
        };

        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();

        assert!(listing.contains("The ANOVA Procedure"), "{listing}");
        assert!(listing.contains("Class Level Information"), "{listing}");
        assert!(listing.contains("Dependent Variable: y"), "{listing}");
        assert!(listing.contains("Type I SS"), "{listing}");
        assert!(listing.contains("Type III SS"), "{listing}");
        // Interaction term label uses `*` join.
        assert!(listing.contains("a*b"), "{listing}");
        // MEANS main-effect table present.
        assert!(listing.contains("Level of a"), "{listing}");
    }
}
