use super::*;

/// Find a variable column index by name (case-insensitive), or error.
pub(super) fn find_var(ds: &SasDataset, name: &str) -> Result<usize> {
    ds.vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", name.to_uppercase())))
}

/// Render a category value for the listing (numeric via format_best, char as
/// the string, missing via its MissingKind display).
pub(super) fn category_label(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Char(s) => s.trim_end().to_string(),
        Value::Missing(k) => k.display(),
    }
}

/// Format a percentage to two decimals.
pub(super) fn fmt_pct(p: f64) -> String {
    format!("{p:.2}")
}

/// Format a (possibly weighted) frequency. Integral values print as plain
/// integers (so the unweighted default path stays byte-identical and integer
/// weights still look like counts); fractional weighted frequencies print with
/// the SAS default of two decimals.
pub(super) fn fmt_freq(f: f64) -> String {
    if (f - f.round()).abs() < 1e-9 {
        format!("{}", f.round() as i64)
    } else {
        format!("{f:.2}")
    }
}

/// A distinct category with its observed (possibly weighted) frequency, in
/// sas_cmp order. With no WEIGHT the frequency is an integer count stored as
/// f64; with WEIGHT it is the sum of the category's weights.
pub(super) struct Category {
    pub(super) value: Value,
    pub(super) freq: f64,
}

/// Tally the distinct values of `col` (restricted to `rows`) into categories
/// ordered by sas_cmp. When `include_missing` is false, missing values are
/// excluded (their frequency is returned separately as `n_missing`).
///
/// When `weights` is `Some`, each observation contributes its weight instead
/// of 1, applying SAS WEIGHT exclusion rules (an observation with a missing or
/// non-positive weight is dropped and counted in `n_weight_excluded`). The
/// "Frequency Missing" tally (`n_missing`) accumulates the WEIGHT of the
/// excluded observations so the weighted accounting stays consistent.
pub(super) fn tally(
    col: &[Value],
    rows: &[usize],
    include_missing: bool,
    weights: Option<&[Value]>,
) -> (Vec<Category>, f64) {
    let mut cats: Vec<Category> = Vec::new();
    let mut n_missing = 0.0_f64;
    for &i in rows {
        let v = &col[i];
        // Resolve this observation's weight (1.0 when no WEIGHT statement).
        let w = match weights {
            None => 1.0,
            Some(wc) => match value_to_num(&wc[i]) {
                Some(wf) if !wf.is_nan() && wf > 0.0 => wf,
                // Missing or non-positive weight: SAS drops the observation
                // entirely (it contributes neither to a cell nor to the
                // frequency-missing tally for the analysis variable here — but
                // a missing analysis value is still counted below).
                _ => continue,
            },
        };
        if v.is_missing() {
            n_missing += w;
            if !include_missing {
                continue;
            }
        }
        match cats
            .iter_mut()
            .find(|c| c.value.sas_cmp(v) == Ordering::Equal)
        {
            Some(c) => c.freq += w,
            None => cats.push(Category {
                value: v.clone(),
                freq: w,
            }),
        }
    }
    cats.sort_by(|a, b| a.value.sas_cmp(&b.value));
    (cats, n_missing)
}

/// Resolve this observation's weight (1.0 when no WEIGHT), applying the SAS
/// exclusion rules (missing/non-positive → None, i.e. drop the observation).
pub(super) fn obs_weight(weights: Option<&[Value]>, i: usize) -> Option<f64> {
    match weights {
        None => Some(1.0),
        Some(wc) => match value_to_num(&wc[i]) {
            Some(wf) if !wf.is_nan() && wf > 0.0 => Some(wf),
            _ => None,
        },
    }
}

/// Distinct sas_cmp-ordered values of `col` over `rows`, keeping missings only
/// when `include_missing` is set, and only for observations with a usable
/// weight.
pub(super) fn distinct_axis(
    col: &[Value],
    rows: &[usize],
    include_missing: bool,
    weights: Option<&[Value]>,
) -> Vec<Value> {
    let mut vals: Vec<Value> = Vec::new();
    for &i in rows {
        if obs_weight(weights, i).is_none() {
            continue;
        }
        let v = &col[i];
        if (include_missing || !v.is_missing())
            && !vals.iter().any(|x| x.sas_cmp(v) == Ordering::Equal)
        {
            vals.push(v.clone());
        }
    }
    vals.sort_by(|a, b| a.sas_cmp(b));
    vals
}

/// Round a weighted frequency matrix to integer counts for the integer-only
/// statistics blocks (Fisher/MEASURES/AGREE/TREND). These tests are defined on
/// counts; with integer weights the rounding is exact, with fractional weights
/// it is a documented approximation (SAS itself only supports these on
/// frequency counts). CHISQ uses the exact weighted values directly.
pub(super) fn round_matrix(freq: &[Vec<f64>]) -> Vec<Vec<usize>> {
    freq.iter()
        .map(|row| row.iter().map(|&f| f.round().max(0.0) as usize).collect())
        .collect()
}
