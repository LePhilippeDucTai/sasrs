// ───────────────────────── Contract diagnostics (J02-P7) ─────────────────────────
//
// Base SAS 9.4 Procedures Guide, The REPORT Procedure: constructions that
// sasrs used to ignore or approximate without a diagnostic (audit d0b4d90).
// CONTRIBUTING §5: a construction that can change a result is an ERROR (step
// rejected at parse time when the parse can tell), a display-only one a
// WARNING. Same wording as the other contract units (J02-P1..P6).

use super::*;

use crate::token::Span;

/// Suffix naming the roadmap-avancee unit that will lift a provisional ERROR.
pub(super) fn planned(unit: Option<&str>) -> String {
    match unit {
        Some(u) => format!(" (planned: roadmap-avancee {u})"),
        None => String::new(),
    }
}

fn unsupported_text(msg: &str, unit: Option<&str>) -> String {
    format!(
        "{msg} is not supported in PROC REPORT; it can affect results and cannot be ignored{}.",
        planned(unit)
    )
}

/// ERROR for a construction sasrs cannot represent faithfully (parse time).
pub(super) fn unsupported_construct(msg: &str, unit: Option<&str>, span: Span) -> SasError {
    SasError::parse(unsupported_text(msg, unit), span)
}

/// WARNING text for a display-only option that is not rendered.
pub(super) fn ignored_display_option(stmt: &str, opt: &str, unit: Option<&str>) -> String {
    let what = if stmt == "PROC" {
        format!("The {opt} option")
    } else {
        format!("The {opt} option of the {stmt} statement")
    };
    format!(
        "{what} is ignored in PROC REPORT; display customization is not supported{}.",
        planned(unit)
    )
}

/// SAS 9.4 BREAK statement: « break-variable is a group or order variable ».
/// SAS rejects any other variable with this ERROR; sasrs used to drop the
/// BREAK statement without a word.
pub(super) const BREAK_VARIABLE_ERROR: &str =
    "You can only BREAK on GROUPing and ORDERing variables.";

/// ERROR for an RBREAK statement in a report without GROUP or ORDER
/// variable: the grand-total line was silently dropped.
pub(super) fn rbreak_in_detail_report() -> String {
    unsupported_text(
        "An RBREAK statement in a detail report (no GROUP or ORDER variable)",
        Some("J11-P2"),
    )
}

/// SAS log NOTE when GROUP variables cannot consolidate the observations
/// because of a DISPLAY or ORDER variable (« Groups are not created because
/// the usage of Title is DISPLAY. To avoid this note, change all GROUP
/// variables to ORDER variables. », SAS 9.4 log).
pub(super) fn groups_not_created_note(name: &str, usage: &str) -> String {
    format!(
        "Groups are not created because the usage of {name} is {usage}. To avoid this note, \
         change all GROUP variables to ORDER variables."
    )
}

/// Reject an expression that the local evaluator cannot compute: function
/// calls (`upcase(sex)`), array references and hash methods used to evaluate
/// to a missing value without a diagnostic — `where upcase(sex)='F'` emptied
/// the report. `place` completes the message (« in the WHERE statement »).
pub(super) fn check_expr(expr: &Expr, place: &str, span: Span) -> Result<()> {
    let what = match expr {
        Expr::Num(_) | Expr::Str(_) | Expr::Missing(_) | Expr::Var(_) => return Ok(()),
        Expr::Unary { expr, .. } => return check_expr(expr, place, span),
        Expr::Binary { left, right, .. } => {
            check_expr(left, place, span)?;
            return check_expr(right, place, span);
        }
        Expr::In { expr, list } => {
            check_expr(expr, place, span)?;
            for item in list {
                check_expr(item, place, span)?;
            }
            return Ok(());
        }
        Expr::Call { name, .. } => format!("A function call ({})", name.to_ascii_uppercase()),
        Expr::Index { name, .. } => format!("An array reference ({})", name.to_ascii_uppercase()),
        Expr::HashMethod(call) => format!(
            "A method call ({}.{})",
            call.object.to_ascii_uppercase(),
            call.method.to_ascii_uppercase()
        ),
    };
    Err(unsupported_construct(
        &format!("{what} {place}"),
        Some("J03-P3"),
        span,
    ))
}

/// `_BREAK_`, the automatic variable of the break lines, is not maintained:
/// it used to read a missing value.
pub(super) fn check_break_variable(expr: &Expr, span: Span) -> Result<()> {
    let mut refs = Vec::new();
    expr_refs(expr, &mut refs);
    if refs.iter().any(|n| n.eq_ignore_ascii_case("_break_")) {
        return Err(unsupported_construct(
            "The _BREAK_ automatic variable",
            Some("J11-P2"),
            span,
        ));
    }
    Ok(())
}

