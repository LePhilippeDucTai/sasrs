//! Statistiques par observation (M36.2) et diagnostics d'influence (M36.3).

use super::*;

/// Per-observation std errors and CL limits for one used row (M36.2).
#[derive(Clone)]
pub(super) struct ObsStat {
    y: f64,
    pub(super) y_hat: f64,
    pub(super) stdp: f64,
    pub(super) stdi: f64,
    pub(super) stdr: f64,
    pub(super) lclm: f64,
    pub(super) uclm: f64,
    pub(super) lcl: f64,
    pub(super) ucl: f64,
}

/// Reconstruct the response vector y = ŷ + resid from a fit (avoids threading
/// the y vector into helpers that already carry the fit).
pub(super) fn reconstruct_y(fit: &OlsFit) -> Vec<f64> {
    fit.y_hat
        .iter()
        .zip(fit.resid.iter())
        .map(|(yh, r)| yh + r)
        .collect()
}

/// Compute the per-observation statistics for every used row from the OLS fit.
/// `mse = sse/dfE`, `h_i` the leverage, `t = t_quantile(1−α/2, dfE)`.
pub(super) fn compute_obs_stats(
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
    p_eff: usize,
    alpha: f64,
    weighting: Option<&Weighting>,
) -> Vec<ObsStat> {
    // df / MSE use Σf_i with FREQ; the weighted hat is h_i = w_i·x_iᵀ(X'WX)⁻¹x_i.
    let (df_e, wts): (f64, Option<&[f64]>) = match weighting {
        Some(w) => (w.total_n - p_eff as f64, Some(&w.wf)),
        None => ((n - p_eff) as f64, None),
    };
    let mse = fit.sse / df_e;
    let t = t_quantile(1.0 - alpha / 2.0, df_e);
    let h0 = leverages(x_mat, &fit.xtx_inv);
    let h: Vec<f64> = match wts {
        Some(w) => h0.iter().zip(w.iter()).map(|(hi, wi)| hi * wi).collect(),
        None => h0,
    };
    (0..n)
        .map(|i| {
            let hi = h[i];
            let stdp = (mse * hi).sqrt();
            let stdi = (mse * (1.0 + hi)).sqrt();
            let stdr = (mse * (1.0 - hi)).max(0.0).sqrt();
            let yh = fit.y_hat[i];
            ObsStat {
                y: y[i],
                y_hat: yh,
                stdp,
                stdi,
                stdr,
                lclm: yh - t * stdp,
                uclm: yh + t * stdp,
                lcl: yh - t * stdi,
                ucl: yh + t * stdi,
            }
        })
        .collect()
}

/// Per-observation influence diagnostics (M36.3). Reuses the same leverage /
/// MSE / dfE infrastructure as `compute_obs_stats` (no duplicate fit).
///
/// `dfbetas[i]` has one entry per parameter (column order matches `fit.beta`:
/// intercept first if present). When `dfE ≤ 1`, RSTUDENT / COVRATIO / DFFITS /
/// DFBETAS are undefined (their leave-one-out variance `MSE_(i)` has 0 df) and
/// are reported as `NaN`; callers render the SAS sentinel `.`.
pub(super) struct InfluenceStat {
    y: f64,
    y_hat: f64,
    pub(super) resid: f64,
    stdp: f64,
    pub(super) stdr: f64,
    pub(super) h: f64,
    pub(super) student: f64,
    pub(super) rstudent: f64,
    pub(super) cookd: f64,
    pub(super) press: f64,
    pub(super) dffits: f64,
    pub(super) covratio: f64,
    /// One DFBETAS per parameter, same column order as `fit.beta`.
    pub(super) dfbetas: Vec<f64>,
}

