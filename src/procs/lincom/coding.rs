use super::*;

/// Reference-cell coding layout of a fitted multiway GLM design.
///
/// This is exactly the information `lsmean_coef_vector` needed: the factor level
/// sets (reference = last), the per-term parent-factor indices, the per-term
/// column specs (each column = product of parent `(factor_idx, dummy_idx)`
/// pairs) and the total design column count (intercept + term columns).
#[derive(Debug, Clone)]
pub struct Coding {
    /// Per factor: `(name, levels)` — distinct non-missing levels in `sas_cmp`
    /// order; reference cell = LAST.
    pub factors: Vec<(String, Vec<Value>)>,
    /// Per effect term: indices into `factors` of the parent factors.
    pub term_factor_idxs: Vec<Vec<usize>>,
    /// Per effect term: list of column specs; each spec is the list of parent
    /// `(factor_idx, dummy_idx)` pairs whose product forms that design column.
    pub col_specs: Vec<Vec<Vec<(usize, usize)>>>,
    /// Total number of design columns (intercept + all term columns).
    pub ncols: usize,
}

impl Coding {
    /// Number of dummy columns a factor contributes (levels − 1, last dropped).
    pub(super) fn n_dummies(&self, fi: usize) -> usize {
        self.factors[fi].1.len().saturating_sub(1)
    }
}

// ───────────────────────── CLASS variable coding ─────────────────────────

/// SAS CLASS-variable parameterization (`PARAM=` option).
///
/// Selects how the levels of a CLASS variable are expanded into design
/// (indicator) columns. The reference cell, where applicable, is the **last**
/// level in `sas_cmp` order (SAS default), matching the existing reference-cell
/// coding used by `mixed`/`glimmix`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    /// `PARAM=REFERENCE` (SAS default for many procs). `L−1` columns of 0/1; the
    /// indicator of level `i` (`i < L−1`); the reference (last) level is all 0.
    Ref,
    /// `PARAM=EFFECT`. `L−1` columns; level `i` (`i < L−1`) → its own 0/1
    /// indicator; the reference (last) level → `−1` in every column, so each
    /// column sums to 0 over the levels.
    Effect,
    /// `PARAM=GLM`. `L` columns (over-parameterized): one 0/1 indicator per
    /// level, no reference dropped. Each level's row has exactly one `1`.
    Glm,
    /// `PARAM=POLY`. `L−1` columns of orthogonal polynomials of degrees `1..=L−1`
    /// evaluated on the equally-spaced integer scores `1, 2, …, L` of the levels
    /// (in `sas_cmp` order). Column 0 is the linear trend; columns are pairwise
    /// orthogonal. (See [`poly_coding`] for the exact normalization.)
    Poly,
}

/// Build the CLASS-variable coding matrix for `levels`, parameterized by `param`.
///
/// **Precondition:** `levels` must already be the distinct levels of the CLASS
/// variable in `sas_cmp` order (deduped + sorted by the caller). This function
/// does NOT reorder — for [`Param::Ref`] / [`Param::Effect`] the reference cell
/// is taken to be the LAST element. Ordering is the caller's responsibility
/// (reuse `Value::sas_cmp`, never raw string compares).
///
/// Returns one row per level (same order as `levels`); each row is that level's
/// coding (indicator/contrast) values. The number of columns depends on `param`:
/// `Ref`/`Effect`/`Poly` → `L−1`; `Glm` → `L` (where `L = levels.len()`). With
/// `L == 0` the result is empty; with `L == 1`, `Ref`/`Effect`/`Poly` yield one
/// empty row and `Glm` yields `[[1.0]]`.
pub fn class_coding(levels: &[Value], param: Param) -> Vec<Vec<f64>> {
    let l = levels.len();
    match param {
        Param::Ref => {
            // L−1 columns; row i = unit vector e_i for i<L−1, reference (last) = 0.
            let ncol = l.saturating_sub(1);
            (0..l)
                .map(|i| {
                    let mut row = vec![0.0; ncol];
                    if i < ncol {
                        row[i] = 1.0;
                    }
                    row
                })
                .collect()
        }
        Param::Effect => {
            // L−1 columns; row i = e_i for i<L−1, reference (last) = all −1.
            let ncol = l.saturating_sub(1);
            (0..l)
                .map(|i| {
                    if i < ncol {
                        let mut row = vec![0.0; ncol];
                        row[i] = 1.0;
                        row
                    } else {
                        vec![-1.0; ncol]
                    }
                })
                .collect()
        }
        Param::Glm => {
            // L columns; row i = unit vector e_i (one indicator per level).
            (0..l)
                .map(|i| {
                    let mut row = vec![0.0; l];
                    row[i] = 1.0;
                    row
                })
                .collect()
        }
        Param::Poly => poly_coding(l),
    }
}

/// Collect the distinct non-missing CLASS levels of a column, in `sas_cmp` order.
///
/// This is the canonical SAS level-collection idiom shared by the modeling
/// procs (GENMOD/LOGISTIC/ANOVA/…): walk the values in row order, skip missing
/// values ([`Value::is_missing`] — numeric missings and blank character values),
/// dedup by `sas_cmp == Equal` (first occurrence kept), then sort by `sas_cmp`.
/// The result is ready to feed [`class_coding`] (reference cell = LAST level).
///
/// Takes any iterator of `&Value` so call sites can pre-filter rows (e.g.
/// `col.iter().take(n_read)` or an iteration over usable-row indices).
pub fn class_levels<'a, I>(vals: I) -> Vec<Value>
where
    I: IntoIterator<Item = &'a Value>,
{
    let mut levels: Vec<Value> = Vec::new();
    for v in vals {
        if v.is_missing() {
            continue;
        }
        if !levels
            .iter()
            .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
        {
            levels.push(v.clone());
        }
    }
    levels.sort_by(|a, b| a.sas_cmp(b));
    levels
}

