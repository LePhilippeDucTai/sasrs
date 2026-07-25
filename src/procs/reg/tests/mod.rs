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

/// Build a single-model AST (no OUTPUT) for the given model.
fn single_model_ast(input: DatasetRef, model: RegModel) -> RegAst {
    RegAst {
        data_options: RegDataOptions {
            input: Some(input),
            outest: None,
            outsscp: None,
            ridge: Vec::new(),
            pcomit: Vec::new(),
            outvif: false,
        },
        models: vec![RegModelEntry {
            model,
            outputs: vec![],
            tests: vec![],
            restricts: vec![],
            mtests: vec![],
            add: vec![],
            delete: vec![],
        }],
        plots_requested: false,
        plot_requests: PlotRequests::default(),
        plot_statements: Vec::new(),
        weight: None,
        freq: None,
        by: Vec::new(),
        id: Vec::new(),
        simple: false,
        corr: false,
        var_list: Vec::new(),
        reweight_seen: false,
        refit_seen: false,
        paint_seen: false,
    }
}

fn basic_model(dep: &str, regs: &[&str]) -> RegModel {
    RegModel {
        dependents: vec![dep.into()],
        regressors: regs.iter().map(|s| s.to_string()).collect(),
        noint: false,
        noprint: false,
        selection: None,
        alpha: 0.05,
        clb: false,
        clm: false,
        cli: false,
        r: false,
        influence: false,
        vif: false,
        tol: false,
        collin: false,
        collinoint: false,
        spec: false,
        dw: false,
        dwprob: false,
        acov: false,
        ss1: false,
        ss2: false,
        stb: false,
        pcorr1: false,
        pcorr2: false,
        scorr1: false,
        scorr2: false,
        seqb: false,
        press_opt: false,
        xpx: false,
        inv: false,
        covb: false,
        corrb: false,
    }
}

// ───────────────────────── M29.3 diagnostics tests ─────────────────────────

fn run_diag(
    ods_on: bool,
    output_dir: Option<PathBuf>,
    file_stem: Option<String>,
) -> String {
    let mut session = make_session();
    session.ods_graphics.enabled = ods_on;
    if let Some(d) = output_dir {
        session.ods_graphics.output_dir = d;
    }
    session.ods_graphics.file_stem = file_stem;
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
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        basic_model("y", &["x"]),
    );
    execute(&ast, &mut session).unwrap();
    session.log.into_string()
}

// ───────────────────────── M36.1 TEST / RESTRICT tests ─────────────────────────

fn eq_terms(eq: &LinEq) -> Vec<(f64, String)> {
    eq.terms.clone()
}

// ───────────────────────── M36.2 CL / OUTPUT-stat tests ─────────────────────────

/// Build a design matrix [1, x...] for the given regressor columns.
fn design(intercept: bool, cols: &[&[f64]], n: usize) -> Vec<Vec<f64>> {
    (0..n)
        .map(|i| {
            let mut row = Vec::new();
            if intercept {
                row.push(1.0);
            }
            for c in cols {
                row.push(c[i]);
            }
            row
        })
        .collect()
}

// ───────────── M36.3 influence-diagnostic oracles ─────────────

/// Sample design reused by the influence oracles (intercept + one regressor,
/// a non-degenerate fit with dfE = n − 2 > 1).
fn infl_setup() -> (Vec<Vec<f64>>, Vec<f64>, OlsFit, usize, usize) {
    let x1 = [1.0_f64, 3.0, 2.0, 5.0, 4.0, 6.0, 8.0, 7.0];
    let y: Vec<f64> = x1.iter().map(|&a| 2.0 + 3.0 * a + (a * 0.7).sin()).collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let p_eff = 2;
    (x, y, fit, n, p_eff)
}

// ───────────────────── M36.5 partial-SS / correlation tests ─────────────────────

