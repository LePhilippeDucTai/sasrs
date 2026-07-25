use super::*;

/// MAXR / MINR stepwise R²-improvement (M36.6). Greedily grows the model one
/// variable at a time (entering var maximises — MAXR — or minimises positively
/// — MINR — the R² increase), then applies improving 1-in/1-out swaps until none
/// helps, printing the best model at each size. Returns the final model.
pub(crate) fn run_rsq_improvement(
    sel: &Selection,
    xcols: &[Vec<f64>],
    y: &[f64],
    regressors: &[String],
    intercept: bool,
    session: &mut Session,
) -> Option<Vec<usize>> {
    let p = regressors.len();
    let n = y.len();
    let int = intercept as usize;

    let sst: f64 = if intercept {
        let ybar = y.iter().sum::<f64>() / n as f64;
        y.iter().map(|yi| (yi - ybar) * (yi - ybar)).sum()
    } else {
        y.iter().map(|yi| yi * yi).sum()
    };
    let r2_of = |cols: &[usize]| -> Option<f64> {
        let p_eff = (cols.len() + int) as f64;
        if (n as f64) - p_eff <= 0.0 {
            return None;
        }
        subset_sse(xcols, y, cols, intercept).map(|sse| {
            if sst > 0.0 {
                1.0 - sse / sst
            } else {
                f64::NAN
            }
        })
    };

    let maximise = matches!(sel.method, SelMethod::MaxR);
    let include = sel.include.min(p);
    let stop = sel.stop.unwrap_or(p).min(p);

    // Forced (INCLUDE=) variables seed the model.
    let mut current: Vec<usize> = (0..include).collect();
    let mut step_rows: Vec<(usize, f64, Vec<usize>)> = Vec::new();

    // Bound on swap iterations to keep the search finite.
    let max_swaps = 4 * p + 8;

    while current.len() < stop {
        // (1) Enter the variable giving the best (max/min positive) R² increase.
        let cur_r2 = r2_of(&current).unwrap_or(f64::NAN);
        let mut chosen: Option<(usize, f64)> = None; // (col, new_r2)
        for c in 0..p {
            if current.contains(&c) {
                continue;
            }
            let mut cand = current.clone();
            cand.push(c);
            cand.sort_unstable();
            if let Some(r2) = r2_of(&cand) {
                let inc = r2 - cur_r2;
                // MINR considers only variables with a non-negative R² increase.
                if !maximise && inc < 0.0 {
                    continue;
                }
                let better = match &chosen {
                    None => true,
                    Some((_, best_r2)) => {
                        if maximise {
                            r2 > *best_r2
                        } else {
                            // smallest positive increase ⇒ smallest new R².
                            r2 < *best_r2
                        }
                    }
                };
                if better {
                    chosen = Some((c, r2));
                }
            }
        }
        let Some((enter, _)) = chosen else { break };
        current.push(enter);
        current.sort_unstable();

        // (2) Swap loop: try every (in, out) pair (out must not be forced), apply
        // the swap that best improves R² until no swap helps.
        let mut iters = 0usize;
        loop {
            iters += 1;
            if iters > max_swaps {
                break;
            }
            let base_r2 = r2_of(&current).unwrap_or(f64::NAN);
            let mut best_swap: Option<(usize, usize, f64)> = None; // (out, in, r2)
            for &out_c in current.iter() {
                if out_c < include {
                    continue; // forced vars never leave
                }
                for in_c in 0..p {
                    if current.contains(&in_c) {
                        continue;
                    }
                    let mut cand: Vec<usize> =
                        current.iter().cloned().filter(|&c| c != out_c).collect();
                    cand.push(in_c);
                    cand.sort_unstable();
                    if let Some(r2) = r2_of(&cand) {
                        // A swap that raises R² always improves the best model of
                        // this size, for both MAXR and MINR.
                        if r2 > base_r2 + 1e-12 {
                            let better = match &best_swap {
                                None => true,
                                Some((_, _, br2)) => r2 > *br2,
                            };
                            if better {
                                best_swap = Some((out_c, in_c, r2));
                            }
                        }
                    }
                }
            }
            match best_swap {
                Some((out_c, in_c, _)) => {
                    current.retain(|&c| c != out_c);
                    current.push(in_c);
                    current.sort_unstable();
                }
                None => break,
            }
        }

        let r2 = r2_of(&current).unwrap_or(f64::NAN);
        step_rows.push((current.len(), r2, current.clone()));
    }

    print_rsq_improvement_table(sel, &step_rows, regressors, session);

    if current.is_empty() {
        None
    } else {
        current.sort_unstable();
        Some(current)
    }
}

