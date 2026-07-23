//! Sélection de variables : SELECTION=, all-subsets et R²-improvement.

use super::*;

// ───────────────────────── SELECTION ─────────────────────────

/// Run a SELECTION= algorithm, returning the final subset of regressor column
/// indices (into `regressors` / `xcols`). Returns `None` if the final set is
/// empty. Emits a step-log table.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_selection(
    sel: &Selection,
    xcols: &[Vec<f64>],
    y: &[f64],
    regressors: &[String],
    intercept: bool,
    model: &RegModel,
    session: &mut Session,
) -> Option<Vec<usize>> {
    let p = regressors.len();
    let n = y.len();
    let all: Vec<usize> = (0..p).collect();
    let int = intercept as usize;

    // Step-log accumulator. Each row: (step, action, var, vars_in, partial_r2,
    // model_r2, f_value, p_value).
    let mut steplog: Vec<SelStep> = Vec::new();

    // Uncorrected/corrected total used for R² reporting in the step log.
    let sst_report: f64 = if intercept {
        let ybar = y.iter().sum::<f64>() / n as f64;
        y.iter().map(|yi| (yi - ybar) * (yi - ybar)).sum()
    } else {
        y.iter().map(|yi| yi * yi).sum()
    };
    let model_r2 = |sse: f64| -> f64 {
        if sst_report > 0.0 {
            1.0 - sse / sst_report
        } else {
            f64::NAN
        }
    };

    // ── M36.6: the all-subsets / R²-improvement / none methods are dispatched
    // here. They have their own printers (or none) and either return the full
    // regressor set (R²-family, NONE — the normal full-model fit then proceeds)
    // or the model they built (MAXR/MINR).
    match sel.method {
        SelMethod::None => {
            // Clean no-op: behave exactly as if no SELECTION= had been given.
            return if p == 0 { None } else { Some((0..p).collect()) };
        }
        SelMethod::RSquare | SelMethod::AdjRsq | SelMethod::Cp => {
            run_all_subsets(sel, xcols, y, regressors, intercept, session);
            // RSQUARE-family selects the FULL model: the table is informational,
            // then the standard full-model fit (and any OUTPUT/diagnostics)
            // proceeds over the complete regressor set.
            return if p == 0 { None } else { Some((0..p).collect()) };
        }
        SelMethod::MaxR | SelMethod::MinR => {
            return run_rsq_improvement(sel, xcols, y, regressors, intercept, session);
        }
        SelMethod::Forward | SelMethod::Backward | SelMethod::Stepwise => {}
    }

    let max_steps = 2 * p + 5;

    let final_set: Vec<usize> = match sel.method {
        SelMethod::Forward => {
            let mut s: Vec<usize> = Vec::new();
            let mut step = 0usize;
            loop {
                let sse_s = subset_sse(xcols, y, &s, intercept).unwrap_or(f64::INFINITY);
                let mut best: Option<(usize, f64, f64)> = None; // (col, f, p)
                for &c in &all {
                    if s.contains(&c) {
                        continue;
                    }
                    let mut cand = s.clone();
                    cand.push(c);
                    let df_full = (n as f64) - (cand.len() as f64) - int as f64;
                    if df_full <= 0.0 {
                        continue;
                    }
                    if let Some(sse_c) = subset_sse(xcols, y, &cand, intercept) {
                        let f = (sse_s - sse_c) / (sse_c / df_full);
                        let pv = (1.0 - f_cdf(f, 1.0, df_full)).clamp(0.0, 1.0);
                        if best.map(|(_, bf, _)| f > bf).unwrap_or(true) {
                            best = Some((c, f, pv));
                        }
                    }
                }
                match best {
                    Some((c, f, pv)) if pv <= sel.slentry => {
                        let mut cand = s.clone();
                        cand.push(c);
                        let sse_c = subset_sse(xcols, y, &cand, intercept).unwrap_or(f64::NAN);
                        let partial = model_r2(sse_c) - model_r2(sse_s);
                        s.push(c);
                        step += 1;
                        steplog.push(SelStep {
                            step,
                            entered: true,
                            var: regressors[c].clone(),
                            vars_in: s.len(),
                            partial_r2: partial,
                            model_r2: model_r2(sse_c),
                            f,
                            p: pv,
                        });
                    }
                    _ => break,
                }
                if step >= max_steps {
                    break;
                }
            }
            s
        }
        SelMethod::Backward => {
            let mut s: Vec<usize> = all.clone();
            let mut step = 0usize;
            loop {
                if s.is_empty() {
                    break;
                }
                let sse_s = subset_sse(xcols, y, &s, intercept).unwrap_or(f64::INFINITY);
                let df_s = (n as f64) - (s.len() as f64) - int as f64;
                if df_s <= 0.0 {
                    break;
                }
                let mse_s = sse_s / df_s;
                let mut worst: Option<(usize, f64, f64)> = None; // (col, f, p)
                for &v in &s {
                    let reduced: Vec<usize> = s.iter().cloned().filter(|&c| c != v).collect();
                    if let Some(sse_r) = subset_sse(xcols, y, &reduced, intercept) {
                        let f = (sse_r - sse_s) / mse_s;
                        let pv = (1.0 - f_cdf(f, 1.0, df_s)).clamp(0.0, 1.0);
                        if worst.map(|(_, wf, _)| f < wf).unwrap_or(true) {
                            worst = Some((v, f, pv));
                        }
                    }
                }
                match worst {
                    Some((v, f, pv)) if pv > sel.slstay => {
                        let reduced: Vec<usize> =
                            s.iter().cloned().filter(|&c| c != v).collect();
                        let sse_r = subset_sse(xcols, y, &reduced, intercept).unwrap_or(f64::NAN);
                        let partial = model_r2(sse_s) - model_r2(sse_r);
                        s.retain(|&c| c != v);
                        step += 1;
                        steplog.push(SelStep {
                            step,
                            entered: false,
                            var: regressors[v].clone(),
                            vars_in: s.len(),
                            partial_r2: partial,
                            model_r2: model_r2(sse_r),
                            f,
                            p: pv,
                        });
                    }
                    _ => break,
                }
                if step >= max_steps {
                    break;
                }
            }
            s
        }
        SelMethod::Stepwise => {
            let mut s: Vec<usize> = Vec::new();
            let mut step = 0usize;
            loop {
                let mut changed = false;
                // (1) Forward step.
                let sse_s = subset_sse(xcols, y, &s, intercept).unwrap_or(f64::INFINITY);
                let mut best: Option<(usize, f64, f64)> = None;
                for &c in &all {
                    if s.contains(&c) {
                        continue;
                    }
                    let mut cand = s.clone();
                    cand.push(c);
                    let df_full = (n as f64) - (cand.len() as f64) - int as f64;
                    if df_full <= 0.0 {
                        continue;
                    }
                    if let Some(sse_c) = subset_sse(xcols, y, &cand, intercept) {
                        let f = (sse_s - sse_c) / (sse_c / df_full);
                        let pv = (1.0 - f_cdf(f, 1.0, df_full)).clamp(0.0, 1.0);
                        if best.map(|(_, bf, _)| f > bf).unwrap_or(true) {
                            best = Some((c, f, pv));
                        }
                    }
                }
                let just_entered = if let Some((c, f, pv)) = best {
                    if pv <= sel.slentry {
                        let mut cand = s.clone();
                        cand.push(c);
                        let sse_c = subset_sse(xcols, y, &cand, intercept).unwrap_or(f64::NAN);
                        let partial = model_r2(sse_c) - model_r2(sse_s);
                        s.push(c);
                        step += 1;
                        changed = true;
                        steplog.push(SelStep {
                            step,
                            entered: true,
                            var: regressors[c].clone(),
                            vars_in: s.len(),
                            partial_r2: partial,
                            model_r2: model_r2(sse_c),
                            f,
                            p: pv,
                        });
                        Some(c)
                    } else {
                        None
                    }
                } else {
                    None
                };

                // (2) Backward step(s): remove any variable (except the one just
                // entered) whose remove-p > slstay.
                loop {
                    if s.is_empty() {
                        break;
                    }
                    let sse_cur = subset_sse(xcols, y, &s, intercept).unwrap_or(f64::INFINITY);
                    let df_cur = (n as f64) - (s.len() as f64) - int as f64;
                    if df_cur <= 0.0 {
                        break;
                    }
                    let mse_cur = sse_cur / df_cur;
                    let mut worst: Option<(usize, f64, f64)> = None;
                    for &v in &s {
                        if Some(v) == just_entered {
                            continue;
                        }
                        let reduced: Vec<usize> =
                            s.iter().cloned().filter(|&c| c != v).collect();
                        if let Some(sse_r) = subset_sse(xcols, y, &reduced, intercept) {
                            let f = (sse_r - sse_cur) / mse_cur;
                            let pv = (1.0 - f_cdf(f, 1.0, df_cur)).clamp(0.0, 1.0);
                            if worst.map(|(_, wf, _)| f < wf).unwrap_or(true) {
                                worst = Some((v, f, pv));
                            }
                        }
                    }
                    match worst {
                        Some((v, f, pv)) if pv > sel.slstay => {
                            let reduced: Vec<usize> =
                                s.iter().cloned().filter(|&c| c != v).collect();
                            let sse_r =
                                subset_sse(xcols, y, &reduced, intercept).unwrap_or(f64::NAN);
                            let partial = model_r2(sse_cur) - model_r2(sse_r);
                            s.retain(|&c| c != v);
                            step += 1;
                            changed = true;
                            steplog.push(SelStep {
                                step,
                                entered: false,
                                var: regressors[v].clone(),
                                vars_in: s.len(),
                                partial_r2: partial,
                                model_r2: model_r2(sse_r),
                                f,
                                p: pv,
                            });
                        }
                        _ => break,
                    }
                }

                if !changed || step >= max_steps {
                    break;
                }
            }
            s
        }
        // All other methods are dispatched (and returned) above.
        SelMethod::RSquare
        | SelMethod::AdjRsq
        | SelMethod::Cp
        | SelMethod::MaxR
        | SelMethod::MinR
        | SelMethod::None => unreachable!("dispatched before the stepwise match"),
    };

    print_selection_summary(sel, &steplog, session);

    if final_set.is_empty() {
        let _ = model; // (model kept for symmetry / future use)
        None
    } else {
        // Keep selected columns in their original regressor order for a stable
        // parameter-estimates layout.
        let mut ordered = final_set;
        ordered.sort_unstable();
        Some(ordered)
    }
}

