//! Parsing des statements de PROC REG (appelé après `proc reg`).

use super::*;

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC REG. Called AFTER `proc reg` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<RegAst> {
    let ProcOptions {
        input,
        simple,
        corr,
        proc_all,
        outest,
        outsscp,
        ridge,
        pcomit,
        outvif,
        mut plot_requests,
    } = parse_proc_options(ts)?;

    // Sub-statements until run;/quit;
    let mut models: Vec<RegModelEntry> = Vec::new();
    let mut plots_requested = false;
    let mut plot_statements: Vec<PlotPair> = Vec::new();
    let mut weight: Option<String> = None;
    let mut freq: Option<String> = None;
    let mut by: Vec<String> = Vec::new();
    let mut id: Vec<String> = Vec::new();
    // M36.10 run-group / VAR bookkeeping.
    let mut var_list: Vec<String> = Vec::new();
    let mut reweight_seen = false;
    let mut refit_seen = false;
    let mut paint_seen = false;

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "model" {
            models.push(parse_model_stmt(ts, proc_all)?);
            Ok(true)
        } else if kw == "output" {
            parse_output_stmt(ts, &mut models)?;
            Ok(true)
        } else if kw == "plots" {
            // M29.3 / M36.11 — PLOTS request. Two surface forms:
            //   PLOTS=(…)  /  PLOTS(UNPACK)=…  — a typed request set (M36.11),
            //   bare PLOTS [/ options];        — the legacy deferred flag (M29.3).
            // `parse_plots_option` consumes the `plots` keyword + value when a
            // `=`/`(` follows; otherwise we fall back to the legacy flag.
            let nxt = &ts.peek2().kind;
            if matches!(nxt, TokenKind::Eq | TokenKind::LParen) {
                parse_plots_option(ts, &mut plot_requests);
                // Consume an optional trailing `/ options` and the terminator.
                ts.skip_to_semi();
            } else {
                ts.next();
                ts.skip_to_semi();
                plots_requested = true;
            }
            Ok(true)
        } else if kw == "plot" {
            // M36.11 — traditional `PLOT y*x [=symbol] [y2*x2 …] [/ opts];`.
            ts.next(); // consume "plot"
            parse_plot_statement(ts, &mut plot_statements);
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
        } else if kw == "mtest" {
            // `MTEST [equations] [/ options];` (unlabeled — a leading `label:` is
            // handled by the catch-all label branch). Equations are optional; an
            // empty list means the default "all regressors = 0" hypothesis.
            ts.next(); // consume "mtest"
            let equations = parse_mtest_equations(ts)?;
            // Options after `/` are accepted and ignored (e.g. CANPRINT, PRINT).
            if ts.peek().kind == TokenKind::Slash {
                ts.skip_to_semi();
            } else {
                ts.expect_semi()?;
            }
            if let Some(entry) = models.last_mut() {
                entry.mtests.push(RegMtest { label: None, equations });
            }
            Ok(true)
        } else if kw == "add" {
            // `ADD x1 x2 …;` — run-group regressor addition (M36.10).
            ts.next();
            if let Some(entry) = models.last_mut() {
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if let Some(name) = ts.peek().ident().map(str::to_string) {
                        entry.add.push(name);
                        ts.next();
                    } else {
                        ts.next();
                    }
                }
            } else {
                ts.skip_to_semi();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "delete" {
            // `DELETE x1 x2 …;` — run-group regressor removal (M36.10).
            ts.next();
            if let Some(entry) = models.last_mut() {
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if let Some(name) = ts.peek().ident().map(str::to_string) {
                        entry.delete.push(name);
                        ts.next();
                    } else {
                        ts.next();
                    }
                }
            } else {
                ts.skip_to_semi();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "var" {
            // `VAR v1 v2 …;` — declare variables for later interactive editing
            // (M36.10). Recorded only.
            ts.next();
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    var_list.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "reweight" {
            // `REWEIGHT <condition>;` — interactive reweighting. Deferred (M36.10).
            ts.next();
            ts.skip_to_semi();
            reweight_seen = true;
            Ok(true)
        } else if kw == "refit" {
            // `REFIT;` — interactive refit. Deferred (M36.10).
            ts.next();
            ts.skip_to_semi();
            refit_seen = true;
            Ok(true)
        } else if kw == "paint" {
            // `PAINT <…>;` — interactive plot painting. Deferred (M36.10).
            ts.next();
            ts.skip_to_semi();
            paint_seen = true;
            Ok(true)
        } else if kw == "weight" {
            // `WEIGHT var;` — a single weight variable (M36.7).
            ts.next(); // consume "weight"
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                weight = Some(name);
                ts.next();
            }
            ts.skip_to_semi();
            Ok(true)
        } else if kw == "freq" {
            // `FREQ var;` — a single frequency variable (M36.7).
            ts.next(); // consume "freq"
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                freq = Some(name);
                ts.next();
            }
            ts.skip_to_semi();
            Ok(true)
        } else if kw == "by" {
            // `BY var1 var2 …;` — by-group processing (M36.7). DESCENDING is
            // accepted but unused here (REG runs the same per-group analysis);
            // we keep just the variable names (in order).
            ts.next(); // consume "by"
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if ts.peek().is_kw("descending") {
                    ts.next();
                    continue;
                }
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    by.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "id" {
            // `ID var1 …;` — identification variables (M36.7).
            ts.next(); // consume "id"
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    id.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
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
        } else if ts.peek2().kind == TokenKind::Colon && ts.peek_nth(2).is_kw("mtest") {
            // `label: MTEST [eq …] [/ opts];` — the leading identifier is a label.
            let label = ts.peek().ident().map(str::to_string);
            ts.next(); // label ident
            ts.next(); // ':'
            ts.next(); // 'mtest'
            let equations = parse_mtest_equations(ts)?;
            if ts.peek().kind == TokenKind::Slash {
                ts.skip_to_semi();
            } else {
                ts.expect_semi()?;
            }
            if let Some(entry) = models.last_mut() {
                entry.mtests.push(RegMtest { label, equations });
            }
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(RegAst {
        data_options: RegDataOptions {
            input,
            outest,
            outsscp,
            ridge,
            pcomit,
            outvif,
        },
        models,
        plots_requested,
        plot_requests,
        plot_statements,
        weight,
        freq,
        by,
        id,
        simple,
        corr,
        var_list,
        reweight_seen,
        refit_seen,
        paint_seen,
    })
}

/// PROC-statement options of PROC REG (MQ5.1) — everything parsed from the
/// `PROC REG …;` statement itself, before the sub-statements.
struct ProcOptions {
    input: Option<DatasetRef>,
    simple: bool,
    corr: bool,
    proc_all: bool,
    outest: Option<OutEst>,
    outsscp: Option<DatasetRef>,
    ridge: Vec<f64>,
    pcomit: Vec<f64>,
    outvif: bool,
    plot_requests: PlotRequests,
}

/// MQ5.1 — parse the options of the `PROC REG …;` statement itself, up to
/// and including its terminating `;`.
fn parse_proc_options(ts: &mut StatementStream) -> Result<ProcOptions> {
    let mut input: Option<DatasetRef> = None;
    // M36.8 — PROC-statement flags / output-dataset requests.
    let mut simple = false;
    let mut corr = false;
    let mut proc_all = false;
    let mut outest: Option<DatasetRef> = None;
    let mut covout = false;
    let mut outseb = false;
    let mut edf = false;
    let mut tableout = false;
    let mut outsscp: Option<DatasetRef> = None;
    // M36.9 — ridge / IPC regression PROC options.
    let mut ridge: Vec<f64> = Vec::new();
    let mut pcomit: Vec<f64> = Vec::new();
    let mut outvif = false;
    // M36.11 — PLOTS= may appear as a PROC-statement option as well as a
    // sub-statement; accumulate into one request set.
    let mut plot_requests = PlotRequests::default();

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
        } else if ts.peek().is_kw("outest") {
            outest = Some(common::parse_dataset_opt(ts, "OUTEST")?);
        } else if ts.peek().is_kw("outsscp") {
            outsscp = Some(common::parse_dataset_opt(ts, "OUTSSCP")?);
        } else if ts.peek().is_kw("covout") {
            covout = true;
            ts.next();
        } else if ts.peek().is_kw("outseb") {
            outseb = true;
            ts.next();
        } else if ts.peek().is_kw("edf") {
            edf = true;
            ts.next();
        } else if ts.peek().is_kw("tableout") {
            tableout = true;
            ts.next();
        } else if ts.peek().is_kw("ridge") {
            // RIDGE=value-list (M36.9): a list of ridge constants k, accepting
            // both an explicit list (`ridge=0 0.01 0.1`) and a SAS numeric range
            // (`ridge=0 to 0.1 by 0.02`).
            common::expect_eq(ts, "RIDGE")?;
            ridge = parse_value_list(ts)?;
        } else if ts.peek().is_kw("pcomit") {
            // PCOMIT=value-list (M36.9): principal-component drop counts m.
            common::expect_eq(ts, "PCOMIT")?;
            pcomit = parse_value_list(ts)?;
        } else if ts.peek().is_kw("outvif") {
            outvif = true;
            ts.next();
        } else if ts.peek().is_kw("simple") {
            simple = true;
            ts.next();
        } else if ts.peek().is_kw("corr") {
            corr = true;
            ts.next();
        } else if ts.peek().is_kw("plots") {
            // M36.11 — PROC-level PLOTS=(…) / PLOTS(UNPACK)=… diagnostic request.
            parse_plots_option(ts, &mut plot_requests);
        } else if ts.peek().is_kw("all") {
            proc_all = true;
            ts.next();
        } else {
            // Skip unknown proc-level options
            ts.next();
        }
    }

    // ALL implies SIMPLE + CORR at PROC level (and the MODEL matrix options,
    // applied per-model below).
    if proc_all {
        simple = true;
        corr = true;
    }
    let outest = outest.map(|out| OutEst {
        out,
        covout,
        outseb,
        edf,
        tableout,
    });
    Ok(ProcOptions {
        input,
        simple,
        corr,
        proc_all,
        outest,
        outsscp,
        ridge,
        pcomit,
        outvif,
        plot_requests,
    })
}

/// MQ5.1 — parse a `MODEL y1 [y2 …] = x1 x2 … [/ options];` statement (the
/// `model` keyword has not been consumed yet). `proc_all` mirrors the
/// PROC-level ALL option, which forces the matrix/CL options on every model.
fn parse_model_stmt(ts: &mut StatementStream, proc_all: bool) -> Result<RegModelEntry> {
    ts.next(); // consume "model"
    // SAS allows MULTIPLE responses on the LHS (`model y1 y2 = x …;`),
    // consumed up to the `=`. At least one is required.
    let mut dependents: Vec<String> = Vec::new();
    while let Some(name) = ts.peek().ident().map(str::to_string) {
        dependents.push(name);
        ts.next();
    }
    if dependents.is_empty() {
        return Err(SasError::parse(
            "expected dependent variable",
            ts.peek().span,
        ));
    }
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
    let mut ss1 = false;
    let mut ss2 = false;
    let mut stb = false;
    let mut pcorr1 = false;
    let mut pcorr2 = false;
    let mut scorr1 = false;
    let mut scorr2 = false;
    let mut seqb = false;
    let mut press_opt = false;
    let mut xpx = false;
    let mut inv = false;
    let mut covb = false;
    let mut corrb = false;
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
                    selection = Some(parse_selection_value(ts)?);
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
                } else if ts.peek().is_kw("best") {
                    common::expect_eq(ts, "BEST")?;
                    let v = read_float(ts)? as usize;
                    if let Some(sel) = selection.as_mut() {
                        sel.best = Some(v);
                    }
                } else if ts.peek().is_kw("include") {
                    common::expect_eq(ts, "INCLUDE")?;
                    let v = read_float(ts)? as usize;
                    if let Some(sel) = selection.as_mut() {
                        sel.include = v;
                    }
                } else if ts.peek().is_kw("start") {
                    common::expect_eq(ts, "START")?;
                    let v = read_float(ts)? as usize;
                    if let Some(sel) = selection.as_mut() {
                        sel.start = Some(v);
                    }
                } else if ts.peek().is_kw("stop") {
                    common::expect_eq(ts, "STOP")?;
                    let v = read_float(ts)? as usize;
                    if let Some(sel) = selection.as_mut() {
                        sel.stop = Some(v);
                    }
                } else if ts.peek().is_kw("groupnames") {
                    // GROUPNAMES="g1" "g2" ... — parsed and ignored
                    // (used by SAS only to label grouped regressors in
                    // the selection display). Consume the `=` and the
                    // following string/ident list.
                    common::expect_eq(ts, "GROUPNAMES")?;
                    while matches!(
                        ts.peek().kind,
                        TokenKind::Str { .. } | TokenKind::Ident(_)
                    ) {
                        ts.next();
                    }
                } else if ts.peek().is_kw("details") {
                    if let Some(sel) = selection.as_mut() {
                        sel.details = true;
                    }
                    ts.next();
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
                } else if ts.peek().is_kw("ss1") {
                    ss1 = true;
                    ts.next();
                } else if ts.peek().is_kw("ss2") {
                    ss2 = true;
                    ts.next();
                } else if ts.peek().is_kw("stb") {
                    stb = true;
                    if let Some(sel) = selection.as_mut() {
                        sel.stb = true;
                    }
                    ts.next();
                } else if ts.peek().is_kw("pcorr1") {
                    pcorr1 = true;
                    ts.next();
                } else if ts.peek().is_kw("pcorr2") {
                    pcorr2 = true;
                    ts.next();
                } else if ts.peek().is_kw("scorr1") {
                    scorr1 = true;
                    ts.next();
                } else if ts.peek().is_kw("scorr2") {
                    scorr2 = true;
                    ts.next();
                } else if ts.peek().is_kw("seqb") {
                    seqb = true;
                    ts.next();
                } else if ts.peek().is_kw("press") {
                    press_opt = true;
                    ts.next();
                } else if ts.peek().is_kw("xpx") {
                    xpx = true;
                    ts.next();
                } else if ts.peek().is_kw("i") {
                    inv = true;
                    ts.next();
                } else if ts.peek().is_kw("covb") {
                    covb = true;
                    ts.next();
                } else if ts.peek().is_kw("corrb") {
                    corrb = true;
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
    // PROC-level ALL turns on the MODEL matrix options (and CLM/CLI) on
    // every model, as SAS does. Other ALL-implied displays (SIMPLE/CORR)
    // are handled at the PROC level.
    if proc_all {
        xpx = true;
        inv = true;
        covb = true;
        corrb = true;
        clm = true;
        cli = true;
    }
    Ok(RegModelEntry {
        model: RegModel {
            dependents,
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
            ss1,
            ss2,
            stb,
            pcorr1,
            pcorr2,
            scorr1,
            scorr2,
            seqb,
            press_opt,
            xpx,
            inv,
            covb,
            corrb,
        },
        outputs: Vec::new(),
        tests: Vec::new(),
        restricts: Vec::new(),
        mtests: Vec::new(),
        add: Vec::new(),
        delete: Vec::new(),
    })
}

/// MQ5.1 — parse an `OUTPUT OUT=… keyword=name …;` statement (the `output`
/// keyword has not been consumed yet) and attach the result to the last
/// MODEL seen, if any.

/// MQ5.1 — parse the value of a `SELECTION=method` MODEL option (the
/// `selection` keyword has not been consumed yet) and build the initial
/// `Selection` request with the method's default SLE/SLS.
fn parse_selection_value(ts: &mut StatementStream) -> Result<Selection> {
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
        "rsquare" => SelMethod::RSquare,
        "adjrsq" => SelMethod::AdjRsq,
        "cp" => SelMethod::Cp,
        "maxr" => SelMethod::MaxR,
        "minr" => SelMethod::MinR,
        "none" => SelMethod::None,
        other => {
            return Err(SasError::parse(
                format!("unsupported SELECTION method '{}'", other),
                ts.peek().span,
            ));
        }
    };
    // Defaults depend on the method. The all-subsets and
    // R²-improvement methods don't use SLE/SLS; keep
    // harmless defaults so the struct is always valid.
    let (def_sle, def_sls) = match method {
        SelMethod::Forward => (0.50, 0.10),
        SelMethod::Backward => (0.50, 0.10),
        SelMethod::Stepwise => (0.15, 0.15),
        _ => (0.50, 0.10),
    };
    Ok(Selection {
        method,
        slentry: def_sle,
        slstay: def_sls,
        best: None,
        include: 0,
        start: None,
        stop: None,
        details: false,
        stb: false,
    })
}
fn parse_output_stmt(ts: &mut StatementStream, models: &mut [RegModelEntry]) -> Result<()> {
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
    Ok(())
}

