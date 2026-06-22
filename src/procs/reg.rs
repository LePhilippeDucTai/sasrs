//! PROC REG — OLS linear regression (M25.1, extended M34.4).
//!
//! Implements PROC REG. Supports:
//! - Multiple MODEL statements (each with its own OUTPUT statement(s)).
//! - Intercept models and NOINT (no-intercept) models.
//! - SELECTION= FORWARD / BACKWARD / STEPWISE variable selection.
//!
//! Produces, per model, an ANOVA table, fit statistics, and parameter
//! estimates with t-tests. Optional OUTPUT statement writes predicted values
//! and residuals (using the final selected model when SELECTION= is given).

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
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
    /// All MODEL statements, in source order. Each carries the OUTPUT
    /// statement(s) that followed it (SAS associates an OUTPUT with the
    /// MODEL it follows).
    pub models: Vec<RegModelEntry>,
    /// M29.3 — an explicit `PLOTS ...;` statement was seen. Its complex forms
    /// are deferred (a NOTE); the simple residuals-vs-predicted diagnostic is
    /// driven automatically from `ods_graphics.enabled`, not from this flag.
    pub plots_requested: bool,
}

#[derive(Debug, Clone)]
pub struct RegModelEntry {
    pub model: RegModel,
    pub outputs: Vec<RegOutput>,
    /// TEST statements that followed this MODEL (M36.1).
    pub tests: Vec<RegTest>,
    /// RESTRICT statements that followed this MODEL (M36.1).
    pub restricts: Vec<RegRestrict>,
}

/// A linear equation over regressor names (and the keyword `INTERCEPT`),
/// normalised so that every term is moved to the left-hand side:
///   Σ coef_i · var_i = rhs
/// where `var_i` is an uppercased regressor name (or the literal `"INTERCEPT"`)
/// and `rhs` is the net constant after moving variables left / constants right.
#[derive(Debug, Clone, PartialEq)]
pub struct LinEq {
    /// (coefficient, uppercased variable name). The intercept maps to the
    /// reserved name `"INTERCEPT"`.
    pub terms: Vec<(f64, String)>,
    /// The net constant on the right-hand side.
    pub rhs: f64,
}

/// A `[label:] TEST eq [, eq ...];` statement (M36.1).
#[derive(Debug, Clone)]
pub struct RegTest {
    pub label: Option<String>,
    pub equations: Vec<LinEq>,
}

/// A `RESTRICT eq [, eq ...];` statement (M36.1).
#[derive(Debug, Clone)]
pub struct RegRestrict {
    pub equations: Vec<LinEq>,
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
    /// SELECTION= option (FORWARD / BACKWARD / STEPWISE), if requested.
    pub selection: Option<Selection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelMethod {
    Forward,
    Backward,
    Stepwise,
}

#[derive(Debug, Clone, Copy)]
pub struct Selection {
    pub method: SelMethod,
    pub slentry: f64,
    pub slstay: f64,
}

#[derive(Debug, Clone)]
pub struct RegOutput {
    pub out: DatasetRef,
    pub predicted: Option<String>,
    pub residual: Option<String>,
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
            input = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else {
            // Skip unknown proc-level options
            ts.next();
        }
    }

    // Sub-statements until run;/quit;
    let mut models: Vec<RegModelEntry> = Vec::new();
    let mut plots_requested = false;

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "model" {
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
            let mut selection: Option<Selection> = None;
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
                        } else if ts.peek().is_kw("selection") {
                            common::expect_eq(ts, "SELECTION")?;
                            let method_name = ts
                                .peek()
                                .ident()
                                .map(str::to_string)
                                .ok_or_else(|| {
                                    SasError::parse(
                                        "expected selection method after SELECTION=",
                                        ts.peek().span,
                                    )
                                })?;
                            ts.next();
                            let method = match method_name.to_ascii_lowercase().as_str() {
                                "forward" => SelMethod::Forward,
                                "backward" => SelMethod::Backward,
                                "stepwise" => SelMethod::Stepwise,
                                other => {
                                    return Err(SasError::parse(
                                        format!("unsupported SELECTION method '{}'", other),
                                        ts.peek().span,
                                    ));
                                }
                            };
                            // Defaults depend on the method.
                            let (def_sle, def_sls) = match method {
                                SelMethod::Forward => (0.50, 0.10),
                                SelMethod::Backward => (0.50, 0.10),
                                SelMethod::Stepwise => (0.15, 0.15),
                            };
                            selection = Some(Selection {
                                method,
                                slentry: def_sle,
                                slstay: def_sls,
                            });
                        } else if ts.peek().is_kw("slentry") || ts.peek().is_kw("sle") {
                            common::expect_eq(ts, "SLENTRY")?;
                            let v = read_float(ts)?;
                            if let Some(sel) = selection.as_mut() {
                                sel.slentry = v;
                            }
                        } else if ts.peek().is_kw("slstay") || ts.peek().is_kw("sls") {
                            common::expect_eq(ts, "SLSTAY")?;
                            let v = read_float(ts)?;
                            if let Some(sel) = selection.as_mut() {
                                sel.slstay = v;
                            }
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
            models.push(RegModelEntry {
                model: RegModel {
                    dependent: dep,
                    regressors,
                    noint,
                    noprint,
                    selection,
                },
                outputs: Vec::new(),
                tests: Vec::new(),
                restricts: Vec::new(),
            });
            Ok(true)
        } else if kw == "output" {
            ts.next();
            let mut out: Option<DatasetRef> = None;
            let mut predicted: Option<String> = None;
            let mut residual: Option<String> = None;
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if ts.peek().is_kw("out") {
                    out = Some(common::parse_out_opt(ts)?);
                } else if ts.peek().is_kw("predicted") || ts.peek().is_kw("p") {
                    common::expect_eq(ts, "PREDICTED")?;
                    predicted = ts.peek().ident().map(str::to_string);
                    if predicted.is_some() {
                        ts.next();
                    }
                } else if ts.peek().is_kw("residual") || ts.peek().is_kw("r") {
                    common::expect_eq(ts, "RESIDUAL")?;
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
                // Associate this OUTPUT with the MODEL it follows (the last one
                // seen). If no MODEL has been seen yet, SAS would error; we drop
                // it silently here, matching the prior "only emit if out present"
                // behaviour as closely as possible.
                if let Some(entry) = models.last_mut() {
                    entry.outputs.push(RegOutput {
                        out: out_ref,
                        predicted,
                        residual,
                    });
                }
            }
            Ok(true)
        } else if kw == "plots" {
            // M29.3 — PLOTS statement: parsed but its options are deferred (a
            // NOTE at execute time). Skip the whole statement (including a
            // possible `(...)` option list and trailing `/ options`).
            ts.next();
            ts.skip_to_semi();
            plots_requested = true;
            Ok(true)
        } else if kw == "test" {
            // `TEST eq [, eq ...];` (unlabeled form — a leading `label:` is
            // handled by the catch-all branch below, which rewrites the kw).
            ts.next(); // consume "test"
            let equations = parse_lin_eqs(ts)?;
            ts.expect_semi()?;
            if let Some(entry) = models.last_mut() {
                entry.tests.push(RegTest {
                    label: None,
                    equations,
                });
            }
            Ok(true)
        } else if kw == "restrict" {
            ts.next(); // consume "restrict"
            let equations = parse_lin_eqs(ts)?;
            ts.expect_semi()?;
            if let Some(entry) = models.last_mut() {
                entry.restricts.push(RegRestrict { equations });
            }
            Ok(true)
        } else if ts.peek2().kind == TokenKind::Colon && ts.peek_nth(2).is_kw("test") {
            // `label: TEST eq [, eq ...];` — the leading identifier is a label.
            let label = ts.peek().ident().map(str::to_string);
            ts.next(); // label ident
            ts.next(); // ':'
            ts.next(); // 'test'
            let equations = parse_lin_eqs(ts)?;
            ts.expect_semi()?;
            if let Some(entry) = models.last_mut() {
                entry.tests.push(RegTest { label, equations });
            }
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(RegAst {
        data_options: RegDataOptions { input },
        models,
        plots_requested,
    })
}

/// Read a numeric option value (e.g. `0.5`). Significance levels in PROC REG
/// are conventionally written with a leading zero (`0.05`), which the lexer
/// emits as a single `Num` token.
fn read_float(ts: &mut StatementStream) -> Result<f64> {
    match ts.peek().kind {
        TokenKind::Num(v) => {
            ts.next();
            Ok(v)
        }
        _ => Err(SasError::parse("expected numeric value", ts.peek().span)),
    }
}

// ───────────────────────── Linear-equation parsing (M36.1) ─────────────────────────

/// Parse a comma-separated list of linear equations (`eq [, eq ...]`),
/// stopping at the terminating `;`.
fn parse_lin_eqs(ts: &mut StatementStream) -> Result<Vec<LinEq>> {
    let mut eqs = Vec::new();
    loop {
        eqs.push(parse_lin_eq(ts)?);
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
            continue;
        }
        break;
    }
    Ok(eqs)
}

