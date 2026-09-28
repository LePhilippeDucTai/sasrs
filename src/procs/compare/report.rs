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
    /// Nombre de paires d'observations appariées (BY/ID/position).
    pub(super) n_matched: usize,
    /// Nombre de paires appariées avec au moins une valeur jugée inégale.
    pub(super) n_unequal: usize,
    /// Observations BASE sans appariement (1-based, ordre de lecture).
    pub(super) base_only_obs: &'a [usize],
    pub(super) comp_only_obs: &'a [usize],
    pub(super) var_diffs: &'a [VarDiffSummary],
    /// Paires appariées (jugement par observation) + datasets, pour la
    /// section « Value Comparison Results » (J07-P9).
    pub(super) matches: &'a [PairMatch],
    pub(super) pairs: &'a [VarPair],
    pub(super) base_ds: &'a SasDataset,
    pub(super) comp_ds: &'a SasDataset,
}

/// Représentation listing d'une valeur (BEST12. pour les numériques,
/// texte trimé pour les caractères, '.' pour un missing).
fn fmt_value(v: &Value) -> String {
    match v {
        Value::Num(x) => crate::value::format_best(*x, 12),
        Value::Char(s) => s.trim_end().to_string(),
        Value::Missing(_) => ".".to_string(),
    }
}

/// Diff listing d'une paire inégale : numérique → y−x (BEST12.) ;
/// caractère → masque positionnel '.' égal / 'X' inégal (doc SAS 9.4).
fn diff_repr(b: &Value, c: &Value, ty: VarType) -> String {
    match ty {
        VarType::Num => match (b, c) {
            (Value::Num(x), Value::Num(y)) => crate::value::format_best(y - x, 12),
            _ => ".".to_string(),
        },
        VarType::Char => {
            let x = match b {
                Value::Char(s) => s.trim_end(),
                _ => "",
            };
            let y = match c {
                Value::Char(s) => s.trim_end(),
                _ => "",
            };
            let n = x.len().max(y.len());
            let mut s = String::with_capacity(n);
            for i in 0..n {
                let xb = x.as_bytes().get(i).copied().unwrap_or(b' ');
                let yb = y.as_bytes().get(i).copied().unwrap_or(b' ');
                s.push(if xb == yb { '.' } else { 'X' });
            }
            s
        }
    }
}

