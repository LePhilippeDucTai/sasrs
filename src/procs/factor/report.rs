use super::*;

/// Eigenvalues table (all p eigenvalues), with its title lines.
pub(super) fn print_eigenvalue_table(
    session: &mut Session,
    lambda: &[f64],
    trace: f64,
    p: usize,
    cov: bool,
) {
    let eig_title = if cov {
        "Eigenvalues of the Covariance Matrix"
    } else {
        "Eigenvalues of the Correlation Matrix"
    };
    let total_label = if cov {
        let avg = if p > 0 { trace / p as f64 } else { 0.0 };
        format!("Total = {:.4}   Average = {:.4}", trace, avg)
    } else {
        format!("Total = {:.0}   Average = 1", p)
    };
    centered(session, eig_title);
    centered(session, &total_label);
    session.listing.blank();
    let headers: Vec<String> = vec![
        String::new(),
        "Eigenvalue".into(),
        "Difference".into(),
        "Proportion".into(),
        "Cumulative".into(),
    ];
    let aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(p);
    let mut cumulative = 0.0_f64;
    for i in 0..p {
        cumulative += lambda[i];
        let diff = if i + 1 < p {
            format!("{:.4}", lambda[i] - lambda[i + 1])
        } else {
            ".".to_string()
        };
        let proportion = if trace != 0.0 { lambda[i] / trace } else { 0.0 };
        let cumul = if trace != 0.0 {
            cumulative / trace
        } else {
            0.0
        };
        rows.push(vec![
            format!("{}", i + 1),
            format!("{:.4}", lambda[i]),
            diff,
            format!("{:.4}", proportion),
            format!("{:.4}", cumul),
        ]);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// Print a `p × k` factor-pattern table under `title` (shared by the initial,
/// varimax-rotated and promax-rotated patterns).
pub(super) fn print_factor_pattern(
    session: &mut Session,
    title: &str,
    names: &[String],
    pattern: &[Vec<f64>],
    k: usize,
) {
    let p = names.len();
    centered(session, title);
    session.listing.blank();
    let mut headers: Vec<String> = vec![String::new()];
    let mut aligns: Vec<Align> = vec![Align::Left];
    for j in 0..k {
        headers.push(format!("Factor{}", j + 1));
        aligns.push(Align::Right);
    }
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(p);
    for i in 0..p {
        let mut row = vec![names[i].clone()];
        for v in &pattern[i][..k] {
            row.push(format!("{v:.4}"));
        }
        rows.push(row);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// "Variance Explained by Each Factor" (Weighted/Unweighted rows).
pub(super) fn print_variance_explained(session: &mut Session, factor_variance: &[f64], k: usize) {
    centered(session, "Variance Explained by Each Factor");
    session.listing.blank();
    let mut headers: Vec<String> = vec![String::new()];
    let mut aligns: Vec<Align> = vec![Align::Left];
    for j in 0..k {
        headers.push(format!("Factor{}", j + 1));
        aligns.push(Align::Right);
    }
    let mut weighted_row = vec!["Weighted".to_string()];
    let mut unweighted_row = vec!["Unweighted".to_string()];
    for v in factor_variance.iter().take(k) {
        weighted_row.push(format!("{v:.4}"));
        unweighted_row.push(format!("{v:.4}"));
    }
    session
        .listing
        .write_table(&headers, &aligns, &[weighted_row, unweighted_row]);
    session.listing.blank();
}

/// "Final Communality Estimates: Total = …" plus the per-variable table.
pub(super) fn print_final_communalities(
    session: &mut Session,
    names: &[String],
    communalities: &[f64],
) {
    let total_communality: f64 = communalities.iter().sum();
    centered(
        session,
        &format!(
            "Final Communality Estimates: Total = {:.4}",
            total_communality
        ),
    );
    session.listing.blank();
    let mut headers: Vec<String> = vec![String::new()];
    let mut aligns: Vec<Align> = vec![Align::Left];
    for nm in names {
        headers.push(nm.clone());
        aligns.push(Align::Right);
    }
    let mut row: Vec<String> = vec![String::new()];
    for &h2 in communalities {
        row.push(format!("{:.4}", h2));
    }
    session.listing.write_table(&headers, &aligns, &[row]);
    session.listing.blank();
}

/// VARIMAX branch: rotate, print the rotated pattern, rotated variances and
/// communalities. Returns the rotated pattern (used for OUT= scoring).
pub(super) fn print_varimax_section(
    session: &mut Session,
    names: &[String],
    loadings: &[Vec<f64>],
    k: usize,
) -> Vec<Vec<f64>> {
    let (l_rot, _rot_matrix) = varimax(loadings);

    // Rotated variance by factor.
    let rot_variance: Vec<f64> = (0..k)
        .map(|j| l_rot.iter().map(|row| row[j] * row[j]).sum::<f64>())
        .collect();

    centered(session, "Rotation Method: Varimax");
    session.listing.blank();

    print_factor_pattern(session, "Rotated Factor Pattern", names, &l_rot, k);

    centered(session, "Variance Explained by Each Rotated Factor");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for j in 0..k {
            headers.push(format!("Factor{}", j + 1));
            aligns.push(Align::Right);
        }
        let mut rot_row: Vec<String> = vec![String::new()];
        for v in rot_variance.iter().take(k) {
            rot_row.push(format!("{v:.4}"));
        }
        session.listing.write_table(&headers, &aligns, &[rot_row]);
        session.listing.blank();
    }

    // Final communalities (invariant under orthogonal rotation).
    let rot_communalities: Vec<f64> = l_rot
        .iter()
        .map(|row| row.iter().map(|&x| x * x).sum())
        .collect();
    print_final_communalities(session, names, &rot_communalities);

    l_rot
}

/// PROMAX branch: varimax pre-rotation, promax(4), oblique pattern and
/// inter-factor correlations. Returns the oblique pattern for OUT= scoring.
pub(super) fn print_promax_section(
    session: &mut Session,
    names: &[String],
    loadings: &[Vec<f64>],
    k: usize,
) -> Result<Vec<Vec<f64>>> {
    // Promax starts from the orthogonal VARIMAX solution.
    let (l_varimax, _rot_matrix) = varimax(loadings);
    let pm = promax(&l_varimax, 4)?;

    centered(session, "Rotation Method: Promax (power = 4)");
    session.listing.blank();

    // Oblique Rotated Factor Pattern (Standardized Regression Coefficients).
    print_factor_pattern(
        session,
        "Rotated Factor Pattern (Standardized Regression Coefficients)",
        names,
        &pm.pattern,
        k,
    );

    // Inter-Factor Correlations.
    centered(session, "Inter-Factor Correlations");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for j in 0..k {
            headers.push(format!("Factor{}", j + 1));
            aligns.push(Align::Right);
        }
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(k);
        for i in 0..k {
            let mut row = vec![format!("Factor{}", i + 1)];
            for j in 0..k {
                row.push(format!("{:.4}", pm.phi[i][j]));
            }
            rows.push(row);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }
    Ok(pm.pattern)
}