/// M36.11 — parse a `PLOTS` request: `PLOTS[(global-opts)]=keyword | (kw …)`.
/// Accepts the panel modifiers `PLOTS(UNPACK)=…` / `PLOTS(ONLY)=…`, the bare
/// keywords DIAGNOSTICS / RESIDUALS / FIT / ALL / NONE, and a parenthesised list
/// `PLOTS=(DIAGNOSTICS RESIDUALS FIT)`. Unknown keywords are consumed cleanly
/// (ignored). The `plots` keyword token is consumed by this function. On a
/// malformed form (no `=`) the keyword alone is consumed and nothing recorded.
fn parse_plots_option(ts: &mut StatementStream, req: &mut PlotRequests) {
    ts.next(); // consume "plots"
    // Optional global-option parenthesis directly after PLOTS: PLOTS(UNPACK)=…
    if ts.peek().kind == TokenKind::LParen {
        ts.next();
        while ts.peek().kind != TokenKind::RParen && ts.peek().kind != TokenKind::Semi
            && ts.peek().kind != TokenKind::Eof
        {
            if let Some(id) = ts.peek().ident() {
                match id.to_ascii_uppercase().as_str() {
                    "UNPACK" => req.unpack = true,
                    "ONLY" => req.only = true,
                    _ => {}
                }
            }
            ts.next();
        }
        if ts.peek().kind == TokenKind::RParen {
            ts.next();
        }
    }
    // The value must be introduced by `=`. Without it there is nothing to record.
    if ts.peek().kind != TokenKind::Eq {
        return;
    }
    ts.next(); // consume "="
    req.explicit = true;
    // Either a parenthesised list of keywords, or a single keyword.
    if ts.peek().kind == TokenKind::LParen {
        ts.next();
        while ts.peek().kind != TokenKind::RParen && ts.peek().kind != TokenKind::Semi
            && ts.peek().kind != TokenKind::Eof
        {
            if let Some(id) = ts.peek().ident() {
                apply_plot_keyword(id, req);
            }
            ts.next();
        }
        if ts.peek().kind == TokenKind::RParen {
            ts.next();
        }
    } else if let Some(id) = ts.peek().ident().map(str::to_string) {
        apply_plot_keyword(&id, req);
        ts.next();
    }
}

