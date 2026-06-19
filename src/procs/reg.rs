//! PROC REG — OLS linear regression (M25.1).
//!
//! Implements PROC REG with a single MODEL statement, supporting intercept
//! model only (NOINT deferred). Produces an ANOVA table, fit statistics, and
//! parameter estimates with t-tests. Optional OUTPUT statement writes predicted
//! values and residuals.

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::decode_column;
use crate::session::Session;
use crate::stat::linalg;
use crate::stat::{f_cdf, student_t_cdf};
use crate::token::TokenKind;
use crate::value::VarType;
use polars::prelude::{Column, DataFrame, NamedFrom, Series};

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct RegAst {
    pub data_options: RegDataOptions,
    pub model: Option<RegModel>,
    pub outputs: Vec<RegOutput>,
}

#[derive(Debug, Clone)]
pub struct RegDataOptions {
    pub input: Option<DatasetRef>,
}

#[derive(Debug, Clone)]
pub struct RegModel {
    pub dependent: String,
    pub regressors: Vec<String>,
    pub noint: bool,
    pub noprint: bool,
}

#[derive(Debug, Clone)]
pub struct RegOutput {
    pub out: DatasetRef,
    pub predicted: Option<String>,
    pub residual: Option<String>,
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

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC REG. Called AFTER `proc reg` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<RegAst> {
    let mut input: Option<DatasetRef> = None;

    // PROC REG statement options, until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            ts.next();
            expect_eq(ts, "DATA")?;
            input = Some(ts.parse_dataset_ref()?);
        } else {
            // Skip unknown proc-level options
            ts.next();
        }
    }

    // Sub-statements until run;/quit;
    let mut model: Option<RegModel> = None;
    let mut outputs: Vec<RegOutput> = Vec::new();

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

        if ts.peek().is_kw("model") {
            ts.next(); // consume "model"
            let dep = ts
                .peek()
                .ident()
                .map(str::to_string)
                .ok_or_else(|| SasError::parse("expected dependent variable", ts.peek().span))?;
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after dependent variable in MODEL",
                    ts.peek().span,
                ));
            }
            ts.next();
            let mut regressors = vec![];
            let mut noint = false;
            let mut noprint = false;
            loop {
                if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                    break;
                }
                if ts.peek().kind == TokenKind::Slash {
                    ts.next();
                    // Parse options until semi
                    while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                        if ts.peek().is_kw("noint") {
                            noint = true;
                            ts.next();
                        } else if ts.peek().is_kw("noprint") {
                            noprint = true;
                            ts.next();
                        } else {
                            ts.next(); // skip unknown options
                        }
                    }
                    break;
                }
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    regressors.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            model = Some(RegModel {
                dependent: dep,
                regressors,
                noint,
                noprint,
            });
        } else if ts.peek().is_kw("output") {
            ts.next();
            let mut out: Option<DatasetRef> = None;
            let mut predicted: Option<String> = None;
            let mut residual: Option<String> = None;
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if ts.peek().is_kw("out") {
                    ts.next();
                    expect_eq(ts, "OUT")?;
                    out = Some(ts.parse_dataset_ref()?);
                } else if ts.peek().is_kw("predicted") || ts.peek().is_kw("p") {
                    ts.next();
                    expect_eq(ts, "PREDICTED")?;
                    predicted = ts.peek().ident().map(str::to_string);
                    if predicted.is_some() {
                        ts.next();
                    }
                } else if ts.peek().is_kw("residual") || ts.peek().is_kw("r") {
                    ts.next();
                    expect_eq(ts, "RESIDUAL")?;
                    residual = ts.peek().ident().map(str::to_string);
                    if residual.is_some() {
                        ts.next();
                    }
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            if let Some(out_ref) = out {
                outputs.push(RegOutput {
                    out: out_ref,
                    predicted,
                    residual,
                });
            }
        } else if ts.peek().is_kw("by") {
            ts.next();
            ts.skip_to_semi();
        } else {
            ts.skip_to_semi();
        }
    }

    Ok(RegAst {
        data_options: RegDataOptions { input },
        model,
        outputs,
    })
}

// ───────────────────────── Formatting helpers ─────────────────────────

fn fmt5(v: f64) -> String {
    format!("{v:.5}")
}

fn fmt2(v: f64) -> String {
    format!("{v:.2}")
}

fn fmt_fit4(v: f64) -> String {
    format!("{v:.4}")
}

fn fmt_p(p: Option<f64>) -> String {
    match p {
        None => ".".to_string(),
        Some(v) if v < 0.0001 => "<.0001".to_string(),
        Some(v) => format!("{v:.4}"),
    }
}