/// Compute the full influence-diagnostic set for every used row. `c = (X'X)⁻¹Xᵀ`
/// (p_eff × n) drives DFBETAS via the closed form
/// `DFBETAS_{ij} = (rstudent_i · c_{ji}) / √(Σ_k c_{jk}²)` — no leave-one-out
/// refits.
pub(super) fn compute_influence_stats(
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
    p_eff: usize,
    weighting: Option<&Weighting>,
) -> Vec<InfluenceStat> {
    let (df_e, wts): (f64, Option<&[f64]>) = match weighting {
        Some(w) => (w.total_n - p_eff as f64, Some(&w.wf)),
        None => ((n - p_eff) as f64, None),
    };
    let mse = fit.sse / df_e;
    let h0 = leverages(x_mat, &fit.xtx_inv);
    let h: Vec<f64> = match wts {
        Some(w) => h0.iter().zip(w.iter()).map(|(hi, wi)| hi * wi).collect(),
        None => h0,
    };

    // c = (X'X)⁻¹ Xᵀ  →  p_eff × n. Row j, col i is c_{ji}.
    let xt = linalg::transpose(x_mat); // p_eff × n
    let c = linalg::matrix_mult(&fit.xtx_inv, &xt); // (p_eff×p_eff)·(p_eff×n)
    // Row norms √(Σ_k c_{jk}²) for the DFBETAS denominator (= √((X'X)⁻¹_{jj})).
    let c_row_norm: Vec<f64> = (0..p_eff)
        .map(|j| c[j].iter().map(|v| v * v).sum::<f64>().sqrt())
        .collect();

    (0..n)
        .map(|i| {
            let hi = h[i];
            let yh = fit.y_hat[i];
            let resid = fit.resid[i];
            let one_minus_h = 1.0 - hi;
            let stdp = (mse * hi).sqrt();
            let stdr = (mse * one_minus_h).max(0.0).sqrt();
            // STUDENT = resid / STDR.
            let student = if stdr > 0.0 { resid / stdr } else { f64::NAN };
            // Leave-one-out MSE_(i): undefined when dfE ≤ 1.
            let (rstudent, mse_i_ok) = if df_e > 1.0 && one_minus_h > 0.0 {
                let mse_i = (df_e * mse - resid * resid / one_minus_h) / (df_e - 1.0);
                if mse_i > 0.0 {
                    (resid / (mse_i * one_minus_h).sqrt(), true)
                } else {
                    (f64::NAN, false)
                }
            } else {
                (f64::NAN, false)
            };
            // Cook's D = (student²/p)·(h/(1−h)).
            let cookd = if one_minus_h > 0.0 && p_eff > 0 {
                (student * student / p_eff as f64) * (hi / one_minus_h)
            } else {
                f64::NAN
            };
            let press = if one_minus_h != 0.0 {
                resid / one_minus_h
            } else {
                f64::NAN
            };
            let dffits = if mse_i_ok && one_minus_h > 0.0 {
                rstudent * (hi / one_minus_h).sqrt()
            } else {
                f64::NAN
            };
            // COVRATIO = 1 / ( ((dfE−1+rstudent²)/dfE)^p · (1−h) ).
            let covratio = if mse_i_ok && one_minus_h > 0.0 {
                let base = (df_e - 1.0 + rstudent * rstudent) / df_e;
                1.0 / (base.powi(p_eff as i32) * one_minus_h)
            } else {
                f64::NAN
            };
            // DFBETAS_{ij} = c_{ji}·rstudent_i / (√(1−h_i)·√((X'X)⁻¹_{jj})).
            // Here √(Σ_k c_{jk}²) = √((X'X)⁻¹_{jj}) since c·cᵀ = (X'X)⁻¹.
            // The extra √(1−h_i) converts e_i/s_(i) into rstudent_i (which
            // carries its own √(1−h_i)); see derivation in the milestone notes.
            let dfbetas: Vec<f64> = (0..p_eff)
                .map(|j| {
                    if mse_i_ok && c_row_norm[j] > 0.0 && one_minus_h > 0.0 {
                        rstudent * c[j][i] / (c_row_norm[j] * one_minus_h.sqrt())
                    } else {
                        f64::NAN
                    }
                })
                .collect();

            InfluenceStat {
                y: y[i],
                y_hat: yh,
                resid,
                stdp,
                stdr,
                h: hi,
                student,
                rstudent,
                cookd,
                press,
                dffits,
                covratio,
                dfbetas,
            }
        })
        .collect()
}

