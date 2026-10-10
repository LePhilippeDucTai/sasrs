use super::*;

mod compute;
mod define;

pub(crate) use compute::*;
pub(crate) use define::*;

/// Statistic keywords accepted after an ANALYSIS usage on a DEFINE.
pub(crate) const ANALYSIS_STATS: &[&str] = &["sum", "mean", "min", "max", "n", "std"];

pub(crate) fn is_analysis_stat(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    ANALYSIS_STATS.iter().any(|k| *k == l)
}

/// Parse a PROC REPORT block. Called AFTER `proc report` has been consumed.
/// Consumes through `run;`/`quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<ReportAst> {
    let mut data: Option<DatasetRef> = None;
    let mut noheader = false;
    let mut columns: Option<Vec<String>> = None;
    let mut defines: Vec<Define> = Vec::new();
    let mut where_: Option<Expr> = None;
    let mut out: Option<DatasetRef> = None;
    let mut breaks: Vec<Break> = Vec::new();
    let mut rbreak: Option<Break> = None;
    let mut computes: Vec<Compute> = Vec::new();

    // --- PROC REPORT statement options, until `;` (combinateur partagé M31) ---
    common::parse_proc_options(ts, "REPORT", |ts, kw| {
        Ok(match kw {
            "data" => {
                data = Some(common::parse_dataset_opt(ts, "DATA")?);
                true
            }
            "out" => {
                out = Some(common::parse_out_opt(ts)?);
                true
            }
            // NOWINDOWS (SAS spelling, alias NOWD; `nowindow` kept for the
            // existing programs): no-op, we never open an interactive window.
            "nowd" | "nowindow" | "nowindows" => {
                ts.next();
                true
            }
            "noheader" => {
                ts.next();
                noheader = true;
                true
            }
            // J02-P7 — rule line / blank line under the headings: not rendered
            // (they used to be accepted without a word).
            "headline" | "headskip" => {
                ts.warn_ignored_display(contract::ignored_display_option(
                    "PROC",
                    &kw.to_ascii_uppercase(),
                    Some("J11-P3"),
                ));
                ts.next();
                true
            }
            _ => false,
        })
    })?;

    // --- sub-statements until run;/quit; ---
    crate::procs::common::parse_proc_body(ts, "REPORT", |ts, _kw| {
        if ts.peek().is_kw("column") || ts.peek().is_kw("columns") {
            ts.next();
            columns = Some(ts.parse_name_list()?);
            ts.expect_semi()?;
        } else if ts.peek().is_kw("define") {
            ts.next();
            defines.push(parse_define(ts)?);
        } else if ts.peek().is_kw("compute") {
            ts.next();
            computes.push(parse_compute(ts)?);
        } else if ts.peek().is_kw("break") {
            ts.next();
            breaks.push(parse_break(ts, false)?);
        } else if ts.peek().is_kw("rbreak") {
            // J02-P7 — a second RBREAK used to replace the first one.
            if rbreak.is_some() {
                return Err(contract::unsupported_construct(
                    "More than one RBREAK statement",
                    Some("J11-P2"),
                    ts.peek().span,
                ));
            }
            ts.next();
            rbreak = Some(parse_break(ts, true)?);
        } else if ts.peek().is_kw("where") {
            ts.next();
            let span = ts.peek().span;
            let cond = crate::parser::expr::parse_expr(ts)?;
            contract::check_expr(&cond, "in the WHERE statement", span)?;
            where_ = Some(cond);
            ts.expect_semi()?;
        } else {
            return Ok(false);
        }
        Ok(true)
    })?;

    let ast = ReportAst {
        data,
        noheader,
        columns,
        defines,
        where_,
        out,
        breaks,
        rbreak,
        computes,
    };
    contract::check_parsed(&ast, ts)?;
    Ok(ast)
}