// ───────────────────────── M36.6: all-subsets & R²-improvement ─────────────────────────

/// Maximum number of regressors for which we enumerate the full 2^p subset
/// lattice. Beyond this the search is exponential and impractical, so we emit a
/// NOTE and cap the enumeration to the forced (INCLUDE=) regressors only.
const ALL_SUBSETS_PMAX: usize = 20;

/// A single evaluated subset in an all-subsets table.
struct SubsetRow {
    /// Number of regressors in the model (= subset.len()).
    k: usize,
    /// R² = 1 − SSE/SST.
    r2: f64,
    /// Adjusted R².
    adj: f64,
    /// Mallows' C(p).
    cp: f64,
    /// Regressor column indices, in MODEL order.
    cols: Vec<usize>,
}

/// RSQUARE / ADJRSQ / CP all-subsets evaluation (M36.6). Enumerates every subset
/// of the regressors with the first `include` regressors forced in, for sizes in
/// `[start.unwrap_or(include).max(1), stop.unwrap_or(p)]`, fits each, and prints
/// the corresponding SAS selection table. Does NOT alter the post-selection fit:
/// the caller selects the full model.
pub(super) fn run_all_subsets(
    sel: &Selection,
    xcols: &[Vec<f64>],
    y: &[f64],
    regressors: &[String],
    intercept: bool,
    session: &mut Session,
) {
    let p = regressors.len();
    let n = y.len();
    let int = intercept as usize;

    // Corrected (intercept) or uncorrected total sum of squares.
    let sst: f64 = if intercept {
        let ybar = y.iter().sum::<f64>() / n as f64;
        y.iter().map(|yi| (yi - ybar) * (yi - ybar)).sum()
    } else {
        y.iter().map(|yi| yi * yi).sum()
    };

    // s² for Mallows' C(p) is the MSE of the FULL model (all p regressors).
    let full_cols: Vec<usize> = (0..p).collect();
    let df_full = (n as f64) - (p as f64) - int as f64;
    let s2 = match subset_sse(xcols, y, &full_cols, intercept) {
        Some(sse) if df_full > 0.0 => sse / df_full,
        _ => f64::NAN,
    };

    let include = sel.include.min(p);
    let forced: Vec<usize> = (0..include).collect();
    let optional: Vec<usize> = (include..p).collect();

    // Combinatorial guard: 2^optional.len() enumeration. If too large, cap to the
    // forced set only and emit a NOTE.
    let mut capped = false;
    if optional.len() > ALL_SUBSETS_PMAX {
        capped = true;
        session.log.note(&format!(
            "SELECTION method requires evaluating 2**{} subsets; the all-subsets \
             search is capped at the {} INCLUDE= regressors.",
            optional.len(),
            include
        ));
    }

    let lo = sel.start.unwrap_or(include).max(1);
    let hi = sel.stop.unwrap_or(p).min(p);

    let mut rows: Vec<SubsetRow> = Vec::new();
    let eval = |cols: &[usize], rows: &mut Vec<SubsetRow>| {
        let k = cols.len();
        if k < lo || k > hi {
            return;
        }
        let p_eff = (k + int) as f64;
        let df = (n as f64) - p_eff;
        if df <= 0.0 {
            return;
        }
        if let Some(sse) = subset_sse(xcols, y, cols, intercept) {
            let r2 = if sst > 0.0 { 1.0 - sse / sst } else { f64::NAN };
            let adj = 1.0 - (1.0 - r2) * (n as f64 - 1.0) / (n as f64 - p_eff);
            let cp = if s2 > 0.0 {
                sse / s2 - (n as f64 - 2.0 * p_eff)
            } else {
                f64::NAN
            };
            rows.push(SubsetRow {
                k,
                r2,
                adj,
                cp,
                cols: cols.to_vec(),
            });
        }
    };

    if capped {
        // Only the forced model is evaluated.
        eval(&forced, &mut rows);
    } else {
        // Enumerate every subset of `optional`, union with `forced`.
        let m = optional.len();
        for mask in 0u32..(1u32 << m) {
            let mut cols = forced.clone();
            for (bit, &c) in optional.iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    cols.push(c);
                }
            }
            cols.sort_unstable();
            eval(&cols, &mut rows);
        }
    }

    print_all_subsets_table(sel, &rows, regressors, session);
}