/// Usage of a COLUMN item as far as the statements tell (parse time): the
/// usage of its DEFINE, `None` without DEFINE (DISPLAY or ANALYSIS by type).
fn parsed_usage<'a>(ast: &'a ReportAst, name: &str) -> Option<&'a Usage> {
    ast.defines
        .iter()
        .find(|d| d.var.eq_ignore_ascii_case(name))
        .map(|d| &d.usage)
}

/// The report items known at parse time: the COLUMN list with the usage of
/// their DEFINE. Without COLUMN, the defined variables (the data set decides
/// the rest at execution, see `check_plan`).
fn parsed_items(ast: &ReportAst) -> Vec<(String, Option<Usage>)> {
    match &ast.columns {
        Some(cols) => cols
            .iter()
            .map(|c| (c.clone(), parsed_usage(ast, c).cloned()))
            .collect(),
        None => ast
            .defines
            .iter()
            .map(|d| (d.var.clone(), Some(d.usage.clone())))
            .collect(),
    }
}

/// Checks that only need the statements (end of parse). Without COLUMN the
/// report items are only known with the data set: `check_plan` repeats these
/// rules at execution, before any output.
pub(super) fn check_parsed(ast: &ReportAst, ts: &mut StatementStream) -> Result<()> {
    let span = ts.peek().span;
    let items = parsed_items(ast);
    let usage_of = |name: &str| {
        items
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .and_then(|(_, u)| u.clone())
    };
    let is_group_or_order = |u: &Option<Usage>| matches!(u, Some(Usage::Group | Usage::Order));

    if items.iter().any(|(_, u)| matches!(u, Some(Usage::Across))) {
        if let Some(what) = across_statement_issue(ast) {
            return Err(SasError::parse(across_text(&what), span));
        }
        for d in ast.defines.iter().filter(|d| d.spacing.is_some()) {
            ts.warn_ignored_display(format!(
                "The SPACING= option of the DEFINE statement for {} is ignored in an ACROSS \
                 report of PROC REPORT; display customization is not supported{}.",
                d.var.to_ascii_uppercase(),
                planned(Some("J11-P3"))
            ));
        }
        return Ok(());
    }
    for brk in &ast.breaks {
        let var = brk.var.as_deref().unwrap_or_default();
        if !is_group_or_order(&usage_of(var)) {
            return Err(SasError::parse(BREAK_VARIABLE_ERROR, span));
        }
    }
    if ast.rbreak.is_some() && !items.iter().any(|(_, u)| is_group_or_order(u)) {
        return Err(SasError::parse(rbreak_in_detail_report(), span));
    }
    if let Some(cols) = &ast.columns {
        let names: Vec<&str> = cols.iter().map(String::as_str).collect();
        check_compute_items(ast, &names).map_err(|msg| SasError::parse(msg, span))?;
    }
    Ok(())
}

/// COMPUTE targets and assignment destinations must be report items (name
/// or `_Cn_`). A target outside the report used to run for nothing; an
/// assignment to any other name overwrote the target column.
fn check_compute_items(ast: &ReportAst, names: &[&str]) -> std::result::Result<(), String> {
    for comp in &ast.computes {
        if comp.target.eq_ignore_ascii_case("after") {
            continue;
        }
        if resolve_in(names, &comp.target).is_none() {
            return Err(format!(
                "The COMPUTE block target {} is not a report item of the COLUMN statement in \
                 PROC REPORT.",
                comp.target.to_ascii_uppercase()
            ));
        }
        for st in &comp.stmts {
            if let ComputeStmt::Assign { col, .. } = st
                && resolve_in(names, col).is_none()
            {
                return Err(unsupported_text(
                    &format!(
                        "An assignment to {}, which is not a report item (a temporary variable \
                         of the COMPUTE block),",
                        col.to_ascii_uppercase()
                    ),
                    None,
                ));
            }
        }
    }
    Ok(())
}

/// ERROR text for a construction `execute_across` does not render.
fn across_text(what: &str) -> String {
    unsupported_text(&format!("{what} in an ACROSS report"), Some("J11-P3"))
}

/// The statement or DEFINE option that `execute_across` used to ignore, if
/// any: BREAK, RBREAK, COMPUTE (summary lines and computed values), OUT= (a
/// NOTE said « OUT= ignored »), FORMAT= and WIDTH=.
fn across_statement_issue(ast: &ReportAst) -> Option<String> {
    if !ast.breaks.is_empty() {
        return Some("A BREAK statement".to_string());
    }
    if ast.rbreak.is_some() {
        return Some("An RBREAK statement".to_string());
    }
    if !ast.computes.is_empty() {
        return Some("A COMPUTE block".to_string());
    }
    if ast.out.is_some() {
        return Some("The OUT= option".to_string());
    }
    for d in &ast.defines {
        if d.format.is_some() {
            return Some(format!(
                "The FORMAT= option of the DEFINE statement ({})",
                d.var.to_ascii_uppercase()
            ));
        }
        if d.width.is_some() {
            return Some(format!(
                "The WIDTH= option of the DEFINE statement ({})",
                d.var.to_ascii_uppercase()
            ));
        }
    }
    None
}