/// Build a model with all the M36.5 statistic flags turned on.
fn seq_model(dep: &str, regs: &[&str]) -> RegModel {
    let mut m = basic_model(dep, regs);
    m.ss1 = true;
    m.ss2 = true;
    m.stb = true;
    m.pcorr1 = true;
    m.pcorr2 = true;
    m.scorr1 = true;
    m.scorr2 = true;
    m.seqb = true;
    m
}

// ───────────────────── M36.6: advanced selection ─────────────────────

fn sel_with(method: SelMethod) -> Selection {
    Selection {
        method,
        slentry: 0.5,
        slstay: 0.1,
        best: None,
        include: 0,
        start: None,
        stop: None,
        details: false,
        stb: false,
    }
}

/// A small fixture with 3 regressors over 8 rows. Returns (xcols, y).
fn three_reg_data() -> (Vec<Vec<f64>>, Vec<f64>) {
    let x0 = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let x1 = vec![2.0, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0];
    let x2 = vec![1.0, 3.0, 2.0, 5.0, 4.0, 7.0, 6.0, 9.0];
    let y = vec![3.0, 5.0, 6.0, 9.0, 11.0, 13.0, 16.0, 18.0];
    (vec![x0, x1, x2], y)
}

fn r2_full(xcols: &[Vec<f64>], y: &[f64], cols: &[usize], intercept: bool) -> f64 {
    let n = y.len();
    let sst = if intercept {
        let ybar = y.iter().sum::<f64>() / n as f64;
        y.iter().map(|v| (v - ybar) * (v - ybar)).sum::<f64>()
    } else {
        y.iter().map(|v| v * v).sum::<f64>()
    };
    let sse = subset_sse(xcols, y, cols, intercept).unwrap();
    1.0 - sse / sst
}

// ───────────── M36.8 self-consistency oracles ─────────────

/// Shared design: intercept + two regressors, non-degenerate.
fn m368_setup() -> (Vec<Vec<f64>>, Vec<f64>, OlsFit, Vec<String>, usize) {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0];
    let y: Vec<f64> = (0..8)
        .map(|i| 1.0 + 2.0 * x1[i] - 0.5 * x2[i] + (x1[i] * 0.3).cos())
        .collect();
    let n = y.len();
    let x = design(true, &[&x1, &x2], n);
    let fit = ols_fit(&x, &y).unwrap();
    let names = vec!["x1".to_string(), "x2".to_string()];
    (x, y, fit, names, n)
}

// ───────────────────────── M36.9 ridge / IPC ─────────────────────────

/// A collinear sample for the ridge / IPC oracles: x2 ≈ x1 + small noise so
/// R has a small eigenvalue (a clear shrinkage signal).
fn m369_setup() -> (Vec<Vec<f64>>, Vec<f64>) {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let x2: Vec<f64> = x1
        .iter()
        .enumerate()
        .map(|(i, v)| v + 0.05 * ((i as f64) * 0.7).sin())
        .collect();
    let y: Vec<f64> = (0..x1.len())
        .map(|i| 3.0 + 1.5 * x1[i] - 0.8 * x2[i] + 0.2 * ((i as f64) * 1.3).cos())
        .collect();
    (vec![x1.to_vec(), x2], y)
}

fn norm2(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

/// Build + execute a model carrying explicit PLOTS=/PLOT requests, returning
/// the captured log. Mirrors `run_diag` but injects the M36.11 requests.
fn run_plots(
    ods_on: bool,
    none: bool,
    plot_requests: PlotRequests,
    plot_statements: Vec<PlotPair>,
) -> String {
    let _ = none;
    let mut session = make_session();
    session.ods_graphics.enabled = ods_on;
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
    let mut ast = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        basic_model("y", &["x"]),
    );
    ast.plot_requests = plot_requests;
    ast.plot_statements = plot_statements;
    execute(&ast, &mut session).unwrap();
    session.log.into_string()
}

mod ols;
mod parse;
mod execute;
mod oracle1;
mod oracle2;
mod oracle3;
mod by_heading;
mod m3610;
