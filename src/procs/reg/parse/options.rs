use super::*;

/// PROC-statement options of PROC REG (MQ5.1) — everything parsed from the
/// `PROC REG …;` statement itself, before the sub-statements.
pub(super) struct ProcOptions {
    pub(super) input: Option<DatasetRef>,
    pub(super) simple: bool,
    pub(super) corr: bool,
    pub(super) proc_all: bool,
    pub(super) outest: Option<OutEst>,
    pub(super) outsscp: Option<DatasetRef>,
    pub(super) ridge: Vec<f64>,
    pub(super) pcomit: Vec<f64>,
    pub(super) outvif: bool,
    pub(super) plot_requests: PlotRequests,
}

/// MQ5.1 — parse the options of the `PROC REG …;` statement itself, up to
/// and including its terminating `;`.
pub(super) fn parse_proc_options(ts: &mut StatementStream) -> Result<ProcOptions> {
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

pub(super) fn parse_output_stmt(ts: &mut StatementStream, models: &mut [RegModelEntry]) -> Result<()> {
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

/// Parse a SAS numeric value-list for RIDGE=/PCOMIT= (M36.9). Accepts a plain
/// list of numbers (`0 0.01 0.05 0.1`) and the SAS range form
/// `start TO stop [BY step]` (default step 1), in any mix. Stops at the next
/// option keyword (a non-numeric, non-`TO`/`BY` identifier), a `;`, or EOF.
/// Negative values are tolerated via a leading `-` (e.g. `-0.5`).
pub(super) fn parse_value_list(ts: &mut StatementStream) -> Result<Vec<f64>> {
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
pub(super) fn read_float(ts: &mut StatementStream) -> Result<f64> {
    match ts.peek().kind {
        TokenKind::Num(v) => {
            ts.next();
            Ok(v)
        }
        _ => Err(SasError::parse("expected numeric value", ts.peek().span)),
    }
}
