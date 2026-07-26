use super::*;

/// Everything the listing report needs (grouped to keep signatures short).
pub(super) struct ReportCtx<'a> {
    pub(super) base_display: &'a str,
    pub(super) comp_display: &'a str,
    pub(super) base_nobs: usize,
    pub(super) base_nvars: usize,
    pub(super) comp_nobs: usize,
    pub(super) comp_nvars: usize,
    pub(super) only_base: &'a [String],
    pub(super) only_comp: &'a [String],
    pub(super) common_vars: &'a [CommonVar],
    pub(super) n_matching: usize,
    pub(super) n_compared: usize,
    pub(super) n_with_diffs: usize,
    pub(super) var_diffs: &'a [VarDiffSummary],
}

/// Full listing report: Data Set Summary, Variables Summary, Observation
/// Summary and (unless NOVALUES) the Values Comparison Summary.
pub(super) fn print_full_report(session: &mut Session, ast: &CompareAst, ctx: &ReportCtx<'_>) {
    // === Data Set Summary ===
    session.listing.write_line("The COMPARE Procedure");
    session.listing.blank();
    session.listing.write_line("Data Set Summary");
    session.listing.blank();

    let ds_headers = vec![
        "Dataset".to_string(),
        "Role".to_string(),
        "Label".to_string(),
        "Observations".to_string(),
        "Variables".to_string(),
    ];
    let ds_aligns = vec![
        Align::Left,
        Align::Left,
        Align::Left,
        Align::Right,
        Align::Right,
    ];
    let ds_rows = vec![
        vec![
            ctx.base_display.to_string(),
            "BASE".to_string(),
            String::new(),
            ctx.base_nobs.to_string(),
            ctx.base_nvars.to_string(),
        ],
        vec![
            ctx.comp_display.to_string(),
            "COMPARE".to_string(),
            String::new(),
            ctx.comp_nobs.to_string(),
            ctx.comp_nvars.to_string(),
        ],
    ];
    session
        .listing
        .write_table(&ds_headers, &ds_aligns, &ds_rows);
    session.listing.blank();

    // === Variables Summary ===
    session.listing.write_line("Variables Summary");
    session.listing.blank();
    let n_common = ctx.n_matching;
    let n_type_mismatch = ctx.common_vars.iter().filter(|cv| !cv.type_match).count();
    session.listing.write_line(&format!(
        "Number of Variables in Common: {}",
        ctx.common_vars.len()
    ));
    if n_type_mismatch > 0 {
        session.listing.write_line(&format!(
            "Number of Variables with Different Types: {}",
            n_type_mismatch
        ));
        for cv in ctx.common_vars.iter().filter(|cv| !cv.type_match) {
            session.listing.write_line(&format!(
                "  Variable {}: BASE type={}, COMPARE type={}",
                cv.name,
                type_str(cv.base_type),
                type_str(cv.comp_type)
            ));
        }
    }
    if !ctx.only_base.is_empty() {
        session.listing.write_line(&format!(
            "Variables in BASE only ({}): {}",
            ctx.only_base.len(),
            ctx.only_base.join(", ")
        ));
    }
    if !ctx.only_comp.is_empty() {
        session.listing.write_line(&format!(
            "Variables in COMPARE only ({}): {}",
            ctx.only_comp.len(),
            ctx.only_comp.join(", ")
        ));
    }
    session.listing.blank();

    // === Observation Summary ===
    session.listing.write_line("Observation Summary");
    session.listing.blank();
    let n_uncompared = (ctx.base_nobs as isize - ctx.comp_nobs as isize).unsigned_abs();
    session.listing.write_line(&format!(
        "Number of Observations in Common: {}",
        ctx.n_compared
    ));
    if ctx.n_compared < ctx.base_nobs.max(ctx.comp_nobs) {
        session.listing.write_line(&format!(
            "Number of Observations Not Compared (different N): {}",
            n_uncompared
        ));
    }
    session.listing.write_line(&format!(
        "Number of Observations with Differences: {}",
        ctx.n_with_diffs
    ));
    session.listing.write_line(&format!(
        "Number of Observations in Agreement: {}",
        ctx.n_compared - ctx.n_with_diffs
    ));
    session.listing.blank();

    // === Values Comparison ===
    if !ast.novalues && n_common > 0 {
        session.listing.write_line("Values Comparison Summary");
        session.listing.blank();

        let val_headers = vec![
            "Variable".to_string(),
            "Type".to_string(),
            "N Diffs".to_string(),
            "Max Diff".to_string(),
        ];
        let val_aligns = vec![Align::Left, Align::Left, Align::Right, Align::Right];
        let val_rows: Vec<Vec<String>> = ctx
            .var_diffs
            .iter()
            .map(|vd| {
                let max_diff_str = if vd.var_type == VarType::Num && vd.n_diffs > 0 {
                    format!("{:.6}", vd.max_diff)
                } else if vd.var_type == VarType::Char {
                    String::new()
                } else {
                    "0".to_string()
                };
                vec![
                    vd.name.clone(),
                    type_str(vd.var_type).to_string(),
                    vd.n_diffs.to_string(),
                    max_diff_str,
                ]
            })
            .collect();
        session
            .listing
            .write_table(&val_headers, &val_aligns, &val_rows);
    }
}

/// BRIEFSUMMARY: condensed report (totals only).
pub(super) fn print_brief_report(session: &mut Session, ctx: &ReportCtx<'_>) {
    session
        .listing
        .write_line("The COMPARE Procedure - Brief Summary");
    session.listing.blank();
    session.listing.write_line(&format!(
        "BASE:    {} ({} obs, {} vars)",
        ctx.base_display, ctx.base_nobs, ctx.base_nvars
    ));
    session.listing.write_line(&format!(
        "COMPARE: {} ({} obs, {} vars)",
        ctx.comp_display, ctx.comp_nobs, ctx.comp_nvars
    ));
    session.listing.write_line(&format!(
        "Observations compared: {}  with differences: {}",
        ctx.n_compared, ctx.n_with_diffs
    ));
}