/// Print the SAS "Output Statistics" table when CLM and/or CLI is requested.
/// Column sets:
///  - CLM only: Obs, Dependent Variable, Predicted Value, Std Error Mean
///    Predict, `<L>% CL Mean` (lower upper), Residual.
///  - CLI only: …, `<L>% CL Predict` (lower upper), Residual.
///  - both: …, `<L>% CL Mean`, `<L>% CL Predict`, Residual.
#[allow(clippy::too_many_arguments)]
pub(super) fn print_output_statistics(
    model: &RegModel,
    _dep_name: &str,
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
    p_eff: usize,
    weighting: Option<&Weighting>,
    id_first: Option<&[String]>,
    session: &mut Session,
) {
    let stats = compute_obs_stats(x_mat, y, fit, n, p_eff, model.alpha, weighting);
    let level = fmt_level(100.0 * (1.0 - model.alpha));

    // ID (M36.7): prepend the first ID variable as a leading column.
    let mut headers: Vec<String> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();
    if id_first.is_some() {
        headers.push("Id".into());
        aligns.push(Align::Right);
    }
    headers.extend([
        "Obs".into(),
        "Dependent Variable".into(),
        "Predicted Value".into(),
        "Std Error Mean Predict".into(),
    ]);
    aligns.extend([Align::Right, Align::Right, Align::Right, Align::Right]);
    if model.clm {
        headers.push(format!("{}% CL Mean (Lower)", level));
        headers.push(format!("{}% CL Mean (Upper)", level));
        aligns.push(Align::Right);
        aligns.push(Align::Right);
    }
    if model.cli {
        headers.push(format!("{}% CL Predict (Lower)", level));
        headers.push(format!("{}% CL Predict (Upper)", level));
        aligns.push(Align::Right);
        aligns.push(Align::Right);
    }
    headers.push("Residual".into());
    aligns.push(Align::Right);

    let rows: Vec<Vec<String>> = stats
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut row: Vec<String> = Vec::new();
            if let Some(ids) = id_first {
                row.push(ids.get(i).cloned().unwrap_or_default());
            }
            row.extend([
                format!("{}", i + 1),
                fmt5(s.y),
                fmt5(s.y_hat),
                fmt5(s.stdp),
            ]);
            if model.clm {
                row.push(fmt5(s.lclm));
                row.push(fmt5(s.uclm));
            }
            if model.cli {
                row.push(fmt5(s.lcl));
                row.push(fmt5(s.ucl));
            }
            row.push(fmt5(s.y - s.y_hat));
            row
        })
        .collect();

    session.listing.blank();
    session.listing.blank();
    centered(session, "Output Statistics");
    session.listing.blank();
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Format a possibly-undefined diagnostic value: SAS prints `.` for a missing
/// (undefined) numeric, otherwise the usual 4-decimal rendering.
pub(super) fn fmt_diag(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.4}")
    } else {
        ".".to_string()
    }
}

/// Render SAS's `-2-1 0 1 2` character gauge for a studentized residual: a
/// 9-cell `|....*...|`-style bar centred on 0, one `*` placed at the residual's
/// position (clamped to ±2.x). Matches the simple gauge SAS prints in the
/// MODEL R "Output Statistics" table.
fn student_gauge(student: f64) -> String {
    // Cells map the range [-2.625, 2.625] across 9 character slots; the centre
    // slot (index 4) is 0. SAS uses one star; ties round toward centre.
    let mut cells = [' '; 9];
    if student.is_finite() {
        let pos = (student / 2.625 * 4.0).round() as i64;
        let idx = (4 + pos).clamp(0, 8) as usize;
        cells[idx] = '*';
    }
    let bar: String = cells.iter().collect();
    format!("|{}|", bar)
}