/// Apply one PLOTS= keyword to the request set. Unknown keywords are ignored.
fn apply_plot_keyword(kw: &str, req: &mut PlotRequests) {
    match kw.to_ascii_uppercase().as_str() {
        "DIAGNOSTICS" | "DIAGNOSTIC" => req.diagnostics = true,
        "RESIDUALS" | "RESIDUAL" | "RESIDUALPLOT" => req.residuals = true,
        "FIT" | "FITPLOT" => req.fit = true,
        "ALL" => req.all = true,
        "NONE" => req.none = true,
        _ => {}
    }
}

/// M36.11 — parse one axis term of a traditional `PLOT` statement. Handles the
/// `keyword.` special variables (`PREDICTED.`/`P.`, `RESIDUAL.`/`R.`) and plain
/// variable names. The trailing `.` of a keyword variable is consumed when
/// present. Returns `None` at a non-identifier token.
fn parse_plot_var(ts: &mut StatementStream) -> Option<PlotVar> {
    let name = ts.peek().ident().map(str::to_string)?;
    ts.next();
    // A trailing dot marks a SAS keyword variable (PREDICTED. / RESIDUAL. / …).
    let has_dot = ts.peek().kind == TokenKind::Dot;
    if has_dot {
        ts.next();
        match name.to_ascii_uppercase().as_str() {
            "PREDICTED" | "PRED" | "P" => return Some(PlotVar::Predicted),
            "RESIDUAL" | "RESID" | "R" => return Some(PlotVar::Residual),
            // An unknown `keyword.` — treat its bare name as a model variable.
            _ => return Some(PlotVar::Named(name.to_ascii_uppercase())),
        }
    }
    Some(PlotVar::Named(name.to_ascii_uppercase()))
}

