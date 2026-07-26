use super::*;

// ───────────────────────── M36.6: all-subsets & R²-improvement ─────────────────────────

/// Maximum number of regressors for which we enumerate the full 2^p subset
/// lattice. Beyond this the search is exponential and impractical, so we emit a
/// NOTE and cap the enumeration to the forced (INCLUDE=) regressors only.
pub(crate) const ALL_SUBSETS_PMAX: usize = 20;

/// A single evaluated subset in an all-subsets table.
pub(crate) struct SubsetRow {
    /// Number of regressors in the model (= subset.len()).
    pub(super) k: usize,
    /// R² = 1 − SSE/SST.
    pub(super) r2: f64,
    /// Adjusted R².
    pub(super) adj: f64,
    /// Mallows' C(p).
    pub(super) cp: f64,
    /// Regressor column indices, in MODEL order.
    pub(super) cols: Vec<usize>,
}

/// RSQUARE / ADJRSQ / CP all-subsets evaluation (M36.6). Enumerates every subset
/// of the regressors with the first `include` regressors forced in, for sizes in
/// `[start.unwrap_or(include).max(1), stop.unwrap_or(p)]`, fits each, and prints
/// the corresponding SAS selection table. Does NOT alter the post-selection fit:
/// the caller selects the full model.
pub(crate) fn run_all_subsets(
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
pub(crate) fn print_all_subsets_table(
    sel: &Selection,
    rows: &[SubsetRow],
    regressors: &[String],
    session: &mut Session,
) {
    let (title, extra_header): (&str, Option<&str>) = match sel.method {
        SelMethod::AdjRsq => (
            "Adjusted R-Square Selection Method",
            Some("Adjusted R-Square"),
        ),
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
                b.adj
                    .partial_cmp(&a.adj)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            if let Some(b) = sel.best {
                display.truncate(b);
            }
        }
        SelMethod::Cp => {
            display.sort_by(|a, b| a.cp.partial_cmp(&b.cp).unwrap_or(std::cmp::Ordering::Equal));
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
            let vars: Vec<&str> = row.cols.iter().map(|&c| regressors[c].as_str()).collect();
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
