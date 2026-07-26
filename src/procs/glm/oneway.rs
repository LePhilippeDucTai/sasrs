use super::*;

/// One-way ANOVA statistics for a single dependent variable.
pub(super) struct OneWayStats {
    pub(super) k: usize,
    pub(super) levels: Vec<Value>,
    pub(super) groups: Vec<Vec<f64>>,
    pub(super) group_means: Vec<f64>,
    pub(super) y_bar: f64,
    pub(super) ssm: f64,
    pub(super) sse: f64,
    pub(super) sst: f64,
    pub(super) df_model: f64,
    pub(super) df_error: f64,
    pub(super) df_total: f64,
    pub(super) msm: f64,
    pub(super) mse: f64,
    pub(super) f_stat: f64,
    pub(super) p_f: Option<f64>,
    pub(super) r2: f64,
    pub(super) root_mse: f64,
    pub(super) cv: f64,
}

/// Resolve the dependent/CLASS columns, apply listwise deletion, group by
/// CLASS level, and compute the one-way ANOVA statistics.
pub(super) fn compute_oneway_stats(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    dep_var: &str,
    eff: &str,
    n_obs: usize,
) -> Result<OneWayStats> {
    // Find dependent column
    let dep_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(dep_var))
        .ok_or_else(|| {
            SasError::runtime(format!("Variable {} not found.", dep_var.to_uppercase()))
        })?;
    if ds.vars[dep_idx].ty != VarType::Num {
        return Err(SasError::runtime(format!(
            "Dependent variable {} must be numeric.",
            dep_var.to_uppercase()
        )));
    }
    let dep_col = decode_column(ds, dep_idx)?;

    let class_col_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(eff))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", eff.to_uppercase())))?;
    let class_col = decode_column(ds, class_col_idx)?;

    // Listwise deletion
    let mut usable_rows: Vec<usize> = Vec::new();
    for i in 0..n_obs {
        let dep_ok = match value_to_num(&dep_col[i]) {
            Some(v) if !v.is_nan() => true,
            _ => false,
        };
        let cls_ok = !class_col[i].is_missing();
        if dep_ok && cls_ok {
            usable_rows.push(i);
        }
    }
    let n = usable_rows.len();

    // Group by CLASS levels (sorted by sas_cmp)
    let mut levels: Vec<Value> = Vec::new();
    for &r in &usable_rows {
        let v = &class_col[r];
        if !levels
            .iter()
            .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
        {
            levels.push(v.clone());
        }
    }
    levels.sort_by(|a, b| a.sas_cmp(b));
    let k = levels.len();

    // Collect values per group
    let mut groups: Vec<Vec<f64>> = vec![Vec::new(); k];
    for &r in &usable_rows {
        let v = &class_col[r];
        let yi = value_to_num(&dep_col[r]).unwrap();
        let gi = levels
            .iter()
            .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            .unwrap();
        groups[gi].push(yi);
    }

    // Compute statistics
    let y_bar = if n > 0 {
        groups.iter().flat_map(|g| g.iter()).sum::<f64>() / n as f64
    } else {
        f64::NAN
    };

    let mut ssm = 0.0_f64;
    let mut sse = 0.0_f64;
    let mut group_means: Vec<f64> = Vec::with_capacity(k);
    for g in &groups {
        let ni = g.len();
        let y_bar_i = if ni > 0 {
            g.iter().sum::<f64>() / ni as f64
        } else {
            f64::NAN
        };
        group_means.push(y_bar_i);
        ssm += ni as f64 * (y_bar_i - y_bar).powi(2);
        sse += g.iter().map(|&y| (y - y_bar_i).powi(2)).sum::<f64>();
    }
    let sst = ssm + sse;

    let df_model = (k as f64 - 1.0).max(0.0);
    let df_error = (n as f64 - k as f64).max(0.0);
    let df_total = (n as f64 - 1.0).max(0.0);

    let msm = if df_model > 0.0 {
        ssm / df_model
    } else {
        f64::NAN
    };
    let mse = if df_error > 0.0 {
        sse / df_error
    } else {
        f64::NAN
    };
    let f_stat = if mse > 0.0 && !mse.is_nan() {
        msm / mse
    } else {
        f64::NAN
    };
    let p_f = if f_stat.is_nan() {
        None
    } else {
        Some((1.0 - f_cdf(f_stat, df_model, df_error)).clamp(0.0, 1.0))
    };

    let r2 = if sst > 0.0 { ssm / sst } else { f64::NAN };
    let root_mse = if !mse.is_nan() { mse.sqrt() } else { f64::NAN };
    let cv = if y_bar.abs() > 1e-15 && !root_mse.is_nan() {
        root_mse / y_bar.abs() * 100.0
    } else {
        f64::NAN
    };

    session
        .log
        .note(&format!("There were {} observations used.", n));

    Ok(OneWayStats {
        k,
        levels,
        groups,
        group_means,
        y_bar,
        ssm,
        sse,
        sst,
        df_model,
        df_error,
        df_total,
        msm,
        mse,
        f_stat,
        p_f,
        r2,
        root_mse,
        cv,
    })
}