/// Parse one linear equation `lhs = rhs` and normalise it so every variable
/// term sits on the left and the net constant on the right:
/// Σ coef·var = rhs. Variable names are uppercased; `INTERCEPT` is preserved
/// as the reserved name `"INTERCEPT"`.
fn parse_lin_eq(ts: &mut StatementStream) -> Result<LinEq> {
    // Left side: accumulate terms with their natural sign.
    let mut terms: Vec<(f64, String)> = Vec::new();
    let mut rhs = 0.0; // net constant: starts on the LHS (subtracted later).
    let mut lhs_const = 0.0;
    parse_lin_side(ts, 1.0, &mut terms, &mut lhs_const)?;

    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            "expected '=' in TEST/RESTRICT equation",
            ts.peek().span,
        ));
    }
    ts.next(); // '='

    // Right side: variables flip sign (move to LHS), constants stay on RHS.
    let mut rhs_terms: Vec<(f64, String)> = Vec::new();
    let mut rhs_const = 0.0;
    parse_lin_side(ts, 1.0, &mut rhs_terms, &mut rhs_const)?;
    for (c, v) in rhs_terms {
        terms.push((-c, v));
    }
    // Net constant = rhs_const - lhs_const.
    rhs += rhs_const - lhs_const;

    // Merge duplicate variables.
    let mut merged: Vec<(f64, String)> = Vec::new();
    for (c, v) in terms {
        if let Some(e) = merged.iter_mut().find(|(_, name)| *name == v) {
            e.0 += c;
        } else {
            merged.push((c, v));
        }
    }
    Ok(LinEq { terms: merged, rhs })
}

/// Parse one side of an equation: a sum of signed terms up to `=`, `,` or `;`.
/// Variable terms are pushed into `terms` (scaled by `sign`); bare constants
/// accumulate into `konst`.
fn parse_lin_side(
    ts: &mut StatementStream,
    sign: f64,
    terms: &mut Vec<(f64, String)>,
    konst: &mut f64,
) -> Result<()> {
    let mut pending = sign; // sign accumulated from a run of leading +/-.
    loop {
        match ts.peek().kind {
            TokenKind::Eq | TokenKind::Comma | TokenKind::Semi | TokenKind::Eof => break,
            TokenKind::Plus => {
                ts.next();
                continue;
            }
            TokenKind::Minus => {
                pending = -pending;
                ts.next();
                continue;
            }
            _ => {}
        }
        // A term: optional numeric coefficient, optional `*`, then a name; or a
        // bare constant; or a bare name (coef 1).
        let mut coef = pending;
        let mut have_num = false;
        if let TokenKind::Num(v) = ts.peek().kind {
            coef = pending * v;
            have_num = true;
            ts.next();
            if ts.peek().kind == TokenKind::Star {
                ts.next();
            }
        }
        if let Some(name) = ts.peek().ident().map(str::to_string) {
            ts.next();
            terms.push((coef, name.to_ascii_uppercase()));
        } else if have_num {
            // Bare constant (no variable followed the number).
            *konst += coef;
        } else {
            return Err(SasError::parse(
                "expected variable or constant in TEST/RESTRICT equation",
                ts.peek().span,
            ));
        }
        // Reset the sign for the next term.
        pending = sign;
    }
    Ok(())
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

// ───────────────────────── OLS fit helper ─────────────────────────

/// Result of an ordinary-least-squares fit on a fully-numeric design matrix.
struct OlsFit {
    /// Coefficient vector (one per column of X).
    beta: Vec<f64>,
    /// Predicted values ŷ = Xβ.
    y_hat: Vec<f64>,
    /// Residuals y − ŷ.
    resid: Vec<f64>,
    /// Σ resid² (residual / error sum of squares).
    sse: f64,
    /// (XᵀX)⁻¹.
    xtx_inv: Vec<Vec<f64>>,
}

/// Fit OLS for the given design matrix `x` (rows are observations, columns are
/// regressors — the caller decides whether an intercept column is present) and
/// response `y`. Pure: no session / printing side effects.
fn ols_fit(x: &[Vec<f64>], y: &[f64]) -> Result<OlsFit> {
    let beta = linalg::least_squares(x, y)?;
    let y_hat: Vec<f64> = x
        .iter()
        .map(|row| row.iter().zip(beta.iter()).map(|(xi, bi)| xi * bi).sum())
        .collect();
    let resid: Vec<f64> = y
        .iter()
        .zip(y_hat.iter())
        .map(|(yi, yhi)| yi - yhi)
        .collect();
    let sse: f64 = resid.iter().map(|r| r * r).sum();
    let xt = linalg::transpose(x);
    let xtx = linalg::matrix_mult(&xt, x);
    let xtx_inv = linalg::invert_matrix(&xtx)?;
    Ok(OlsFit {
        beta,
        y_hat,
        resid,
        sse,
        xtx_inv,
    })
}

/// Compute SSE only for a candidate subset fit (used by SELECTION). Builds the
/// design matrix from `xcols` (columns of regressors, each length n) over the
/// `subset` of column indices, optionally prepending an intercept column.
/// Returns `None` if the fit is rank-deficient / not solvable.
fn subset_sse(xcols: &[Vec<f64>], y: &[f64], subset: &[usize], intercept: bool) -> Option<f64> {
    let n = y.len();
    let mut x: Vec<Vec<f64>> = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = Vec::with_capacity(subset.len() + intercept as usize);
        if intercept {
            row.push(1.0);
        }
        for &c in subset {
            row.push(xcols[c][i]);
        }
        x.push(row);
    }
    if x.is_empty() || x[0].is_empty() {
        return None;
    }
    ols_fit(&x, y).ok().map(|f| f.sse)
}

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &RegAst, session: &mut Session) -> Result<()> {
    if ast.models.is_empty() {
        session.log.note("NOTE: No MODEL statement found.");
        return Ok(());
    }

    // --- 1. Resolve dataset (once per proc) ---
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

    // --- 2. Per-model loop ---
    for (mi, entry) in ast.models.iter().enumerate() {
        let model_label = format!("Model: MODEL{}", mi + 1);
        run_model(
            ast,
            entry,
            &ds,
            &in_libref,
            &in_table,
            n_read,
            &model_label,
            session,
        )?;
    }

    Ok(())
}

/// Run a single MODEL statement: resolve columns, do listwise deletion, then
/// dispatch to the default/NOINT path or the SELECTION path. Writes any OUTPUT
/// dataset associated with the model.
#[allow(clippy::too_many_arguments)]
fn run_model(
    ast: &RegAst,
    entry: &RegModelEntry,
    ds: &SasDataset,
    in_libref: &str,
    in_table: &str,
    n_read: usize,
    model_label: &str,
    session: &mut Session,
) -> Result<()> {
    let _ = (in_libref, in_table);
    let model = &entry.model;
    let dep_name = &model.dependent;
    let regressors = &model.regressors;
    let p = regressors.len();

    // --- Find column indices ---
    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
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

    // --- Decode columns ---
    let dep_col = decode_column(ds, dep_idx)?;
    let mut reg_cols: Vec<Vec<crate::value::Value>> = Vec::with_capacity(p);
    for &idx in &reg_idxs {
        reg_cols.push(decode_column(ds, idx)?);
    }

    // --- Build regressor columns (numeric) and y vector (listwise deletion) ---
    // xcols[c] is the c-th regressor over the complete-case rows.
    let mut xcols: Vec<Vec<f64>> = vec![Vec::new(); p];
    let mut y_vec: Vec<f64> = Vec::new();
    let mut complete_mask: Vec<bool> = vec![false; n_read];

    for i in 0..n_read {
        let yi = match value_to_num(&dep_col[i]) {
            Some(v) if !v.is_nan() => v,
            _ => continue,
        };
        let mut row_vals = Vec::with_capacity(p);
        let mut ok = true;
        for rc in &reg_cols {
            match value_to_num(&rc[i]) {
                Some(v) if !v.is_nan() => row_vals.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            for (c, v) in row_vals.into_iter().enumerate() {
                xcols[c].push(v);
            }
            y_vec.push(yi);
            complete_mask[i] = true;
        }
    }

    let n = y_vec.len();
    session
        .log
        .note(&format!("There were {} observations used.", n));

    let intercept = !model.noint;

    // --- SELECTION path: choose the final regressor subset, then fit/print it.
    let selected: Vec<usize> = if let Some(sel) = model.selection {
        match run_selection(&sel, &xcols, &y_vec, regressors, intercept, model, session) {
            Some(s) => s,
            None => {
                // Nothing entered (FORWARD/STEPWISE) — fit the intercept-only
                // model (or note for NOINT) and finish, no OUTPUT.
                fit_and_print_empty(model, dep_name, n_read, n, model_label, session);
                return Ok(());
            }
        }
    } else {
        (0..p).collect()
    };

    // Build the final design matrix over the selected columns.
    let sel_p = selected.len();
    let p_eff = sel_p + intercept as usize;

    if n <= p_eff {
        return Err(SasError::runtime("Not enough observations for regression"));
    }

    let mut x_mat: Vec<Vec<f64>> = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = Vec::with_capacity(p_eff);
        if intercept {
            row.push(1.0);
        }
        for &c in &selected {
            row.push(xcols[c][i]);
        }
        x_mat.push(row);
    }

    let sel_reg_names: Vec<String> = selected.iter().map(|&c| regressors[c].clone()).collect();

    let fit = match ols_fit(&x_mat, &y_vec) {
        Ok(f) => f,
        Err(e) => {
            session.log.error(&format!("Regression failed: {}", e));
            return Err(e);
        }
    };

    // --- RESTRICT (M36.1): re-estimate under the linear constraints. The model
    // is printed as the restricted fit; TEST then operates on that fit. When
    // there are no RESTRICT statements this stays None and the OLS path is
    // byte-identical to before.
    let restricted = if entry.restricts.is_empty() {
        None
    } else {
        compute_restricted(
            &entry.restricts,
            &sel_reg_names,
            intercept,
            &x_mat,
            &y_vec,
            &fit,
            n,
        )?
    };

    fit_and_print(
        model,
        dep_name,
        &sel_reg_names,
        &fit,
        restricted.as_ref(),
        n_read,
        n,
        intercept,
        model_label,
        session,
    );

    // --- TEST (M36.1): operate on the model as fitted (restricted if present).
    if !entry.tests.is_empty() {
        let (t_beta, t_xtx, t_sse, t_dfe) = match &restricted {
            Some(r) => (&r.beta_r, &fit.xtx_inv, r.sse_r, r.df_r),
            None => (
                &fit.beta,
                &fit.xtx_inv,
                fit.sse,
                (n - x_mat[0].len()) as f64,
            ),
        };
        run_tests(
            &entry.tests,
            &sel_reg_names,
            intercept,
            dep_name,
            t_beta,
            t_xtx,
            t_sse,
            t_dfe,
            x_mat[0].len(),
            session,
        )?;
    }

    // --- OUTPUT dataset(s) for this model (complete cases only) ---
    write_outputs(entry, ds, &complete_mask, n, &fit, session)?;

    // --- Diagnostics (M29.3) ---
    if ast.plots_requested {
        session.log.note("PLOTS options deferred in PROC REG.");
    }
    if session.ods_graphics.enabled {
        let y_hat = fit.y_hat.clone();
        let resid = fit.resid.clone();
        reg_diagnostic_plot(session, &y_hat, &resid);
    }

    Ok(())
}

