use super::*;

// ───────────────────────── Formatting helpers ─────────────────────────

/// Header counts table (Observations/Variables/DF/Classes).
pub(super) fn print_counts_header(session: &mut Session, model: &LdaModel, p: usize) {
    let n = model.n_total;
    let g = model.n_groups;
    let headers: Vec<String> = vec![String::new(), String::new(), String::new(), String::new()];
    let aligns = vec![Align::Left, Align::Right, Align::Left, Align::Right];
    let rows: Vec<Vec<String>> = vec![
        vec![
            "Observations".into(),
            n.to_string(),
            "Variables".into(),
            p.to_string(),
        ],
        vec![
            "DF Total".into(),
            (n as i64 - 1).to_string(),
            "Classes".into(),
            g.to_string(),
        ],
        vec![
            "DF Within Classes".into(),
            (n as i64 - g as i64).to_string(),
            "DF Between Classes".into(),
            (g as i64 - 1).to_string(),
        ],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// "Class Level Information" table.
pub(super) fn print_class_level_info(session: &mut Session, class_name: &str, model: &LdaModel) {
    let n = model.n_total;
    let g = model.n_groups;
    centered(session, "Class Level Information");
    session.listing.blank();
    let headers: Vec<String> = vec![
        class_name.to_string(),
        "Variable".into(),
        "Frequency".into(),
        "Weight".into(),
        "Proportion".into(),
    ];
    let aligns = vec![
        Align::Left,
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(g);
    for k in 0..g {
        let prop = model.counts[k] as f64 / n as f64;
        rows.push(vec![
            model.class_labels[k].clone(),
            make_class_var_name(&model.class_labels[k]),
            model.counts[k].to_string(),
            format!("{:.4}", model.counts[k] as f64),
            fmt6(prop),
        ]);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// Per-class Within-Class covariance matrices + the pooled matrix.
pub(super) fn print_covariance_matrices(
    session: &mut Session,
    var_vars: &[String],
    model: &LdaModel,
) {
    let n = model.n_total;
    let g = model.n_groups;

    // Within-Class Covariance Matrix (per class).
    centered(session, "Within-Class Covariance Matrix");
    session.listing.blank();
    for k in 0..g {
        session.listing.write_line(&format!(
            "{}    DF = {}",
            model.class_labels[k],
            model.counts[k] as i64 - 1
        ));
        session.listing.blank();
        write_matrix(session, var_vars, &model.within_cov[k]);
        session.listing.blank();
    }

    // Pooled Within-Class Covariance Matrix.
    centered(session, "Pooled Within-Class Covariance Matrix");
    session.listing.blank();
    session
        .listing
        .write_line(&format!("DF = {}", n as i64 - g as i64));
    session.listing.blank();
    write_matrix(session, var_vars, &model.pooled);
    session.listing.blank();
}

/// "Pairwise Squared Distances Between Groups" table.
pub(super) fn print_pairwise_distances(session: &mut Session, model: &LdaModel) {
    let g = model.n_groups;
    centered(session, "Pairwise Squared Distances Between Groups");
    session.listing.blank();
    let mut headers: Vec<String> = vec![String::new()];
    let mut aligns: Vec<Align> = vec![Align::Left];
    for k in 0..g {
        headers.push(model.class_labels[k].clone());
        aligns.push(Align::Right);
    }
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(g);
    for i in 0..g {
        let mut row = vec![model.class_labels[i].clone()];
        for j in 0..g {
            row.push(fmt4(model.group_distance(i, j)));
        }
        rows.push(row);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// "Linear Discriminant Function Coefficients" table.
pub(super) fn print_discrim_coefficients(
    session: &mut Session,
    var_vars: &[String],
    model: &LdaModel,
) {
    let g = model.n_groups;
    centered(session, "Linear Discriminant Function Coefficients");
    session.listing.blank();
    let mut headers: Vec<String> = vec!["Variable".into()];
    let mut aligns: Vec<Align> = vec![Align::Left];
    for k in 0..g {
        headers.push(model.class_labels[k].clone());
        aligns.push(Align::Right);
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    // Constant row.
    let mut crow = vec!["Constant".to_string()];
    for k in 0..g {
        crow.push(fmt4(model.constants[k]));
    }
    rows.push(crow);
    // One row per variable.
    for (d, vname) in var_vars.iter().enumerate() {
        let mut vrow = vec![vname.clone()];
        for k in 0..g {
            vrow.push(fmt4(model.coefs[k][d]));
        }
        rows.push(vrow);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// "Classification Results for Training Data": classify each kept observation,
/// print posteriors, and return per-class error counts.
pub(super) fn print_classification_results(
    session: &mut Session,
    ast: &DiscrimAst,
    model: &LdaModel,
    kept: &[Obs],
    id_col: &Option<Vec<Value>>,
) -> Vec<usize> {
    let g = model.n_groups;
    let n_used = kept.len();
    let mut error_count: Vec<usize> = vec![0; g];

    centered(session, "Classification Results for Training Data");
    session.listing.blank();
    let mut headers: Vec<String> = vec![
        if id_col.is_some() {
            ast.id_var.clone().unwrap()
        } else {
            "Obs".into()
        },
        "From CLASS".into(),
        "Classified Into CLASS".into(),
    ];
    let mut aligns: Vec<Align> = vec![Align::Right, Align::Left, Align::Left];
    for k in 0..g {
        headers.push(model.class_labels[k].clone());
        aligns.push(Align::Right);
    }
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(n_used);
    for (n_obs_idx, obs) in kept.iter().enumerate() {
        let from = class_index_of(&model.classes, &obs.class);
        let into = model.classify(&obs.x);
        if from != into {
            error_count[from] += 1;
        }
        let post = model.posteriors(&obs.x);
        let label = if let Some(ic) = &id_col {
            value_label(&ic[obs.orig_row])
        } else {
            (n_obs_idx + 1).to_string()
        };
        let mut row = vec![
            label,
            model.class_labels[from].clone(),
            model.class_labels[into].clone(),
        ];
        for prob in post.iter().take(g) {
            row.push(fmt4(*prob));
        }
        rows.push(row);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();

    error_count
}

/// "Error Count Estimates for Training Data" (Rate/Priors rows).
pub(super) fn print_error_estimates(
    session: &mut Session,
    model: &LdaModel,
    error_count: &[usize],
) {
    let g = model.n_groups;
    centered(session, "Error Count Estimates for Training Data");
    session.listing.blank();
    let mut headers: Vec<String> = vec![String::new()];
    let mut aligns: Vec<Align> = vec![Align::Left];
    for k in 0..g {
        headers.push(model.class_labels[k].clone());
        aligns.push(Align::Right);
    }
    headers.push("Total".into());
    aligns.push(Align::Right);

    // Rate row.
    let mut rate_row = vec!["Rate".to_string()];
    let mut total_err = 0usize;
    for (k, &errors) in error_count.iter().enumerate().take(g) {
        let rate = if model.counts[k] > 0 {
            errors as f64 / model.counts[k] as f64
        } else {
            0.0
        };
        rate_row.push(fmt4(rate));
        total_err += error_count[k];
    }
    // Total rate = Σ priors_k * rate_k (SAS weights error rates by priors).
    let total_rate: f64 = (0..g)
        .map(|k| {
            let rate = if model.counts[k] > 0 {
                error_count[k] as f64 / model.counts[k] as f64
            } else {
                0.0
            };
            model.priors[k] * rate
        })
        .sum();
    let _ = total_err;
    rate_row.push(fmt4(total_rate));
    // Priors row.
    let mut priors_row = vec!["Priors".to_string()];
    for k in 0..g {
        priors_row.push(fmt4(model.priors[k]));
    }
    priors_row.push(String::new());

    session
        .listing
        .write_table(&headers, &aligns, &[rate_row, priors_row]);
    session.listing.blank();
}
