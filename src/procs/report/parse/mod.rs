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
            "nowd" | "nowindow" => {
                // No-op: we never open an interactive window.
                ts.next();
                true
            }
            "noheader" => {
                ts.next();
                noheader = true;
                true
            }
            "headline" | "headskip" => {
                // No-op cosmetic options (rule line / skip line under headers).
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
            ts.next();
            rbreak = Some(parse_break(ts, true)?);
        } else if ts.peek().is_kw("where") {
            ts.next();
            where_ = Some(crate::parser::expr::parse_expr(ts)?);
            ts.expect_semi()?;
        } else {
            return Ok(false);
        }
        Ok(true)
    })?;

    Ok(ReportAst {
        data,
        noheader,
        columns,
        defines,
        where_,
        out,
        breaks,
        rbreak,
        computes,
    })
}