/// M36.11 — parse the body of a `PLOT y*x [=symbol] [y2*x2 …] [/ opts];` statement
/// (the `plot` keyword has already been consumed). Each `y*x` pair is recorded;
/// an optional `=symbol` after a pair and a trailing `/ options` clause are
/// consumed and ignored. Stops at `;`/EOF.
fn parse_plot_statement(ts: &mut StatementStream, out: &mut Vec<PlotPair>) {
    loop {
        match ts.peek().kind {
            TokenKind::Semi => {
                ts.next();
                break;
            }
            TokenKind::Eof => break,
            TokenKind::Slash => {
                // Trailing `/ options` — consume the rest of the statement.
                ts.skip_to_semi();
                break;
            }
            _ => {}
        }
        // Expect `y * x`. If we cannot read a y, skip a token to make progress.
        let y = match parse_plot_var(ts) {
            Some(v) => v,
            None => {
                ts.next();
                continue;
            }
        };
        if ts.peek().kind != TokenKind::Star {
            // Not a valid pair (e.g. a stray symbol); skip and continue.
            continue;
        }
        ts.next(); // consume "*"
        let x = match parse_plot_var(ts) {
            Some(v) => v,
            None => continue,
        };
        out.push(PlotPair { y, x });
        // Optional `=symbol` (a plot symbol assignment) — consume `= token`.
        if ts.peek().kind == TokenKind::Eq {
            ts.next();
            if ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                ts.next();
            }
        }
    }
}