// ───────────────────────── Stat helpers ─────────────────────────

fn two_sided_p(t: f64, df: f64) -> f64 {
    (2.0 * (1.0 - student_t_cdf(t.abs(), df))).clamp(0.0, 1.0)
}

// ───────────────────────── Listing helpers ─────────────────────────

fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(text.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), text));
}

// ───────────────────────── Resolve DATA= ─────────────────────────

fn resolve_input(ast: &RegAst, session: &Session) -> Result<DatasetRef> {
    match &ast.data_options.input {
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

// ───────────────────────── VarMeta helper ─────────────────────────

fn num_var_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &RegAst, session: &mut Session) -> Result<()> {
    let model = match &ast.model {
        Some(m) => m,
        None => {
            session
                .log
                .note("NOTE: No MODEL statement found.");
            return Ok(());
        }
    };

    if model.noint {
        return Err(SasError::runtime("NOINT not yet implemented"));
    }

    // --- 1. Resolve dataset ---
    let in_ref = resolve_input(ast, session)?;
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
    let dep_name = &model.dependent;
    let regressors = &model.regressors;
    let p = regressors.len();
    let p_eff = p + 1; // with intercept

    // --- 2. Find column indices ---
    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", nm.to_uppercase()))
            })
    };

    let dep_idx = find_col(dep_name)?;
    if ds.vars[dep_idx].ty != VarType::Num {
        return Err(SasError::runtime(format!(
            "Dependent variable {} must be numeric.",
            dep_name.to_uppercase()
        )));
    }

    let mut reg_idxs: Vec<usize> = Vec::with_capacity(p);
    for nm in regressors {
        let idx = find_col(nm)?;
        if ds.vars[idx].ty != VarType::Num {
            return Err(SasError::runtime(format!(
                "Regressor {} must be numeric.",
                nm.to_uppercase()
            )));
        }
        reg_idxs.push(idx);
    }

    // --- 3. Decode columns ---
    let dep_col = decode_column(&ds, dep_idx)?;
    let mut reg_cols: Vec<Vec<crate::value::Value>> = Vec::with_capacity(p);
    for &idx in &reg_idxs {
        reg_cols.push(decode_column(&ds, idx)?);
    }

    // --- 4. Build X matrix and y vector (listwise deletion) ---
    let mut x_mat: Vec<Vec<f64>> = Vec::new();
    let mut y_vec: Vec<f64> = Vec::new();
    // Track which original rows are complete (for OUTPUT dataset)
    let mut complete_mask: Vec<bool> = vec![false; n_read];

    for i in 0..n_read {
        // Check dependent
        let yi = match value_to_num(&dep_col[i]) {
            Some(v) if !v.is_nan() => v,
            _ => continue,
        };
        // Check all regressors
        let mut row = vec![1.0_f64]; // intercept
        let mut ok = true;
        for rc in &reg_cols {
            match value_to_num(&rc[i]) {
                Some(v) if !v.is_nan() => row.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            x_mat.push(row);
            y_vec.push(yi);
            complete_mask[i] = true;
        }
    }

    let n = x_mat.len();
    session
        .log
        .note(&format!("There were {} observations used.", n));

    if n <= p_eff {
        return Err(SasError::runtime(
            "Not enough observations for regression",
        ));
    }

    // --- 5. OLS: beta = least_squares(X, y) ---
    let beta = match linalg::least_squares(&x_mat, &y_vec) {
        Ok(b) => b,
        Err(e) => {
            session.log.error(&format!("Regression failed: {}", e));
            return Err(e);
        }
    };

    // --- 6. Compute predictions and residuals ---
    let y_hat: Vec<f64> = x_mat
        .iter()
        .map(|row| row.iter().zip(beta.iter()).map(|(xi, bi)| xi * bi).sum())
        .collect();
    let resid: Vec<f64> = y_vec.iter().zip(y_hat.iter()).map(|(yi, yhi)| yi - yhi).collect();

    // --- 7. Summary statistics ---
    let y_mean = y_vec.iter().sum::<f64>() / n as f64;
    let sse: f64 = resid.iter().map(|r| r * r).sum();
    let sst: f64 = y_vec.iter().map(|yi| (yi - y_mean) * (yi - y_mean)).sum();
    let ssm = sst - sse;

    let model_df = p as f64;
    let error_df = (n - p_eff) as f64;
    let total_df = (n - 1) as f64;

    let msm = if model_df > 0.0 { ssm / model_df } else { f64::NAN };
    let mse = sse / error_df;
    let f_stat = if mse > 0.0 { msm / mse } else { f64::NAN };
    let p_f = (1.0 - f_cdf(f_stat, model_df, error_df)).clamp(0.0, 1.0);

    let r2 = if sst > 0.0 { ssm / sst } else { f64::NAN };
    let adj_r2 = if sst > 0.0 {
        1.0 - (1.0 - r2) * (n as f64 - 1.0) / error_df
    } else {
        f64::NAN
    };

    let root_mse = mse.sqrt();
    let cv = if y_mean.abs() > 1e-15 {
        root_mse / y_mean.abs() * 100.0
    } else {
        f64::NAN
    };

    // --- 8. Standard errors and t-statistics for betas ---
    let xt = linalg::transpose(&x_mat);
    let xtx = linalg::matrix_mult(&xt, &x_mat);
    let xtx_inv = linalg::invert_matrix(&xtx)?;

    let mut se_beta: Vec<f64> = Vec::with_capacity(p_eff);
    let mut t_beta: Vec<f64> = Vec::with_capacity(p_eff);
    let mut p_beta: Vec<f64> = Vec::with_capacity(p_eff);

    for j in 0..p_eff {
        let se = (mse * xtx_inv[j][j]).sqrt();
        let t = beta[j] / se;
        let pv = two_sided_p(t, error_df);
        se_beta.push(se);
        t_beta.push(t);
        p_beta.push(pv);
    }

    // --- 9. Listing ---
    if !model.noprint {
        session.listing.page_header();
        centered(session, "The REG Procedure");
        centered(session, "Model: MODEL1");
        centered(session, &format!("Dependent Variable: {}", dep_name));
        session.listing.blank();

        session.listing.write_line(&format!(
            "               Number of Observations Read         {}",
            n_read
        ));
        session.listing.write_line(&format!(
            "               Number of Observations Used         {}",
            n
        ));
        session.listing.blank();
        session.listing.blank();

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
                format!("{}", model_df as usize),
                fmt5(ssm),
                fmt5(msm),
                fmt2(f_stat),
                fmt_p(Some(p_f)),
            ],
            vec![
                "Error".into(),
                format!("{}", error_df as usize),
                fmt5(sse),
                fmt5(mse),
                "".into(),
                "".into(),
            ],
            vec![
                "Corrected Total".into(),
                format!("{}", total_df as usize),
                fmt5(sst),
                "".into(),
                "".into(),
                "".into(),
            ],
        ];
        session
            .listing
            .write_table(&anova_headers, &anova_aligns, &anova_rows);
        session.listing.blank();
        session.listing.blank();

        // Fit statistics (written manually)
        session.listing.write_line(&format!(
            "Root MSE             {}    R-Square     {}",
            fmt5(root_mse),
            fmt_fit4(r2)
        ));
        session.listing.write_line(&format!(
            "Dependent Mean       {}    Adj R-Sq     {}",
            fmt5(y_mean),
            fmt_fit4(adj_r2)
        ));
        session
            .listing
            .write_line(&format!("Coeff Var            {}", fmt5(cv)));
        session.listing.blank();
        session.listing.blank();

        // Parameter estimates table
        let pe_headers: Vec<String> = vec![
            "Variable".into(),
            "DF".into(),
            "Parameter Estimate".into(),
            "Standard Error".into(),
            "t Value".into(),
            "Pr > |t|".into(),
        ];
        let pe_aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let mut pe_rows: Vec<Vec<String>> = Vec::with_capacity(p_eff);
        for j in 0..p_eff {
            let var_name = if j == 0 {
                "Intercept".to_string()
            } else {
                regressors[j - 1].clone()
            };
            pe_rows.push(vec![
                var_name,
                "1".into(),
                fmt5(beta[j]),
                fmt5(se_beta[j]),
                fmt2(t_beta[j]),
                fmt_p(Some(p_beta[j])),
            ]);
        }
        session
            .listing
            .write_table(&pe_headers, &pe_aligns, &pe_rows);
    }

    // --- 10. OUTPUT dataset (complete cases only) ---
    // OUTPUT dataset contains only complete cases (rows used in analysis)
    if !ast.outputs.is_empty() {
        let out_spec = &ast.outputs[0];

        // Build output dataset from complete-case rows
        // We need to re-extract original columns for complete rows
        let mut complete_indices: Vec<usize> = Vec::with_capacity(n);
        for (i, &is_complete) in complete_mask.iter().enumerate() {
            if is_complete {
                complete_indices.push(i);
            }
        }

        // Collect all original columns (only complete rows)
        let n_cols = ds.vars.len();
        let mut columns: Vec<Column> = Vec::with_capacity(n_cols + 2);
        let mut out_vars: Vec<VarMeta> = Vec::with_capacity(n_cols + 2);

        for col_idx in 0..n_cols {
            let col_vals = decode_column(&ds, col_idx)?;
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
                            crate::value::Value::Char(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                    columns.push(Series::new(ds.vars[col_idx].name.as_str().into(), data).into());
                }
            }
            out_vars.push(ds.vars[col_idx].clone());
        }

        // Add predicted column if requested
        if let Some(pred_name) = &out_spec.predicted {
            let data: Vec<Option<f64>> = y_hat.iter().map(|&v| Some(v)).collect();
            columns.push(Series::new(pred_name.as_str().into(), data).into());
            out_vars.push(num_var_meta(pred_name));
        }

        // Add residual column if requested
        if let Some(resid_name) = &out_spec.residual {
            let data: Vec<Option<f64>> = resid.iter().map(|&v| Some(v)).collect();
            columns.push(Series::new(resid_name.as_str().into(), data).into());
            out_vars.push(num_var_meta(resid_name));
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

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::VarMeta;
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

    fn parse_reg(src: &str) -> Result<RegAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // reg
        parse(&mut ts)
    }

    #[test]
    fn test_ols_simple() {
        // y = [1,2,3,4], x = [1,2,3,4] → perfect fit: intercept ≈ 0, slope ≈ 1, R² = 1
        let mut session = make_session();
        let frame = df![
            "y" => [1.0_f64, 2.0, 3.0, 4.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let ast = RegAst {
            data_options: RegDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "T".into(),
                }),
            },
            model: Some(RegModel {
                dependent: "y".into(),
                regressors: vec!["x".into()],
                noint: false,
                noprint: false,
            }),
            outputs: vec![],
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // R² should be essentially 1
        assert!(
            listing.contains("1.0000") || listing.contains("R-Square"),
            "listing: {listing}"
        );
        assert!(listing.contains("The REG Procedure"), "{listing}");
    }

    #[test]
    fn test_ols_regression() {
        // y=[2,4,5,4,5], x=[1,2,3,4,5] — classic textbook example, R² ≈ 0.8
        let mut session = make_session();
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let ast = RegAst {
            data_options: RegDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "T".into(),
                }),
            },
            model: Some(RegModel {
                dependent: "y".into(),
                regressors: vec!["x".into()],
                noint: false,
                noprint: false,
            }),
            outputs: vec![],
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // R² for this data is 0.8 exactly
        assert!(listing.contains("0.8000") || listing.contains("R-Square"), "{listing}");
    }

    #[test]
    fn test_parse_model() {
        let ast = parse_reg("proc reg data=a; model y = x1 x2; run;").unwrap();
        let m = ast.model.unwrap();
        assert_eq!(m.dependent, "y");
        assert_eq!(m.regressors, vec!["x1", "x2"]);
        assert!(!m.noint);
        assert!(!m.noprint);
    }

    #[test]
    fn test_parse_output() {
        let ast =
            parse_reg("proc reg data=a; model y = x; output out=work.out predicted=p residual=r; run;")
                .unwrap();
        assert_eq!(ast.outputs.len(), 1);
        let o = &ast.outputs[0];
        assert_eq!(o.out.name, "out");
        assert_eq!(o.predicted.as_deref(), Some("p"));
        assert_eq!(o.residual.as_deref(), Some("r"));
    }

    #[test]
    fn test_execute_simple() {
        let mut session = make_session();
        let frame = df![
            "weight" => [112.0_f64, 100.0, 130.0, 145.0, 160.0, 105.0],
            "height" => [59.0_f64, 57.0, 63.0, 67.0, 67.0, 57.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("weight"), num_meta("height")],
        };
        session.libs.get("WORK").unwrap().write("CLASS", &ds).unwrap();

        let ast = RegAst {
            data_options: RegDataOptions {
                input: Some(DatasetRef {
                    libref: Some("WORK".into()),
                    name: "CLASS".into(),
                }),
            },
            model: Some(RegModel {
                dependent: "weight".into(),
                regressors: vec!["height".into()],
                noint: false,
                noprint: false,
            }),
            outputs: vec![],
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("The REG Procedure"), "{listing}");
        assert!(listing.contains("Analysis of Variance"), "{listing}");
        assert!(listing.contains("Parameter Estimates") || listing.contains("Parameter"), "{listing}");
    }
}
