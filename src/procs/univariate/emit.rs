use super::*;

use crate::procs::common::centered;

/// Emit the full report for a single analysis variable. `data` holds the
/// non-missing (value, obs_number) pairs in original observation order.
pub(super) fn emit_variable(
    session: &mut Session,
    name: &str,
    data: &[(f64, usize)],
    n_missing: usize,
    n_total: usize,
    normal: bool,
) {
    session.listing.blank();
    centered(session, &format!("Variable: {name}"));
    session.listing.blank();

    let n = data.len();
    // Plain non-missing values.
    let xs: Vec<f64> = data.iter().map(|(v, _)| *v).collect();
    // Sorted values (for quantiles / mode / median / extremes).
    let mut sorted: Vec<f64> = xs.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

    let nf = n as f64;
    let sum: f64 = xs.iter().sum();
    let mean = if n > 0 { Some(sum / nf) } else { None };
    let s = sample_std(&xs);
    let variance = s.map(|v| v * v);
    let uss: f64 = xs.iter().map(|x| x * x).sum();
    let css: f64 = match mean {
        Some(m) => xs.iter().map(|x| (x - m) * (x - m)).sum(),
        None => 0.0,
    };
    let cv = match (mean, s) {
        (Some(m), Some(sd)) if m != 0.0 => Some(100.0 * sd / m),
        _ => None,
    };
    let std_err = match s {
        Some(sd) if n >= 1 => Some(sd / nf.sqrt()),
        _ => None,
    };
    let skew = skewness(&xs);
    let kurt = kurtosis(&xs);

    // ── Moments ──
    centered(session, "Moments");
    session.listing.blank();
    let moments: Vec<(&str, String, &str, String)> = vec![
        (
            "N",
            format!("{n}"),
            "Sum Weights",
            format!("{n}"),
        ),
        (
            "Mean",
            fmt_opt(mean),
            "Sum Observations",
            fmt_num(sum),
        ),
        (
            "Std Deviation",
            fmt_opt(s),
            "Variance",
            fmt_opt(variance),
        ),
        (
            "Skewness",
            fmt_opt(skew),
            "Kurtosis",
            fmt_opt(kurt),
        ),
        (
            "Uncorrected SS",
            fmt_num(uss),
            "Corrected SS",
            fmt_num(css),
        ),
        (
            "Coeff Variation",
            fmt_opt(cv),
            "Std Error Mean",
            fmt_opt(std_err),
        ),
    ];
    let m_rows: Vec<Vec<String>> = moments
        .into_iter()
        .map(|(la, va, lb, vb)| vec![la.to_string(), va, lb.to_string(), vb])
        .collect();
    session.listing.write_table(
        &[
            "Label1".into(),
            "Value1".into(),
            "Label2".into(),
            "Value2".into(),
        ],
        &[Align::Left, Align::Right, Align::Left, Align::Right],
        &m_rows,
    );

    // ── Basic Statistical Measures ──
    session.listing.blank();
    centered(session, "Basic Statistical Measures");
    session.listing.blank();
    let median = quantile_def5(&sorted, 0.5);
    let mode_v = mode(&sorted);
    let range = if n > 0 {
        Some(sorted[n - 1] - sorted[0])
    } else {
        None
    };
    let q3 = quantile_def5(&sorted, 0.75);
    let q1 = quantile_def5(&sorted, 0.25);
    let iqr = match (q3, q1) {
        (Some(a), Some(b)) => Some(a - b),
        _ => None,
    };
    let basic_rows: Vec<Vec<String>> = vec![
        vec![
            "Mean".into(),
            fmt_opt(mean),
            "Std Deviation".into(),
            fmt_opt(s),
        ],
        vec![
            "Median".into(),
            fmt_opt(median),
            "Variance".into(),
            fmt_opt(variance),
        ],
        vec![
            "Mode".into(),
            fmt_opt(mode_v),
            "Range".into(),
            fmt_opt(range),
        ],
        vec![
            "".into(),
            "".into(),
            "Interquartile Range".into(),
            fmt_opt(iqr),
        ],
    ];
    session.listing.write_table(
        &[
            "LocLabel".into(),
            "LocValue".into(),
            "VarLabel".into(),
            "VarValue".into(),
        ],
        &[Align::Left, Align::Right, Align::Left, Align::Right],
        &basic_rows,
    );

    // ── Tests for Normality (only when requested via NORMAL) ──
    if normal {
        emit_normality_tests(session, &sorted, mean, s, n);
    }

    // ── Quantiles (Definition 5) ──
    session.listing.blank();
    centered(session, "Quantiles (Definition 5)");
    session.listing.blank();
    let levels: &[(&str, f64)] = &[
        ("100% Max", 1.0),
        ("99%", 0.99),
        ("95%", 0.95),
        ("90%", 0.90),
        ("75% Q3", 0.75),
        ("50% Median", 0.50),
        ("25% Q1", 0.25),
        ("10%", 0.10),
        ("5%", 0.05),
        ("1%", 0.01),
        ("0% Min", 0.0),
    ];
    let q_rows: Vec<Vec<String>> = levels
        .iter()
        .map(|(label, p)| vec![label.to_string(), fmt_opt(quantile_def5(&sorted, *p))])
        .collect();
    session.listing.write_table(
        &["Quantile".into(), "Estimate".into()],
        &[Align::Left, Align::Right],
        &q_rows,
    );

    // ── Extreme Observations ──
    session.listing.blank();
    centered(session, "Extreme Observations");
    session.listing.blank();
    // Order data by value, then by obs number (stable for ties).
    let mut by_val: Vec<(f64, usize)> = data.to_vec();
    by_val.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    let k = by_val.len().min(5);
    let lowest = &by_val[..k];
    let highest = &by_val[by_val.len().saturating_sub(5)..];
    // Pair them up row-by-row (both columns show up to 5 entries).
    let mut ext_rows: Vec<Vec<String>> = Vec::new();
    for i in 0..5 {
        let (lv, lo) = match lowest.get(i) {
            Some((v, o)) => (fmt_num(*v), format!("{o}")),
            None => (String::new(), String::new()),
        };
        // Highest displayed ascending too (SAS shows the top 5 in ascending
        // order within the Highest column).
        let (hv, ho) = match highest.get(i) {
            Some((v, o)) => (fmt_num(*v), format!("{o}")),
            None => (String::new(), String::new()),
        };
        ext_rows.push(vec![lv, lo, hv, ho]);
    }
    session.listing.write_table(
        &[
            "Lowest Value".into(),
            "Lowest Obs".into(),
            "Highest Value".into(),
            "Highest Obs".into(),
        ],
        &[Align::Right, Align::Right, Align::Right, Align::Right],
        &ext_rows,
    );

    // ── Missing Values ──
    if n_missing > 0 {
        session.listing.blank();
        centered(session, "Missing Values");
        session.listing.blank();
        let pct = if n_total > 0 {
            100.0 * n_missing as f64 / n_total as f64
        } else {
            0.0
        };
        session.listing.write_table(
            &[
                "Missing Value".into(),
                "Count".into(),
                "Percent Of All Obs".into(),
            ],
            &[Align::Left, Align::Right, Align::Right],
            &[vec![".".into(), format!("{n_missing}"), fmt_num(pct)]],
        );
    }
}