/// Parse a SAS numeric value-list for RIDGE=/PCOMIT= (M36.9). Accepts a plain
/// list of numbers (`0 0.01 0.05 0.1`) and the SAS range form
/// `start TO stop [BY step]` (default step 1), in any mix. Stops at the next
/// option keyword (a non-numeric, non-`TO`/`BY` identifier), a `;`, or EOF.
/// Negative values are tolerated via a leading `-` (e.g. `-0.5`).
fn parse_value_list(ts: &mut StatementStream) -> Result<Vec<f64>> {
    let mut out: Vec<f64> = Vec::new();
    // Read one signed number; returns None if the current token is not numeric.
    fn read_num(ts: &mut StatementStream) -> Option<f64> {
        let neg = if ts.peek().kind == TokenKind::Minus {
            ts.next();
            true
        } else {
            false
        };
        match ts.peek().kind {
            TokenKind::Num(v) => {
                ts.next();
                Some(if neg { -v } else { v })
            }
            _ => None,
        }
    }
    loop {
        match ts.peek().kind {
            TokenKind::Semi | TokenKind::Eof => break,
            TokenKind::Num(_) | TokenKind::Minus => {
                let start = match read_num(ts) {
                    Some(v) => v,
                    None => break,
                };
                // Optional `TO stop [BY step]` range continuation.
                if ts.peek().is_kw("to") {
                    ts.next();
                    let stop = read_num(ts).ok_or_else(|| {
                        SasError::parse("expected value after TO", ts.peek().span)
                    })?;
                    let step = if ts.peek().is_kw("by") {
                        ts.next();
                        read_num(ts).ok_or_else(|| {
                            SasError::parse("expected value after BY", ts.peek().span)
                        })?
                    } else {
                        1.0
                    };
                    if step == 0.0 {
                        out.push(start);
                    } else {
                        // Enumerate start, start+step, … up to stop inclusive
                        // (with a small tolerance so the endpoint is captured).
                        let n_steps = ((stop - start) / step).floor() as i64;
                        for i in 0..=n_steps.max(0) {
                            out.push(start + step * i as f64);
                        }
                        // Guard the inclusive endpoint against rounding.
                        let last = start + step * n_steps.max(0) as f64;
                        if (stop - last).abs() > step.abs() * 1e-9
                            && ((step > 0.0 && last + step <= stop + step.abs() * 1e-9)
                                || (step < 0.0 && last + step >= stop - step.abs() * 1e-9))
                        {
                            out.push(last + step);
                        }
                    }
                } else {
                    out.push(start);
                }
            }
            // A non-numeric identifier (other than the range keywords, which are
            // only valid after a start value) terminates the list — it's the
            // next PROC option.
            _ => break,
        }
    }
    Ok(out)
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

/// Parse the (optional) equation list of an MTEST statement (M36.10). Each
/// comma-separated entry is a linear combination of regressors; unlike
/// TEST/RESTRICT the `= rhs` is OPTIONAL and defaults to `= 0` (MTEST tests
/// linear combinations of the parameters against zero across all responses).
/// Stops at the terminating `;` or a `/` option separator. An empty list
/// (the statement is just `MTEST;`) yields no equations, meaning the default
/// "all non-intercept coefficients = 0" hypothesis.
fn parse_mtest_equations(ts: &mut StatementStream) -> Result<Vec<LinEq>> {
    let mut eqs = Vec::new();
    loop {
        // Terminators with no (further) equation.
        if matches!(
            ts.peek().kind,
            TokenKind::Semi | TokenKind::Eof | TokenKind::Slash
        ) {
            break;
        }
        // Left side: signed sum of regressor terms (and bare constants).
        let mut terms: Vec<(f64, String)> = Vec::new();
        let mut lhs_const = 0.0;
        parse_lin_side(ts, 1.0, &mut terms, &mut lhs_const)?;
        let mut rhs = -lhs_const;
        // Optional `= rhs`.
        if ts.peek().kind == TokenKind::Eq {
            ts.next();
            let mut rhs_terms: Vec<(f64, String)> = Vec::new();
            let mut rhs_const = 0.0;
            parse_lin_side(ts, 1.0, &mut rhs_terms, &mut rhs_const)?;
            for (c, v) in rhs_terms {
                terms.push((-c, v));
            }
            rhs += rhs_const;
        }
        // Merge duplicate variables.
        let mut merged: Vec<(f64, String)> = Vec::new();
        for (c, v) in terms {
            if let Some(e) = merged.iter_mut().find(|(_, name)| *name == v) {
                e.0 += c;
            } else {
                merged.push((c, v));
            }
        }
        eqs.push(LinEq { terms: merged, rhs });
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
