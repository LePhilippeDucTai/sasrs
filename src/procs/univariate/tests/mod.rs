use super::*;
use crate::dataset::SasDataset;
use crate::session::Session;
use crate::source::SourceFile;
use crate::testkit::*;
use polars::df;

fn parse_univ(src: &str) -> Result<UnivariateAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "univariate"
    parse(&mut ts)
}

// ───────────────────────────── parse tests ─────────────────────────────

// ─────────────────────────── normality tests ──────────────────────────

fn sorted_of(xs: &[f64]) -> Vec<f64> {
    let mut s = xs.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s
}

fn moments(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let s = sample_std(xs).unwrap();
    (mean, s)
}

// ───────────────────────── M29.3 plot tests ─────────────────────────

/// Helper: write a small numeric dataset and run UNIVARIATE with the given
/// plots and ODS GRAPHICS state, returning the log.
fn run_plots(
    ods_on: bool,
    plots: Vec<UnivariatePlot>,
    output_dir: Option<std::path::PathBuf>,
    file_stem: Option<String>,
) -> String {
    let mut session = make_session();
    session.ods_graphics.enabled = ods_on;
    if let Some(d) = output_dir {
        session.ods_graphics.output_dir = d;
    }
    session.ods_graphics.file_stem = file_stem;
    let df = df!["x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);
    let ast = UnivariateAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        output: None,
        normal: false,
        plots,
    };
    execute(&ast, &mut session).unwrap();
    session.log.into_string()
}

fn hist() -> Vec<UnivariatePlot> {
    vec![UnivariatePlot {
        kind: UnivariatePlotKind::Histogram,
        var: Some("x".into()),
        normal: false,
    }]
}

fn qq() -> Vec<UnivariatePlot> {
    vec![UnivariatePlot {
        kind: UnivariatePlotKind::QqPlot,
        var: Some("x".into()),
        normal: false,
    }]
}

// ─────────────────────────── quantile def-5 tests ──────────────────────

fn q(xs: &[f64], p: f64) -> f64 {
    let mut s = xs.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    quantile_def5(&s, p).unwrap()
}

// ───────────────────────────── execute tests ───────────────────────────

// ─────────────────────────── BY / OUTPUT tests ─────────────────────────

fn read_num_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
    decode_column(&ds, idx).unwrap()
}

// ──────────────────── weighted quantile (M33.2) tests ───────────────────

fn wq(pairs: &[(f64, f64)], p: f64) -> f64 {
    let mut s = pairs.to_vec();
    s.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    weighted_quantile_def5(&s, p).unwrap()
}

mod execute;
mod fitted_normal;
mod ods_select;
mod phi;
mod skewness;
