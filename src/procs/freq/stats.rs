use super::*;

/// Fisher's exact test. Full exact two-sided p-value for 2x2 tables (sum of
/// hypergeometric probabilities ≤ that of the observed table), plus the
/// left/right one-sided tails and the observed table probability. Tables
/// larger than 2x2 are deferred with a graceful note (no panic).
pub(super) fn fisher_block(
    session: &mut Session,
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
) {
    let nr = row_tot.len();
    let nc = col_tot.len();
    session.listing.blank();
    session.listing.write_line("Fisher's Exact Test");
    session.listing.blank();

    if nr != 2 || nc != 2 {
        session
            .listing
            .write_line("Fisher's exact test for tables larger than 2x2 is not supported.");
        return;
    }
    if grand == 0 {
        session
            .listing
            .write_line("Fisher's Exact Test is not computable for this table.");
        return;
    }

    // Margins are fixed. With r1 = row_tot[0], c1 = col_tot[0], n = grand, the
    // count a = freq[0][0] determines the whole table. a ranges over
    // [max(0, r1+c1-n), min(r1, c1)]. The hypergeometric probability of a is
    // C(r1,a)·C(r2,c1-a)/C(n,c1).
    let r1 = row_tot[0] as i64;
    let r2 = row_tot[1] as i64;
    let c1 = col_tot[0] as i64;
    let n = grand as i64;
    let a_obs = freq[0][0] as i64;

    let ln_p = |a: i64| -> f64 {
        let b = c1 - a; // freq[1][0]
        ln_choose(r1 as u64, a as u64) + ln_choose(r2 as u64, b as u64)
            - ln_choose(n as u64, c1 as u64)
    };

    let lo = 0.max(r1 + c1 - n);
    let hi = r1.min(c1);
    let p_obs = ln_p(a_obs).exp();

    let mut p_left = 0.0_f64; // P(A <= a_obs)
    let mut p_right = 0.0_f64; // P(A >= a_obs)
    let mut p_two = 0.0_f64; // sum of probs <= p_obs (with tolerance)
    let tol = 1e-7;
    for a in lo..=hi {
        let p = ln_p(a).exp();
        if a <= a_obs {
            p_left += p;
        }
        if a >= a_obs {
            p_right += p;
        }
        if p <= p_obs * (1.0 + tol) {
            p_two += p;
        }
    }
    let clamp = |p: f64| p.clamp(0.0, 1.0);

    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec!["Cell (1,1) Frequency (F)".to_string(), format!("{a_obs}")],
        vec![
            "Left-sided Pr <= F".to_string(),
            fmt_chisq_p(clamp(p_left)),
        ],
        vec![
            "Right-sided Pr >= F".to_string(),
            fmt_chisq_p(clamp(p_right)),
        ],
        vec!["Table Probability (P)".to_string(), fmt_chisq_p(clamp(p_obs))],
        vec!["Two-sided Pr <= P".to_string(), fmt_chisq_p(clamp(p_two))],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Cochran-Armitage trend test. Requires a 2-row (or 2-column) table; the
/// non-binary dimension supplies ordinal scores 1..k. Reports the Z statistic
/// with one- and two-sided normal-approximation p-values. Other shapes are
/// deferred with a graceful note.
pub(super) fn trend_block(
    session: &mut Session,
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
) {
    let nr = row_tot.len();
    let nc = col_tot.len();
    session.listing.blank();
    session.listing.write_line("Cochran-Armitage Trend Test");
    session.listing.blank();

    if grand == 0 || (nr != 2 && nc != 2) || nr < 2 || nc < 2 {
        session
            .listing
            .write_line("The Cochran-Armitage Trend Test requires a 2xC or Rx2 table.");
        return;
    }

    // Orient so that there are 2 rows and `k` ordinal columns. If the table is
    // Rx2 instead, transpose roles (scores along rows).
    // We compute using the first row's counts (n_{1i}) against column totals.
    // T = Σ s_i (n_{1i} - r1 * c_i / N).
    // Var(T) = (r1*r2/N) * [ Σ c_i s_i² - (Σ c_i s_i)² / N ].
    let (cells_row1, marg): (Vec<f64>, Vec<f64>);
    let r1f: f64;
    let r2f: f64;
    if nr == 2 {
        cells_row1 = (0..nc).map(|c| freq[0][c] as f64).collect();
        marg = col_tot.iter().map(|&c| c as f64).collect();
        r1f = row_tot[0] as f64;
        r2f = row_tot[1] as f64;
    } else {
        // Rx2: treat columns as the binary dimension, rows as ordinal scores.
        cells_row1 = (0..nr).map(|r| freq[r][0] as f64).collect();
        marg = row_tot.iter().map(|&r| r as f64).collect();
        r1f = col_tot[0] as f64;
        r2f = col_tot[1] as f64;
    }
    let k = cells_row1.len();
    let scores: Vec<f64> = (1..=k).map(|i| i as f64).collect();
    let nf = grand as f64;

    let mut t = 0.0_f64;
    let mut sum_cs = 0.0_f64; // Σ c_i s_i
    let mut sum_cs2 = 0.0_f64; // Σ c_i s_i²
    for i in 0..k {
        t += scores[i] * (cells_row1[i] - r1f * marg[i] / nf);
        sum_cs += marg[i] * scores[i];
        sum_cs2 += marg[i] * scores[i] * scores[i];
    }
    let var = (r1f * r2f / nf) * (sum_cs2 - sum_cs * sum_cs / nf);

    if var <= 0.0 {
        session
            .listing
            .write_line("The Cochran-Armitage Trend Test is not computable for this table.");
        return;
    }
    let z = t / var.sqrt();
    // One-sided p toward the observed direction; two-sided = 2*one-sided.
    let p_one = 1.0 - probnorm(z.abs());
    let p_two = (2.0 * p_one).min(1.0);

    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec!["Statistic (Z)".to_string(), format!("{z:.4}")],
        vec!["One-sided Pr".to_string(), fmt_chisq_p(p_one.clamp(0.0, 1.0))],
        vec!["Two-sided Pr".to_string(), fmt_chisq_p(p_two.clamp(0.0, 1.0))],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// MEASURES / RELRISK: odds ratio and the two cohort relative risks for a 2x2
/// table, each with a 95% confidence interval (Wald, on the log scale). Cells
/// containing zeros yield missing estimates rather than dividing by zero.
pub(super) fn measures_block(session: &mut Session, freq: &[Vec<usize>]) {
    session.listing.blank();
    session
        .listing
        .write_line("Estimates of the Relative Risk (Row1/Row2)");
    session.listing.blank();

    if freq.len() != 2 || freq[0].len() != 2 || freq[1].len() != 2 {
        session
            .listing
            .write_line("Relative risk estimates require a 2x2 table.");
        return;
    }

    let a = freq[0][0] as f64;
    let b = freq[0][1] as f64;
    let c = freq[1][0] as f64;
    let d = freq[1][1] as f64;

    let headers = vec![
        "Type of Study".to_string(),
        "Value".to_string(),
        "95% Confidence Limits".to_string(),
    ];
    let aligns = vec![Align::Left, Align::Right, Align::Left];
    let mut rows: Vec<Vec<String>> = Vec::new();

    // Helper rendering "lo   hi" or "." when an estimate is undefined.
    let limits = |lo: f64, hi: f64, ok: bool| -> String {
        if ok {
            format!("{lo:.4}   {hi:.4}")
        } else {
            ".".to_string()
        }
    };

    // Odds ratio = ad/bc; SE(ln OR) = sqrt(1/a+1/b+1/c+1/d).
    if a > 0.0 && b > 0.0 && c > 0.0 && d > 0.0 {
        let or = (a * d) / (b * c);
        let se = (1.0 / a + 1.0 / b + 1.0 / c + 1.0 / d).sqrt();
        let (lo, hi) = (
            (or.ln() - 1.96 * se).exp(),
            (or.ln() + 1.96 * se).exp(),
        );
        rows.push(vec![
            "Case-Control (Odds Ratio)".to_string(),
            format!("{or:.4}"),
            limits(lo, hi, true),
        ]);
    } else {
        rows.push(vec![
            "Case-Control (Odds Ratio)".to_string(),
            ".".to_string(),
            ".".to_string(),
        ]);
    }

    // Cohort (Col1 Risk): RR = [a/(a+b)] / [c/(c+d)].
    let r1 = a + b;
    let r2 = c + d;
    if r1 > 0.0 && r2 > 0.0 && a > 0.0 && c > 0.0 {
        let rr = (a / r1) / (c / r2);
        let se = (b / (a * r1) + d / (c * r2)).sqrt();
        let (lo, hi) = ((rr.ln() - 1.96 * se).exp(), (rr.ln() + 1.96 * se).exp());
        rows.push(vec![
            "Cohort (Col1 Risk)".to_string(),
            format!("{rr:.4}"),
            limits(lo, hi, true),
        ]);
    } else {
        rows.push(vec![
            "Cohort (Col1 Risk)".to_string(),
            ".".to_string(),
            ".".to_string(),
        ]);
    }

    // Cohort (Col2 Risk): RR = [b/(a+b)] / [d/(c+d)].
    if r1 > 0.0 && r2 > 0.0 && b > 0.0 && d > 0.0 {
        let rr = (b / r1) / (d / r2);
        let se = (a / (b * r1) + c / (d * r2)).sqrt();
        let (lo, hi) = ((rr.ln() - 1.96 * se).exp(), (rr.ln() + 1.96 * se).exp());
        rows.push(vec![
            "Cohort (Col2 Risk)".to_string(),
            format!("{rr:.4}"),
            limits(lo, hi, true),
        ]);
    } else {
        rows.push(vec![
            "Cohort (Col2 Risk)".to_string(),
            ".".to_string(),
            ".".to_string(),
        ]);
    }

    session.listing.write_table(&headers, &aligns, &rows);
}

/// AGREE: Cohen's simple kappa coefficient for a square table, with its
/// asymptotic standard error and a 95% confidence interval. Non-square tables
/// are rejected with a graceful note.
pub(super) fn agree_block(
    session: &mut Session,
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
) {
    let nr = row_tot.len();
    let nc = col_tot.len();
    session.listing.blank();
    session.listing.write_line("Simple Kappa Coefficient");
    session.listing.blank();

    if nr != nc {
        session
            .listing
            .write_line("AGREE requires a square table.");
        return;
    }
    if grand == 0 {
        session
            .listing
            .write_line("Simple Kappa Coefficient is not computable for this table.");
        return;
    }

    let n = grand as f64;
    // Observed agreement Po = Σ p_ii ; expected Pe = Σ p_i+ · p_+i.
    let mut po = 0.0_f64;
    let mut pe = 0.0_f64;
    for i in 0..nr {
        po += freq[i][i] as f64 / n;
        pe += (row_tot[i] as f64 / n) * (col_tot[i] as f64 / n);
    }

    if (1.0 - pe).abs() < 1e-12 {
        session
            .listing
            .write_line("Simple Kappa Coefficient is not computable (perfect expected agreement).");
        return;
    }
    let kappa = (po - pe) / (1.0 - pe);

    // Asymptotic standard error under H1 (Fleiss et al.), the SAS ASE.
    // ASE = sqrt( [ A + B - C ] / [ (1-Pe)² · n ] ) with
    //   A = Σ p_ii [1 - (p_i+ + p_+i)(1 - kappa)]²
    //   B = (1-kappa)² Σ_{i≠j} p_ij (p_+i + p_j+)²
    //   C = (kappa - Pe(1-kappa))²
    let p = |i: usize, j: usize| freq[i][j] as f64 / n;
    let pr = |i: usize| row_tot[i] as f64 / n; // p_i+ (row marginal)
    let pc = |j: usize| col_tot[j] as f64 / n; // p_+j (col marginal)

    let mut term_a = 0.0_f64;
    for i in 0..nr {
        let s = 1.0 - (pr(i) + pc(i)) * (1.0 - kappa);
        term_a += p(i, i) * s * s;
    }
    let mut term_b = 0.0_f64;
    for i in 0..nr {
        for j in 0..nc {
            if i != j {
                let s = pc(i) + pr(j);
                term_b += p(i, j) * s * s;
            }
        }
    }
    term_b *= (1.0 - kappa) * (1.0 - kappa);
    let term_c = (kappa - pe * (1.0 - kappa)).powi(2);

    let var = (term_a + term_b - term_c) / ((1.0 - pe).powi(2) * n);
    let ase = if var > 0.0 { var.sqrt() } else { 0.0 };
    let lower = kappa - 1.96 * ase;
    let upper = kappa + 1.96 * ase;

    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec!["Kappa".to_string(), format!("{kappa:.4}")],
        vec!["ASE".to_string(), format!("{ase:.4}")],
        vec!["95% Lower Conf Limit".to_string(), format!("{lower:.4}")],
        vec!["95% Upper Conf Limit".to_string(), format!("{upper:.4}")],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Format a p-value SAS-style: `<.0001`, else 4 decimals (mirrors corr.rs).
pub(super) fn fmt_chisq_p(p: f64) -> String {
    if p < 0.0001 {
        "<.0001".to_string()
    } else {
        format!("{p:.4}")
    }
}

/// Print the "Statistics for Table of <row> by <col>" CHISQ block for a
/// two-way table: Pearson chi-square and the likelihood-ratio chi-square,
/// each with DF and an upper-tail p-value. Degenerate tables (grand total 0,
/// any zero margin, or DF <= 0) are skipped gracefully with a note.
pub(super) fn chisq_block(
    session: &mut Session,
    row_name: &str,
    col_name: &str,
    freq: &[Vec<f64>],
    row_tot: &[f64],
    col_tot: &[f64],
    grand: f64,
) {
    session.listing.blank();
    session
        .listing
        .write_line(&format!("Statistics for Table of {row_name} by {col_name}"));
    session.listing.blank();

    let nr = row_tot.len();
    let nc = col_tot.len();
    let df = (nr.saturating_sub(1)) * (nc.saturating_sub(1));

    // Guard against degenerate tables: no expected counts are defined.
    if grand <= 0.0
        || df == 0
        || row_tot.iter().any(|&t| t <= 0.0)
        || col_tot.iter().any(|&t| t <= 0.0)
    {
        session
            .listing
            .write_line("Chi-Square statistics are not computable for this table.");
        return;
    }

    let g = grand;
    let mut pearson = 0.0_f64;
    let mut lratio = 0.0_f64;
    for r in 0..nr {
        for c in 0..nc {
            let e = row_tot[r] * col_tot[c] / g;
            let n = freq[r][c];
            if e > 0.0 {
                let d = n - e;
                pearson += d * d / e;
            }
            if n > 0.0 && e > 0.0 {
                lratio += n * (n / e).ln();
            }
        }
    }
    lratio *= 2.0;

    let df_f = df as f64;
    let p_pearson = chisq_sf(pearson, df_f);
    let p_lratio = chisq_sf(lratio, df_f);

    let headers = vec![
        "Statistic".to_string(),
        "DF".to_string(),
        "Value".to_string(),
        "Prob".to_string(),
    ];
    let aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
    let rows = vec![
        vec![
            "Chi-Square".to_string(),
            format!("{df}"),
            format!("{pearson:.4}"),
            fmt_chisq_p(p_pearson),
        ],
        vec![
            "Likelihood Ratio Chi-Square".to_string(),
            format!("{df}"),
            format!("{lratio:.4}"),
            fmt_chisq_p(p_lratio),
        ],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}