/// Print the SAS "R-Square / Adjusted R-Square / C(p) Selection Method" table.
fn print_all_subsets_table(
    sel: &Selection,
    rows: &[SubsetRow],
    regressors: &[String],
    session: &mut Session,
) {
    let (title, extra_header): (&str, Option<&str>) = match sel.method {
        SelMethod::AdjRsq => ("Adjusted R-Square Selection Method", Some("Adjusted R-Square")),
        SelMethod::Cp => ("C(p) Selection Method", Some("C(p)")),
        _ => ("R-Square Selection Method", None),
    };

    // Order + BEST= filtering.
    let mut display: Vec<&SubsetRow> = rows.iter().collect();
    match sel.method {
        SelMethod::RSquare => {
            // Group by size; within a size sort by R² desc. BEST= limits per size.
            display.sort_by(|a, b| {
                a.k.cmp(&b.k)
                    .then(b.r2.partial_cmp(&a.r2).unwrap_or(std::cmp::Ordering::Equal))
            });
            if let Some(b) = sel.best {
                let mut kept: Vec<&SubsetRow> = Vec::new();
                let mut cur_k = usize::MAX;
                let mut cnt = 0usize;
                for row in display {
                    if row.k != cur_k {
                        cur_k = row.k;
                        cnt = 0;
                    }
                    if cnt < b {
                        kept.push(row);
                        cnt += 1;
                    }
                }
                display = kept;
            }
        }
        SelMethod::AdjRsq => {
            display.sort_by(|a, b| {
                b.adj.partial_cmp(&a.adj).unwrap_or(std::cmp::Ordering::Equal)
            });
            if let Some(b) = sel.best {
                display.truncate(b);
            }
        }
        SelMethod::Cp => {
            display.sort_by(|a, b| {
                a.cp.partial_cmp(&b.cp).unwrap_or(std::cmp::Ordering::Equal)
            });
            if let Some(b) = sel.best {
                display.truncate(b);
            }
        }
        _ => {}
    }

    session.listing.page_header();
    centered(session, "The REG Procedure");
    centered(session, title);
    session.listing.blank();

    // Build the table. "Variables in Model" is a free-form regressor list, so the
    // layout is rendered directly (write_table can't hold a ragged final column).
    let mut headers: Vec<String> = vec!["Number in Model".into(), "R-Square".into()];
    if let Some(h) = extra_header {
        headers.push(h.into());
    }
    headers.push("Variables in Model".into());

    let mut aligns = vec![Align::Right, Align::Right];
    if extra_header.is_some() {
        aligns.push(Align::Right);
    }
    aligns.push(Align::Left);

    let rows_str: Vec<Vec<String>> = display
        .iter()
        .map(|row| {
            let vars: Vec<&str> =
                row.cols.iter().map(|&c| regressors[c].as_str()).collect();
            let mut cells = vec![format!("{}", row.k), fmt_fit4(row.r2)];
            match sel.method {
                SelMethod::AdjRsq => cells.push(fmt_fit4(row.adj)),
                SelMethod::Cp => cells.push(fmt2(row.cp)),
                _ => {}
            }
            cells.push(vars.join(" "));
            cells
        })
        .collect();

    session.listing.write_table(&headers, &aligns, &rows_str);
    session.listing.blank();
    session.listing.blank();
}

/// MAXR / MINR stepwise R²-improvement (M36.6). Greedily grows the model one
/// variable at a time (entering var maximises — MAXR — or minimises positively
/// — MINR — the R² increase), then applies improving 1-in/1-out swaps until none
/// helps, printing the best model at each size. Returns the final model.
pub(super) fn run_rsq_improvement(
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
fn print_rsq_improvement_table(
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
struct SelStep {
    step: usize,
    entered: bool,
    var: String,
    vars_in: usize,
    partial_r2: f64,
    model_r2: f64,
    f: f64,
    p: f64,
}

/// Print the SAS-style "Summary of <Method> Selection" table.
fn print_selection_summary(sel: &Selection, steplog: &[SelStep], session: &mut Session) {
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
