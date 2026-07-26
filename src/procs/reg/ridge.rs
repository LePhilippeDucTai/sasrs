//! Régression ridge / IPC (M36.9) : standardisation et ajustements pénalisés.

use super::*;

// ───────────────────────── Ridge / IPC regression (M36.9) ─────────────────────────

/// Standardization summary for ridge / IPC regression (M36.9). The model is fit
/// on centered, unit-scaled (corrected-SD) regressors and response. We keep the
/// per-regressor and response means / SDs so standardized estimates can be
/// back-transformed to the original scale.
pub(super) struct Standardized {
    /// Correlation matrix R = Xs'Xs/(n−1) of the standardized regressors (p×p).
    pub(super) r: Vec<Vec<f64>>,
    /// Correlations r_xy of each standardized regressor with the response (len p).
    pub(super) r_xy: Vec<f64>,
    /// Regressor means (len p).
    x_mean: Vec<f64>,
    /// Regressor corrected SDs (len p).
    x_sd: Vec<f64>,
    /// Response mean.
    pub(super) y_mean: f64,
    /// Response corrected SD.
    y_sd: f64,
}

/// Build the standardized correlation structure for ridge / IPC (M36.9). `cols`
/// are the (in-model, no-intercept) regressor columns over the complete-case
/// rows; `y` is the response. R is the Pearson correlation matrix of the
/// regressors, r_xy the regressor/response correlations.
pub(super) fn standardize_for_ridge(cols: &[Vec<f64>], y: &[f64]) -> Standardized {
    let p = cols.len();
    let n = y.len();
    let nf = n as f64;
    let x_mean: Vec<f64> = cols.iter().map(|c| c.iter().sum::<f64>() / nf).collect();
    let x_sd: Vec<f64> = cols.iter().map(|c| sample_sd(c)).collect();
    let y_mean = y.iter().sum::<f64>() / nf;
    let y_sd = sample_sd(y);
    // Standardized columns zs_j = (x_j − x̄_j)/sd_j, response zy = (y − ȳ)/sd_y.
    let zcols: Vec<Vec<f64>> = (0..p)
        .map(|j| {
            let s = if x_sd[j] > 0.0 { x_sd[j] } else { 1.0 };
            cols[j].iter().map(|v| (v - x_mean[j]) / s).collect()
        })
        .collect();
    let zy: Vec<f64> = {
        let s = if y_sd > 0.0 { y_sd } else { 1.0 };
        y.iter().map(|v| (v - y_mean) / s).collect()
    };
    // R = Zs'Zs/(n−1) — the correlation matrix. r_xy = Zs'zy/(n−1).
    let denom = nf - 1.0;
    let mut r = vec![vec![0.0; p]; p];
    for a in 0..p {
        for b in a..p {
            let s: f64 = (0..n).map(|i| zcols[a][i] * zcols[b][i]).sum::<f64>() / denom;
            r[a][b] = s;
            r[b][a] = s;
        }
    }
    let r_xy: Vec<f64> = (0..p)
        .map(|j| (0..n).map(|i| zcols[j][i] * zy[i]).sum::<f64>() / denom)
        .collect();
    Standardized {
        r,
        r_xy,
        x_mean,
        x_sd,
        y_mean,
        y_sd,
    }
}

/// Back-transform standardized coefficients `b_star` (one per regressor) to the
/// original scale (M36.9). Returns `(intercept, slopes)`:
///   β_j = b*_j · sd(y)/sd(x_j);  β_0 = ȳ − Σ β_j x̄_j.
pub(super) fn back_transform(b_star: &[f64], std: &Standardized) -> (f64, Vec<f64>) {
    let p = b_star.len();
    let mut slopes = vec![0.0; p];
    for j in 0..p {
        let sx = if std.x_sd[j] > 0.0 { std.x_sd[j] } else { 1.0 };
        slopes[j] = b_star[j] * std.y_sd / sx;
    }
    let intercept = std.y_mean - (0..p).map(|j| slopes[j] * std.x_mean[j]).sum::<f64>();
    (intercept, slopes)
}

/// Standardized ridge estimates b*(k) = (R + kI)⁻¹ r_xy (M36.9).
pub(super) fn ridge_beta_star(r: &[Vec<f64>], r_xy: &[f64], k: f64) -> Result<Vec<f64>> {
    let p = r.len();
    let mut a = r.to_vec();
    for i in 0..p {
        a[i][i] += k;
    }
    let inv = linalg::invert_matrix(&a)?;
    Ok(linalg::matrix_vec_mult(&inv, r_xy))
}