/// Emit the report for a single analysis variable with a WEIGHT variable in
/// effect. `pairs` are the usable (value, weight) pairs (excluding missing
/// values, missing weights, and weights ≤ 0); `n_missing` is the excluded
/// count, `n_total` the group's total row count.
///
/// Moments and Basic Measures mean/std/variance use the weighted formulas
/// (see file header). Skewness/Kurtosis are computed on the UNWEIGHTED values
/// (documented divergence). Quantiles use the SAS WEIGHTED Definition 5
/// (`weighted_quantile_def5`); the Extreme Observations section lists the raw
/// extreme VALUES with their obs numbers (extremes are not weighted in SAS).
/// `obs_pairs` are the usable `(value, obs_number)` pairs in row order.
pub(super) fn emit_variable_weighted(
    session: &mut Session,
    name: &str,
    pairs: &[(f64, f64)],
    obs_pairs: &[(f64, usize)],
    n_missing: usize,
    n_total: usize,
) {
    session.listing.blank();
    centered(session, &format!("Variable: {name}"));
    session.listing.blank();

    let n = pairs.len();
    let nf = n as f64;
    let xs: Vec<f64> = pairs.iter().map(|(x, _)| *x).collect();

    // Pairs sorted ascending by value, for the weighted quantiles / median /
    // mode / range (weights stay attached to their value).
    let mut sorted_pairs: Vec<(f64, f64)> = pairs.to_vec();
    sorted_pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));

    let sum_w: f64 = pairs.iter().map(|(_, w)| *w).sum();
    let sum_wx: f64 = pairs.iter().map(|(x, w)| w * x).sum();
    let mean_w = if sum_w != 0.0 {
        Some(sum_wx / sum_w)
    } else {
        None
    };
    // Weighted corrected / uncorrected sums of squares.
    let css_w: f64 = match mean_w {
        Some(m) => pairs.iter().map(|(x, w)| w * (x - m) * (x - m)).sum(),
        None => 0.0,
    };
    let uss_w: f64 = pairs.iter().map(|(x, w)| w * x * x).sum();
    let variance = if n >= 2 {
        Some(css_w / (nf - 1.0))
    } else {
        None
    };
    let std = variance.map(|v| v.sqrt());
    let cv = match (mean_w, std) {
        (Some(m), Some(sd)) if m != 0.0 => Some(100.0 * sd / m),
        _ => None,
    };
    // SAS weighted std error of the mean: Std / sqrt(Σ w_i).
    let std_err = match std {
        Some(sd) if sum_w > 0.0 => Some(sd / sum_w.sqrt()),
        _ => None,
    };
    // Skewness / kurtosis deferred → computed on UNWEIGHTED values.
    let skew = skewness(&xs);
    let kurt = kurtosis(&xs);

    // ── Moments ──
    centered(session, "Moments");
    session.listing.blank();
    let moments: Vec<(&str, String, &str, String)> = vec![
        ("N", format!("{n}"), "Sum Weights", fmt_num(sum_w)),
        ("Mean", fmt_opt(mean_w), "Sum Observations", fmt_num(sum_wx)),
        ("Std Deviation", fmt_opt(std), "Variance", fmt_opt(variance)),
        ("Skewness", fmt_opt(skew), "Kurtosis", fmt_opt(kurt)),
        ("Uncorrected SS", fmt_num(uss_w), "Corrected SS", fmt_num(css_w)),
        ("Coeff Variation", fmt_opt(cv), "Std Error Mean", fmt_opt(std_err)),
    ];
    let m_rows: Vec<Vec<String>> = moments
        .into_iter()
        .map(|(la, va, lb, vb)| vec![la.to_string(), va, lb.to_string(), vb])
        .collect();
    session.listing.write_table(
        &[
            "Label1".into(),
            "Value1".into(),
            "Label2".into(),
            "Value2".into(),
        ],
        &[Align::Left, Align::Right, Align::Left, Align::Right],
        &m_rows,
    );

    // ── Basic Statistical Measures ── (weighted mean/std/variance; weighted
    // median/Q1/Q3/range via the weighted Definition-5 quantiles; mode is the
    // most frequent VALUE, as in the unweighted path).
    session.listing.blank();
    centered(session, "Basic Statistical Measures");
    session.listing.blank();
    let median = weighted_quantile_def5(&sorted_pairs, 0.50);
    let q1 = weighted_quantile_def5(&sorted_pairs, 0.25);
    let q3 = weighted_quantile_def5(&sorted_pairs, 0.75);
    let iqr = match (q3, q1) {
        (Some(a), Some(b)) => Some(a - b),
        _ => None,
    };
    let sorted_vals: Vec<f64> = sorted_pairs.iter().map(|(x, _)| *x).collect();
    let mode_v = mode(&sorted_vals);
    let range = if n > 0 {
        Some(sorted_vals[n - 1] - sorted_vals[0])
    } else {
        None
    };
    let basic_rows: Vec<Vec<String>> = vec![
        vec![
            "Mean".into(),
            fmt_opt(mean_w),
            "Std Deviation".into(),
            fmt_opt(std),
        ],
        vec![
            "Median".into(),
            fmt_opt(median),
            "Variance".into(),
            fmt_opt(variance),
        ],
        vec![
            "Mode".into(),
            fmt_opt(mode_v),
            "Range".into(),
            fmt_opt(range),
        ],
        vec![
            "".into(),
            "".into(),
            "Interquartile Range".into(),
            fmt_opt(iqr),
        ],
    ];
    session.listing.write_table(
        &[
            "LocLabel".into(),
            "LocValue".into(),
            "VarLabel".into(),
            "VarValue".into(),
        ],
        &[Align::Left, Align::Right, Align::Left, Align::Right],
        &basic_rows,
    );

    // ── Quantiles (Definition 5, weighted) ──
    session.listing.blank();
    centered(session, "Quantiles (Definition 5)");
    session.listing.blank();
    let levels: &[(&str, f64)] = &[
        ("100% Max", 1.0),
        ("99%", 0.99),
        ("95%", 0.95),
        ("90%", 0.90),
        ("75% Q3", 0.75),
        ("50% Median", 0.50),
        ("25% Q1", 0.25),
        ("10%", 0.10),
        ("5%", 0.05),
        ("1%", 0.01),
        ("0% Min", 0.0),
    ];
    let q_rows: Vec<Vec<String>> = levels
        .iter()
        .map(|(label, p)| {
            vec![
                label.to_string(),
                fmt_opt(weighted_quantile_def5(&sorted_pairs, *p)),
            ]
        })
        .collect();
    session.listing.write_table(
        &["Quantile".into(), "Estimate".into()],
        &[Align::Left, Align::Right],
        &q_rows,
    );

    // ── Extreme Observations ── (raw extreme VALUES + obs numbers; extremes
    // are not weighted, matching SAS).
    session.listing.blank();
    centered(session, "Extreme Observations");
    session.listing.blank();
    let mut by_val: Vec<(f64, usize)> = obs_pairs.to_vec();
    by_val.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    let k = by_val.len().min(5);
    let lowest = &by_val[..k];
    let highest = &by_val[by_val.len().saturating_sub(5)..];
    let mut ext_rows: Vec<Vec<String>> = Vec::new();
    for i in 0..5 {
        let (lv, lo) = match lowest.get(i) {
            Some((v, o)) => (fmt_num(*v), format!("{o}")),
            None => (String::new(), String::new()),
        };
        let (hv, ho) = match highest.get(i) {
            Some((v, o)) => (fmt_num(*v), format!("{o}")),
            None => (String::new(), String::new()),
        };
        ext_rows.push(vec![lv, lo, hv, ho]);
    }
    session.listing.write_table(
        &[
            "Lowest Value".into(),
            "Lowest Obs".into(),
            "Highest Value".into(),
            "Highest Obs".into(),
        ],
        &[Align::Right, Align::Right, Align::Right, Align::Right],
        &ext_rows,
    );

    // ── Missing Values ──
    if n_missing > 0 {
        session.listing.blank();
        centered(session, "Missing Values");
        session.listing.blank();
        let pct = if n_total > 0 {
            100.0 * n_missing as f64 / n_total as f64
        } else {
            0.0
        };
        session.listing.write_table(
            &[
                "Missing Value".into(),
                "Count".into(),
                "Percent Of All Obs".into(),
            ],
            &[Align::Left, Align::Right, Align::Right],
            &[vec![".".into(), format!("{n_missing}"), fmt_num(pct)]],
        );
    }
}
