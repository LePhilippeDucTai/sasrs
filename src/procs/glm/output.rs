use super::*;
use crate::dataset::SasDataset;
use polars::prelude::*;

/// J02-P2 (issue #17) — parsed OUTPUT statement of PROC GLM.
/// `output out=<ds> p=<name> r=<name>;` — OUT= required, at least one
/// statistic keyword required. Only PREDICTED (P=) and RESIDUAL (R=) are
/// honored; other keywords are rejected explicitly at parse time.
#[derive(Debug, Clone)]
pub struct GlmOutput {
    /// OUT= dataset reference (required).
    pub out: DatasetRef,
    /// P= / PREDICTED= output column name.
    pub p: Option<String>,
    /// R= / RESIDUAL= output column name.
    pub r: Option<String>,
}

/// J02-P2 (issue #17) — build and write the OUT= dataset of the OUTPUT
/// statement for the one-way design: every input variable is carried over,
/// plus one column per requested statistic. PREDICTED = fitted value
/// (group mean of the observation's CLASS level), RESIDUAL = dependent −
/// fitted. Observations excluded from the analysis (missing dependent or
/// missing CLASS value) keep all input variables and get SAS-missing
/// statistics, following the SAS/STAT GLM « OUTPUT Statement » semantics
/// (the fitted model is the one-way ANOVA of `stats`).
pub(super) fn write_output(
    session: &mut Session,
    ds: &SasDataset,
    dep_var: &str,
    eff: &str,
    stats: &OneWayStats,
    oreq: &GlmOutput,
) -> Result<()> {
    let dep_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(dep_var))
        .ok_or_else(|| {
            SasError::runtime(format!("Variable {} not found.", dep_var.to_uppercase()))
        })?;
    let dep_col = decode_column(ds, dep_idx)?;

    let class_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(eff))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", eff.to_uppercase())))?;
    let class_col = decode_column(ds, class_idx)?;

    // Per observation: fitted value (group mean) and residual, SAS-missing
    // for the observations excluded from the analysis.
    let fitted: Vec<Option<f64>> = (0..ds.n_obs())
        .map(|i| {
            if class_col[i].is_missing() {
                return None;
            }
            let g = stats
                .levels
                .iter()
                .position(|l| l.sas_cmp(&class_col[i]) == std::cmp::Ordering::Equal)?;
            Some(stats.group_means[g])
        })
        .collect();
    let resid: Vec<Option<f64>> = fitted
        .iter()
        .zip(dep_col.iter())
        .map(|(f, y)| match (f, value_to_num(y)) {
            (Some(f), Some(v)) if !v.is_nan() => Some(v - f),
            _ => None,
        })
        .collect();

    // Input variables carried over, then the requested statistic columns in
    // keyword order (P first, R second), like the SAS OUT= dataset layout.
    let mut df = ds.df.clone();
    let mut vars = ds.vars.clone();
    if let Some(name) = &oreq.p {
        df.with_column(Series::new(name.as_str().into(), fitted.clone()))
            .map_err(|e| SasError::runtime(format!("OUTPUT OUT= P=: {e}")))?;
        vars.push(num_var_meta(name));
    }
    if let Some(name) = &oreq.r {
        df.with_column(Series::new(name.as_str().into(), resid))
            .map_err(|e| SasError::runtime(format!("OUTPUT OUT= R=: {e}")))?;
        vars.push(num_var_meta(name));
    }
    let out_ds = SasDataset { df, vars };

    let out_libref = oreq.out.libref_or_work();
    let out_table = oreq.out.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());

    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));

    Ok(())
}