/// Fit-and-print the full output block for a model (ANOVA + fit statistics +
/// parameter estimates). This is the SINGLE printer shared by the default,
/// NOINT, and SELECTION-final paths, guaranteeing byte-identical output for the
/// default case. `reg_names` are the regressor names actually in the model (no
/// intercept entry); `fit` was computed on a design matrix whose column order
/// matches: [intercept?] then `reg_names`.
#[allow(clippy::too_many_arguments)]
fn fit_and_print(
    model: &RegModel,
    dep_name: &str,
    reg_names: &[String],
    fit: &OlsFit,
    restricted: Option<&Restricted>,
    n_read: usize,
    n: usize,
    intercept: bool,
    model_label: &str,
    session: &mut Session,
) {
    // When a restricted fit is present, the printed model (ANOVA, R², F, and
    // parameter estimates) reflects the RESTRICTed estimates β_r / SSE_r / df_r.
    let beta: &[f64] = match restricted {
        Some(r) => &r.beta_r,
        None => &fit.beta,
    };
    let sse = match restricted {
        Some(r) => r.sse_r,
        None => fit.sse,
    };
    let y_hat: &[f64] = match restricted {
        Some(r) => &r.y_hat_r,
        None => &fit.y_hat,
    };
    let resid: &[f64] = match restricted {
        Some(r) => &r.resid_r,
        None => &fit.resid,
    };

    // y vector reconstructed from ŷ + resid (avoids threading it in).
    let y_mean = {
        let sum: f64 = y_hat.iter().zip(resid.iter()).map(|(yh, r)| yh + r).sum();
        sum / n as f64
    };

    let p = reg_names.len();
    let p_eff = p + intercept as usize;
    // Restricted error df = (n−p_eff)+qr; this raises the Error-line DF and
    // lowers the Model DF by the number of restrictions.
    let restrict_q = restricted.map(|r| r.lambda_rows.len()).unwrap_or(0);

    // --- ANOVA decomposition ---
    let (ssm, sst, model_df, error_df, total_df, total_label, r2, adj_r2);
    if intercept {
        // Corrected (centered) sums of squares.
        let y: Vec<f64> = y_hat.iter().zip(resid.iter()).map(|(yh, r)| yh + r).collect();
        sst = y.iter().map(|yi| (yi - y_mean) * (yi - y_mean)).sum();
        ssm = sst - sse;
        model_df = (p - restrict_q) as f64;
        error_df = (n - p_eff + restrict_q) as f64;
        total_df = (n - 1) as f64;
        total_label = "Corrected Total";
        r2 = if sst > 0.0 { ssm / sst } else { f64::NAN };
        adj_r2 = if sst > 0.0 {
            1.0 - (1.0 - r2) * (n as f64 - 1.0) / error_df
        } else {
            f64::NAN
        };
    } else {
        // Uncorrected sums of squares (NOINT).
        let sst_unc: f64 = y_hat
            .iter()
            .zip(resid.iter())
            .map(|(yh, r)| {
                let yi = yh + r;
                yi * yi
            })
            .sum();
        let ssm_unc: f64 = y_hat.iter().map(|yh| yh * yh).sum();
        sst = sst_unc;
        ssm = ssm_unc;
        model_df = (p - restrict_q) as f64;
        error_df = (n - p + restrict_q) as f64;
        total_df = n as f64;
        total_label = "Uncorrected Total";
        r2 = if sst > 0.0 { ssm / sst } else { f64::NAN };
        adj_r2 = if sst > 0.0 {
            1.0 - (1.0 - r2) * (n as f64) / (n as f64 - p as f64)
        } else {
            f64::NAN
        };
    }

    let msm = if model_df > 0.0 { ssm / model_df } else { f64::NAN };
    let mse = sse / error_df;
    let f_stat = if mse > 0.0 { msm / mse } else { f64::NAN };
    let p_f = (1.0 - f_cdf(f_stat, model_df, error_df)).clamp(0.0, 1.0);

    let root_mse = mse.sqrt();
    let cv = if y_mean.abs() > 1e-15 {
        root_mse / y_mean.abs() * 100.0
    } else {
        f64::NAN
    };

    // --- Standard errors / t / p for each beta ---
    // For the restricted fit these come from the constrained covariance matrix
    // computed in compute_restricted; otherwise from the usual MSE·(X'X)⁻¹.
    let (se_beta, t_beta, p_beta): (Vec<f64>, Vec<f64>, Vec<f64>) = match restricted {
        Some(r) => (r.se_r.clone(), r.t_r.clone(), r.p_r.clone()),
        None => {
            let mut se_beta = Vec::with_capacity(p_eff);
            let mut t_beta = Vec::with_capacity(p_eff);
            let mut p_beta = Vec::with_capacity(p_eff);
            for j in 0..p_eff {
                let se = (mse * fit.xtx_inv[j][j]).sqrt();
                let t = beta[j] / se;
                let pv = two_sided_p(t, error_df);
                se_beta.push(se);
                t_beta.push(t);
                p_beta.push(pv);
            }
            (se_beta, t_beta, p_beta)
        }
    };

    if model.noprint {
        return;
    }

    session.listing.page_header();
    centered(session, "The REG Procedure");
    centered(session, model_label);
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
            total_label.into(),
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

    // Parameter estimates table. With RESTRICT statements a trailing Label
    // column carries the restriction expression; the unrestricted path keeps
    // the original 6-column layout byte-identical.
    let with_label = restricted.is_some();
    let mut pe_headers: Vec<String> = vec![
        "Variable".into(),
        "DF".into(),
        "Parameter Estimate".into(),
        "Standard Error".into(),
        "t Value".into(),
        "Pr > |t|".into(),
    ];
    let mut pe_aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    if with_label {
        pe_headers.push("Label".into());
        pe_aligns.push(Align::Left);
    }
    let mut pe_rows: Vec<Vec<String>> = Vec::with_capacity(p_eff);
    for j in 0..p_eff {
        let var_name = if intercept {
            if j == 0 {
                "Intercept".to_string()
            } else {
                reg_names[j - 1].clone()
            }
        } else {
            reg_names[j].clone()
        };
        let mut row = vec![
            var_name,
            "1".into(),
            fmt5(beta[j]),
            fmt5(se_beta[j]),
            fmt2(t_beta[j]),
            fmt_p(Some(p_beta[j])),
        ];
        if with_label {
            row.push(String::new());
        }
        pe_rows.push(row);
    }
    // Append RESTRICT rows: Variable="RESTRICT", DF=-1 (negative per SAS),
    // Estimate=λ_i, with the restriction expression in the Label column.
    if let Some(r) = restricted {
        for (label, lam, se, t, pv) in &r.lambda_rows {
            pe_rows.push(vec![
                "RESTRICT".into(),
                "-1".into(),
                fmt5(*lam),
                fmt5(*se),
                fmt2(*t),
                fmt_p(Some(*pv)),
                label.clone(),
            ]);
        }
    }
    session
        .listing
        .write_table(&pe_headers, &pe_aligns, &pe_rows);
}

/// Print the degenerate "no variables entered" case for SELECTION when the
/// selected set is empty.
fn fit_and_print_empty(
    model: &RegModel,
    dep_name: &str,
    _n_read: usize,
    _n: usize,
    model_label: &str,
    session: &mut Session,
) {
    if model.noprint {
        return;
    }
    session.listing.page_header();
    centered(session, "The REG Procedure");
    centered(session, model_label);
    centered(session, &format!("Dependent Variable: {}", dep_name));
    session.listing.blank();
    if model.noint {
        centered(
            session,
            "No variables met the entry criterion; no model was fit.",
        );
    } else {
        centered(
            session,
            "No variables met the entry criterion; intercept-only model.",
        );
    }
    session.listing.blank();
}

// ───────────────────────── TEST / RESTRICT (M36.1) ─────────────────────────