/// Ridge VIF_j(k) = [ (R+kI)⁻¹ R (R+kI)⁻¹ ]_jj (M36.9). k=0 → ordinary VIF.
pub(super) fn ridge_vif(r: &[Vec<f64>], k: f64) -> Result<Vec<f64>> {
    let p = r.len();
    let mut a = r.to_vec();
    for i in 0..p {
        a[i][i] += k;
    }
    let inv = linalg::invert_matrix(&a)?;
    let m = linalg::matrix_mult(&inv, r);
    let m = linalg::matrix_mult(&m, &inv);
    Ok((0..p).map(|i| m[i][i]).collect())
}

/// Standardized IPC estimates dropping the `m` smallest-eigenvalue principal
/// components of R (M36.9): b*_IPC = Σ_{i=1}^{p−m} (v_iᵀ r_xy / λ_i) v_i, using
/// the eigenpairs of R sorted by descending eigenvalue. m=0 ⇒ full OLS-standardized
/// estimates; m=p ⇒ all-zero.
pub(super) fn ipc_beta_star(r: &[Vec<f64>], r_xy: &[f64], m: usize) -> Result<Vec<f64>> {
    let p = r.len();
    // eigenvectors_jacobi returns (V, λ) with λ descending; V columns are the
    // eigenvectors. Retain the first (p−m) (largest-eigenvalue) components.
    let (v, lambda) = linalg::eigenvectors_jacobi(r)?;
    let keep = p.saturating_sub(m);
    let mut b = vec![0.0; p];
    for comp in 0..keep {
        let lam = lambda[comp];
        if lam.abs() < 1e-12 {
            continue;
        }
        // v_i = comp-th column of V.
        let vi: Vec<f64> = (0..p).map(|row| v[row][comp]).collect();
        let dot: f64 = (0..p).map(|j| vi[j] * r_xy[j]).sum();
        let coef = dot / lam;
        for j in 0..p {
            b[j] += coef * vi[j];
        }
    }
    Ok(b)
}

