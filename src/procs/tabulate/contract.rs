// ───────────────────────── Contract diagnostics (J02-P7) ─────────────────────────
//
// Base SAS 9.4 Procedures Guide, The TABULATE Procedure, Syntax
// (https://documentation.sas.com/doc/en/proc/9.4/p1g617vn5t3p39n0z9o601rw74jz.htm,
// TABLE statement): constructions that sasrs used to ignore or approximate
// without a diagnostic (audit d0b4d90). CONTRIBUTING §5: a construction that
// can change a result is an ERROR (step rejected at parse time), a display-only
// one a WARNING. Same wording as the other contract units (J02-P1..P6).

use super::*;

use crate::token::Span;

/// Suffix naming the roadmap-avancee unit that will lift a provisional ERROR.
pub(super) fn planned(unit: Option<&str>) -> String {
    match unit {
        Some(u) => format!(" (planned: roadmap-avancee {u})"),
        None => String::new(),
    }
}

/// ERROR for a construction sasrs cannot represent faithfully.
pub(super) fn unsupported_construct(msg: &str, unit: Option<&str>, span: Span) -> SasError {
    SasError::parse(
        format!(
            "{msg} is not supported in PROC TABULATE; it can affect results and cannot be \
             ignored{}.",
            planned(unit)
        ),
        span,
    )
}

/// Statistic keywords of SAS 9.4 PROC TABULATE (« Statistics That Are
/// Available in PROC TABULATE ») that sasrs does not compute. They used to be
/// rejected during execution, after the titles and the procedure heading had
/// been written to the listing.
pub(super) const UNSUPPORTED_STATS: &[&str] = &[
    "colpctn",
    "colpctsum",
    "css",
    "cv",
    "kurt",
    "kurtosis",
    "lclm",
    "median",
    "mode",
    "p1",
    "p5",
    "p10",
    "p20",
    "p25",
    "p30",
    "p40",
    "p50",
    "p60",
    "p70",
    "p75",
    "p80",
    "p90",
    "p95",
    "p99",
    "pagepctn",
    "pagepctsum",
    "probt",
    "prt",
    "q1",
    "q3",
    "qrange",
    "range",
    "reppctn",
    "reppctsum",
    "rowpctn",
    "rowpctsum",
    "skew",
    "skewness",
    "stddev",
    "stderr",
    "sumwgt",
    "t",
    "uclm",
    "uss",
    "var",
];

/// Valid PROC TABULATE statements that only label or style the headings
/// (KEYLABEL, CLASSLEV, KEYWORD): ignoring them leaves the computed table
/// unchanged, hence a display WARNING. FREQ and WEIGHT keep the shared
/// « not supported » ERROR of `common::unhandled_proc_statement`.
pub(super) const DISPLAY_STATEMENTS: &[&str] = &["classlev", "keylabel", "keyword"];

/// Check every name of the TABLE dimensions against the CLASS and VAR lists
/// before execution: an unsupported statistic keyword is the contract ERROR,
/// any other unknown name the historical « not yet supported » ERROR. Both
/// used to surface only after the listing heading was written.
pub(super) fn check_table_names(dims: &[&DimExpr], class: &[String], var: &[String]) -> Result<()> {
    for dim in dims {
        for term in &dim.terms {
            for factor in &term.factors {
                match factor {
                    Factor::Group(inner) => check_table_names(&[inner], class, var)?,
                    Factor::Name { name, span, .. } => check_table_name(name, *span, class, var)?,
                }
            }
        }
    }
    Ok(())
}

fn check_table_name(name: &str, span: Span, class: &[String], var: &[String]) -> Result<()> {
    let known = |list: &[String]| list.iter().any(|n| n.eq_ignore_ascii_case(name));
    if name.eq_ignore_ascii_case("all") || is_stat_keyword(name) || known(class) || known(var) {
        return Ok(());
    }
    let lower = name.to_ascii_lowercase();
    if UNSUPPORTED_STATS.contains(&lower.as_str()) {
        return Err(unsupported_construct(
            &format!("The {} statistic", name.to_ascii_uppercase()),
            None,
            span,
        ));
    }
    Err(SasError::parse(
        format!("PROC TABULATE: {} not yet supported", name.to_uppercase()),
        span,
    ))
}

/// NOTE written at every execution with OUT= (« Approximations documentées »
/// of docs/support-contract.md): SAS writes one observation per combination of
/// class variable values, with all of its statistics (Base SAS 9.4, PROC
/// TABULATE, « Output Data Set »); sasrs writes one per cell and statistic.
pub(super) const OUT_APPROXIMATION_NOTE: &str = "PROC TABULATE approximates the SAS OUT= data \
     set: it writes one observation per table cell and statistic, where SAS writes one \
     observation per combination of class variable values with all of its statistics \
     (planned: roadmap-avancee J03-P2).";