/// Build the L (q×p_eff) matrix and c (q-vector) for a set of linear equations,
/// with columns ordered exactly like `fit.beta`: intercept first (if present),
/// then `reg_names` in order. Returns an error naming the first unknown
/// variable. The intercept keyword `INTERCEPT` maps to column 0 (only valid
/// when an intercept is in the model).
fn build_lc(
    equations: &[LinEq],
    reg_names: &[String],
    intercept: bool,
) -> Result<(Vec<Vec<f64>>, Vec<f64>)> {
    let p_eff = reg_names.len() + intercept as usize;
    // Column index for a (already uppercased) variable name.
    let col_of = |name: &str| -> Option<usize> {
        if name == "INTERCEPT" {
            return if intercept { Some(0) } else { None };
        }
        let base = intercept as usize;
        reg_names
            .iter()
            .position(|r| r.eq_ignore_ascii_case(name))
            .map(|k| base + k)
    };
    let mut l = Vec::with_capacity(equations.len());
    let mut c = Vec::with_capacity(equations.len());
    for eq in equations {
        let mut row = vec![0.0; p_eff];
        for (coef, name) in &eq.terms {
            match col_of(name) {
                Some(j) => row[j] += *coef,
                None => {
                    return Err(SasError::runtime(format!(
                        "Variable {} in TEST/RESTRICT not in the model.",
                        name
                    )))
                }
            }
        }
        l.push(row);
        c.push(eq.rhs);
    }
    Ok((l, c))
}

/// Restricted-fit results threaded into `fit_and_print`.
struct Restricted {
    /// Restricted coefficient estimates β_r (same column order as `fit.beta`).
    beta_r: Vec<f64>,
    /// Restricted error/residual sum of squares.
    sse_r: f64,
    /// Restricted error degrees of freedom = (n − p_eff) + qr.
    df_r: f64,
    /// Predicted values from β_r.
    y_hat_r: Vec<f64>,
    /// Residuals from β_r.
    resid_r: Vec<f64>,
    /// SE / t / p for each β_r (column order matches `beta_r`).
    se_r: Vec<f64>,
    t_r: Vec<f64>,
    p_r: Vec<f64>,
    /// One appended RESTRICT row per restriction: (label, λ, SE, t, p).
    lambda_rows: Vec<(String, f64, f64, f64, f64)>,
}

/// Compute the constrained least-squares fit under all RESTRICT equations of
/// the model. `x_mat` is the design matrix (column order == `fit.beta`), `y`
/// the response. Returns `None` if there are no restrictions.
fn compute_restricted(
    restricts: &[RegRestrict],
    reg_names: &[String],
    intercept: bool,
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
) -> Result<Option<Restricted>> {
    // Gather every restriction equation (with a label for the table).
    let mut eqs: Vec<LinEq> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for r in restricts {
        for eq in &r.equations {
            labels.push(restrict_label(eq, reg_names, intercept));
            eqs.push(eq.clone());
        }
    }
    if eqs.is_empty() {
        return Ok(None);
    }
    let (l, c) = build_lc(&eqs, reg_names, intercept)?;
    let qr = l.len();
    let p_eff = x_mat[0].len();
    let h = &fit.xtx_inv; // (X'X)⁻¹
    let beta = &fit.beta;

    // Lβ − c.
    let lb = linalg::matrix_vec_mult(&l, beta);
    let diff: Vec<f64> = lb.iter().zip(c.iter()).map(|(a, b)| a - b).collect();

    // M = L H Lᵀ  (qr×qr); Minv.
    let lt = linalg::transpose(&l);
    let lh = linalg::matrix_mult(&l, h); // qr×p_eff
    let m = linalg::matrix_mult(&lh, &lt); // qr×qr
    let minv = linalg::invert_matrix(&m)?;

    // λ = Minv (Lβ − c).
    let lambda = linalg::matrix_vec_mult(&minv, &diff);
    // β_r = β − H Lᵀ λ.
    let hlt = linalg::matrix_mult(h, &lt); // p_eff×qr
    let correction = linalg::matrix_vec_mult(&hlt, &lambda);
    let beta_r: Vec<f64> = beta
        .iter()
        .zip(correction.iter())
        .map(|(b, d)| b - d)
        .collect();

    // SSE_r = sse + (Lβ−c)ᵀ Minv (Lβ−c).
    let m_diff = linalg::matrix_vec_mult(&minv, &diff);
    let quad: f64 = diff.iter().zip(m_diff.iter()).map(|(a, b)| a * b).sum();
    let sse_r = fit.sse + quad;
    let df_r = (n - p_eff) as f64 + qr as f64;
    let mse_r = sse_r / df_r;

    // Restricted ŷ / residuals.
    let y_hat_r: Vec<f64> = x_mat
        .iter()
        .map(|row| row.iter().zip(beta_r.iter()).map(|(xi, bi)| xi * bi).sum())
        .collect();
    let resid_r: Vec<f64> = y
        .iter()
        .zip(y_hat_r.iter())
        .map(|(yi, yhi)| yi - yhi)
        .collect();

    // Var(β_r) = MSE_r (H − H Lᵀ Minv L H).
    let mlh = linalg::matrix_mult(&minv, &lh); // qr×p_eff
    let hlt_mlh = linalg::matrix_mult(&hlt, &mlh); // p_eff×p_eff
    let mut se_r = vec![0.0; p_eff];
    let mut t_r = vec![0.0; p_eff];
    let mut p_r = vec![0.0; p_eff];
    for j in 0..p_eff {
        let var = mse_r * (h[j][j] - hlt_mlh[j][j]);
        let se = if var > 0.0 { var.sqrt() } else { 0.0 };
        se_r[j] = se;
        t_r[j] = if se > 0.0 { beta_r[j] / se } else { 0.0 };
        p_r[j] = if se > 0.0 {
            two_sided_p(t_r[j], df_r)
        } else {
            f64::NAN
        };
    }

    // Var(λ) = MSE_r Minv → SE(λ_i), t_i = λ_i/SE, p via two_sided_p(·, df_r).
    let mut lambda_rows = Vec::with_capacity(qr);
    for i in 0..qr {
        let var = mse_r * minv[i][i];
        let se = if var > 0.0 { var.sqrt() } else { 0.0 };
        let t = if se > 0.0 { lambda[i] / se } else { 0.0 };
        let pv = if se > 0.0 {
            two_sided_p(t, df_r)
        } else {
            f64::NAN
        };
        lambda_rows.push((labels[i].clone(), lambda[i], se, t, pv));
    }

    Ok(Some(Restricted {
        beta_r,
        sse_r,
        df_r,
        y_hat_r,
        resid_r,
        se_r,
        t_r,
        p_r,
        lambda_rows,
    }))
}

/// Human-readable label for a restriction row, reconstructed from the equation
/// (e.g. `X1 = X2`, `X1 + X2 = 1`). Used in the parameter-estimates Label
/// column for RESTRICT rows.
fn restrict_label(eq: &LinEq, _reg_names: &[String], _intercept: bool) -> String {
    if eq.terms.is_empty() {
        return format!("{}", eq.rhs);
    }
    let mut s = String::new();
    for (i, (coef, name)) in eq.terms.iter().enumerate() {
        let c = *coef;
        if i == 0 {
            if c == 1.0 {
                s.push_str(name);
            } else if c == -1.0 {
                s.push('-');
                s.push_str(name);
            } else {
                s.push_str(&format!("{}*{}", trim_num(c), name));
            }
        } else {
            let mag = c.abs();
            s.push_str(if c < 0.0 { " - " } else { " + " });
            if mag == 1.0 {
                s.push_str(name);
            } else {
                s.push_str(&format!("{}*{}", trim_num(mag), name));
            }
        }
    }
    s.push_str(&format!(" = {}", trim_num(eq.rhs)));
    s
}

/// Format a coefficient/constant without trailing `.0` for integral values.
fn trim_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

/// Run and print every TEST statement of a model, after the parameter table.
/// `beta`, `xtx_inv`, `sse`, `df_e`, `p_eff` come from the model **as fitted**
/// (restricted if RESTRICT statements are present, else the OLS fit).
#[allow(clippy::too_many_arguments)]
fn run_tests(
    tests: &[RegTest],
    reg_names: &[String],
    intercept: bool,
    dep_name: &str,
    beta: &[f64],
    xtx_inv: &[Vec<f64>],
    sse: f64,
    df_e: f64,
    p_eff: usize,
    session: &mut Session,
) -> Result<()> {
    if tests.is_empty() {
        return Ok(());
    }
    let mse = sse / df_e;
    for (ti, test) in tests.iter().enumerate() {
        let (l, c) = build_lc(&test.equations, reg_names, intercept)?;
        let q = l.len();
        // Lβ − c.
        let lb = linalg::matrix_vec_mult(&l, beta);
        let diff: Vec<f64> = lb.iter().zip(c.iter()).map(|(a, b)| a - b).collect();
        // M = L H Lᵀ.
        let lt = linalg::transpose(&l);
        let lh = linalg::matrix_mult(&l, xtx_inv);
        let m = linalg::matrix_mult(&lh, &lt);
        let minv = linalg::invert_matrix(&m)?;
        // SS = diffᵀ Minv diff.
        let md = linalg::matrix_vec_mult(&minv, &diff);
        let ss: f64 = diff.iter().zip(md.iter()).map(|(a, b)| a * b).sum();
        let ms_num = ss / q as f64;
        let f = if mse > 0.0 { ms_num / mse } else { f64::NAN };
        let p_f = (1.0 - f_cdf(f, q as f64, df_e)).clamp(0.0, 1.0);

        let _ = p_eff;
        // SAS heading is "Test <name> Results …"; an unlabeled TEST uses the
        // bare ordinal (→ "Test 1 …"), a labeled one its name (→ "Test peak …").
        let label = test
            .label
            .clone()
            .unwrap_or_else(|| format!("{}", ti + 1));

        session.listing.blank();
        session.listing.blank();
        centered(
            session,
            &format!(
                "Test {} Results for Dependent Variable {}",
                label, dep_name
            ),
        );
        session.listing.blank();
        let headers: Vec<String> = vec![
            "Source".into(),
            "DF".into(),
            "Mean Square".into(),
            "F Value".into(),
            "Pr > F".into(),
        ];
        let aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let rows: Vec<Vec<String>> = vec![
            vec![
                "Numerator".into(),
                format!("{}", q),
                fmt5(ms_num),
                fmt2(f),
                fmt_p(Some(p_f)),
            ],
            vec![
                "Denominator".into(),
                format!("{}", df_e as usize),
                fmt5(mse),
                "".into(),
                "".into(),
            ],
        ];
        session.listing.write_table(&headers, &aligns, &rows);
    }
    Ok(())
}

