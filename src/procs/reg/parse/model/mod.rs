use super::*;

mod lineq;

pub(super) use lineq::*;

/// MQ5.1 — parse a `MODEL y1 [y2 …] = x1 x2 … [/ options];` statement (the
/// `model` keyword has not been consumed yet). `proc_all` mirrors the
/// PROC-level ALL option, which forces the matrix/CL options on every model.
pub(super) fn parse_model_stmt(ts: &mut StatementStream, proc_all: bool) -> Result<RegModelEntry> {
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
                    common::consume_option_eq(ts, "SLENTRY")?;
                    let v = read_float(ts)?;
                    if let Some(sel) = selection.as_mut() {
                        sel.slentry = v;
                    }
                } else if ts.peek().is_kw("slstay") || ts.peek().is_kw("sls") {
                    common::consume_option_eq(ts, "SLSTAY")?;
                    let v = read_float(ts)?;
                    if let Some(sel) = selection.as_mut() {
                        sel.slstay = v;
                    }
                } else if ts.peek().is_kw("best") {
                    common::consume_option_eq(ts, "BEST")?;
                    let v = read_float(ts)? as usize;
                    if let Some(sel) = selection.as_mut() {
                        sel.best = Some(v);
                    }
                } else if ts.peek().is_kw("include") {
                    common::consume_option_eq(ts, "INCLUDE")?;
                    let v = read_float(ts)? as usize;
                    if let Some(sel) = selection.as_mut() {
                        sel.include = v;
                    }
                } else if ts.peek().is_kw("start") {
                    common::consume_option_eq(ts, "START")?;
                    let v = read_float(ts)? as usize;
                    if let Some(sel) = selection.as_mut() {
                        sel.start = Some(v);
                    }
                } else if ts.peek().is_kw("stop") {
                    common::consume_option_eq(ts, "STOP")?;
                    let v = read_float(ts)? as usize;
                    if let Some(sel) = selection.as_mut() {
                        sel.stop = Some(v);
                    }
                } else if ts.peek().is_kw("groupnames") {
                    // GROUPNAMES="g1" "g2" ... — parsed and ignored
                    // (used by SAS only to label grouped regressors in
                    // the selection display). Consume the `=` and the
                    // following string/ident list.
                    common::consume_option_eq(ts, "GROUPNAMES")?;
                    while matches!(ts.peek().kind, TokenKind::Str { .. } | TokenKind::Ident(_)) {
                        ts.next();
                    }
                } else if ts.peek().is_kw("details") {
                    if let Some(sel) = selection.as_mut() {
                        sel.details = true;
                    }
                    ts.next();
                } else if ts.peek().is_kw("alpha") {
                    common::consume_option_eq(ts, "ALPHA")?;
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