/// Print the MODEL R "Output Statistics" table (residual analysis), followed by
/// the Sum of Residuals / Sum of Squared Residuals / PRESS summary block
/// (M36.3). Reuses `compute_influence_stats`.
pub(super) fn print_r_statistics(
    _model: &RegModel,
    stats: &[InfluenceStat],
    id_first: Option<&[String]>,
    // M36.7: when WEIGHT/FREQ is active the residual-summary sums must be
    // weighted (wf_i) so they agree with the weighted ANOVA Error SS. `None` ⇒
    // all-ones weights ⇒ the original unweighted sums (byte-identical).
    weighting: Option<&Weighting>,
    session: &mut Session,
) {
    let mut headers: Vec<String> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();
    if id_first.is_some() {
        headers.push("Id".into());
        aligns.push(Align::Right);
    }
    headers.extend([
        "Obs".into(),
        "Dependent Variable".into(),
        "Predicted Value".into(),
        "Std Error Mean Predict".into(),
        "Residual".into(),
        "Std Error Residual".into(),
        "Student Residual".into(),
        "-2-1 0 1 2".into(),
        "Cook's D".into(),
    ]);
    aligns.extend([
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Left,
        Align::Right,
    ]);
    let rows: Vec<Vec<String>> = stats
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut row: Vec<String> = Vec::new();
            if let Some(ids) = id_first {
                row.push(ids.get(i).cloned().unwrap_or_default());
            }
            row.extend([
                format!("{}", i + 1),
                fmt5(s.y),
                fmt5(s.y_hat),
                fmt5(s.stdp),
                fmt5(s.resid),
                fmt5(s.stdr),
                fmt_diag(s.student),
                student_gauge(s.student),
                fmt_diag(s.cookd),
            ]);
            row
        })
        .collect();

    session.listing.blank();
    session.listing.blank();
    centered(session, "Output Statistics");
    session.listing.blank();
    session.listing.write_table(&headers, &aligns, &rows);

    // Summary block SAS prints after the R table. With WEIGHT/FREQ active these
    // sums are weighted by wf_i so they agree with the weighted ANOVA Error SS
    // (M36.7). `s.press` = e_i/(1−h_i) already uses the WEIGHTED leverage h_i, so
    // the weighted PRESS is Σ wf_i·(e_i/(1−h_i))². With no weighting `wf` is the
    // all-ones slice and these collapse to the original unweighted sums
    // (byte-identical):
    //   Sum of Residuals         = Σ wf_i·e_i
    //   Sum of Squared Residuals = Σ wf_i·e_i²   (= ANOVA Error SS)
    //   PRESS                    = Σ wf_i·(e_i/(1−h_i))²
    let ones = vec![1.0; stats.len()];
    let wf: &[f64] = match weighting {
        Some(w) => &w.wf,
        None => &ones,
    };
    let sum_resid: f64 = stats.iter().zip(wf).map(|(s, &w)| w * s.resid).sum();
    let sum_sq_resid: f64 = stats
        .iter()
        .zip(wf)
        .map(|(s, &w)| w * s.resid * s.resid)
        .sum();
    let press: f64 = stats
        .iter()
        .zip(wf)
        .filter_map(|(s, &w)| {
            if s.press.is_finite() {
                Some(w * s.press * s.press)
            } else {
                None
            }
        })
        .sum();
    session.listing.blank();
    session
        .listing
        .write_line(&format!("Sum of Residuals             {}", fmt5(sum_resid)));
    session.listing.write_line(&format!(
        "Sum of Squared Residuals     {}",
        fmt5(sum_sq_resid)
    ));
    session.listing.write_line(&format!(
        "Predicted Residual SS (PRESS)    {}",
        fmt5(press)
    ));
}

/// Print the MODEL INFLUENCE diagnostics table (M36.3): Obs, Residual,
/// RStudent, Hat Diag H, Cov Ratio, DFFITS, then one `DFBETAS <var>` column per
/// parameter (Intercept first if present). Reuses `compute_influence_stats`.
pub(super) fn print_influence_statistics(
    stats: &[InfluenceStat],
    reg_names: &[String],
    intercept: bool,
    id_first: Option<&[String]>,
    session: &mut Session,
) {
    let p_eff = reg_names.len() + intercept as usize;
    let mut headers: Vec<String> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();
    if id_first.is_some() {
        headers.push("Id".into());
        aligns.push(Align::Right);
    }
    headers.extend([
        "Obs".into(),
        "Residual".into(),
        "RStudent".into(),
        "Hat Diag H".into(),
        "Cov Ratio".into(),
        "DFFITS".into(),
    ]);
    aligns.extend([
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ]);
    for j in 0..p_eff {
        let var = if intercept {
            if j == 0 {
                "Intercept".to_string()
            } else {
                reg_names[j - 1].clone()
            }
        } else {
            reg_names[j].clone()
        };
        headers.push(format!("DFBETAS {}", var));
        aligns.push(Align::Right);
    }

    let rows: Vec<Vec<String>> = stats
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut row: Vec<String> = Vec::new();
            if let Some(ids) = id_first {
                row.push(ids.get(i).cloned().unwrap_or_default());
            }
            row.extend([
                format!("{}", i + 1),
                fmt5(s.resid),
                fmt_diag(s.rstudent),
                fmt_fit4(s.h),
                fmt_diag(s.covratio),
                fmt_diag(s.dffits),
            ]);
            for j in 0..p_eff {
                row.push(fmt_diag(s.dfbetas[j]));
            }
            row
        })
        .collect();

    session.listing.blank();
    session.listing.blank();
    centered(session, "Output Statistics");
    session.listing.blank();
    session.listing.write_table(&headers, &aligns, &rows);
}