/// A fixed-effects design column together with its parameter label.
pub struct DesignColumn {
    pub label: String,
    pub values: Vec<f64>,
}

/// Build the fixed-effects design matrix from the MODEL effects (shared by
/// PROC MIXED and PROC GLIMMIX).
///
/// Columns (in order): intercept (unless NOINT), then for each MODEL effect a
/// continuous column (if the variable is not in CLASS) or reference-cell coded
/// indicator columns (L−1, last level = reference per `sas_cmp` order) for a
/// CLASS variable. Returns the design columns (each with its parameter label).
/// Continuous values come pre-extracted as f64 (already validated as
/// non-missing by the caller's listwise deletion).
pub fn build_design(
    cols: &[(String, Vec<Value>)],
    class_vars: &[String],
    fixed: &[String],
    noint: bool,
    n: usize,
) -> Result<Vec<DesignColumn>> {
    let mut design: Vec<DesignColumn> = Vec::new();
    if !noint {
        design.push(DesignColumn {
            label: "Intercept".to_string(),
            values: vec![1.0; n],
        });
    }

    let find = |nm: &str| -> Option<&(String, Vec<Value>)> {
        cols.iter().find(|(name, _)| name.eq_ignore_ascii_case(nm))
    };
    let is_class = |nm: &str| class_vars.iter().any(|c| c.eq_ignore_ascii_case(nm));

    for eff in fixed {
        let col = find(eff).ok_or_else(|| {
            SasError::runtime(format!("Variable {} not found.", eff.to_uppercase()))
        })?;
        if is_class(eff) {
            // Reference-cell coding: levels sorted by sas_cmp, last is reference.
            // NOTE: unlike `class_levels`, missing values are NOT skipped here —
            // the caller's listwise deletion guarantees none remain, and the
            // historical (byte-identity) code path did not filter them.
            let mut levels: Vec<Value> = Vec::new();
            for v in &col.1 {
                if !levels
                    .iter()
                    .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
                {
                    levels.push(v.clone());
                }
            }
            levels.sort_by(|a, b| a.sas_cmp(b));
            // PARAM=REFERENCE coding (last level = reference, dropped).
            // `coding[li]` is the coding row of level `li`; column `j` is the
            // indicator of `levels[j]` (j < L−1), so for a data value `v` whose
            // level index is `li`, the column-`j` value is `coding[li][j]`.
            let coding = class_coding(&levels, Param::Ref);
            for (j, lvl) in levels.iter().take(levels.len().saturating_sub(1)).enumerate() {
                let label = format!("{} {}", eff, value_label(lvl));
                let values: Vec<f64> = col
                    .1
                    .iter()
                    .map(|v| {
                        let li = levels
                            .iter()
                            .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
                            .expect("data value must match a deduped level");
                        coding[li][j]
                    })
                    .collect();
                design.push(DesignColumn { label, values });
            }
        } else {
            // Continuous column.
            let values: Vec<f64> = col
                .1
                .iter()
                .map(|v| match v {
                    Value::Num(f) => *f,
                    _ => f64::NAN,
                })
                .collect();
            design.push(DesignColumn {
                label: eff.clone(),
                values,
            });
        }
    }
    Ok(design)
}

/// Orthogonal-polynomial coding on equally-spaced integer scores `1..=L`.
///
/// Produces `L` rows (one per level, in score order `1, 2, …, L`) of `L−1`
/// columns; column `d−1` holds the degree-`d` orthogonal polynomial
/// (`d = 1..=L−1`) evaluated at the scores, built by Gram–Schmidt of the power
/// basis `{1, x, x², …}` against the constant. Each contrast vector is scaled to
/// unit length (Euclidean norm 1), so columns are orthonormal; column 0 is the
/// monotone linear trend. (`L ≤ 1` → no contrast columns.)
pub(super) fn poly_coding(l: usize) -> Vec<Vec<f64>> {
    if l == 0 {
        return Vec::new();
    }
    let ncol = l - 1;
    // Scores x_k = k+1 (1-based), as f64.
    let x: Vec<f64> = (0..l).map(|k| (k + 1) as f64).collect();
    // Orthogonal basis vectors, starting with the constant (degree 0), which we
    // keep only to orthogonalize against (it is NOT emitted as a column).
    let mut basis: Vec<Vec<f64>> = Vec::with_capacity(ncol + 1);
    basis.push(vec![1.0; l]); // degree 0 (constant)
    for deg in 1..=ncol {
        // Raw power vector x^deg.
        let mut v: Vec<f64> = x.iter().map(|&xi| xi.powi(deg as i32)).collect();
        // Gram–Schmidt against all previous (orthogonal) basis vectors.
        for u in &basis {
            let uu: f64 = u.iter().map(|&a| a * a).sum();
            if uu == 0.0 {
                continue;
            }
            let uv: f64 = u.iter().zip(v.iter()).map(|(&a, &b)| a * b).sum();
            let coef = uv / uu;
            for (vi, &ui) in v.iter_mut().zip(u.iter()) {
                *vi -= coef * ui;
            }
        }
        // Normalize to unit length.
        let norm: f64 = v.iter().map(|&a| a * a).sum::<f64>().sqrt();
        if norm > 0.0 {
            for vi in v.iter_mut() {
                *vi /= norm;
            }
        }
        basis.push(v);
    }
    // Emit rows: row k = (col_1[k], …, col_{L-1}[k]); skip basis[0] (constant).
    (0..l)
        .map(|k| basis[1..].iter().map(|col| col[k]).collect())
        .collect()
}
