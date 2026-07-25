use super::*;
use crate::dataset::VarMeta;
use crate::session::Session;
use crate::source::SourceFile;
use crate::value::VarType;
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

mod one_parse;
mod execute;
mod two_unbalanced;