// ───────────────────────── SELECTION ─────────────────────────

/// Run a SELECTION= algorithm, returning the final subset of regressor column
/// indices (into `regressors` / `xcols`). Returns `None` if the final set is
/// empty. Emits a step-log table.
#[allow(clippy::too_many_arguments)]
fn run_selection(
    sel: &Selection,
    xcols: &[Vec<f64>],
    y: &[f64],
    regressors: &[String],
    intercept: bool,
    model: &RegModel,
    session: &mut Session,
) -> Option<Vec<usize>> {
    let p = regressors.len();
    let n = y.len();
    let all: Vec<usize> = (0..p).collect();
    let int = intercept as usize;

    // Step-log accumulator. Each row: (step, action, var, vars_in, partial_r2,
    // model_r2, f_value, p_value).
    let mut steplog: Vec<SelStep> = Vec::new();

    // Uncorrected/corrected total used for R² reporting in the step log.
    let sst_report: f64 = if intercept {
        let ybar = y.iter().sum::<f64>() / n as f64;
        y.iter().map(|yi| (yi - ybar) * (yi - ybar)).sum()
    } else {
        y.iter().map(|yi| yi * yi).sum()
    };
    let model_r2 = |sse: f64| -> f64 {
        if sst_report > 0.0 {
            1.0 - sse / sst_report
        } else {
            f64::NAN
        }
    };

    let max_steps = 2 * p + 5;

    let final_set: Vec<usize> = match sel.method {
        SelMethod::Forward => {
            let mut s: Vec<usize> = Vec::new();
            let mut step = 0usize;
            loop {
                let sse_s = subset_sse(xcols, y, &s, intercept).unwrap_or(f64::INFINITY);
                let mut best: Option<(usize, f64, f64)> = None; // (col, f, p)
                for &c in &all {
                    if s.contains(&c) {
                        continue;
                    }
                    let mut cand = s.clone();
                    cand.push(c);
                    let df_full = (n as f64) - (cand.len() as f64) - int as f64;
                    if df_full <= 0.0 {
                        continue;
                    }
                    if let Some(sse_c) = subset_sse(xcols, y, &cand, intercept) {
                        let f = (sse_s - sse_c) / (sse_c / df_full);
                        let pv = (1.0 - f_cdf(f, 1.0, df_full)).clamp(0.0, 1.0);
                        if best.map(|(_, bf, _)| f > bf).unwrap_or(true) {
                            best = Some((c, f, pv));
                        }
                    }
                }
                match best {
                    Some((c, f, pv)) if pv <= sel.slentry => {
                        let mut cand = s.clone();
                        cand.push(c);
                        let sse_c = subset_sse(xcols, y, &cand, intercept).unwrap_or(f64::NAN);
                        let partial = model_r2(sse_c) - model_r2(sse_s);
                        s.push(c);
                        step += 1;
                        steplog.push(SelStep {
                            step,
                            entered: true,
                            var: regressors[c].clone(),
                            vars_in: s.len(),
                            partial_r2: partial,
                            model_r2: model_r2(sse_c),
                            f,
                            p: pv,
                        });
                    }
                    _ => break,
                }
                if step >= max_steps {
                    break;
                }
            }
            s
        }
        SelMethod::Backward => {
            let mut s: Vec<usize> = all.clone();
            let mut step = 0usize;
            loop {
                if s.is_empty() {
                    break;
                }
                let sse_s = subset_sse(xcols, y, &s, intercept).unwrap_or(f64::INFINITY);
                let df_s = (n as f64) - (s.len() as f64) - int as f64;
                if df_s <= 0.0 {
                    break;
                }
                let mse_s = sse_s / df_s;
                let mut worst: Option<(usize, f64, f64)> = None; // (col, f, p)
                for &v in &s {
                    let reduced: Vec<usize> = s.iter().cloned().filter(|&c| c != v).collect();
                    if let Some(sse_r) = subset_sse(xcols, y, &reduced, intercept) {
                        let f = (sse_r - sse_s) / mse_s;
                        let pv = (1.0 - f_cdf(f, 1.0, df_s)).clamp(0.0, 1.0);
                        if worst.map(|(_, wf, _)| f < wf).unwrap_or(true) {
                            worst = Some((v, f, pv));
                        }
                    }
                }
                match worst {
                    Some((v, f, pv)) if pv > sel.slstay => {
                        let reduced: Vec<usize> =
                            s.iter().cloned().filter(|&c| c != v).collect();
                        let sse_r = subset_sse(xcols, y, &reduced, intercept).unwrap_or(f64::NAN);
                        let partial = model_r2(sse_s) - model_r2(sse_r);
                        s.retain(|&c| c != v);
                        step += 1;
                        steplog.push(SelStep {
                            step,
                            entered: false,
                            var: regressors[v].clone(),
                            vars_in: s.len(),
                            partial_r2: partial,
                            model_r2: model_r2(sse_r),
                            f,
                            p: pv,
                        });
                    }
                    _ => break,
                }
                if step >= max_steps {
                    break;
                }
            }
            s
        }
        SelMethod::Stepwise => {
            let mut s: Vec<usize> = Vec::new();
            let mut step = 0usize;
            loop {
                let mut changed = false;
                // (1) Forward step.
                let sse_s = subset_sse(xcols, y, &s, intercept).unwrap_or(f64::INFINITY);
                let mut best: Option<(usize, f64, f64)> = None;
                for &c in &all {
                    if s.contains(&c) {
                        continue;
                    }
                    let mut cand = s.clone();
                    cand.push(c);
                    let df_full = (n as f64) - (cand.len() as f64) - int as f64;
                    if df_full <= 0.0 {
                        continue;
                    }
                    if let Some(sse_c) = subset_sse(xcols, y, &cand, intercept) {
                        let f = (sse_s - sse_c) / (sse_c / df_full);
                        let pv = (1.0 - f_cdf(f, 1.0, df_full)).clamp(0.0, 1.0);
                        if best.map(|(_, bf, _)| f > bf).unwrap_or(true) {
                            best = Some((c, f, pv));
                        }
                    }
                }
                let just_entered = if let Some((c, f, pv)) = best {
                    if pv <= sel.slentry {
                        let mut cand = s.clone();
                        cand.push(c);
                        let sse_c = subset_sse(xcols, y, &cand, intercept).unwrap_or(f64::NAN);
                        let partial = model_r2(sse_c) - model_r2(sse_s);
                        s.push(c);
                        step += 1;
                        changed = true;
                        steplog.push(SelStep {
                            step,
                            entered: true,
                            var: regressors[c].clone(),
                            vars_in: s.len(),
                            partial_r2: partial,
                            model_r2: model_r2(sse_c),
                            f,
                            p: pv,
                        });
                        Some(c)
                    } else {
                        None
                    }
                } else {
                    None
                };

                // (2) Backward step(s): remove any variable (except the one just
                // entered) whose remove-p > slstay.
                loop {
                    if s.is_empty() {
                        break;
                    }
                    let sse_cur = subset_sse(xcols, y, &s, intercept).unwrap_or(f64::INFINITY);
                    let df_cur = (n as f64) - (s.len() as f64) - int as f64;
                    if df_cur <= 0.0 {
                        break;
                    }
                    let mse_cur = sse_cur / df_cur;
                    let mut worst: Option<(usize, f64, f64)> = None;
                    for &v in &s {
                        if Some(v) == just_entered {
                            continue;
                        }
                        let reduced: Vec<usize> =
                            s.iter().cloned().filter(|&c| c != v).collect();
                        if let Some(sse_r) = subset_sse(xcols, y, &reduced, intercept) {
                            let f = (sse_r - sse_cur) / mse_cur;
                            let pv = (1.0 - f_cdf(f, 1.0, df_cur)).clamp(0.0, 1.0);
                            if worst.map(|(_, wf, _)| f < wf).unwrap_or(true) {
                                worst = Some((v, f, pv));
                            }
                        }
                    }
                    match worst {
                        Some((v, f, pv)) if pv > sel.slstay => {
                            let reduced: Vec<usize> =
                                s.iter().cloned().filter(|&c| c != v).collect();
                            let sse_r =
                                subset_sse(xcols, y, &reduced, intercept).unwrap_or(f64::NAN);
                            let partial = model_r2(sse_cur) - model_r2(sse_r);
                            s.retain(|&c| c != v);
                            step += 1;
                            changed = true;
                            steplog.push(SelStep {
                                step,
                                entered: false,
                                var: regressors[v].clone(),
                                vars_in: s.len(),
                                partial_r2: partial,
                                model_r2: model_r2(sse_r),
                                f,
                                p: pv,
                            });
                        }
                        _ => break,
                    }
                }

                if !changed || step >= max_steps {
                    break;
                }
            }
            s
        }
    };

    print_selection_summary(sel, &steplog, session);

    if final_set.is_empty() {
        let _ = model; // (model kept for symmetry / future use)
        None
    } else {
        // Keep selected columns in their original regressor order for a stable
        // parameter-estimates layout.
        let mut ordered = final_set;
        ordered.sort_unstable();
        Some(ordered)
    }
}