/// Compute, print, and (for OUTEST=) return the ridge / IPC regression results
/// for one model (M36.9). `cols` are the in-model regressor columns (no
/// intercept) over the complete-case rows; `y` the response. Requires an
/// intercept model (the caller guards NOINT). Returns the back-transformed
/// rows to be folded into the model's OUTEST entry (empty when OUTEST= is off
/// makes no difference — the rows are cheap and the writer is gated).
#[allow(clippy::too_many_arguments)]
pub(super) fn fit_and_print_ridge_ipc(
    ast: &RegAst,
    model: &RegModel,
    dep_name: &str,
    reg_names: &[String],
    cols: &[Vec<f64>],
    y: &[f64],
    opts: &FitReportOptions,
    session: &mut Session,
) -> Vec<RidgeIpcRow> {
    // Header context shared with `fit_and_print`; the optional statistics
    // blocks in `opts` do not apply to the ridge/IPC report.
    let &FitReportOptions {
        n_read,
        n,
        model_label,
        by_heading,
        ..
    } = opts;
    let std = standardize_for_ridge(cols, y);
    let p = reg_names.len();
    let want_ridge = !ast.data_options.ridge.is_empty();
    let want_ipc = !ast.data_options.pcomit.is_empty();
    let outvif = ast.data_options.outvif;
    let mut rows: Vec<RidgeIpcRow> = Vec::new();

    // Deferred ridge-trace graphics (consistent with the existing PLOTS
    // deferral): emit a single clean NOTE, no image.
    if want_ridge {
        session
            .log
            .note("The ridge trace plot is not produced in this environment.");
    }

    if !model.noprint {
        session.listing.page_header();
        centered(session, "The REG Procedure");
        if let Some(h) = by_heading {
            centered(session, h);
        }
        centered(session, model_label);
        centered(session, &format!("Dependent Variable: {}", dep_name));
        session.listing.blank();
        session.listing.write_line(&format!(
            "               Number of Observations Read         {}",
            n_read
        ));
        session.listing.write_line(&format!(
            "               Number of Observations Used         {}",
            n
        ));
        session.listing.blank();
        session.listing.blank();
    }

    // Column layout shared by the printed ridge / IPC tables: a selector column
    // (_RIDGE_ or _PCOMIT_), then Intercept and one column per regressor.
    let build_headers = |selector: &str| -> (Vec<String>, Vec<Align>) {
        let mut h = vec![selector.to_string(), "Intercept".to_string()];
        let mut a = vec![Align::Right, Align::Right];
        for nm in reg_names {
            h.push(nm.clone());
            a.push(Align::Right);
        }
        (h, a)
    };

    // --- RIDGE ---
    if want_ridge {
        if !model.noprint {
            centered(session, "Ridge Regression Parameter Estimates");
            session.listing.blank();
        }
        let (headers, aligns) = build_headers("Ridge");
        let mut table_rows: Vec<Vec<String>> = Vec::new();
        let mut vif_rows: Vec<Vec<String>> = Vec::new();
        let (vif_headers, vif_aligns) = {
            // VIF table has no intercept column.
            let mut h = vec!["Ridge".to_string()];
            let mut a = vec![Align::Right];
            for nm in reg_names {
                h.push(nm.clone());
                a.push(Align::Right);
            }
            (h, a)
        };
        for &k in &ast.data_options.ridge {
            let b_star = match ridge_beta_star(&std.r, &std.r_xy, k) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let (b0, slopes) = back_transform(&b_star, &std);
            // Printed back-transformed row.
            let mut row = vec![fmt_ridge_sel(k), fmt5(b0)];
            for s in &slopes {
                row.push(fmt5(*s));
            }
            table_rows.push(row);
            // OUTEST RIDGE row: Intercept then slopes (param_names order).
            let mut vals: Vec<Option<f64>> = Vec::with_capacity(p + 1);
            vals.push(Some(b0));
            for s in &slopes {
                vals.push(Some(*s));
            }
            rows.push(RidgeIpcRow {
                kind: "RIDGE",
                ridge_k: Some(k),
                pcomit_m: None,
                values: vals,
            });
            // OUTVIF: ridge VIF row (no intercept value).
            if outvif {
                if let Ok(vif) = ridge_vif(&std.r, k) {
                    let mut vrow = vec![fmt_ridge_sel(k)];
                    for v in &vif {
                        vrow.push(fmt5(*v));
                    }
                    vif_rows.push(vrow);
                    let mut vvals: Vec<Option<f64>> = Vec::with_capacity(p + 1);
                    vvals.push(None); // intercept slot
                    for v in &vif {
                        vvals.push(Some(*v));
                    }
                    rows.push(RidgeIpcRow {
                        kind: "RIDGEVIF",
                        ridge_k: Some(k),
                        pcomit_m: None,
                        values: vvals,
                    });
                }
            }
        }
        if !model.noprint {
            session.listing.write_table(&headers, &aligns, &table_rows);
            if outvif && !vif_rows.is_empty() {
                session.listing.blank();
                session.listing.blank();
                centered(session, "Ridge Regression VIF");
                session.listing.blank();
                session
                    .listing
                    .write_table(&vif_headers, &vif_aligns, &vif_rows);
            }
        }
    }

    // --- IPC ---
    if want_ipc {
        if !model.noprint {
            if want_ridge {
                session.listing.blank();
                session.listing.blank();
            }
            centered(
                session,
                "Incomplete Principal Components Parameter Estimates",
            );
            session.listing.blank();
        }
        let (headers, aligns) = build_headers("PCOMIT");
        let mut table_rows: Vec<Vec<String>> = Vec::new();
        for &mv in &ast.data_options.pcomit {
            let m = mv.max(0.0).trunc() as usize;
            let m = m.min(p);
            let b_star = match ipc_beta_star(&std.r, &std.r_xy, m) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let (b0, slopes) = back_transform(&b_star, &std);
            let mut row = vec![format!("{}", m), fmt5(b0)];
            for s in &slopes {
                row.push(fmt5(*s));
            }
            table_rows.push(row);
            let mut vals: Vec<Option<f64>> = Vec::with_capacity(p + 1);
            vals.push(Some(b0));
            for s in &slopes {
                vals.push(Some(*s));
            }
            rows.push(RidgeIpcRow {
                kind: "IPC",
                ridge_k: None,
                pcomit_m: Some(m as f64),
                values: vals,
            });
        }
        if !model.noprint {
            session.listing.write_table(&headers, &aligns, &table_rows);
        }
    }

    rows
}

/// Format a ridge constant for the printed `_RIDGE_` selector column (M36.9).
fn fmt_ridge_sel(k: f64) -> String {
    // Render integers without trailing decimals, otherwise a compact decimal.
    if (k - k.round()).abs() < 1e-12 {
        format!("{}", k.round() as i64)
    } else {
        let s = format!("{:.5}", k);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}
