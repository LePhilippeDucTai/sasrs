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
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("POIS", &ds)
        .unwrap();

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

// ── Execute tests — Poisson oracle ───────────────────────────────────

fn run_poisson() -> String {
    let (mut session, ast) = make_poisson_session();
    execute(&ast, &mut session).unwrap();
    session.listing.take_string()
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
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("POIS", &ds)
        .unwrap();

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
    let listing = session.listing.take_string();
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

mod class;
mod parse;
