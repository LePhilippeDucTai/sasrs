use super::*;

/// Parse `proc univariate [data=a] [noprint] ; [var v...;] [by ...;] ... run;`.
/// Called AFTER "proc univariate" has been consumed. Consumes through
/// `run;`/`quit;`. Unknown sub-statements (e.g. BY, HISTOGRAM) are skipped
/// leniently to their terminating `;` (BY grouping is out of M5 scope).
pub fn parse(ts: &mut StatementStream) -> Result<UnivariateAst> {
    let mut data: Option<DatasetRef> = None;
    let mut var: Vec<String> = Vec::new();
    let mut normal = false;
    let mut plots: Vec<UnivariatePlot> = Vec::new();

    // --- PROC UNIVARIATE statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next(); // consume `;`
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            common::expect_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("noprint") {
            // Accepted and ignored for rendering: UNIVARIATE always shows its
            // report here. (NOPRINT only matters paired with OUTPUT in SAS.)
            ts.next();
        } else if ts.peek().is_kw("normal") || ts.peek().is_kw("normaltest") {
            // PROC-level request for the Tests for Normality block.
            ts.next();
            normal = true;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            // Unknown header option: skip its token (and a possible `=value`)
            // leniently rather than error, to stay synchronized.
            ts.next();
            if ts.peek().kind == TokenKind::Eq {
                ts.next();
                if ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    ts.next();
                }
            }
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC UNIVARIATE statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut by: Vec<(String, bool)> = Vec::new();
    let mut weight: Option<String> = None;
    let mut output: Option<UnivariateOutput> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        // Graphical statement (HISTOGRAM/QQPLOT/…) — keyword-driven via a token
        // probe rather than the lowercase `kw`, so handle it before the match.
        if let Some(kind) = graphics_kind(ts.peek()) {
            // Capture the kind and its target variable (the first identifier
            // after the keyword), then skip the rest of the body to `;`
            // (trailing `/ options` are tolerated but ignored). Rendering is
            // wired to ODS GRAPHICS (M29.3).
            ts.next(); // consume keyword
            let var = ts.peek().ident().map(str::to_string);
            if var.is_some() {
                ts.next();
            }
            ts.skip_to_semi();
            plots.push(UnivariatePlot { kind, var });
            return Ok(true);
        }
        Ok(match kw {
            "var" => {
                ts.next();
                var = ts.parse_name_list()?;
                // Optional `/ option…` clause on VAR (e.g. `var x / normal;`).
                if ts.peek().kind == TokenKind::Slash {
                    ts.next();
                    while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                        if ts.peek().is_kw("normal") || ts.peek().is_kw("normaltest") {
                            normal = true;
                        }
                        ts.next();
                    }
                }
                ts.expect_semi()?;
                true
            }
            "by" => {
                ts.next();
                by = crate::procs::means::parse_by_list(ts)?;
                true
            }
            "weight" => {
                ts.next();
                weight = Some(crate::procs::means::parse_single_var(ts, "WEIGHT")?);
                true
            }
            "output" => {
                ts.next();
                output = Some(parse_output(ts)?);
                true
            }
            _ => false,
        })
    })?;

    Ok(UnivariateAst {
        data,
        var,
        by,
        weight,
        output,
        normal,
        plots,
    })
}

/// Map a token to a graphical-statement kind, or `None` if it is not one.
pub(super) fn graphics_kind(tok: &crate::token::Token) -> Option<UnivariatePlotKind> {
    if tok.is_kw("histogram") {
        Some(UnivariatePlotKind::Histogram)
    } else if tok.is_kw("qqplot") {
        Some(UnivariatePlotKind::QqPlot)
    } else if tok.is_kw("probplot") {
        Some(UnivariatePlotKind::ProbPlot)
    } else if tok.is_kw("cdfplot") {
        Some(UnivariatePlotKind::CdfPlot)
    } else if tok.is_kw("ppplot") {
        Some(UnivariatePlotKind::PpPlot)
    } else {
        None
    }
}

/// Recognized OUTPUT statistic keywords (paired positionally with VAR list).
pub(super) fn is_output_stat(s: &str) -> bool {
    matches!(
        s,
        "mean"
            | "std"
            | "stddev"
            | "min"
            | "max"
            | "median"
            | "n"
            | "nmiss"
            | "sum"
            | "q1"
            | "q3"
            | "p25"
            | "p75"
            | "p50"
            | "p1"
            | "p5"
            | "p10"
            | "p90"
            | "p95"
            | "p99"
            | "range"
            | "qrange"
            | "var"
    )
}

/// Parse the OUTPUT statement body (after "output" consumed), through `;`.
/// `output out=lib.t [stat=name [name...]] ... ;` — each statistic keyword is
/// followed by one or more output variable names, paired positionally with the
/// VAR list. `var=` is accepted as the VARIANCE keyword.
pub(super) fn parse_output(ts: &mut StatementStream) -> Result<UnivariateOutput> {
    let mut out: Option<DatasetRef> = None;
    let mut specs: Vec<(String, Vec<String>)> = Vec::new();

    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("out") {
            common::expect_eq(ts, "OUT")?;
            out = Some(ts.parse_dataset_ref()?);
        } else if let Some(kw) = ts.peek().ident().map(str::to_string) {
            let stat = kw.to_ascii_lowercase();
            if !is_output_stat(&stat) {
                return Err(SasError::parse(
                    format!("Unsupported statistic '{}' in OUTPUT statement.", kw.to_uppercase()),
                    ts.peek().span,
                ));
            }
            common::expect_eq(ts, "OUTPUT statistic")?;
            // Collect one or more output names until the next stat keyword,
            // `out`, or `;`.
            let mut names: Vec<String> = Vec::new();
            while let Some(n) = ts.peek().ident().map(str::to_string) {
                let nl = n.to_ascii_lowercase();
                // Stop if this ident is actually the next keyword followed
                // by '=' (e.g. `mean=mx n=nx`).
                if (is_output_stat(&nl) || nl == "out") && ts.peek2().kind == TokenKind::Eq {
                    break;
                }
                ts.next();
                names.push(n);
            }
            if names.is_empty() {
                return Err(SasError::parse(
                    format!("expected an output variable name after {}=", stat),
                    ts.peek().span,
                ));
            }
            specs.push((stat, names));
        } else {
            return Err(SasError::parse(
                "unexpected token in OUTPUT statement",
                ts.peek().span,
            ));
        }
    }

    let out = out.ok_or_else(|| {
        SasError::runtime("The OUTPUT statement requires the OUT= option in PROC UNIVARIATE.")
    })?;
    Ok(UnivariateOutput { out, specs })
}