/// Checks that need the data set (execution, before any output): the same
/// rules as `check_parsed` on the resolved column plan, plus the ACROSS
/// layout, the COMPUTE targets and assignments, and the WHERE variables.
pub(super) fn check_plan(
    ast: &ReportAst,
    plan: &[ColPlan],
    ds: &crate::dataset::SasDataset,
    display_name: &str,
) -> Result<()> {
    // WHERE: a variable absent from the data set used to evaluate to a
    // missing value (SAS: ERROR, nothing is read).
    if let Some(cond) = &ast.where_ {
        let mut names = Vec::new();
        expr_refs(cond, &mut names);
        for name in names {
            if !ds.vars.iter().any(|m| m.name.eq_ignore_ascii_case(&name)) {
                return Err(SasError::runtime(format!(
                    "Variable {name} is not on file {display_name}."
                )));
            }
        }
    }

    let has_across = plan.iter().any(|c| c.usage == Usage::Across);
    if has_across {
        return check_across_plan(ast, plan, ds);
    }
    let is_group_or_order = |c: &ColPlan| matches!(c.usage, Usage::Group | Usage::Order);
    for brk in &ast.breaks {
        let var = brk.var.as_deref().unwrap_or_default();
        if !plan
            .iter()
            .any(|c| is_group_or_order(c) && c.name.eq_ignore_ascii_case(var))
        {
            return Err(SasError::runtime(BREAK_VARIABLE_ERROR));
        }
    }
    if ast.rbreak.is_some() && !plan.iter().any(is_group_or_order) {
        return Err(SasError::runtime(rbreak_in_detail_report()));
    }
    let names: Vec<&str> = plan.iter().map(|c| c.name.as_str()).collect();
    check_compute_items(ast, &names).map_err(SasError::runtime)
}

/// `execute_across` renders GROUP rows × one ACROSS variable × one ANALYSIS
/// variable; every other column, and any format, used to be dropped.
fn check_across_plan(
    ast: &ReportAst,
    plan: &[ColPlan],
    ds: &crate::dataset::SasDataset,
) -> Result<()> {
    let across = |what: String| SasError::runtime(across_text(&what));
    if let Some(what) = across_statement_issue(ast) {
        return Err(across(what));
    }
    if plan.iter().filter(|c| c.usage == Usage::Across).count() > 1 {
        return Err(across("More than one ACROSS variable".to_string()));
    }
    if plan
        .iter()
        .filter(|c| matches!(c.usage, Usage::Analysis(_)))
        .count()
        > 1
    {
        return Err(across("More than one ANALYSIS variable".to_string()));
    }
    for c in plan {
        let usage = match c.usage {
            Usage::Order => "An ORDER variable",
            Usage::Display => "A DISPLAY variable",
            Usage::Computed => "A COMPUTED variable",
            _ => "",
        };
        if !usage.is_empty() {
            return Err(across(format!("{usage} ({})", c.name.to_ascii_uppercase())));
        }
        // Stored formats are applied by default (DEFINE FORMAT= first); the
        // ACROSS rendering applies none, so a stored format would be silently
        // dropped there (grouping and headings included).
        if c.idx != usize::MAX
            && let Some(fmt) = &ds.vars[c.idx].format
        {
            return Err(across(format!(
                "The format {} of {}",
                fmt.to_ascii_uppercase(),
                c.name.to_ascii_uppercase()
            )));
        }
    }
    Ok(())
}

/// Resolve a COMPUTE reference to a report item: its name in the COLUMN
/// statement (variable name, or the name of a COMPUTED item) or its absolute
/// column reference `_Cn_` (SAS 9.4 COMPUTE statement, « Four Ways to
/// Reference Report Items in a Compute Block »). The DEFINE label is a
/// heading, never a name: references by label used to resolve and those by
/// name to fail (missing value, assignment lost).
pub(super) fn resolve_item(plan: &[ColPlan], name: &str) -> Option<usize> {
    let names: Vec<&str> = plan.iter().map(|c| c.name.as_str()).collect();
    resolve_in(&names, name)
}

/// `resolve_item` over the names of the report items, in COLUMN order.
fn resolve_in(names: &[&str], name: &str) -> Option<usize> {
    if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(name)) {
        return Some(i);
    }
    let lower = name.to_ascii_lowercase();
    let n: usize = lower.strip_prefix("_c")?.strip_suffix('_')?.parse().ok()?;
    (1..=names.len()).contains(&n).then(|| n - 1)
}