/// Print the MAXR/MINR "Maximum/Minimum R-Square Improvement" model-per-size
/// table (M36.6).
pub(crate) fn print_rsq_improvement_table(
    sel: &Selection,
    steps: &[(usize, f64, Vec<usize>)],
    regressors: &[String],
    session: &mut Session,
) {
    let title = if matches!(sel.method, SelMethod::MaxR) {
        "Maximum R-Square Improvement Selection Method"
    } else {
        "Minimum R-Square Improvement Selection Method"
    };

    session.listing.page_header();
    centered(session, "The REG Procedure");
    centered(session, title);
    session.listing.blank();

    let headers: Vec<String> = vec![
        "Number in Model".into(),
        "R-Square".into(),
        "Variables in Model".into(),
    ];
    let aligns = vec![Align::Right, Align::Right, Align::Left];
    let rows_str: Vec<Vec<String>> = steps
        .iter()
        .map(|(k, r2, cols)| {
            let vars: Vec<&str> = cols.iter().map(|&c| regressors[c].as_str()).collect();
            vec![format!("{}", k), fmt_fit4(*r2), vars.join(" ")]
        })
        .collect();
    session.listing.write_table(&headers, &aligns, &rows_str);
    session.listing.blank();
    session.listing.blank();
}

/// One row of a selection step log.
pub(crate) struct SelStep {
    pub(super) step: usize,
    pub(super) entered: bool,
    pub(super) var: String,
    pub(super) vars_in: usize,
    pub(super) partial_r2: f64,
    pub(super) model_r2: f64,
    pub(super) f: f64,
    pub(super) p: f64,
}

/// Print the SAS-style "Summary of <Method> Selection" table.
pub(crate) fn print_selection_summary(sel: &Selection, steplog: &[SelStep], session: &mut Session) {
    let method = match sel.method {
        SelMethod::Forward => "Forward",
        SelMethod::Backward => "Backward Elimination",
        _ => "Stepwise",
    };
    let title = match sel.method {
        SelMethod::Forward => "Summary of Forward Selection".to_string(),
        SelMethod::Backward => "Summary of Backward Elimination".to_string(),
        _ => "Summary of Stepwise Selection".to_string(),
    };
    let _ = method;

    session.listing.page_header();
    centered(session, "The REG Procedure");
    centered(session, &title);
    session.listing.blank();

    let headers: Vec<String> = vec![
        "Step".into(),
        "Variable Entered".into(),
        "Variable Removed".into(),
        "Number Vars In".into(),
        "Partial R-Square".into(),
        "Model R-Square".into(),
        "F Value".into(),
        "Pr > F".into(),
    ];
    let aligns = vec![
        Align::Right,
        Align::Left,
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let rows: Vec<Vec<String>> = steplog
        .iter()
        .map(|st| {
            let (entered, removed) = if st.entered {
                (st.var.clone(), String::new())
            } else {
                (String::new(), st.var.clone())
            };
            vec![
                format!("{}", st.step),
                entered,
                removed,
                format!("{}", st.vars_in),
                fmt_fit4(st.partial_r2),
                fmt_fit4(st.model_r2),
                fmt2(st.f),
                fmt_p(Some(st.p)),
            ]
        })
        .collect();
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
    session.listing.blank();
}
