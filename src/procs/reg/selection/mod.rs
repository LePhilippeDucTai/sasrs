//! Sélection de variables : SELECTION=, all-subsets et R²-improvement.

use super::*;


mod subsets;
mod rsq;

pub(crate) use subsets::*;
pub(crate) use rsq::*;

// ───────────────────────── SELECTION ─────────────────────────

/// Run a SELECTION= algorithm, returning the final subset of regressor column
/// indices (into `regressors` / `xcols`). Returns `None` if the final set is
/// empty. Emits a step-log table.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_selection(
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
