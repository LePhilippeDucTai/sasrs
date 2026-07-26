use super::*;

/// Print the SAS "Output Statistics" table when CLM and/or CLI is requested.
/// Column sets:
///  - CLM only: Obs, Dependent Variable, Predicted Value, Std Error Mean
///    Predict, `<L>% CL Mean` (lower upper), Residual.
///  - CLI only: …, `<L>% CL Predict` (lower upper), Residual.
///  - both: …, `<L>% CL Mean`, `<L>% CL Predict`, Residual.
#[allow(clippy::too_many_arguments)]
pub(crate) fn print_output_statistics(
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
            row.extend([format!("{}", i + 1), fmt5(s.y), fmt5(s.y_hat), fmt5(s.stdp)]);
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
pub(crate) fn fmt_diag(v: f64) -> String {
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
pub(crate) fn student_gauge(student: f64) -> String {
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
pub(crate) fn print_r_statistics(
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
    session
        .listing
        .write_line(&format!("Predicted Residual SS (PRESS)    {}", fmt5(press)));
}

/// Print the MODEL INFLUENCE diagnostics table (M36.3): Obs, Residual,
/// RStudent, Hat Diag H, Cov Ratio, DFFITS, then one `DFBETAS <var>` column per
/// parameter (Intercept first if present). Reuses `compute_influence_stats`.
pub(crate) fn print_influence_statistics(
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
