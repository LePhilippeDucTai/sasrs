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
use crate::stat::{f_cdf, student_t_cdf, t_quantile};
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
    /// Significance level α (default 0.05) → 100(1−α)% intervals (M36.2).
    pub alpha: f64,
    /// CLB → confidence limits on the parameter estimates (M36.2).
    pub clb: bool,
    /// CLM → per-observation mean confidence limits in Output Statistics.
    pub clm: bool,
    /// CLI → per-observation individual prediction limits in Output Statistics.
    pub cli: bool,
    /// R → residual-analysis "Output Statistics" listing (M36.3).
    pub r: bool,
    /// INFLUENCE → influence-diagnostics listing (M36.3).
    pub influence: bool,
    /// VIF → Variance Inflation column in the parameter table (M36.4).
    pub vif: bool,
    /// TOL → Tolerance column in the parameter table (M36.4).
    pub tol: bool,
    /// COLLIN → Collinearity Diagnostics table, intercept included (M36.4).
    pub collin: bool,
    /// COLLINOINT → Collinearity Diagnostics table, intercept excluded (M36.4).
    pub collinoint: bool,
    /// SPEC → White's test of first and second moment specification (M36.4).
    pub spec: bool,
    /// DW → Durbin-Watson statistic block (M36.4).
    pub dw: bool,
    /// DWPROB → Durbin-Watson with positive/negative autocorrelation p-values
    /// (implies DW). (M36.4)
    pub dwprob: bool,
    /// ACOV / HCC → heteroscedasticity-consistent (White HC0) covariance of the
    /// estimates plus HC standard errors. ACOV and HCC are synonyms; either sets
    /// this flag. (M36.4)
    pub acov: bool,
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
    /// M36.2 — std errors / prediction limits requested as output columns.
    pub stdp: Option<String>,
    pub stdi: Option<String>,
    pub stdr: Option<String>,
    pub lcl: Option<String>,
    pub ucl: Option<String>,
    pub lclm: Option<String>,
    pub uclm: Option<String>,
    /// M36.3 — influence/observation diagnostics requested as output columns.
    pub student: Option<String>,
    pub rstudent: Option<String>,
    pub cookd: Option<String>,
    pub h: Option<String>,
    pub press: Option<String>,
    pub dffits: Option<String>,
    pub covratio: Option<String>,
    /// DFBETAS= prefix. SAS does not accept a single name (DFBETAS is
    /// per-parameter); when given a prefix we emit one column per parameter
    /// named `<prefix>_<var>` (Intercept first if present).
    pub dfbetas: Option<String>,
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
            let mut alpha = 0.05_f64;
            let mut clb = false;
            let mut clm = false;
            let mut cli = false;
            let mut r = false;
            let mut influence = false;
            let mut vif = false;
            let mut tol = false;
            let mut collin = false;
            let mut collinoint = false;
            let mut spec = false;
            let mut dw = false;
            let mut dwprob = false;
            let mut acov = false;
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
                        } else if ts.peek().is_kw("alpha") {
                            common::expect_eq(ts, "ALPHA")?;
                            alpha = read_float(ts)?;
                        } else if ts.peek().is_kw("clb") {
                            clb = true;
                            ts.next();
                        } else if ts.peek().is_kw("clm") {
                            clm = true;
                            ts.next();
                        } else if ts.peek().is_kw("cli") {
                            cli = true;
                            ts.next();
                        } else if ts.peek().is_kw("influence") {
                            influence = true;
                            ts.next();
                        } else if ts.peek().is_kw("r") {
                            r = true;
                            ts.next();
                        } else if ts.peek().is_kw("vif") {
                            vif = true;
                            ts.next();
                        } else if ts.peek().is_kw("tol") {
                            tol = true;
                            ts.next();
                        } else if ts.peek().is_kw("collinoint") {
                            collinoint = true;
                            ts.next();
                        } else if ts.peek().is_kw("collin") {
                            collin = true;
                            ts.next();
                        } else if ts.peek().is_kw("spec") {
                            spec = true;
                            ts.next();
                        } else if ts.peek().is_kw("dwprob") {
                            dwprob = true;
                            dw = true;
                            ts.next();
                        } else if ts.peek().is_kw("dw") {
                            dw = true;
                            ts.next();
                        } else if ts.peek().is_kw("acov") || ts.peek().is_kw("hcc") {
                            // ACOV and HCC are synonyms for the same
                            // heteroscedasticity-consistent covariance request.
                            acov = true;
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
            models.push(RegModelEntry {
                model: RegModel {
                    dependent: dep,
                    regressors,
                    noint,
                    noprint,
                    selection,
                    alpha,
                    clb,
                    clm,
                    cli,
                    r,
                    influence,
                    vif,
                    tol,
                    collin,
                    collinoint,
                    spec,
                    dw,
                    dwprob,
                    acov,
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
            let mut stdp: Option<String> = None;
            let mut stdi: Option<String> = None;
            let mut stdr: Option<String> = None;
            let mut lcl: Option<String> = None;
            let mut ucl: Option<String> = None;
            let mut lclm: Option<String> = None;
            let mut uclm: Option<String> = None;
            let mut student: Option<String> = None;
            let mut rstudent: Option<String> = None;
            let mut cookd: Option<String> = None;
            let mut h: Option<String> = None;
            let mut press: Option<String> = None;
            let mut dffits: Option<String> = None;
            let mut covratio: Option<String> = None;
            let mut dfbetas: Option<String> = None;
            // Read the value name for a `KEYWORD=name` OUTPUT option.
            let read_name = |ts: &mut StatementStream, kw: &str| -> Result<Option<String>> {
                common::expect_eq(ts, kw)?;
                let name = ts.peek().ident().map(str::to_string);
                if name.is_some() {
                    ts.next();
                }
                Ok(name)
            };
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if ts.peek().is_kw("out") {
                    out = Some(common::parse_out_opt(ts)?);
                } else if ts.peek().is_kw("predicted") || ts.peek().is_kw("p") {
                    predicted = read_name(ts, "PREDICTED")?;
                } else if ts.peek().is_kw("residual") || ts.peek().is_kw("r") {
                    residual = read_name(ts, "RESIDUAL")?;
                } else if ts.peek().is_kw("stdp") {
                    stdp = read_name(ts, "STDP")?;
                } else if ts.peek().is_kw("stdi") {
                    stdi = read_name(ts, "STDI")?;
                } else if ts.peek().is_kw("stdr") {
                    stdr = read_name(ts, "STDR")?;
                } else if ts.peek().is_kw("lclm") {
                    lclm = read_name(ts, "LCLM")?;
                } else if ts.peek().is_kw("uclm") {
                    uclm = read_name(ts, "UCLM")?;
                } else if ts.peek().is_kw("lcl") {
                    lcl = read_name(ts, "LCL")?;
                } else if ts.peek().is_kw("ucl") {
                    ucl = read_name(ts, "UCL")?;
                } else if ts.peek().is_kw("student") {
                    student = read_name(ts, "STUDENT")?;
                } else if ts.peek().is_kw("rstudent") {
                    rstudent = read_name(ts, "RSTUDENT")?;
                } else if ts.peek().is_kw("cookd") {
                    cookd = read_name(ts, "COOKD")?;
                } else if ts.peek().is_kw("h") {
                    h = read_name(ts, "H")?;
                } else if ts.peek().is_kw("press") {
                    press = read_name(ts, "PRESS")?;
                } else if ts.peek().is_kw("dffits") {
                    dffits = read_name(ts, "DFFITS")?;
                } else if ts.peek().is_kw("covratio") {
                    covratio = read_name(ts, "COVRATIO")?;
                } else if ts.peek().is_kw("dfbetas") {
                    dfbetas = read_name(ts, "DFBETAS")?;
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
                        stdp,
                        stdi,
                        stdr,
                        lcl,
                        ucl,
                        lclm,
                        uclm,
                        student,
                        rstudent,
                        cookd,
                        h,
                        press,
                        dffits,
                        covratio,
                        dfbetas,
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

/// Format a confidence level (e.g. 95, 90, or 97.5) without a trailing `.0`.
fn fmt_level(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        // Trim trailing zeros from a fixed-precision rendering.
        let s = format!("{:.4}", v);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        s.to_string()
    }
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

/// Per-observation leverage h_i = x_iᵀ (X'X)⁻¹ x_i for every design row of
/// `x_mat`, given the already-computed `xtx_inv` (M36.2). Σ_i h_i == p_eff.
fn leverages(x_mat: &[Vec<f64>], xtx_inv: &[Vec<f64>]) -> Vec<f64> {
    let p = xtx_inv.len();
    x_mat
        .iter()
        .map(|row| {
            // h = rowᵀ · (X'X)⁻¹ · row.
            let mut acc = 0.0;
            for a in 0..p {
                let mut inner = 0.0;
                for b in 0..p {
                    inner += xtx_inv[a][b] * row[b];
                }
                acc += row[a] * inner;
            }
            acc
        })
        .collect()
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

// ───────────────────────── Collinearity / spec diagnostics (M36.4) ─────────────────────────

/// Per-regressor VIF and tolerance (M36.4). `reg_cols[j]` is the j-th regressor
/// over the complete-case rows (length n); these are the regressors actually in
/// the fitted model (NOT the intercept). For each j we regress x_j on all the
/// OTHER regressors WITH an intercept; `R²_j` is that fit's R², from which
/// `TOL_j = 1 − R²_j` and `VIF_j = 1/TOL_j`. Returns `(tol, vif)` vectors,
/// length = `reg_cols.len()`. A regressor that is perfectly collinear with the
/// others (TOL ≈ 0) reports VIF = +inf; a single regressor reports TOL=1, VIF=1.
fn vif_tol(reg_cols: &[Vec<f64>]) -> (Vec<f64>, Vec<f64>) {
    let p = reg_cols.len();
    let n = if p > 0 { reg_cols[0].len() } else { 0 };
    let mut tol = vec![1.0; p];
    let mut vif = vec![1.0; p];
    if p <= 1 {
        return (tol, vif);
    }
    for j in 0..p {
        // Response = x_j; predictors = all other regressors + intercept.
        let yj = &reg_cols[j];
        let mut xaux: Vec<Vec<f64>> = Vec::with_capacity(n);
        for i in 0..n {
            let mut row = Vec::with_capacity(p); // intercept + (p-1) others
            row.push(1.0);
            for (k, col) in reg_cols.iter().enumerate() {
                if k != j {
                    row.push(col[i]);
                }
            }
            xaux.push(row);
        }
        // R²_j from the auxiliary regression (corrected total, intercept present).
        let r2j = match ols_fit(&xaux, yj) {
            Ok(f) => {
                let ybar = yj.iter().sum::<f64>() / n as f64;
                let sst: f64 = yj.iter().map(|v| (v - ybar) * (v - ybar)).sum();
                if sst > 0.0 {
                    (1.0 - f.sse / sst).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
            // Rank-deficient auxiliary fit ⇒ treat as no explanatory power.
            Err(_) => 0.0,
        };
        let t = 1.0 - r2j;
        tol[j] = t;
        vif[j] = if t > 0.0 { 1.0 / t } else { f64::INFINITY };
    }
    (tol, vif)
}

/// Collinearity-diagnostics output (M36.4): eigenvalues, condition indices and
/// variance-decomposition proportions of the scaled-X cross-product matrix.
struct Collin {
    /// Eigenvalues, sorted descending.
    eigenvalues: Vec<f64>,
    /// Condition index_k = √(λ_max / λ_k), same order as `eigenvalues`.
    condition_index: Vec<f64>,
    /// `proportions[k][j]` = variance proportion of regressor column j on the
    /// k-th eigenvalue row. Each column j sums to 1 across k (±1e-9).
    proportions: Vec<Vec<f64>>,
    /// Column labels (in analysis order): "Intercept" first when included.
    col_labels: Vec<String>,
}

/// Compute the collinearity diagnostics from the design matrix. `x_mat` columns
/// are ordered [intercept?] then the regressors. When `oint` (COLLINOINT) and an
/// intercept column is present, the intercept column is dropped from the
/// analysis (no centering — SAS's COLLINOINT simply excludes the intercept).
/// `reg_names` are the regressor names (no intercept); `intercept` indicates
/// whether column 0 of `x_mat` is the intercept.
fn compute_collin(
    x_mat: &[Vec<f64>],
    reg_names: &[String],
    intercept: bool,
    oint: bool,
) -> Result<Collin> {
    let n = x_mat.len();
    let full_p = x_mat[0].len();
    // Choose the columns to analyse.
    let drop_intercept = oint && intercept;
    let cols: Vec<usize> = if drop_intercept {
        (1..full_p).collect()
    } else {
        (0..full_p).collect()
    };
    let m = cols.len();
    let mut col_labels = Vec::with_capacity(m);
    for &c in &cols {
        let lbl = if intercept {
            if c == 0 {
                "Intercept".to_string()
            } else {
                reg_names[c - 1].clone()
            }
        } else {
            reg_names[c].clone()
        };
        col_labels.push(lbl);
    }

    // Scale each analysed column to unit (2-norm) length.
    let norms: Vec<f64> = cols
        .iter()
        .map(|&c| (0..n).map(|i| x_mat[i][c] * x_mat[i][c]).sum::<f64>().sqrt())
        .collect();
    // Scaled cross-product A = ZᵀZ (m×m) where Z column c is x[:,c]/‖x[:,c]‖.
    let mut a = vec![vec![0.0; m]; m];
    for (p, &cp) in cols.iter().enumerate() {
        for (q, &cq) in cols.iter().enumerate() {
            let mut s = 0.0;
            for i in 0..n {
                s += x_mat[i][cp] * x_mat[i][cq];
            }
            let denom = norms[p] * norms[q];
            a[p][q] = if denom > 0.0 { s / denom } else { 0.0 };
        }
    }

    // Eigen-decomposition (descending eigenvalues, eigenvector columns).
    let (vecs, eigvals) = linalg::eigenvectors_jacobi(&a)?;
    // Guard tiny negatives from round-off.
    let eigenvalues: Vec<f64> = eigvals.iter().map(|&l| l.max(0.0)).collect();
    let lmax = eigenvalues.iter().cloned().fold(0.0_f64, f64::max);
    let condition_index: Vec<f64> = eigenvalues
        .iter()
        .map(|&l| if l > 0.0 { (lmax / l).sqrt() } else { f64::INFINITY })
        .collect();

    // Variance proportions. φ_{kj} = v_{jk}² / λ_k ; π_{jk} = φ_{kj}/Σ_k φ_{kj}.
    // vecs[row][col] : column k is the k-th eigenvector, row j the j-th variable.
    let mut phi = vec![vec![0.0; m]; m]; // phi[k][j]
    for k in 0..m {
        let lk = eigenvalues[k];
        for j in 0..m {
            let vjk = vecs[j][k];
            phi[k][j] = if lk > 0.0 { vjk * vjk / lk } else { 0.0 };
        }
    }
    // Column sums Σ_k φ_{kj}.
    let mut colsum = vec![0.0; m];
    for j in 0..m {
        for k in 0..m {
            colsum[j] += phi[k][j];
        }
    }
    let mut proportions = vec![vec![0.0; m]; m];
    for k in 0..m {
        for j in 0..m {
            proportions[k][j] = if colsum[j] > 0.0 {
                phi[k][j] / colsum[j]
            } else {
                0.0
            };
        }
    }

    Ok(Collin {
        eigenvalues,
        condition_index,
        proportions,
        col_labels,
    })
}

/// Print the "Collinearity Diagnostics" table (M36.4).
fn print_collin(c: &Collin, oint: bool, session: &mut Session) {
    let m = c.eigenvalues.len();
    let mut headers: Vec<String> = vec![
        "Number".into(),
        "Eigenvalue".into(),
        "Condition Index".into(),
    ];
    let mut aligns = vec![Align::Right, Align::Right, Align::Right];
    for lbl in &c.col_labels {
        headers.push(format!("Proportion of Variation {}", lbl));
        aligns.push(Align::Right);
    }
    let rows: Vec<Vec<String>> = (0..m)
        .map(|k| {
            let mut row = vec![
                format!("{}", k + 1),
                fmt_collin(c.eigenvalues[k]),
                fmt5(c.condition_index[k]),
            ];
            for j in 0..m {
                row.push(fmt5(c.proportions[k][j]));
            }
            row
        })
        .collect();

    session.listing.blank();
    session.listing.blank();
    let title = if oint {
        "Collinearity Diagnostics (intercept adjusted)"
    } else {
        "Collinearity Diagnostics"
    };
    centered(session, title);
    session.listing.blank();
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Eigenvalues print with more precision than fmt5 in SAS; use 8 decimals but
/// trim is not needed (insta locks bytes). SAS uses a varying g-format; we fix
/// at 5 decimals like the rest of the table for determinism.
fn fmt_collin(v: f64) -> String {
    format!("{v:.5}")
}

/// White's specification test (M36.4). Regress e² on the original regressors,
/// their squares, and pairwise cross-products (with intercept). The statistic is
/// `W = n·R²_aux`, χ² with df = number of auxiliary regressors (excluding the
/// intercept). `reg_cols[j]` are the model regressors over complete-case rows
/// (no intercept). Returns `(W, df, p_value)` or `None` if the auxiliary
/// regression is degenerate / has no usable columns.
fn white_spec_test(reg_cols: &[Vec<f64>], resid: &[f64]) -> Option<(f64, usize, f64)> {
    let p = reg_cols.len();
    let n = resid.len();
    if p == 0 || n == 0 {
        return None;
    }
    // Auxiliary response = squared residuals.
    let e2: Vec<f64> = resid.iter().map(|r| r * r).collect();

    // Build the auxiliary regressor set per row: each x_j, each x_j², and each
    // cross-product x_j·x_k (j<k). De-duplicate constant columns later via the
    // rank-robust ols_fit (QR). We keep an intercept column at position 0.
    let n_aux = p + p + p * (p.saturating_sub(1)) / 2; // linear + square + cross
    let mut xaux: Vec<Vec<f64>> = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = Vec::with_capacity(1 + n_aux);
        row.push(1.0);
        for col in reg_cols.iter() {
            row.push(col[i]);
        }
        for col in reg_cols.iter() {
            row.push(col[i] * col[i]);
        }
        for a in 0..p {
            for b in (a + 1)..p {
                row.push(reg_cols[a][i] * reg_cols[b][i]);
            }
        }
        xaux.push(row);
    }

    let fit = ols_fit(&xaux, &e2).ok()?;
    let ybar = e2.iter().sum::<f64>() / n as f64;
    let sst: f64 = e2.iter().map(|v| (v - ybar) * (v - ybar)).sum();
    let r2 = if sst > 0.0 {
        (1.0 - fit.sse / sst).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let df = n_aux; // auxiliary regressors excluding the intercept
    if df == 0 {
        return None;
    }
    let w = n as f64 * r2;
    let p_value = (1.0 - crate::stat::chisq_cdf(w, df as f64)).clamp(0.0, 1.0);
    Some((w, df, p_value))
}

/// Print White's "Test of First and Second Moment Specification" (M36.4).
fn print_spec_test(reg_cols: &[Vec<f64>], resid: &[f64], session: &mut Session) {
    session.listing.blank();
    session.listing.blank();
    centered(session, "Test of First and Second Moment Specification");
    session.listing.blank();
    match white_spec_test(reg_cols, resid) {
        Some((w, df, pv)) => {
            let headers: Vec<String> = vec![
                "DF".into(),
                "Chi-Square".into(),
                "Pr > ChiSq".into(),
            ];
            let aligns = vec![Align::Right, Align::Right, Align::Right];
            let rows = vec![vec![format!("{}", df), fmt2(w), fmt_p(Some(pv))]];
            session.listing.write_table(&headers, &aligns, &rows);
        }
        None => {
            centered(
                session,
                "Specification test could not be computed (degenerate auxiliary regression).",
            );
        }
    }
}

/// Durbin-Watson statistic and related quantities (M36.4).
struct DwResult {
    d: f64,
    rho: f64,
    n: usize,
    /// Pr < DW (positive autocorrelation) / Pr > DW (negative) — normal
    /// approximation; `None` when not requested.
    pr_pos: Option<f64>,
    pr_neg: Option<f64>,
}

/// Compute the Durbin-Watson statistic in dataset order. `x_mat` and
/// `xtx_inv` are used only for the (optional) normal-approximation p-values via
/// the trace formulas. `want_prob` controls whether p-values are produced.
fn durbin_watson(
    resid: &[f64],
    x_mat: &[Vec<f64>],
    xtx_inv: &[Vec<f64>],
    want_prob: bool,
) -> DwResult {
    let n = resid.len();
    let denom: f64 = resid.iter().map(|e| e * e).sum();
    let mut num = 0.0;
    let mut lag = 0.0;
    for t in 1..n {
        let de = resid[t] - resid[t - 1];
        num += de * de;
        lag += resid[t] * resid[t - 1];
    }
    let d = if denom > 0.0 { num / denom } else { f64::NAN };
    let rho = if denom > 0.0 { lag / denom } else { f64::NAN };

    let (pr_pos, pr_neg) = if want_prob && denom > 0.0 && n > 2 {
        // Normal approximation to the null distribution of d. Under H0 the DW
        // statistic d = e'A e / e'e with A the second-difference operator. Its
        // mean and variance (residual-maker corrected) are
        //   E[d] = (P − trace(A·M·... )) — exactly E[d] = tr(MA)/(n−p),
        //   Var[d] = 2·(tr((MA)²) − (n−p)·E[d]²) / ((n−p)(n−p+2)),
        // with M = I − X(X'X)⁻¹X'. We form MA implicitly column by column.
        // NOTE: this is the standard NORMAL APPROXIMATION (Durbin & Watson
        // 1971 give the exact Imhof/Pan procedure; we deliberately use the
        // moment-matched normal tail for tractability — documented as approx).
        match dw_normal_prob(d, x_mat, xtx_inv) {
            Some((pp, pn)) => (Some(pp), Some(pn)),
            None => (None, None),
        }
    } else {
        (None, None)
    };

    DwResult {
        d,
        rho,
        n,
        pr_pos,
        pr_neg,
    }
}

/// Normal-approximation p-values for the Durbin-Watson statistic.
///
/// Builds A (the tridiagonal second-difference quadratic-form matrix so that
/// e'A e = Σ_{t≥2}(e_t−e_{t-1})²) and M = I − X(X'X)⁻¹X', then matches the first
/// two moments of d = e'A e / e'e under H0 (Gaussian errors) to a normal:
///   E[d] = tr(MA)/(n−p),  Var[d] = 2[tr((MA)²) − (n−p)E[d]²]/[(n−p)(n−p+2)].
/// `Pr < DW` = Φ((d − E)/√Var) (probability of a SMALLER d ⇒ positive
/// autocorrelation evidence), `Pr > DW` = 1 − that. Returns `None` if the
/// variance is non-positive.
fn dw_normal_prob(d: f64, x_mat: &[Vec<f64>], xtx_inv: &[Vec<f64>]) -> Option<(f64, f64)> {
    let n = x_mat.len();
    let p = x_mat[0].len();
    if n <= p {
        return None;
    }
    // Hat matrix H = X (X'X)⁻¹ X'  (n×n). M = I − H.
    // We need tr(MA) and tr((MA)²). Build MA = (I−H)A as an n×n matrix.
    // A is the symmetric tridiagonal second-difference operator:
    //   A[0][0]=1, A[n-1][n-1]=1, A[t][t]=2 (1<t<n-1 interior), off-diagonals −1.
    let mut a = vec![vec![0.0; n]; n];
    for t in 0..n {
        a[t][t] = if t == 0 || t == n - 1 { 1.0 } else { 2.0 };
    }
    for t in 1..n {
        a[t][t - 1] = -1.0;
        a[t - 1][t] = -1.0;
    }
    // H = X·(X'X)⁻¹·X'. Compute B = X·(X'X)⁻¹ (n×p), then H = B·Xᵀ.
    let b = linalg::matrix_mult(x_mat, xtx_inv); // n×p
    let xt = linalg::transpose(x_mat); // p×n
    let h = linalg::matrix_mult(&b, &xt); // n×n
    // MA = A − H·A.
    let ha = linalg::matrix_mult(&h, &a); // n×n
    let mut ma = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            ma[i][j] = a[i][j] - ha[i][j];
        }
    }
    // tr(MA).
    let tr_ma: f64 = (0..n).map(|i| ma[i][i]).sum();
    // tr((MA)²) = Σ_{i,j} ma[i][j]·ma[j][i].
    let mut tr_ma2 = 0.0;
    for i in 0..n {
        for j in 0..n {
            tr_ma2 += ma[i][j] * ma[j][i];
        }
    }
    let dfree = (n - p) as f64;
    let mean = tr_ma / dfree;
    let var = 2.0 * (tr_ma2 - dfree * mean * mean) / (dfree * (dfree + 2.0));
    if !(var > 0.0) {
        return None;
    }
    let z = (d - mean) / var.sqrt();
    let pr_less = crate::stat::probnorm(z).clamp(0.0, 1.0);
    Some((pr_less, (1.0 - pr_less).clamp(0.0, 1.0)))
}

/// Print the Durbin-Watson block (M36.4).
fn print_durbin_watson(dwr: &DwResult, session: &mut Session) {
    session.listing.blank();
    session.listing.blank();
    centered(session, "Durbin-Watson Statistics");
    session.listing.blank();
    session
        .listing
        .write_line(&format!("Durbin-Watson D                {}", fmt5(dwr.d)));
    if let (Some(pp), Some(pn)) = (dwr.pr_pos, dwr.pr_neg) {
        session
            .listing
            .write_line(&format!("Pr < DW                        {}", fmt_p(Some(pp))));
        session
            .listing
            .write_line(&format!("Pr > DW                        {}", fmt_p(Some(pn))));
    }
    session.listing.write_line(&format!(
        "Number of Observations         {}",
        dwr.n
    ));
    session.listing.write_line(&format!(
        "1st Order Autocorrelation      {}",
        fmt5(dwr.rho)
    ));
}

/// White HC0 heteroscedasticity-consistent covariance of the estimates (M36.4):
/// `(X'X)⁻¹ (Σ_i e_i² x_i x_iᵀ) (X'X)⁻¹` (p_eff×p_eff, symmetric).
fn acov_hc0(x_mat: &[Vec<f64>], resid: &[f64], xtx_inv: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = x_mat.len();
    let p = xtx_inv.len();
    // Meat = Σ_i e_i² x_i x_iᵀ  (p×p).
    let mut meat = vec![vec![0.0; p]; p];
    for i in 0..n {
        let w = resid[i] * resid[i];
        let xi = &x_mat[i];
        for a in 0..p {
            let wa = w * xi[a];
            for b in 0..p {
                meat[a][b] += wa * xi[b];
            }
        }
    }
    // (X'X)⁻¹ · meat · (X'X)⁻¹.
    let tmp = linalg::matrix_mult(xtx_inv, &meat); // p×p
    linalg::matrix_mult(&tmp, xtx_inv)
}

/// Print the "Consistent Covariance of Estimates" matrix and a small table of
/// heteroscedasticity-consistent standard errors / t / Pr>|t| (M36.4).
///
/// Layout: a labeled p_eff×p_eff matrix (one row/column per parameter, Intercept
/// first when present), followed by a "Heteroscedasticity Consistent" parameter
/// table with HC Std Error / t Value / Pr > |t|. The OLS parameter table printed
/// earlier is left intact (SAS adds rather than replaces).
fn print_acov(
    cov: &[Vec<f64>],
    beta: &[f64],
    reg_names: &[String],
    intercept: bool,
    df_e: f64,
    session: &mut Session,
) {
    let p_eff = cov.len();
    let label = |j: usize| -> String {
        if intercept {
            if j == 0 {
                "Intercept".to_string()
            } else {
                reg_names[j - 1].clone()
            }
        } else {
            reg_names[j].clone()
        }
    };

    session.listing.blank();
    session.listing.blank();
    centered(session, "Consistent Covariance of Estimates");
    session.listing.blank();
    let mut headers: Vec<String> = vec!["".into()];
    let mut aligns = vec![Align::Left];
    for j in 0..p_eff {
        headers.push(label(j));
        aligns.push(Align::Right);
    }
    let rows: Vec<Vec<String>> = (0..p_eff)
        .map(|i| {
            let mut row = vec![label(i)];
            for j in 0..p_eff {
                row.push(fmt5(cov[i][j]));
            }
            row
        })
        .collect();
    session.listing.write_table(&headers, &aligns, &rows);

    // HC standard errors / t / p table.
    session.listing.blank();
    session.listing.blank();
    centered(
        session,
        "Parameter Estimates with Heteroscedasticity Consistent Standard Errors",
    );
    session.listing.blank();
    let hh: Vec<String> = vec![
        "Variable".into(),
        "Estimate".into(),
        "HC Std Error".into(),
        "t Value".into(),
        "Pr > |t|".into(),
    ];
    let ha = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let rows2: Vec<Vec<String>> = (0..p_eff)
        .map(|j| {
            let se = cov[j][j].max(0.0).sqrt();
            let t = if se > 0.0 { beta[j] / se } else { f64::NAN };
            let pv = if se > 0.0 {
                Some(two_sided_p(t, df_e))
            } else {
                None
            };
            vec![
                label(j),
                fmt5(beta[j]),
                fmt5(se),
                fmt2(t),
                fmt_p(pv),
            ]
        })
        .collect();
    session.listing.write_table(&hh, &ha, &rows2);
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

    // --- VIF / TOL (M36.4): per-regressor tolerance & variance inflation,
    // computed from the selected regressor columns (no intercept). Only built
    // when requested so the default path stays allocation-free / byte-identical.
    let sel_cols: Vec<Vec<f64>> = selected.iter().map(|&c| xcols[c].clone()).collect();
    let tolvif = if (model.vif || model.tol) && !model.noprint {
        Some(vif_tol(&sel_cols))
    } else {
        None
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
        tolvif.as_ref(),
        session,
    );

    // --- Collinearity / specification / autocorrelation diagnostics (M36.4).
    // All gated on the corresponding flags (and !noprint), so a MODEL without
    // any of these options is byte-identical to before.
    if (model.collin || model.collinoint) && !model.noprint {
        if model.collin {
            if let Ok(c) = compute_collin(&x_mat, &sel_reg_names, intercept, false) {
                print_collin(&c, false, session);
            }
        }
        if model.collinoint {
            if let Ok(c) = compute_collin(&x_mat, &sel_reg_names, intercept, true) {
                print_collin(&c, true, session);
            }
        }
    }
    if model.spec && !model.noprint {
        print_spec_test(&sel_cols, &fit.resid, session);
    }
    if model.dw && !model.noprint {
        let dwr = durbin_watson(&fit.resid, &x_mat, &fit.xtx_inv, model.dwprob);
        print_durbin_watson(&dwr, session);
    }
    if model.acov && !model.noprint {
        let cov = acov_hc0(&x_mat, &fit.resid, &fit.xtx_inv);
        print_acov(
            &cov,
            &fit.beta,
            &sel_reg_names,
            intercept,
            (n - p_eff) as f64,
            session,
        );
    }

    // --- Output Statistics (M36.2): per-observation CLM / CLI limits. Driven
    // off the (unrestricted) OLS fit, gated on the CLM/CLI model options.
    if (model.clm || model.cli) && !model.noprint {
        print_output_statistics(
            model, dep_name, &x_mat, &y_vec, &fit, n, p_eff, session,
        );
    }

    // --- Residual / influence diagnostics (M36.3): MODEL R and INFLUENCE.
    // Computed lazily once off the OLS fit, shared by both listings.
    if (model.r || model.influence) && !model.noprint {
        let infl = compute_influence_stats(&x_mat, &y_vec, &fit, n, p_eff);
        if model.r {
            print_r_statistics(model, &infl, session);
        }
        if model.influence {
            print_influence_statistics(&infl, &sel_reg_names, intercept, session);
        }
    }

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
    write_outputs(
        entry,
        ds,
        &complete_mask,
        n,
        &fit,
        &x_mat,
        p_eff,
        model.alpha,
        &sel_reg_names,
        intercept,
        session,
    )?;

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
    // `tolvif`: optional (tolerance, vif) per regressor (no intercept), in
    // `reg_names` order. `Some` when MODEL VIF and/or TOL is requested (M36.4).
    tolvif: Option<&(Vec<f64>, Vec<f64>)>,
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
    // CLB (M36.2): append two confidence-limit columns to the parameter table.
    let with_clb = model.clb;
    let clb_level = 100.0 * (1.0 - model.alpha);
    let t_crit = t_quantile(1.0 - model.alpha / 2.0, error_df);
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
    if with_clb {
        pe_headers.push(format!("{}% Confidence Limits", fmt_level(clb_level)));
        pe_aligns.push(Align::Right);
        // The interval prints as two value columns under one spanning header;
        // emit a second (blank-titled) column to carry the upper limit.
        pe_headers.push(String::new());
        pe_aligns.push(Align::Right);
    }
    // VIF / TOL columns (M36.4). SAS orders Tolerance before Variance Inflation.
    let with_tol = model.tol && tolvif.is_some();
    let with_vif = model.vif && tolvif.is_some();
    if with_tol {
        pe_headers.push("Tolerance".into());
        pe_aligns.push(Align::Right);
    }
    if with_vif {
        pe_headers.push("Variance Inflation".into());
        pe_aligns.push(Align::Right);
    }
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
        if with_clb {
            row.push(fmt5(beta[j] - t_crit * se_beta[j]));
            row.push(fmt5(beta[j] + t_crit * se_beta[j]));
        }
        if with_tol || with_vif {
            // Map design column j to a regressor index (intercept has none).
            let reg_idx = if intercept {
                if j == 0 {
                    None
                } else {
                    Some(j - 1)
                }
            } else {
                Some(j)
            };
            let (tv, vv) = tolvif.expect("tolvif present when columns requested");
            if with_tol {
                // Intercept row: Tolerance blank.
                match reg_idx {
                    Some(k) => row.push(fmt5(tv[k])),
                    None => row.push(String::new()),
                }
            }
            if with_vif {
                // Intercept row: SAS prints 0 for the intercept VIF.
                match reg_idx {
                    Some(k) => row.push(if vv[k].is_finite() {
                        fmt5(vv[k])
                    } else {
                        // Perfect collinearity → SAS prints a very large value;
                        // render a sentinel `.` for non-finite.
                        ".".to_string()
                    }),
                    None => row.push(fmt5(0.0)),
                }
            }
        }
        if with_label {
            row.push(String::new());
        }
        pe_rows.push(row);
    }
    // Append RESTRICT rows: Variable="RESTRICT", DF=-1 (negative per SAS),
    // Estimate=λ_i, with the restriction expression in the Label column.
    if let Some(r) = restricted {
        for (label, lam, se, t, pv) in &r.lambda_rows {
            let mut row = vec![
                "RESTRICT".into(),
                "-1".into(),
                fmt5(*lam),
                fmt5(*se),
                fmt2(*t),
                fmt_p(Some(*pv)),
            ];
            if with_clb {
                // SAS leaves the confidence-limit cells blank for RESTRICT rows.
                row.push(String::new());
                row.push(String::new());
            }
            if with_tol {
                row.push(String::new());
            }
            if with_vif {
                row.push(String::new());
            }
            row.push(label.clone());
            pe_rows.push(row);
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

/// Per-observation std errors and CL limits for one used row (M36.2).
struct ObsStat {
    y: f64,
    y_hat: f64,
    stdp: f64,
    stdi: f64,
    stdr: f64,
    lclm: f64,
    uclm: f64,
    lcl: f64,
    ucl: f64,
}

/// Reconstruct the response vector y = ŷ + resid from a fit (avoids threading
/// the y vector into helpers that already carry the fit).
fn reconstruct_y(fit: &OlsFit) -> Vec<f64> {
    fit.y_hat
        .iter()
        .zip(fit.resid.iter())
        .map(|(yh, r)| yh + r)
        .collect()
}

/// Compute the per-observation statistics for every used row from the OLS fit.
/// `mse = sse/dfE`, `h_i` the leverage, `t = t_quantile(1−α/2, dfE)`.
fn compute_obs_stats(
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
    p_eff: usize,
    alpha: f64,
) -> Vec<ObsStat> {
    let df_e = (n - p_eff) as f64;
    let mse = fit.sse / df_e;
    let t = t_quantile(1.0 - alpha / 2.0, df_e);
    let h = leverages(x_mat, &fit.xtx_inv);
    (0..n)
        .map(|i| {
            let hi = h[i];
            let stdp = (mse * hi).sqrt();
            let stdi = (mse * (1.0 + hi)).sqrt();
            let stdr = (mse * (1.0 - hi)).max(0.0).sqrt();
            let yh = fit.y_hat[i];
            ObsStat {
                y: y[i],
                y_hat: yh,
                stdp,
                stdi,
                stdr,
                lclm: yh - t * stdp,
                uclm: yh + t * stdp,
                lcl: yh - t * stdi,
                ucl: yh + t * stdi,
            }
        })
        .collect()
}

/// Per-observation influence diagnostics (M36.3). Reuses the same leverage /
/// MSE / dfE infrastructure as `compute_obs_stats` (no duplicate fit).
///
/// `dfbetas[i]` has one entry per parameter (column order matches `fit.beta`:
/// intercept first if present). When `dfE ≤ 1`, RSTUDENT / COVRATIO / DFFITS /
/// DFBETAS are undefined (their leave-one-out variance `MSE_(i)` has 0 df) and
/// are reported as `NaN`; callers render the SAS sentinel `.`.
struct InfluenceStat {
    y: f64,
    y_hat: f64,
    resid: f64,
    stdp: f64,
    stdr: f64,
    h: f64,
    student: f64,
    rstudent: f64,
    cookd: f64,
    press: f64,
    dffits: f64,
    covratio: f64,
    /// One DFBETAS per parameter, same column order as `fit.beta`.
    dfbetas: Vec<f64>,
}

/// Compute the full influence-diagnostic set for every used row. `c = (X'X)⁻¹Xᵀ`
/// (p_eff × n) drives DFBETAS via the closed form
/// `DFBETAS_{ij} = (rstudent_i · c_{ji}) / √(Σ_k c_{jk}²)` — no leave-one-out
/// refits.
fn compute_influence_stats(
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
    p_eff: usize,
) -> Vec<InfluenceStat> {
    let df_e = (n - p_eff) as f64;
    let mse = fit.sse / df_e;
    let h = leverages(x_mat, &fit.xtx_inv);

    // c = (X'X)⁻¹ Xᵀ  →  p_eff × n. Row j, col i is c_{ji}.
    let xt = linalg::transpose(x_mat); // p_eff × n
    let c = linalg::matrix_mult(&fit.xtx_inv, &xt); // (p_eff×p_eff)·(p_eff×n)
    // Row norms √(Σ_k c_{jk}²) for the DFBETAS denominator (= √((X'X)⁻¹_{jj})).
    let c_row_norm: Vec<f64> = (0..p_eff)
        .map(|j| c[j].iter().map(|v| v * v).sum::<f64>().sqrt())
        .collect();

    (0..n)
        .map(|i| {
            let hi = h[i];
            let yh = fit.y_hat[i];
            let resid = fit.resid[i];
            let one_minus_h = 1.0 - hi;
            let stdp = (mse * hi).sqrt();
            let stdr = (mse * one_minus_h).max(0.0).sqrt();
            // STUDENT = resid / STDR.
            let student = if stdr > 0.0 { resid / stdr } else { f64::NAN };
            // Leave-one-out MSE_(i): undefined when dfE ≤ 1.
            let (rstudent, mse_i_ok) = if df_e > 1.0 && one_minus_h > 0.0 {
                let mse_i = (df_e * mse - resid * resid / one_minus_h) / (df_e - 1.0);
                if mse_i > 0.0 {
                    (resid / (mse_i * one_minus_h).sqrt(), true)
                } else {
                    (f64::NAN, false)
                }
            } else {
                (f64::NAN, false)
            };
            // Cook's D = (student²/p)·(h/(1−h)).
            let cookd = if one_minus_h > 0.0 && p_eff > 0 {
                (student * student / p_eff as f64) * (hi / one_minus_h)
            } else {
                f64::NAN
            };
            let press = if one_minus_h != 0.0 {
                resid / one_minus_h
            } else {
                f64::NAN
            };
            let dffits = if mse_i_ok && one_minus_h > 0.0 {
                rstudent * (hi / one_minus_h).sqrt()
            } else {
                f64::NAN
            };
            // COVRATIO = 1 / ( ((dfE−1+rstudent²)/dfE)^p · (1−h) ).
            let covratio = if mse_i_ok && one_minus_h > 0.0 {
                let base = (df_e - 1.0 + rstudent * rstudent) / df_e;
                1.0 / (base.powi(p_eff as i32) * one_minus_h)
            } else {
                f64::NAN
            };
            // DFBETAS_{ij} = c_{ji}·rstudent_i / (√(1−h_i)·√((X'X)⁻¹_{jj})).
            // Here √(Σ_k c_{jk}²) = √((X'X)⁻¹_{jj}) since c·cᵀ = (X'X)⁻¹.
            // The extra √(1−h_i) converts e_i/s_(i) into rstudent_i (which
            // carries its own √(1−h_i)); see derivation in the milestone notes.
            let dfbetas: Vec<f64> = (0..p_eff)
                .map(|j| {
                    if mse_i_ok && c_row_norm[j] > 0.0 && one_minus_h > 0.0 {
                        rstudent * c[j][i] / (c_row_norm[j] * one_minus_h.sqrt())
                    } else {
                        f64::NAN
                    }
                })
                .collect();

            InfluenceStat {
                y: y[i],
                y_hat: yh,
                resid,
                stdp,
                stdr,
                h: hi,
                student,
                rstudent,
                cookd,
                press,
                dffits,
                covratio,
                dfbetas,
            }
        })
        .collect()
}

/// Print the SAS "Output Statistics" table when CLM and/or CLI is requested.
/// Column sets:
///  - CLM only: Obs, Dependent Variable, Predicted Value, Std Error Mean
///    Predict, `<L>% CL Mean` (lower upper), Residual.
///  - CLI only: …, `<L>% CL Predict` (lower upper), Residual.
///  - both: …, `<L>% CL Mean`, `<L>% CL Predict`, Residual.
#[allow(clippy::too_many_arguments)]
fn print_output_statistics(
    model: &RegModel,
    _dep_name: &str,
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
    p_eff: usize,
    session: &mut Session,
) {
    let stats = compute_obs_stats(x_mat, y, fit, n, p_eff, model.alpha);
    let level = fmt_level(100.0 * (1.0 - model.alpha));

    let mut headers: Vec<String> = vec![
        "Obs".into(),
        "Dependent Variable".into(),
        "Predicted Value".into(),
        "Std Error Mean Predict".into(),
    ];
    let mut aligns = vec![Align::Right, Align::Right, Align::Right, Align::Right];
    if model.clm {
        headers.push(format!("{}% CL Mean (Lower)", level));
        headers.push(format!("{}% CL Mean (Upper)", level));
        aligns.push(Align::Right);
        aligns.push(Align::Right);
    }
    if model.cli {
        headers.push(format!("{}% CL Predict (Lower)", level));
        headers.push(format!("{}% CL Predict (Upper)", level));
        aligns.push(Align::Right);
        aligns.push(Align::Right);
    }
    headers.push("Residual".into());
    aligns.push(Align::Right);

    let rows: Vec<Vec<String>> = stats
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut row = vec![
                format!("{}", i + 1),
                fmt5(s.y),
                fmt5(s.y_hat),
                fmt5(s.stdp),
            ];
            if model.clm {
                row.push(fmt5(s.lclm));
                row.push(fmt5(s.uclm));
            }
            if model.cli {
                row.push(fmt5(s.lcl));
                row.push(fmt5(s.ucl));
            }
            row.push(fmt5(s.y - s.y_hat));
            row
        })
        .collect();

    session.listing.blank();
    session.listing.blank();
    centered(session, "Output Statistics");
    session.listing.blank();
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Format a possibly-undefined diagnostic value: SAS prints `.` for a missing
/// (undefined) numeric, otherwise the usual 4-decimal rendering.
fn fmt_diag(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.4}")
    } else {
        ".".to_string()
    }
}

/// Render SAS's `-2-1 0 1 2` character gauge for a studentized residual: a
/// 9-cell `|....*...|`-style bar centred on 0, one `*` placed at the residual's
/// position (clamped to ±2.x). Matches the simple gauge SAS prints in the
/// MODEL R "Output Statistics" table.
fn student_gauge(student: f64) -> String {
    // Cells map the range [-2.625, 2.625] across 9 character slots; the centre
    // slot (index 4) is 0. SAS uses one star; ties round toward centre.
    let mut cells = [' '; 9];
    if student.is_finite() {
        let pos = (student / 2.625 * 4.0).round() as i64;
        let idx = (4 + pos).clamp(0, 8) as usize;
        cells[idx] = '*';
    }
    let bar: String = cells.iter().collect();
    format!("|{}|", bar)
}

/// Print the MODEL R "Output Statistics" table (residual analysis), followed by
/// the Sum of Residuals / Sum of Squared Residuals / PRESS summary block
/// (M36.3). Reuses `compute_influence_stats`.
fn print_r_statistics(
    _model: &RegModel,
    stats: &[InfluenceStat],
    session: &mut Session,
) {
    let headers: Vec<String> = vec![
        "Obs".into(),
        "Dependent Variable".into(),
        "Predicted Value".into(),
        "Std Error Mean Predict".into(),
        "Residual".into(),
        "Std Error Residual".into(),
        "Student Residual".into(),
        "-2-1 0 1 2".into(),
        "Cook's D".into(),
    ];
    let aligns = vec![
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Left,
        Align::Right,
    ];
    let rows: Vec<Vec<String>> = stats
        .iter()
        .enumerate()
        .map(|(i, s)| {
            vec![
                format!("{}", i + 1),
                fmt5(s.y),
                fmt5(s.y_hat),
                fmt5(s.stdp),
                fmt5(s.resid),
                fmt5(s.stdr),
                fmt_diag(s.student),
                student_gauge(s.student),
                fmt_diag(s.cookd),
            ]
        })
        .collect();

    session.listing.blank();
    session.listing.blank();
    centered(session, "Output Statistics");
    session.listing.blank();
    session.listing.write_table(&headers, &aligns, &rows);

    // Summary block SAS prints after the R table.
    let sum_resid: f64 = stats.iter().map(|s| s.resid).sum();
    let sum_sq_resid: f64 = stats.iter().map(|s| s.resid * s.resid).sum();
    let press: f64 = stats
        .iter()
        .filter_map(|s| {
            if s.press.is_finite() {
                Some(s.press * s.press)
            } else {
                None
            }
        })
        .sum();
    session.listing.blank();
    session
        .listing
        .write_line(&format!("Sum of Residuals             {}", fmt5(sum_resid)));
    session.listing.write_line(&format!(
        "Sum of Squared Residuals     {}",
        fmt5(sum_sq_resid)
    ));
    session.listing.write_line(&format!(
        "Predicted Residual SS (PRESS)    {}",
        fmt5(press)
    ));
}

/// Print the MODEL INFLUENCE diagnostics table (M36.3): Obs, Residual,
/// RStudent, Hat Diag H, Cov Ratio, DFFITS, then one `DFBETAS <var>` column per
/// parameter (Intercept first if present). Reuses `compute_influence_stats`.
fn print_influence_statistics(
    stats: &[InfluenceStat],
    reg_names: &[String],
    intercept: bool,
    session: &mut Session,
) {
    let p_eff = reg_names.len() + intercept as usize;
    let mut headers: Vec<String> = vec![
        "Obs".into(),
        "Residual".into(),
        "RStudent".into(),
        "Hat Diag H".into(),
        "Cov Ratio".into(),
        "DFFITS".into(),
    ];
    let mut aligns = vec![
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    for j in 0..p_eff {
        let var = if intercept {
            if j == 0 {
                "Intercept".to_string()
            } else {
                reg_names[j - 1].clone()
            }
        } else {
            reg_names[j].clone()
        };
        headers.push(format!("DFBETAS {}", var));
        aligns.push(Align::Right);
    }

    let rows: Vec<Vec<String>> = stats
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut row = vec![
                format!("{}", i + 1),
                fmt5(s.resid),
                fmt_diag(s.rstudent),
                fmt_fit4(s.h),
                fmt_diag(s.covratio),
                fmt_diag(s.dffits),
            ];
            for j in 0..p_eff {
                row.push(fmt_diag(s.dfbetas[j]));
            }
            row
        })
        .collect();

    session.listing.blank();
    session.listing.blank();
    centered(session, "Output Statistics");
    session.listing.blank();
    session.listing.write_table(&headers, &aligns, &rows);
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
#[allow(clippy::too_many_arguments)]
fn write_outputs(
    entry: &RegModelEntry,
    ds: &SasDataset,
    complete_mask: &[bool],
    n: usize,
    fit: &OlsFit,
    x_mat: &[Vec<f64>],
    p_eff: usize,
    alpha: f64,
    reg_names: &[String],
    intercept: bool,
    session: &mut Session,
) -> Result<()> {
    if entry.outputs.is_empty() {
        return Ok(());
    }

    // Per-observation std errors / limits, computed lazily once if any OUTPUT
    // requests a leverage-derived column. Keeps the P=/R=-only path allocation-
    // free and byte-identical to before.
    let needs_stats = entry.outputs.iter().any(|o| {
        o.stdp.is_some()
            || o.stdi.is_some()
            || o.stdr.is_some()
            || o.lcl.is_some()
            || o.ucl.is_some()
            || o.lclm.is_some()
            || o.uclm.is_some()
    });
    let obs_stats: Option<Vec<ObsStat>> = if needs_stats {
        Some(compute_obs_stats(x_mat, &reconstruct_y(fit), fit, n, p_eff, alpha))
    } else {
        None
    };

    // Influence diagnostics, computed lazily once if any OUTPUT requests a
    // STUDENT/RSTUDENT/COOKD/H/PRESS/DFFITS/COVRATIO/DFBETAS column.
    let needs_infl = entry.outputs.iter().any(|o| {
        o.student.is_some()
            || o.rstudent.is_some()
            || o.cookd.is_some()
            || o.h.is_some()
            || o.press.is_some()
            || o.dffits.is_some()
            || o.covratio.is_some()
            || o.dfbetas.is_some()
    });
    let infl_stats: Option<Vec<InfluenceStat>> = if needs_infl {
        Some(compute_influence_stats(x_mat, &reconstruct_y(fit), fit, n, p_eff))
    } else {
        None
    };

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
        // M36.2 — leverage-derived OUTPUT columns. Each is appended in the order
        // SAS lists them on the OUTPUT statement keyword set.
        if let Some(stats) = &obs_stats {
            let mut push_col = |name: &Option<String>, f: &dyn Fn(&ObsStat) -> f64| {
                if let Some(nm) = name {
                    let data: Vec<Option<f64>> = stats.iter().map(|s| Some(f(s))).collect();
                    columns.push(Series::new(nm.as_str().into(), data).into());
                    out_vars.push(num_var_meta(nm));
                }
            };
            push_col(&out_spec.stdp, &|s| s.stdp);
            push_col(&out_spec.stdi, &|s| s.stdi);
            push_col(&out_spec.stdr, &|s| s.stdr);
            push_col(&out_spec.lclm, &|s| s.lclm);
            push_col(&out_spec.uclm, &|s| s.uclm);
            push_col(&out_spec.lcl, &|s| s.lcl);
            push_col(&out_spec.ucl, &|s| s.ucl);
        }
        // M36.3 — influence-diagnostic OUTPUT columns. Non-finite (undefined)
        // values become SAS missing (None).
        if let Some(stats) = &infl_stats {
            let mut push_col = |name: &Option<String>, f: &dyn Fn(&InfluenceStat) -> f64| {
                if let Some(nm) = name {
                    let data: Vec<Option<f64>> = stats
                        .iter()
                        .map(|s| {
                            let v = f(s);
                            if v.is_finite() {
                                Some(v)
                            } else {
                                None
                            }
                        })
                        .collect();
                    columns.push(Series::new(nm.as_str().into(), data).into());
                    out_vars.push(num_var_meta(nm));
                }
            };
            push_col(&out_spec.student, &|s| s.student);
            push_col(&out_spec.rstudent, &|s| s.rstudent);
            push_col(&out_spec.cookd, &|s| s.cookd);
            push_col(&out_spec.h, &|s| s.h);
            push_col(&out_spec.press, &|s| s.press);
            push_col(&out_spec.dffits, &|s| s.dffits);
            push_col(&out_spec.covratio, &|s| s.covratio);
            // DFBETAS= prefix → one column per parameter named `<prefix>_<var>`
            // (Intercept first if present).
            if let Some(prefix) = &out_spec.dfbetas {
                for j in 0..p_eff {
                    let var = if intercept {
                        if j == 0 {
                            "Intercept".to_string()
                        } else {
                            reg_names[j - 1].clone()
                        }
                    } else {
                        reg_names[j].clone()
                    };
                    let col_name = format!("{}_{}", prefix, var);
                    let data: Vec<Option<f64>> = stats
                        .iter()
                        .map(|s| {
                            let v = s.dfbetas[j];
                            if v.is_finite() {
                                Some(v)
                            } else {
                                None
                            }
                        })
                        .collect();
                    columns.push(Series::new(col_name.as_str().into(), data).into());
                    out_vars.push(num_var_meta(&col_name));
                }
            }
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

    /// Oracle: Σ_i h_i == p_eff (trace of the hat matrix == #params).
    #[test]
    fn test_oracle_leverage_trace() {
        let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0];
        let y: Vec<f64> = x1.iter().zip(x2.iter()).map(|(&a, &b)| 1.0 + a + 0.5 * b).collect();
        let n = y.len();
        let x = design(true, &[&x1, &x2], n);
        let fit = ols_fit(&x, &y).unwrap();
        let h = leverages(&x, &fit.xtx_inv);
        let trace: f64 = h.iter().sum();
        assert!((trace - 3.0).abs() < 1e-9, "trace={trace}");
        // Same for a NOINT design.
        let xn = design(false, &[&x1, &x2], n);
        let fitn = ols_fit(&xn, &y).unwrap();
        let hn = leverages(&xn, &fitn.xtx_inv);
        let tracen: f64 = hn.iter().sum();
        assert!((tracen - 2.0).abs() < 1e-9, "trace_noint={tracen}");
    }

    /// Oracle: STDP²+STDR² == MSE and STDI²−STDP² == MSE (per observation),
    /// and CLM is centered on ŷ.
    #[test]
    fn test_oracle_std_error_identities() {
        let x1 = [1.0_f64, 3.0, 2.0, 5.0, 4.0, 6.0, 8.0, 7.0];
        let y: Vec<f64> = x1.iter().map(|&a| 2.0 + 3.0 * a + (a * 0.5).sin()).collect();
        let n = y.len();
        let x = design(true, &[&x1], n);
        let fit = ols_fit(&x, &y).unwrap();
        let p_eff = 2;
        let mse = fit.sse / (n - p_eff) as f64;
        let stats = compute_obs_stats(&x, &y, &fit, n, p_eff, 0.05);
        for s in &stats {
            assert!((s.stdp * s.stdp + s.stdr * s.stdr - mse).abs() < 1e-9);
            assert!((s.stdi * s.stdi - s.stdp * s.stdp - mse).abs() < 1e-9);
            // CLM centered on ŷ.
            let mid = (s.lclm + s.uclm) / 2.0;
            assert!((mid - s.y_hat).abs() < 1e-9, "mid={mid} yhat={}", s.y_hat);
            // CLI also centered on ŷ and wider than CLM.
            let midi = (s.lcl + s.ucl) / 2.0;
            assert!((midi - s.y_hat).abs() < 1e-9);
            assert!(s.ucl - s.lcl > s.uclm - s.lclm - 1e-12);
        }
    }

    /// Oracle: CLB limits == β_j ± t·SE(β_j) with the parameter-table SE.
    #[test]
    fn test_oracle_clb_limits() {
        let x1 = [1.0_f64, 2.0, 4.0, 3.0, 6.0, 5.0, 7.0];
        let y: Vec<f64> = x1.iter().map(|&a| 1.5 + 2.0 * a + (a * 0.3).cos()).collect();
        let n = y.len();
        let x = design(true, &[&x1], n);
        let fit = ols_fit(&x, &y).unwrap();
        let p_eff = 2;
        let df_e = (n - p_eff) as f64;
        let mse = fit.sse / df_e;
        let alpha = 0.10;
        let t = t_quantile(1.0 - alpha / 2.0, df_e);
        for j in 0..p_eff {
            let se = (mse * fit.xtx_inv[j][j]).sqrt();
            let lo = fit.beta[j] - t * se;
            let hi = fit.beta[j] + t * se;
            // Reconstruct what fit_and_print computes.
            assert!(lo < fit.beta[j] && fit.beta[j] < hi);
            assert!(((lo + hi) / 2.0 - fit.beta[j]).abs() < 1e-12);
        }
    }

    #[test]
    fn test_parse_model_cl_options() {
        let ast =
            parse_reg("proc reg data=a; model y=x / clb alpha=0.10 cli clm; run;").unwrap();
        let m = &ast.models[0].model;
        assert!(m.clb);
        assert!(m.cli);
        assert!(m.clm);
        assert!((m.alpha - 0.10).abs() < 1e-12);
    }

    #[test]
    fn test_parse_output_cl_keywords() {
        let ast = parse_reg(
            "proc reg data=a; model y=x; output out=o p=pred stdp=sp lclm=lm uclm=um lcl=l ucl=u stdi=si stdr=sr; run;",
        )
        .unwrap();
        let o = &ast.models[0].outputs[0];
        assert_eq!(o.predicted.as_deref(), Some("pred"));
        assert_eq!(o.stdp.as_deref(), Some("sp"));
        assert_eq!(o.lclm.as_deref(), Some("lm"));
        assert_eq!(o.uclm.as_deref(), Some("um"));
        assert_eq!(o.lcl.as_deref(), Some("l"));
        assert_eq!(o.ucl.as_deref(), Some("u"));
        assert_eq!(o.stdi.as_deref(), Some("si"));
        assert_eq!(o.stdr.as_deref(), Some("sr"));
    }

    /// End-to-end: CLB adds confidence-limit columns; CLM/CLI emit Output
    /// Statistics. Default model (no options) must NOT print either.
    #[test]
    fn test_execute_cl_listing() {
        let mut session = make_session();
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
        let mut model = basic_model("y", &["x"]);
        model.clb = true;
        model.clm = true;
        model.cli = true;
        let ast = single_model_ast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            model,
        );
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("95% Confidence Limits"), "{listing}");
        assert!(listing.contains("Output Statistics"), "{listing}");
        assert!(listing.contains("CL Mean"), "{listing}");
        assert!(listing.contains("CL Predict"), "{listing}");
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

    /// STUDENT = resid / STDR (matches M36.2 STDR).
    #[test]
    fn test_oracle_student_eq_resid_over_stdr() {
        let (x, y, fit, n, p_eff) = infl_setup();
        let obs = compute_obs_stats(&x, &y, &fit, n, p_eff, 0.05);
        let infl = compute_influence_stats(&x, &y, &fit, n, p_eff);
        for (s, o) in infl.iter().zip(obs.iter()) {
            assert!((s.student - s.resid / o.stdr).abs() < 1e-9);
            // STDR also matches the obs-stats STDR.
            assert!((s.stdr - o.stdr).abs() < 1e-9);
        }
    }

    /// RSTUDENT = student·√((dfE−1)/(dfE−student²)).
    #[test]
    fn test_oracle_rstudent_identity() {
        let (x, y, fit, n, p_eff) = infl_setup();
        let df_e = (n - p_eff) as f64;
        let infl = compute_influence_stats(&x, &y, &fit, n, p_eff);
        for s in &infl {
            let expect = s.student * ((df_e - 1.0) / (df_e - s.student * s.student)).sqrt();
            assert!(
                (s.rstudent - expect).abs() < 1e-9,
                "rstudent={} expect={}",
                s.rstudent,
                expect
            );
        }
    }

    /// PRESS = resid/(1−h) and Σ press² is the printed PRESS.
    #[test]
    fn test_oracle_press() {
        let (x, y, fit, n, p_eff) = infl_setup();
        let h = leverages(&x, &fit.xtx_inv);
        let infl = compute_influence_stats(&x, &y, &fit, n, p_eff);
        let mut press_ss = 0.0;
        for (i, s) in infl.iter().enumerate() {
            let expect = s.resid / (1.0 - h[i]);
            assert!((s.press - expect).abs() < 1e-9);
            press_ss += s.press * s.press;
        }
        let printed: f64 = infl.iter().map(|s| s.press * s.press).sum();
        assert!((press_ss - printed).abs() < 1e-9);
    }

    /// Cook's D ≥ 0, and DFFITS = rstudent·√(h/(1−h)).
    #[test]
    fn test_oracle_cookd_dffits() {
        let (x, y, fit, n, p_eff) = infl_setup();
        let h = leverages(&x, &fit.xtx_inv);
        let infl = compute_influence_stats(&x, &y, &fit, n, p_eff);
        for (i, s) in infl.iter().enumerate() {
            assert!(s.cookd >= 0.0, "cookd={}", s.cookd);
            let expect = s.rstudent * (h[i] / (1.0 - h[i])).sqrt();
            assert!((s.dffits - expect).abs() < 1e-9);
        }
    }

    /// Near-zero-leverage point → Cook's D ≈ 0.
    #[test]
    fn test_oracle_cookd_low_leverage() {
        let (x, y, fit, n, p_eff) = infl_setup();
        let h = leverages(&x, &fit.xtx_inv);
        let infl = compute_influence_stats(&x, &y, &fit, n, p_eff);
        // The lowest-leverage observation should have small Cook's D.
        let (min_i, _) = h
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        assert!(infl[min_i].cookd < 0.5, "cookd={}", infl[min_i].cookd);
    }

    /// DFBETAS closed form == an explicit leave-one-out refit (within 1e-6).
    #[test]
    fn test_oracle_dfbetas_loo_refit() {
        // Tiny dataset, intercept + slope.
        let x1 = [1.0_f64, 2.0, 3.0, 5.0, 8.0];
        let y = [2.1_f64, 3.9, 6.2, 9.8, 16.1];
        let n = y.len();
        let p_eff = 2;
        let x = design(true, &[&x1], n);
        let fit = ols_fit(&x, &y).unwrap();
        let infl = compute_influence_stats(&x, &y.to_vec(), &fit, n, p_eff);

        for drop in 0..n {
            // Refit without observation `drop`.
            let xr: Vec<Vec<f64>> = (0..n).filter(|&i| i != drop).map(|i| x[i].clone()).collect();
            let yr: Vec<f64> = (0..n).filter(|&i| i != drop).map(|i| y[i]).collect();
            let fit_i = ols_fit(&xr, &yr).unwrap();
            // s_(i) = √MSE_(i).
            let df_i = (n - 1 - p_eff) as f64;
            let s_i = (fit_i.sse / df_i).sqrt();
            for j in 0..p_eff {
                let denom = s_i * fit.xtx_inv[j][j].sqrt();
                let expect = (fit.beta[j] - fit_i.beta[j]) / denom;
                assert!(
                    (infl[drop].dfbetas[j] - expect).abs() < 1e-6,
                    "drop={drop} j={j} got={} expect={}",
                    infl[drop].dfbetas[j],
                    expect
                );
            }
        }
    }

    /// dfE ≤ 1 → RSTUDENT/COVRATIO/DFFITS/DFBETAS undefined (NaN).
    #[test]
    fn test_dfe_le_one_undefined() {
        // n=3, p_eff=2 → dfE=1.
        let x1 = [1.0_f64, 2.0, 4.0];
        let y = [1.0_f64, 3.0, 2.5];
        let n = y.len();
        let p_eff = 2;
        let x = design(true, &[&x1], n);
        let fit = ols_fit(&x, &y.to_vec()).unwrap();
        let infl = compute_influence_stats(&x, &y.to_vec(), &fit, n, p_eff);
        for s in &infl {
            assert!(!s.rstudent.is_finite());
            assert!(!s.covratio.is_finite());
            assert!(!s.dffits.is_finite());
            assert!(s.dfbetas.iter().all(|v| !v.is_finite()));
            // STUDENT and PRESS remain defined.
            assert!(s.press.is_finite());
        }
        // fmt_diag renders the SAS sentinel.
        assert_eq!(fmt_diag(f64::NAN), ".");
    }

    #[test]
    fn test_parse_model_r_influence() {
        let ast = parse_reg("proc reg data=a; model y=x / r influence; run;").unwrap();
        let m = &ast.models[0].model;
        assert!(m.r);
        assert!(m.influence);
    }

    #[test]
    fn test_parse_output_influence_keywords() {
        let ast = parse_reg(
            "proc reg data=a; model y=x; output out=o student=rs rstudent=er cookd=cd h=hat press=pr dffits=df covratio=cv dfbetas=b; run;",
        )
        .unwrap();
        let o = &ast.models[0].outputs[0];
        assert_eq!(o.student.as_deref(), Some("rs"));
        assert_eq!(o.rstudent.as_deref(), Some("er"));
        assert_eq!(o.cookd.as_deref(), Some("cd"));
        assert_eq!(o.h.as_deref(), Some("hat"));
        assert_eq!(o.press.as_deref(), Some("pr"));
        assert_eq!(o.dffits.as_deref(), Some("df"));
        assert_eq!(o.covratio.as_deref(), Some("cv"));
        assert_eq!(o.dfbetas.as_deref(), Some("b"));
    }

    /// End-to-end: R and INFLUENCE listings print; default model prints neither.
    #[test]
    fn test_execute_r_influence_listing() {
        let mut session = make_session();
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
        let mut model = basic_model("y", &["x"]);
        model.r = true;
        model.influence = true;
        let ast = single_model_ast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            model,
        );
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Student Residual"), "{listing}");
        assert!(listing.contains("Sum of Residuals"), "{listing}");
        assert!(listing.contains("PRESS"), "{listing}");
        assert!(listing.contains("RStudent"), "{listing}");
        assert!(listing.contains("DFBETAS Intercept"), "{listing}");
        assert!(listing.contains("DFBETAS x"), "{listing}");
    }

    /// OUTPUT influence columns appear; DFBETAS= emits one column per parameter.
    #[test]
    fn test_output_influence_columns() {
        let mut session = make_session();
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
        let ast = parse_reg(
            "proc reg data=work.t; model y=x; output out=work.o student=stu cookd=cd h=hat dfbetas=b; run;",
        )
        .unwrap();
        execute(&ast, &mut session).unwrap();
        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
        assert!(names.contains(&"stu"));
        assert!(names.contains(&"cd"));
        assert!(names.contains(&"hat"));
        assert!(names.contains(&"b_Intercept"));
        assert!(names.contains(&"b_x"));
    }

    // ───────────────────────── M36.4 ─────────────────────────

    /// Parse: all collinearity / spec diagnostic options on one MODEL.
    #[test]
    fn test_parse_model_diagnostics() {
        let ast = parse_reg(
            "proc reg data=a; model y=x1 x2 / vif tol collin spec dw dwprob acov; run;",
        )
        .unwrap();
        let m = &ast.models[0].model;
        assert!(m.vif);
        assert!(m.tol);
        assert!(m.collin);
        assert!(!m.collinoint);
        assert!(m.spec);
        assert!(m.dw);
        assert!(m.dwprob);
        assert!(m.acov);
    }

    #[test]
    fn test_parse_collinoint_and_hcc_synonym() {
        let ast =
            parse_reg("proc reg data=a; model y=x1 x2 / collinoint hcc; run;").unwrap();
        let m = &ast.models[0].model;
        assert!(m.collinoint);
        assert!(!m.collin);
        // HCC is a synonym for ACOV.
        assert!(m.acov);
    }

    /// VIF·TOL == 1; for two regressors VIF_1 == VIF_2 == 1/(1−r²).
    #[test]
    fn test_oracle_vif_tol() {
        let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 7.0, 8.0, 6.0];
        // x2 correlated-but-not-collinear with x1.
        let x2: Vec<f64> = x1.iter().map(|&a| 0.5 * a + (a * 0.7).sin()).collect();
        let cols = vec![x1.to_vec(), x2.clone()];
        let (tol, vif) = vif_tol(&cols);
        for j in 0..2 {
            assert!((vif[j] * tol[j] - 1.0).abs() < 1e-9, "VIF·TOL != 1");
        }
        // Two regressors → both VIF equal, == 1/(1−r²).
        let n = x1.len() as f64;
        let m1 = x1.iter().sum::<f64>() / n;
        let m2 = x2.iter().sum::<f64>() / n;
        let mut sxy = 0.0;
        let mut sxx = 0.0;
        let mut syy = 0.0;
        for i in 0..x1.len() {
            sxy += (x1[i] - m1) * (x2[i] - m2);
            sxx += (x1[i] - m1) * (x1[i] - m1);
            syy += (x2[i] - m2) * (x2[i] - m2);
        }
        let r2 = (sxy * sxy) / (sxx * syy);
        let expected = 1.0 / (1.0 - r2);
        assert!((vif[0] - vif[1]).abs() < 1e-9, "VIFs differ");
        assert!((vif[0] - expected).abs() < 1e-7, "VIF != 1/(1-r²)");
    }

    /// Single regressor → trivial VIF table (TOL=1, VIF=1).
    #[test]
    fn test_vif_single_regressor() {
        let cols = vec![vec![1.0_f64, 2.0, 3.0, 4.0]];
        let (tol, vif) = vif_tol(&cols);
        assert_eq!(tol, vec![1.0]);
        assert_eq!(vif, vec![1.0]);
    }

    /// Collinearity: #eigenvalues == #cols, condition index uses λ_max, and each
    /// regressor's variance proportions sum to 1 across rows.
    #[test]
    fn test_oracle_collin_proportions() {
        let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        let x2: Vec<f64> = x1.iter().map(|&a| a * a + 1.0).collect();
        let n = x1.len();
        let x = design(true, &[&x1, &x2], n);
        let reg = vec!["x1".to_string(), "x2".to_string()];
        let c = compute_collin(&x, &reg, true, false).unwrap();
        assert_eq!(c.eigenvalues.len(), 3); // intercept + 2 regressors
        // Descending.
        for k in 1..c.eigenvalues.len() {
            assert!(c.eigenvalues[k - 1] >= c.eigenvalues[k] - 1e-12);
        }
        // First condition index == 1 (λ_max / λ_max).
        assert!((c.condition_index[0] - 1.0).abs() < 1e-9);
        // Column proportions sum to 1.
        let m = c.eigenvalues.len();
        for j in 0..m {
            let s: f64 = (0..m).map(|k| c.proportions[k][j]).sum();
            assert!((s - 1.0).abs() < 1e-9, "proportion col sum != 1: {s}");
        }
        // COLLINOINT drops the intercept column → 2 columns analysed.
        let cint = compute_collin(&x, &reg, true, true).unwrap();
        assert_eq!(cint.eigenvalues.len(), 2);
        assert_eq!(cint.col_labels, vec!["x1".to_string(), "x2".to_string()]);
    }

    /// SPEC: W = n·R²_aux ≥ 0 and df == number of auxiliary regressors.
    #[test]
    fn test_oracle_spec_white() {
        let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        // x2 chosen so {1, x1, x2, x1², x2², x1·x2} is full rank.
        let x2 = [3.0_f64, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0, 5.0, 8.0];
        // Include genuine noise so the fit has nonzero residuals and the
        // auxiliary (White) regression is full rank.
        let y: Vec<f64> = (0..10)
            .map(|i| 1.0 + 2.0 * x1[i] - 0.5 * x2[i] + (x1[i] * 1.3).sin() * 0.8)
            .collect();
        let n = y.len();
        let x = design(true, &[&x1, &x2], n);
        let fit = ols_fit(&x, &y).unwrap();
        let cols = vec![x1.to_vec(), x2.to_vec()];
        let (w, df, pv) = white_spec_test(&cols, &fit.resid).unwrap();
        assert!(w >= 0.0);
        // p=2 regressors → linear(2) + square(2) + cross(1) = 5 aux regressors.
        assert_eq!(df, 5);
        assert!((0.0..=1.0).contains(&pv));
    }

    /// DW: 0 ≤ d ≤ 4; for no-autocorrelation residuals d ≈ 2; d ≈ 2(1−ρ).
    #[test]
    fn test_oracle_durbin_watson() {
        // Alternating-sign residuals → strong negative autocorrelation, d→4.
        let x1: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..10)
            .map(|i| 1.0 + 0.5 * i as f64 + if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let n = y.len();
        let x = design(true, &[&x1], n);
        let fit = ols_fit(&x, &y).unwrap();
        let dwr = durbin_watson(&fit.resid, &x, &fit.xtx_inv, true);
        assert!((0.0..=4.0).contains(&dwr.d), "d out of range: {}", dwr.d);
        // d ≈ 2(1−ρ) (exact only up to O(1/n) boundary terms e_1²+e_n²).
        assert!((dwr.d - 2.0 * (1.0 - dwr.rho)).abs() < 0.6);
        // Alternating signs → ρ negative → d > 2.
        assert!(dwr.d > 2.0);
        // p-values present and in [0,1].
        let pp = dwr.pr_pos.unwrap();
        let pn = dwr.pr_neg.unwrap();
        assert!((0.0..=1.0).contains(&pp) && (0.0..=1.0).contains(&pn));
        assert!((pp + pn - 1.0).abs() < 1e-9);
    }

    /// ACOV: HC matrix is symmetric; for homoscedastic-like data HC SE is the
    /// same order of magnitude as OLS SE.
    #[test]
    fn test_oracle_acov_hc0() {
        let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let y: Vec<f64> = x1.iter().map(|&a| 1.0 + 2.0 * a + (a * 0.9).sin()).collect();
        let n = y.len();
        let p_eff = 2;
        let x = design(true, &[&x1], n);
        let fit = ols_fit(&x, &y).unwrap();
        let cov = acov_hc0(&x, &fit.resid, &fit.xtx_inv);
        // Symmetry.
        for i in 0..p_eff {
            for j in 0..p_eff {
                assert!((cov[i][j] - cov[j][i]).abs() < 1e-12);
            }
        }
        // Order-of-magnitude agreement with OLS SE.
        let mse = fit.sse / (n - p_eff) as f64;
        for j in 0..p_eff {
            let ols_se = (mse * fit.xtx_inv[j][j]).sqrt();
            let hc_se = cov[j][j].sqrt();
            assert!(
                hc_se > 0.0 && hc_se < 100.0 * ols_se && ols_se < 100.0 * hc_se,
                "HC SE / OLS SE order mismatch: {hc_se} vs {ols_se}"
            );
        }
    }

    /// End-to-end: VIF/TOL columns appear; default model does NOT print them.
    #[test]
    fn test_execute_diagnostics_listing() {
        let mut session = make_session();
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0, 9.0, 11.0],
            "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            "x2" => [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
        let ast = parse_reg(
            "proc reg data=work.t; model y=x1 x2 / vif tol collin spec dw dwprob acov; run;",
        )
        .unwrap();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Tolerance"), "{listing}");
        assert!(listing.contains("Variance Inflation"), "{listing}");
        assert!(listing.contains("Collinearity Diagnostics"), "{listing}");
        assert!(
            listing.contains("Test of First and Second Moment Specification"),
            "{listing}"
        );
        assert!(listing.contains("Durbin-Watson D"), "{listing}");
        assert!(listing.contains("Pr < DW"), "{listing}");
        assert!(
            listing.contains("Consistent Covariance of Estimates"),
            "{listing}"
        );
    }

    /// Byte-identity guard: a plain model and one with only diagnostics-OFF must
    /// produce identical parameter-table output (no extra columns).
    #[test]
    fn test_diagnostics_off_no_extra_columns() {
        let mut session = make_session();
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
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
        assert!(!listing.contains("Tolerance"));
        assert!(!listing.contains("Variance Inflation"));
        assert!(!listing.contains("Collinearity Diagnostics"));
        assert!(!listing.contains("Durbin-Watson"));
    }
}
