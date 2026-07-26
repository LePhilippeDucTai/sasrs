use super::*;

/// Extract a sub-matrix (rows `r`, cols `c` from index 1 onwards).
pub(super) fn submatrix_predictors(mat: &[Vec<f64>], p: usize) -> Vec<Vec<f64>> {
    (1..=p)
        .map(|i| (1..=p).map(|j| mat[i][j]).collect())
        .collect()
}

// ───────────────────────── Design matrix ─────────────────────────

/// Metadata for one MODEL effect (a continuous predictor or a CLASS variable).
/// CLASS variables expand to `levels.len() - 1` reference-cell design columns,
/// with the LAST level (in `sas_cmp` order) as the reference (PARAM=REF, REF=LAST).
pub(super) struct Effect {
    /// Predictor name as written in MODEL.
    pub(super) name: String,
    /// Index into the decoded `pred_cols` (== predictor position in MODEL).
    pub(super) pred_col_idx: usize,
    /// `true` if this effect is a CLASS variable.
    pub(super) is_class: bool,
    /// Non-reference levels (one design column each), in `sas_cmp` order.
    /// Empty for continuous effects.
    pub(super) levels: Vec<Value>,
    /// Reference level label (CLASS only).
    pub(super) ref_label: String,
}

/// The expanded design: parameter labels (one per non-intercept column, index 0
/// = first non-intercept column) and the list of effects in MODEL order.
pub(super) struct Design {
    pub(super) effects: Vec<Effect>,
    /// Label for each non-intercept design column.
    pub(super) col_labels: Vec<String>,
}

impl Design {
    pub(super) fn n_cols(&self) -> usize {
        self.col_labels.len()
    }
}

// ───────────────────────── Execute ─────────────────────────

// ───────────────────────── Execute helpers ─────────────────────────

/// Build the CLASS-expanded design (reference-cell coding).
///
/// PARAM=REF, REF=LAST: a CLASS var with L levels (sas_cmp order) adds L−1
/// design columns, one per non-reference level; the LAST level is reference.
/// (SAS default is EFFECT coding — documented deviation; matches PARAM=REF.)
pub(super) fn build_design(
    class_vars: &[String],
    predictors: &[String],
    pred_cols: &[Vec<Value>],
    n_read: usize,
) -> Result<Design> {
    let nb_preds = predictors.len();
    let class_set: Vec<String> = class_vars.to_vec();
    let is_class_var = |nm: &str| class_set.iter().any(|c| c.eq_ignore_ascii_case(nm));

    let mut effects: Vec<Effect> = Vec::with_capacity(nb_preds);
    let mut col_labels: Vec<String> = Vec::new();
    for (pi, nm) in predictors.iter().enumerate() {
        if is_class_var(nm) {
            // Collect distinct non-missing levels of this CLASS column.
            let col = &pred_cols[pi];
            let levs = crate::procs::lincom::class_levels(col.iter().take(n_read));
            if levs.len() < 2 {
                return Err(SasError::runtime(format!(
                    "CLASS variable {} must have at least 2 levels.",
                    nm.to_uppercase()
                )));
            }
            let ref_label = value_label(&levs[levs.len() - 1]);
            let non_ref: Vec<Value> = levs[..levs.len() - 1].to_vec();
            for lv in &non_ref {
                col_labels.push(format!("{} {}", nm, value_label(lv)));
            }
            effects.push(Effect {
                name: nm.clone(),
                pred_col_idx: pi,
                is_class: true,
                levels: non_ref,
                ref_label,
            });
        } else {
            col_labels.push(nm.clone());
            effects.push(Effect {
                name: nm.clone(),
                pred_col_idx: pi,
                is_class: false,
                levels: Vec::new(),
                ref_label: String::new(),
            });
        }
    }
    Ok(Design {
        effects,
        col_labels,
    })
}

/// Determine the binary event level (EVENT= / DESCENDING / default).
/// Returns `(event_level, event_label, nonevent_label)`.
pub(super) fn determine_event_level(
    model: &LogisticModel,
    levels: &[Value],
    resp_name: &str,
) -> Result<(Value, String, String)> {
    let event_level: &Value = if let Some(ev_str) = &model.event {
        levels
            .iter()
            .find(|lv| value_matches_event(lv, ev_str))
            .ok_or_else(|| {
                SasError::runtime(format!(
                    "Event value '{}' not found in response variable {}.",
                    ev_str,
                    resp_name.to_uppercase()
                ))
            })?
    } else if model.descending {
        &levels[1]
    } else {
        &levels[0]
    };

    let event_label = value_label(event_level);
    let nonevent_level: &Value = if std::ptr::eq(event_level, &levels[0]) {
        &levels[1]
    } else {
        &levels[0]
    };
    let nonevent_label = value_label(nonevent_level);

    Ok((event_level.clone(), event_label, nonevent_label))
}

/// Listwise deletion + encoding: build y (1=event), X (leading intercept
/// column) and the frequency vector. `complete_mask[i]` marks rows used in
/// the fit (for OUTPUT OUT=).
pub(super) fn build_model_matrices(
    design: &Design,
    pred_cols: &[Vec<Value>],
    resp_col: &[Value],
    freq_col: &Option<Vec<Value>>,
    event_level: &Value,
    n_read: usize,
) -> (Vec<f64>, Vec<Vec<f64>>, Vec<f64>, Vec<bool>) {
    let mut y_vec: Vec<f64> = Vec::new();
    let mut x_mat: Vec<Vec<f64>> = Vec::new();
    let mut freq_vec: Vec<f64> = Vec::new();
    let mut complete_mask: Vec<bool> = vec![false; n_read];

    for i in 0..n_read {
        // Skip if response is missing
        if resp_col[i].is_missing() {
            continue;
        }

        // Check freq
        let w = if let Some(fc) = &freq_col {
            match value_to_num(&fc[i]) {
                Some(f) if !f.is_nan() && f > 0.0 => f,
                _ => continue,
            }
        } else {
            1.0
        };

        // Build design row: intercept then expanded effect columns.
        let mut row = vec![1.0_f64]; // intercept
        let mut ok = true;
        for eff in &design.effects {
            let col = &pred_cols[eff.pred_col_idx];
            if eff.is_class {
                // Reference-cell dummies for the current level.
                let v = &col[i];
                if v.is_missing() {
                    ok = false;
                    break;
                }
                for lv in &eff.levels {
                    row.push(if v.sas_cmp(lv) == std::cmp::Ordering::Equal {
                        1.0
                    } else {
                        0.0
                    });
                }
            } else {
                match value_to_num(&col[i]) {
                    Some(v) if !v.is_nan() => row.push(v),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if !ok {
            continue;
        }

        // Encode response: 1.0 if event, 0.0 otherwise
        let yi = if resp_col[i].sas_cmp(event_level) == std::cmp::Ordering::Equal {
            1.0
        } else {
            0.0
        };

        y_vec.push(yi);
        x_mat.push(row);
        freq_vec.push(w);
        complete_mask[i] = true;
    }

    (y_vec, x_mat, freq_vec, complete_mask)
}
