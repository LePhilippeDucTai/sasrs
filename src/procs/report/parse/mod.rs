use super::*;


mod define;
mod compute;

pub(crate) use define::*;
pub(crate) use compute::*;

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
    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }

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
        } else if is_global_stmt_kw(ts.peek().ident()) {
            // TITLE/FOOTNOTE (and numbered variants) are global statements that
            // SAS accepts anywhere, including inside a PROC step. We don't act on
            // them here (global title/footnote state is owned by the executor and
            // is set by the global statements placed before the step) — we just
            // skip them gracefully rather than aborting the whole REPORT, matching
            // the leniency of PROC PRINT and others.
            ts.skip_to_semi();
        } else {
            let span = ts.peek().span;
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unexpected statement '{bad}' in PROC REPORT."),
                span,
            ));
        }
    }

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

/// True if `ident` names a global statement (TITLE/FOOTNOTE, plain or numbered
/// e.g. TITLE2/FOOTNOTE3) that SAS allows inside a PROC step. Used to skip such
/// statements gracefully in the REPORT sub-statement loop.
pub(crate) fn is_global_stmt_kw(ident: Option<&str>) -> bool {
    let Some(w) = ident else { return false };
    let lw = w.to_ascii_lowercase();
    let stem = lw.trim_end_matches(|c: char| c.is_ascii_digit());
    matches!(stem, "title" | "footnote")
}