/// Full listing report: Data Set Summary, Variables Summary, Observation
/// Summary and (unless NOVALUES) the Values Comparison Summary.
pub(super) fn print_full_report(session: &mut Session, ast: &CompareAst, ctx: &ReportCtx<'_>) {
    // === Data Set Summary ===
    session.listing.write_line("The COMPARE Procedure");
    session.listing.blank();
    session.listing.write_line(&format!(
        "Comparison of {} with {}",
        ctx.base_display, ctx.comp_display
    ));
    if ast.method == CmpMethod::Exact {
        session.listing.write_line("(Method=EXACT)");
    } else if ast.criterion > 0.0 {
        let m = match ast.method {
            CmpMethod::Absolute => "ABSOLUTE",
            CmpMethod::Relative => "RELATIVE",
            CmpMethod::Percent => "PERCENT",
            CmpMethod::Exact => "EXACT",
        };
        session
            .listing
            .write_line(&format!("(Method={}, Criterion={})", m, ast.criterion));
    }
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
    let n_type_mismatch = ctx.common_vars.iter().filter(|cv| !cv.type_match).count();
    session.listing.write_line(&format!(
        "Number of Variables in Common: {}",
        ctx.common_vars.len()
    ));
    if n_type_mismatch > 0 {
        session.listing.write_line(&format!(
            "Number of Variables with Conflicting Types: {}",
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
            "Number of Variables in {} but not in {}: {} ({})",
            ctx.base_display,
            ctx.comp_display,
            ctx.only_base.len(),
            ctx.only_base.join(", ")
        ));
    }
    if !ctx.only_comp.is_empty() {
        session.listing.write_line(&format!(
            "Number of Variables in {} but not in {}: {} ({})",
            ctx.comp_display,
            ctx.base_display,
            ctx.only_comp.len(),
            ctx.only_comp.join(", ")
        ));
    }
    session.listing.blank();

    // === Observation Summary ===
    session.listing.write_line("Observation Summary");
    session.listing.blank();
    session.listing.write_line(&format!(
        "Number of Observations in Common: {}",
        ctx.n_matched
    ));
    if !ctx.base_only_obs.is_empty() {
        session.listing.write_line(&format!(
            "Number of Observations in {} but not in {}: {}",
            ctx.base_display,
            ctx.comp_display,
            ctx.base_only_obs.len()
        ));
    }
    if !ctx.comp_only_obs.is_empty() {
        session.listing.write_line(&format!(
            "Number of Observations in {} but not in {}: {}",
            ctx.comp_display,
            ctx.base_display,
            ctx.comp_only_obs.len()
        ));
    }
    session.listing.write_line(&format!(
        "Total Number of Observations Read from {}: {}",
        ctx.base_display, ctx.base_nobs
    ));
    session.listing.write_line(&format!(
        "Total Number of Observations Read from {}: {}",
        ctx.comp_display, ctx.comp_nobs
    ));
    session.listing.blank();
    session.listing.write_line(&format!(
        "Number of Observations with Some Compared Variables Unequal: {}",
        ctx.n_unequal
    ));
    session.listing.write_line(&format!(
        "Number of Observations with All Compared Variables Equal: {}",
        ctx.n_matched - ctx.n_unequal
    ));
    session.listing.blank();

    // === Values Comparison ===
    if !ast.novalues && ctx.n_matching > 0 {
        session.listing.write_line("Values Comparison Summary");
        session.listing.blank();

        // LISTALL : toutes les variables comparées ; sinon seulement
        // celles avec des valeurs jugées inégales.
        let shown: Vec<&VarDiffSummary> = ctx
            .var_diffs
            .iter()
            .filter(|vd| ast.listall || vd.n_diffs > 0)
            .collect();
        let n_equal = ctx.var_diffs.len() - ctx.var_diffs.iter().filter(|v| v.n_diffs > 0).count();
        session.listing.write_line(&format!(
            "Number of Variables Compared with All Observations Equal: {}",
            n_equal
        ));
        session.listing.write_line(&format!(
            "Number of Variables Compared with Some Observations Unequal: {}",
            ctx.var_diffs.iter().filter(|v| v.n_diffs > 0).count()
        ));
        let total_unequal: usize = ctx.var_diffs.iter().map(|v| v.n_diffs).sum();
        session.listing.write_line(&format!(
            "Total Number of Values which Compare Unequal: {}",
            total_unequal
        ));
        if total_unequal > 0 {
            let max_diff = ctx
                .var_diffs
                .iter()
                .filter(|v| v.var_type == VarType::Num && v.n_diffs > 0)
                .map(|v| v.max_diff)
                .fold(0.0_f64, f64::max);
            session
                .listing
                .write_line(&format!("Maximum Difference: {max_diff}"));
        }
        if !shown.is_empty() {
            session.listing.blank();
            // LISTALL : toutes les variables comparées ; sinon seulement
            // celles avec des valeurs jugées inégales.
            if ast.listall {
                session.listing.write_line("All Compared Variables");
            } else {
                session.listing.write_line("Variables with Unequal Values");
            }
            session.listing.blank();
            let val_headers = vec![
                "Variable".to_string(),
                "Type".to_string(),
                "N Diffs".to_string(),
                "Max Diff".to_string(),
            ];
            let val_aligns = vec![Align::Left, Align::Left, Align::Right, Align::Right];
            let val_rows: Vec<Vec<String>> = shown
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
        session.listing.blank();

        // === Value Comparison Results (plafonné par MAXPRINT=, J07-P9) ===
        // Doc SAS 9.4 (option MAXPRINT=) : n différences imprimées par
        // observation au maximum, p observations avec différences au
        // maximum ; une NOTE signale la troncature.
        print_value_comparison_results(session, ast, ctx);
    }
}

/// Section « Value Comparison Results » : une ligne par différence jugée
/// inégale (observation par observation), plafonnée par MAXPRINT=(n,p).
fn print_value_comparison_results(session: &mut Session, ast: &CompareAst, ctx: &ReportCtx<'_>) {
    let unequal_obs: Vec<&PairMatch> = ctx.matches.iter().filter(|m| m.unequal).collect();
    if unequal_obs.is_empty() {
        return;
    }
    let (max_per_obs, max_obs) = ast.maxprint;

    session.listing.write_line("Value Comparison Results");
    session.listing.blank();

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    for m in unequal_obs.iter().take(max_obs) {
        let mut printed_in_obs = 0usize;
        for p in ctx.pairs {
            let bv = get_value_at(&ctx.base_ds.df, p.base_idx, m.base_idx, p.var_type);
            let cv = get_value_at(&ctx.comp_ds.df, p.comp_idx, m.comp_idx, p.var_type);
            if !judged_unequal(&bv, &cv, ast.method, ast.criterion, p.var_type) {
                continue;
            }
            if printed_in_obs == max_per_obs {
                // n atteint pour cette observation : le reste est tronqué.
                truncated = true;
                break;
            }
            printed_in_obs += 1;
            rows.push(vec![
                (m.base_idx + 1).to_string(),
                p.out_name.clone(),
                type_str(p.var_type).to_string(),
                fmt_value(&bv),
                fmt_value(&cv),
                diff_repr(&bv, &cv, p.var_type),
            ]);
        }
    }
    // Plus d'observations inégales que p, ou n/p nul : des différences
    // existantes ne sont pas imprimées.
    if unequal_obs.len() > max_obs || max_obs == 0 || max_per_obs == 0 {
        truncated = true;
    }

    if !rows.is_empty() {
        let res_headers = vec![
            "Observation".to_string(),
            "Variable".to_string(),
            "Type".to_string(),
            "Base Value".to_string(),
            "Compare Value".to_string(),
            "Diff".to_string(),
        ];
        let res_aligns = vec![
            Align::Right,
            Align::Left,
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        session
            .listing
            .write_table(&res_headers, &res_aligns, &rows);
        session.listing.blank();
    }
    if truncated {
        session.log.note(&format!(
            "MAXPRINT= limit reached ({} differences per observation, {} observations); some differences were not printed.",
            max_per_obs, max_obs
        ));
    }
}

/// BRIEF/BRIEFSUMMARY: condensed report (totals only).
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
        ctx.n_matched, ctx.n_unequal
    ));
}