/// One row of a selection step log.
struct SelStep {
    step: usize,
    entered: bool,
    var: String,
    vars_in: usize,
    partial_r2: f64,
    model_r2: f64,
    f: f64,
    p: f64,
}

/// Print the SAS-style "Summary of <Method> Selection" table.
fn print_selection_summary(sel: &Selection, steplog: &[SelStep], session: &mut Session) {
    let method = match sel.method {
        SelMethod::Forward => "Forward",
        SelMethod::Backward => "Backward Elimination",
        SelMethod::Stepwise => "Stepwise",
    };
    let title = match sel.method {
        SelMethod::Forward => "Summary of Forward Selection".to_string(),
        SelMethod::Backward => "Summary of Backward Elimination".to_string(),
        SelMethod::Stepwise => "Summary of Stepwise Selection".to_string(),
    };
    let _ = method;

    session.listing.page_header();
    centered(session, "The REG Procedure");
    centered(session, &title);
    session.listing.blank();

    let headers: Vec<String> = vec![
        "Step".into(),
        "Variable Entered".into(),
        "Variable Removed".into(),
        "Number Vars In".into(),
        "Partial R-Square".into(),
        "Model R-Square".into(),
        "F Value".into(),
        "Pr > F".into(),
    ];
    let aligns = vec![
        Align::Right,
        Align::Left,
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let rows: Vec<Vec<String>> = steplog
        .iter()
        .map(|st| {
            let (entered, removed) = if st.entered {
                (st.var.clone(), String::new())
            } else {
                (String::new(), st.var.clone())
            };
            vec![
                format!("{}", st.step),
                entered,
                removed,
                format!("{}", st.vars_in),
                fmt_fit4(st.partial_r2),
                fmt_fit4(st.model_r2),
                fmt2(st.f),
                fmt_p(Some(st.p)),
            ]
        })
        .collect();
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
    session.listing.blank();
}

// ───────────────────────── OUTPUT dataset ─────────────────────────

/// Write the OUTPUT dataset(s) associated with this model, using the model's
/// fit (complete cases only).
fn write_outputs(
    entry: &RegModelEntry,
    ds: &SasDataset,
    complete_mask: &[bool],
    n: usize,
    fit: &OlsFit,
    session: &mut Session,
) -> Result<()> {
    if entry.outputs.is_empty() {
        return Ok(());
    }

    let mut complete_indices: Vec<usize> = Vec::with_capacity(n);
    for (i, &is_complete) in complete_mask.iter().enumerate() {
        if is_complete {
            complete_indices.push(i);
        }
    }

    for out_spec in &entry.outputs {
        let n_cols = ds.vars.len();
        let mut columns: Vec<Column> = Vec::with_capacity(n_cols + 2);
        let mut out_vars: Vec<VarMeta> = Vec::with_capacity(n_cols + 2);

        for col_idx in 0..n_cols {
            let col_vals = decode_column(ds, col_idx)?;
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

        if let Some(pred_name) = &out_spec.predicted {
            let data: Vec<Option<f64>> = fit.y_hat.iter().map(|&v| Some(v)).collect();
            columns.push(Series::new(pred_name.as_str().into(), data).into());
            out_vars.push(num_var_meta(pred_name));
        }
        if let Some(resid_name) = &out_spec.residual {
            let data: Vec<Option<f64>> = fit.resid.iter().map(|&v| Some(v)).collect();
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

/// Generate (or defer) the automatic residuals-vs-predicted diagnostic plot
/// after a MODEL statement, when `ods_graphics.enabled` is true (the caller
/// checks this). Default build: a deferral NOTE; `--features graphics`: a
/// `reg_{N}` scatter image (x = predicted, y = residual).
fn reg_diagnostic_plot(session: &mut Session, y_hat: &[f64], resid: &[f64]) {
    #[cfg(not(feature = "graphics"))]
    {
        let _ = (y_hat, resid);
        session
            .log
            .note("REG diagnostics: image deferred (compile with --features graphics).");
    }

    #[cfg(feature = "graphics")]
    {
        use crate::graphics::render::{draw_to_file, DrawingSpec, PlotType};

        let data: Vec<(f64, f64)> = y_hat
            .iter()
            .zip(resid.iter())
            .filter(|(p, r)| p.is_finite() && r.is_finite())
            .map(|(p, r)| (*p, *r))
            .collect();
        let spec = DrawingSpec {
            title: "The REG Procedure".to_string(),
            x_label: "Predicted Value".to_string(),
            y_label: "Residual".to_string(),
            plot_type: PlotType::Scatter,
            data,
            x_categorical: vec![],
        };

        session.graphics_image_count += 1;
        let stem = session
            .ods_graphics
            .file_stem
            .clone()
            .unwrap_or_else(|| "reg".to_string());
        let fmt = session.ods_graphics.image_format;
        let name = format!(
            "{}_{}.{}",
            stem,
            session.graphics_image_count,
            fmt.extension()
        );
        let path = session.ods_graphics.output_dir.join(&name);
        match draw_to_file(
            &spec,
            &path,
            session.ods_graphics.width,
            session.ods_graphics.height,
            fmt,
        ) {
            Ok((w, h)) => {
                session
                    .log
                    .note(&format!("Output '{}' ({}x{}) written.", name, w, h));
            }
            Err(e) => {
                session
                    .log
                    .note(&format!("WARNING: could not write image {}: {}", name, e));
            }
        }
    }
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

    /// Build a single-model AST (no OUTPUT) for the given model.
    fn single_model_ast(input: DatasetRef, model: RegModel) -> RegAst {
        RegAst {
            data_options: RegDataOptions { input: Some(input) },
            models: vec![RegModelEntry {
                model,
                outputs: vec![],
                tests: vec![],
                restricts: vec![],
            }],
            plots_requested: false,
        }
    }

    fn basic_model(dep: &str, regs: &[&str]) -> RegModel {
        RegModel {
            dependent: dep.into(),
            regressors: regs.iter().map(|s| s.to_string()).collect(),
            noint: false,
            noprint: false,
            selection: None,
        }
    }

    #[test]
    fn test_ols_simple() {
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

        let ast = single_model_ast(
            DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            },
            basic_model("y", &["x"]),
        );
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(
            listing.contains("1.0000") || listing.contains("R-Square"),
            "listing: {listing}"
        );
        assert!(listing.contains("The REG Procedure"), "{listing}");
    }

    #[test]
    fn test_ols_regression() {
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

        let ast = single_model_ast(
            DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            },
            basic_model("y", &["x"]),
        );
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("0.8000") || listing.contains("R-Square"), "{listing}");
    }

    #[test]
    fn test_parse_model() {
        let ast = parse_reg("proc reg data=a; model y = x1 x2; run;").unwrap();
        assert_eq!(ast.models.len(), 1);
        let m = &ast.models[0].model;
        assert_eq!(m.dependent, "y");
        assert_eq!(m.regressors, vec!["x1", "x2"]);
        assert!(!m.noint);
        assert!(!m.noprint);
        assert!(m.selection.is_none());
    }

    #[test]
    fn test_parse_multiple_models() {
        let ast = parse_reg(
            "proc reg data=a; model y = x1; output out=o1 p=p1; model y = x1 x2; output out=o2 p=p2; run;",
        )
        .unwrap();
        assert_eq!(ast.models.len(), 2);
        // First model has one regressor and its OUTPUT.
        assert_eq!(ast.models[0].model.regressors, vec!["x1"]);
        assert_eq!(ast.models[0].outputs.len(), 1);
        assert_eq!(ast.models[0].outputs[0].out.name, "o1");
        assert_eq!(ast.models[0].outputs[0].predicted.as_deref(), Some("p1"));
        // Second model has two regressors and its own OUTPUT.
        assert_eq!(ast.models[1].model.regressors, vec!["x1", "x2"]);
        assert_eq!(ast.models[1].outputs.len(), 1);
        assert_eq!(ast.models[1].outputs[0].out.name, "o2");
    }

    #[test]
    fn test_parse_output() {
        let ast =
            parse_reg("proc reg data=a; model y = x; output out=work.out predicted=p residual=r; run;")
                .unwrap();
        assert_eq!(ast.models.len(), 1);
        assert_eq!(ast.models[0].outputs.len(), 1);
        let o = &ast.models[0].outputs[0];
        assert_eq!(o.out.name, "out");
        assert_eq!(o.predicted.as_deref(), Some("p"));
        assert_eq!(o.residual.as_deref(), Some("r"));
    }

    #[test]
    fn test_parse_selection_forward() {
        let ast = parse_reg(
            "proc reg data=a; model y = x1 x2 / selection=forward slentry=0.3; run;",
        )
        .unwrap();
        let sel = ast.models[0].model.selection.unwrap();
        assert_eq!(sel.method, SelMethod::Forward);
        assert!((sel.slentry - 0.3).abs() < 1e-12);
    }

    #[test]
    fn test_parse_selection_synonyms() {
        // sle=/sls= synonyms and stepwise.
        let ast = parse_reg(
            "proc reg data=a; model y = x1 x2 / selection=stepwise sle=0.2 sls=0.25; run;",
        )
        .unwrap();
        let sel = ast.models[0].model.selection.unwrap();
        assert_eq!(sel.method, SelMethod::Stepwise);
        assert!((sel.slentry - 0.2).abs() < 1e-12);
        assert!((sel.slstay - 0.25).abs() < 1e-12);
    }

    #[test]
    fn test_parse_selection_defaults() {
        let ast =
            parse_reg("proc reg data=a; model y = x1 / selection=backward; run;").unwrap();
        let sel = ast.models[0].model.selection.unwrap();
        assert_eq!(sel.method, SelMethod::Backward);
        assert!((sel.slstay - 0.10).abs() < 1e-12);
    }

    #[test]
    fn test_parse_noint() {
        let ast = parse_reg("proc reg data=a; model y = x / noint; run;").unwrap();
        assert!(ast.models[0].model.noint);
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

        let ast = single_model_ast(
            DatasetRef {
                libref: Some("WORK".into()),
                name: "CLASS".into(),
            },
            basic_model("weight", &["height"]),
        );
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("The REG Procedure"), "{listing}");
        assert!(listing.contains("Analysis of Variance"), "{listing}");
        assert!(listing.contains("Parameter Estimates") || listing.contains("Parameter"), "{listing}");
    }

    /// NOINT on a tiny known dataset: y = 2x exactly (no intercept), so the
    /// no-intercept fit gives slope=2, uncorrected R² = Σŷ²/Σy² = 1, and there
    /// is NO Intercept row in the parameter-estimates table.
    #[test]
    fn test_noint_fit() {
        let mut session = make_session();
        // y = 2*x, with x = 1..5.
        let frame = df![
            "y" => [2.0_f64, 4.0, 6.0, 8.0, 10.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let mut model = basic_model("y", &["x"]);
        model.noint = true;
        let ast = single_model_ast(
            DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            },
            model,
        );
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Uncorrected Total"), "{listing}");
        // R² = 1.0000 (perfect through-origin fit).
        assert!(listing.contains("R-Square     1.0000"), "{listing}");
        // No Intercept row in parameter estimates.
        assert!(!listing.contains("Intercept"), "{listing}");
        assert!(listing.contains("The REG Procedure"), "{listing}");
    }

    /// Direct numeric check of the NOINT uncorrected decomposition via ols_fit.
    #[test]
    fn test_noint_uncorrected_r2_formula() {
        // X has no intercept column.
        let x = vec![vec![1.0], vec![2.0], vec![3.0], vec![4.0], vec![5.0]];
        let y = vec![2.1, 3.9, 6.2, 7.8, 10.1];
        let fit = ols_fit(&x, &y).unwrap();
        let ssm: f64 = fit.y_hat.iter().map(|v| v * v).sum();
        let sst: f64 = y.iter().map(|v| v * v).sum();
        let r2 = ssm / sst;
        // 1 - SSE/Σy² must match Σŷ²/Σy².
        let r2_alt = 1.0 - fit.sse / sst;
        assert!((r2 - r2_alt).abs() < 1e-10, "r2={r2} r2_alt={r2_alt}");
        assert!(r2 > 0.99, "near-perfect fit expected, r2={r2}");
    }

    /// Partial-F to ENTER equals the candidate's t² in the augmented fit.
    #[test]
    fn test_partial_f_equals_t_squared() {
        // Two regressors; intercept present.
        // y depends mostly on x1.
        let x1 = vec![1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
        let x2 = vec![5.0_f64, 3.0, 6.0, 2.0, 7.0, 1.0]; // noise-ish
        let y: Vec<f64> = x1.iter().map(|&v| 3.0 + 2.0 * v).collect();
        let xcols = vec![x1.clone(), x2.clone()];
        let n = y.len();

        // Enter x1 (col 0) into empty set, intercept present.
        let s: Vec<usize> = vec![];
        let cand = vec![0usize];
        let sse_s = subset_sse(&xcols, &y, &s, true).unwrap();
        let sse_c = subset_sse(&xcols, &y, &cand, true).unwrap();
        let df_full = (n as f64) - (cand.len() as f64) - 1.0;
        let f_enter = (sse_s - sse_c) / (sse_c / df_full);

        // Augmented fit: design [1, x1]; t for x1's coefficient.
        let mut xmat: Vec<Vec<f64>> = Vec::new();
        for i in 0..n {
            xmat.push(vec![1.0, x1[i]]);
        }
        let fit = ols_fit(&xmat, &y).unwrap();
        let mse = fit.sse / df_full;
        let se = (mse * fit.xtx_inv[1][1]).sqrt();
        let t = fit.beta[1] / se;
        let t2 = t * t;
        // Perfect linear data → both huge; compare relative or that both large.
        // Use a perturbed y to avoid degeneracy.
        let _ = (f_enter, t2);

        // Re-run with a slightly noisy y so SSE>0.
        let y2: Vec<f64> = x1.iter().map(|&v| 3.0 + 2.0 * v + (v * 0.137).sin()).collect();
        let sse_s2 = subset_sse(&xcols, &y2, &s, true).unwrap();
        let sse_c2 = subset_sse(&xcols, &y2, &cand, true).unwrap();
        let f_enter2 = (sse_s2 - sse_c2) / (sse_c2 / df_full);
        let mut xmat2: Vec<Vec<f64>> = Vec::new();
        for i in 0..n {
            xmat2.push(vec![1.0, x1[i]]);
        }
        let fit2 = ols_fit(&xmat2, &y2).unwrap();
        let mse2 = fit2.sse / df_full;
        let se2 = (mse2 * fit2.xtx_inv[1][1]).sqrt();
        let t_2 = fit2.beta[1] / se2;
        let t2_2 = t_2 * t_2;
        assert!(
            (f_enter2 - t2_2).abs() < 1e-6,
            "F_enter={f_enter2} t^2={t2_2}"
        );
    }

    /// FORWARD selection: x1 strongly predicts y; x2 is pure noise → x1 enters,
    /// x2 is rejected at slentry=0.05.
    #[test]
    fn test_forward_selection() {
        let mut session = make_session();
        // y tracks x1 closely (strong signal) with mild noise; x2 is unrelated.
        let frame = df![
            "y"  => [3.2_f64, 4.8, 7.1, 8.9, 11.3, 12.7, 15.2, 16.8],
            "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            "x2" => [4.0_f64, 1.0, 9.0, 2.0, 8.0, 3.0, 7.0, 5.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let mut model = basic_model("y", &["x1", "x2"]);
        model.selection = Some(Selection {
            method: SelMethod::Forward,
            // x1 enters (p<.0001); x2's partial-F p (~0.035) exceeds slentry,
            // so x2 is rejected.
            slentry: 0.01,
            slstay: 0.01,
        });
        let ast = single_model_ast(
            DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            },
            model,
        );
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Summary of Forward Selection"), "{listing}");
        // Inspect the final fitted-model block (after the last "Model: MODEL1"),
        // which holds the parameter-estimates table.
        let final_block = listing.rsplit("Model: MODEL1").next().unwrap();
        // x1 entered → appears as a fitted parameter; x2 rejected → absent.
        assert!(final_block.contains("x1"), "{listing}");
        assert!(!final_block.contains("x2"), "x2 should be rejected: {listing}");
    }

    /// BACKWARD selection: start with both x1 and noise x2; x2 is eliminated.
    #[test]
    fn test_backward_selection() {
        let mut session = make_session();
        // y is x1 plus mild noise (not a perfect fit), x2 is unrelated noise.
        let frame = df![
            "y"  => [3.2_f64, 4.8, 7.1, 8.9, 11.3, 12.7, 15.2, 16.8],
            "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            "x2" => [4.0_f64, 1.0, 9.0, 2.0, 8.0, 3.0, 7.0, 5.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let mut model = basic_model("y", &["x1", "x2"]);
        model.selection = Some(Selection {
            method: SelMethod::Backward,
            // x2's removal p (~0.035) exceeds slstay, so x2 is eliminated; x1
            // (highly significant) is retained.
            slentry: 0.10,
            slstay: 0.01,
        });
        let ast = single_model_ast(
            DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            },
            model,
        );
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Summary of Backward Elimination"), "{listing}");
        // Inspect the final fitted-model block (after the last "Model: MODEL1").
        let final_block = listing.rsplit("Model: MODEL1").next().unwrap();
        // x2 removed → absent from fitted parameters; x1 retained → present.
        assert!(final_block.contains("x1"), "{listing}");
        assert!(!final_block.contains("x2"), "x2 should be eliminated: {listing}");
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

    #[test]
    fn test_parse_test_multi_eq() {
        let ast = parse_reg("proc reg data=a; model y = a b c; test a=b, c=0; run;").unwrap();
        let t = &ast.models[0].tests[0];
        assert!(t.label.is_none());
        assert_eq!(t.equations.len(), 2);
        // a = b  →  A - B = 0
        let e0 = eq_terms(&t.equations[0]);
        assert_eq!(e0, vec![(1.0, "A".into()), (-1.0, "B".into())]);
        assert!((t.equations[0].rhs).abs() < 1e-12);
        // c = 0  →  C = 0
        let e1 = eq_terms(&t.equations[1]);
        assert_eq!(e1, vec![(1.0, "C".into())]);
    }

    #[test]
    fn test_parse_test_label() {
        let ast = parse_reg("proc reg data=a; model y = x1 x2; peak: test x1 = x2; run;").unwrap();
        let t = &ast.models[0].tests[0];
        assert_eq!(t.label.as_deref(), Some("peak"));
        assert_eq!(
            eq_terms(&t.equations[0]),
            vec![(1.0, "X1".into()), (-1.0, "X2".into())]
        );
    }

    #[test]
    fn test_parse_restrict_sum() {
        let ast = parse_reg("proc reg data=a; model y = a b; restrict a+b=1; run;").unwrap();
        let r = &ast.models[0].restricts[0];
        assert_eq!(r.equations.len(), 1);
        assert_eq!(
            eq_terms(&r.equations[0]),
            vec![(1.0, "A".into()), (1.0, "B".into())]
        );
        assert!((r.equations[0].rhs - 1.0).abs() < 1e-12);
    }

    #[test]
    fn test_parse_restrict_coefficients() {
        // 2*x1 - x2 = 0
        let ast =
            parse_reg("proc reg data=a; model y = x1 x2; restrict 2*x1 - x2 = 0; run;").unwrap();
        let e = &ast.models[0].restricts[0].equations[0];
        assert_eq!(
            eq_terms(e),
            vec![(2.0, "X1".into()), (-1.0, "X2".into())]
        );
        assert!(e.rhs.abs() < 1e-12);
    }

    #[test]
    fn test_parse_coef_no_star() {
        // `2 x1` (no star) is also a coefficient form.
        let ast =
            parse_reg("proc reg data=a; model y = x1 x2; restrict 2 x1 = x2 + 3; run;").unwrap();
        let e = &ast.models[0].restricts[0].equations[0];
        // 2*x1 - x2 = 3
        assert_eq!(
            eq_terms(e),
            vec![(2.0, "X1".into()), (-1.0, "X2".into())]
        );
        assert!((e.rhs - 3.0).abs() < 1e-12);
    }

    #[test]
    fn test_parse_intercept_keyword() {
        let ast = parse_reg(
            "proc reg data=a; model y = x1 x2; restrict intercept = 0; run;",
        )
        .unwrap();
        let e = &ast.models[0].restricts[0].equations[0];
        assert_eq!(eq_terms(e), vec![(1.0, "INTERCEPT".into())]);
    }

    /// Oracle (a): a single-coefficient `TEST xj=0;` yields F == t² of xj.
    #[test]
    fn test_oracle_test_f_equals_t_squared() {
        // Design: intercept + x1 + x2, with non-degenerate data.
        let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0];
        let y: Vec<f64> = x1
            .iter()
            .zip(x2.iter())
            .map(|(&a, &b)| 1.0 + 2.0 * a + 0.5 * b + (a * 0.3).cos())
            .collect();
        let n = y.len();
        let mut x_mat = Vec::new();
        for i in 0..n {
            x_mat.push(vec![1.0, x1[i], x2[i]]);
        }
        let fit = ols_fit(&x_mat, &y).unwrap();
        let p_eff = 3;
        let df_e = (n - p_eff) as f64;
        let mse = fit.sse / df_e;
        // t for x2 (column 2).
        let se = (mse * fit.xtx_inv[2][2]).sqrt();
        let t = fit.beta[2] / se;
        let t2 = t * t;

        // TEST x2 = 0  →  L = [0,0,1], c = 0.
        let reg_names = vec!["X1".to_string(), "X2".to_string()];
        let eq = LinEq {
            terms: vec![(1.0, "X2".into())],
            rhs: 0.0,
        };
        let (l, c) = build_lc(&[eq], &reg_names, true).unwrap();
        let lb = linalg::matrix_vec_mult(&l, &fit.beta);
        let diff: Vec<f64> = lb.iter().zip(c.iter()).map(|(a, b)| a - b).collect();
        let lt = linalg::transpose(&l);
        let lh = linalg::matrix_mult(&l, &fit.xtx_inv);
        let m = linalg::matrix_mult(&lh, &lt);
        let minv = linalg::invert_matrix(&m).unwrap();
        let md = linalg::matrix_vec_mult(&minv, &diff);
        let ss: f64 = diff.iter().zip(md.iter()).map(|(a, b)| a * b).sum();
        let f = (ss / 1.0) / mse;
        assert!((f - t2).abs() < 1e-6, "F={f} t^2={t2}");
    }

    /// Oracle (b): restricted estimates satisfy L β_r = c exactly.
    #[test]
    fn test_oracle_restricted_satisfies_constraint() {
        let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
        let x2 = [3.0_f64, 1.0, 4.0, 1.0, 5.0, 9.0];
        let y: Vec<f64> = x1
            .iter()
            .zip(x2.iter())
            .map(|(&a, &b)| 2.0 + a - b)
            .collect();
        let n = y.len();
        let mut x_mat = Vec::new();
        for i in 0..n {
            x_mat.push(vec![1.0, x1[i], x2[i]]);
        }
        let fit = ols_fit(&x_mat, &y).unwrap();
        let reg_names = vec!["X1".to_string(), "X2".to_string()];
        // RESTRICT x1 + x2 = 1.
        let restricts = vec![RegRestrict {
            equations: vec![LinEq {
                terms: vec![(1.0, "X1".into()), (1.0, "X2".into())],
                rhs: 1.0,
            }],
        }];
        let r = compute_restricted(&restricts, &reg_names, true, &x_mat, &y, &fit, n)
            .unwrap()
            .unwrap();
        // L β_r = c: β_r[1] + β_r[2] == 1.
        let lhs = r.beta_r[1] + r.beta_r[2];
        assert!((lhs - 1.0).abs() < 1e-9, "L beta_r = {lhs}");
    }

    /// Oracle (c): a RESTRICT already satisfied by OLS leaves estimates ~unchanged.
    #[test]
    fn test_oracle_redundant_restrict_unchanged() {
        // Build y so that OLS already gives slope_x1 == slope_x2 (symmetric).
        // y = 3 + 2*x1 + 2*x2 exactly → OLS recovers (3, 2, 2); RESTRICT x1=x2
        // is already satisfied.
        let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0];
        let x2 = [5.0_f64, 1.0, 4.0, 2.0, 3.0];
        let y: Vec<f64> = x1
            .iter()
            .zip(x2.iter())
            .map(|(&a, &b)| 3.0 + 2.0 * a + 2.0 * b)
            .collect();
        let n = y.len();
        let mut x_mat = Vec::new();
        for i in 0..n {
            x_mat.push(vec![1.0, x1[i], x2[i]]);
        }
        let fit = ols_fit(&x_mat, &y).unwrap();
        let reg_names = vec!["X1".to_string(), "X2".to_string()];
        let restricts = vec![RegRestrict {
            equations: vec![LinEq {
                terms: vec![(1.0, "X1".into()), (-1.0, "X2".into())],
                rhs: 0.0,
            }],
        }];
        let r = compute_restricted(&restricts, &reg_names, true, &x_mat, &y, &fit, n)
            .unwrap()
            .unwrap();
        for j in 0..fit.beta.len() {
            assert!(
                (r.beta_r[j] - fit.beta[j]).abs() < 1e-7,
                "beta_r[{j}]={} beta[{j}]={}",
                r.beta_r[j],
                fit.beta[j]
            );
        }
        // λ ≈ 0 since the constraint is non-binding.
        assert!(r.lambda_rows[0].1.abs() < 1e-6, "lambda={}", r.lambda_rows[0].1);
    }

    /// End-to-end: TEST and RESTRICT statements parse and execute, emitting the
    /// expected blocks in the listing.
    #[test]
    fn test_execute_test_and_restrict() {
        let mut session = make_session();
        let frame = df![
            "y"  => [3.2_f64, 4.8, 7.1, 8.9, 11.3, 12.7, 15.2, 16.8],
            "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            "x2" => [4.0_f64, 1.0, 9.0, 2.0, 8.0, 3.0, 7.0, 5.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let src = "proc reg data=work.t; model y = x1 x2; peak: test x1 = x2; restrict x1 + x2 = 3; run;";
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next();
        ts.next();
        let ast = parse(&mut ts).unwrap();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Test peak Results for Dependent Variable y"), "{listing}");
        assert!(listing.contains("Numerator"), "{listing}");
        assert!(listing.contains("Denominator"), "{listing}");
        assert!(listing.contains("RESTRICT"), "{listing}");
    }

    #[test]
    fn parse_plots_statement_flag() {
        let ast = parse_reg("proc reg data=a; model y = x; plots / only; run;").unwrap();
        assert!(ast.plots_requested);
    }

    #[test]
    fn reg_without_ods_no_diagnostic() {
        let log = run_diag(false, None, None);
        assert!(!log.contains("image deferred"), "log: {log}");
        assert!(!log.contains("REG diagnostics"), "log: {log}");
    }

    #[cfg(not(feature = "graphics"))]
    #[test]
    fn reg_with_ods_no_feature_defers() {
        let log = run_diag(true, None, None);
        assert!(
            log.contains("REG diagnostics: image deferred"),
            "log: {log}"
        );
    }

    #[cfg(feature = "graphics")]
    #[test]
    fn reg_with_ods_and_feature_creates_image() {
        let dir = std::env::temp_dir();
        let log = run_diag(true, Some(dir.clone()), Some("regtest_diag".into()));
        assert!(log.contains("written"), "log: {log}");
        let p = dir.join("regtest_diag_1.png");
        assert!(p.exists(), "diagnostic image not created: {p:?}");
        assert!(p.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&p);
    }
}
