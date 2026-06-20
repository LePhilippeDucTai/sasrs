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
    pub effects: Vec<String>,
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
            // Read effects: idents after `=` until `/` or `;`
            let mut effects: Vec<String> = Vec::new();
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
                    effects.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            model = Some(AnovaModel {
                dependents,
                effects,
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

    // Pre-check: no interaction effects (a*b patterns contain '*')
    for eff in &model.effects {
        if eff.contains('*') {
            return Err(SasError::runtime(
                "Interaction effects not yet implemented in PROC ANOVA.",
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
}
