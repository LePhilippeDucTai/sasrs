use super::*;

// ───────────────────────── CLASS design terms ─────────────────────────

/// One term on the MODEL right-hand side, expanded into design columns.
///
/// PARAM = reference coding with ref = LAST level (matches SAS GENMOD default
/// `PARAM=GLM`-style ordering with the last level as the reference cell). A
/// CLASS factor with L distinct non-missing levels (in `Value::sas_cmp` order)
/// contributes L−1 design columns; the last level is the dropped reference.
pub(super) enum DesignTerm {
    /// Continuous predictor → a single design column carrying the numeric value.
    Continuous { name: String, col: usize },
    /// CLASS predictor → L−1 indicator columns (reference = last level).
    Class {
        name: String,
        col: usize,
        /// Distinct non-missing levels, `sas_cmp` order. Reference = last.
        levels: Vec<Value>,
    },
}

impl DesignTerm {
    /// Number of design columns this term contributes.
    pub(super) fn n_cols(&self) -> usize {
        match self {
            DesignTerm::Continuous { .. } => 1,
            DesignTerm::Class { levels, .. } => levels.len().saturating_sub(1),
        }
    }
}

/// Level label for a CLASS value, matching the Class Level Information scheme.
pub(super) fn class_level_label(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim_end().to_string(),
        Value::Num(f) => format!("{f}"),
        Value::Missing(k) => k.display(),
    }
}

// ───────────────────────── Execute helpers ─────────────────────────

/// Build the design terms for the MODEL right-hand side.
///
/// Each predictor is either continuous (1 column) or a CLASS factor (L−1
/// indicator columns). CLASS levels are the distinct non-missing values in
/// `Value::sas_cmp` order; the last level is the reference cell.
pub(super) fn build_design_terms(
    ast: &GenmodAst,
    ds: &crate::dataset::SasDataset,
    predictors: &[String],
    pred_idxs: &[usize],
    pred_cols: &[Vec<Value>],
) -> Result<Vec<DesignTerm>> {
    let nb_preds = predictors.len();
    let mut design_terms: Vec<DesignTerm> = Vec::with_capacity(nb_preds);
    for (pi, nm) in predictors.iter().enumerate() {
        let is_class = ast.class_vars.iter().any(|c| c.eq_ignore_ascii_case(nm));
        if is_class {
            let col = &pred_cols[pi];
            let levels = crate::procs::lincom::class_levels(col.iter());
            if levels.len() < 2 {
                return Err(SasError::runtime(format!(
                    "CLASS variable {} must have at least 2 levels.",
                    nm.to_uppercase()
                )));
            }
            design_terms.push(DesignTerm::Class {
                name: ds.vars[pred_idxs[pi]].name.clone(),
                col: pi,
                levels,
            });
        } else {
            design_terms.push(DesignTerm::Continuous {
                name: ds.vars[pred_idxs[pi]].name.clone(),
                col: pi,
            });
        }
    }
    Ok(design_terms)
}

/// Binomial response information: event level plus the Response Profile
/// labels and totals. All fields are `None`/0 for non-binomial distributions.
pub(super) struct BinomialResponse {
    pub(super) event_level: Option<Value>,
    pub(super) event_label: Option<String>,
    pub(super) nonevent_label: Option<String>,
    pub(super) n_event_total: f64,
    pub(super) n_nonevent_total: f64,
}

/// Determine the Binomial event level (EVENT= / DESCENDING / default) and
/// pre-compute the event/non-event totals for the Response Profile.
///
/// For non-binomial, collect distinct levels for reference but encode y
/// numerically. For Binomial, reproduce LOGISTIC's level-determination.
pub(super) fn prepare_binomial_response(
    model: &GenmodModel,
    resp_col: &[Value],
    freq_col: &Option<Vec<Value>>,
    n_read: usize,
    resp_name: &str,
) -> Result<BinomialResponse> {
    let dist = &model.dist;
    let binomial_event_level: Option<Value>;
    let binomial_event_label: Option<String>;
    let binomial_nonevent_label: Option<String>;
    let binomial_n_event_total: f64;
    let binomial_n_nonevent_total: f64;

    if *dist == Distribution::Binomial {
        // Collect distinct non-missing levels
        let levels = crate::procs::lincom::class_levels(resp_col.iter().take(n_read));

        if levels.len() != 2 {
            return Err(SasError::runtime(format!(
                "Response variable {} must have exactly 2 non-missing levels for DIST=BINOMIAL (found {}).",
                resp_name.to_uppercase(),
                levels.len()
            )));
        }

        // Determine event level
        let event_level_ref: &Value = if let Some(ev_str) = &model.event {
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
            &levels[1] // max level
        } else {
            &levels[0] // min level (default)
        };

        let el = value_label(event_level_ref);
        let nel = value_label(if std::ptr::eq(event_level_ref, &levels[0]) {
            &levels[1]
        } else {
            &levels[0]
        });

        // Pre-compute event/nonevent totals for Response Profile
        let mut ev_total = 0.0_f64;
        let mut nev_total = 0.0_f64;
        for i in 0..n_read {
            if resp_col[i].is_missing() {
                continue;
            }
            let w = if let Some(fc) = &freq_col {
                match value_to_num(&fc[i]) {
                    Some(f) if !f.is_nan() && f > 0.0 => f,
                    _ => continue,
                }
            } else {
                1.0
            };
            if resp_col[i].sas_cmp(event_level_ref) == std::cmp::Ordering::Equal {
                ev_total += w;
            } else {
                nev_total += w;
            }
        }

        binomial_event_level = Some(event_level_ref.clone());
        binomial_event_label = Some(el);
        binomial_nonevent_label = Some(nel);
        binomial_n_event_total = ev_total;
        binomial_n_nonevent_total = nev_total;
    } else {
        binomial_event_level = None;
        binomial_event_label = None;
        binomial_nonevent_label = None;
        binomial_n_event_total = 0.0;
        binomial_n_nonevent_total = 0.0;
    }

    Ok(BinomialResponse {
        event_level: binomial_event_level,
        event_label: binomial_event_label,
        nonevent_label: binomial_nonevent_label,
        n_event_total: binomial_n_event_total,
        n_nonevent_total: binomial_n_nonevent_total,
    })
}

/// Listwise deletion + encoding: build the response vector, the design matrix
/// (with leading intercept column) and the frequency vector.
pub(super) fn build_model_matrices(
    design_terms: &[DesignTerm],
    pred_cols: &[Vec<Value>],
    resp_col: &[Value],
    freq_col: &Option<Vec<Value>>,
    dist: &Distribution,
    binomial_event_level: &Option<Value>,
    n_read: usize,
) -> (Vec<f64>, Vec<Vec<f64>>, Vec<f64>) {
    let mut y_vec: Vec<f64> = Vec::new();
    let mut x_mat: Vec<Vec<f64>> = Vec::new();
    let mut freq_vec: Vec<f64> = Vec::new();

    for i in 0..n_read {
        if resp_col[i].is_missing() {
            continue;
        }

        let w = if let Some(fc) = &freq_col {
            match value_to_num(&fc[i]) {
                Some(f) if !f.is_nan() && f > 0.0 => f,
                _ => continue,
            }
        } else {
            1.0
        };

        let mut row = vec![1.0_f64]; // intercept
        let mut ok = true;
        for term in design_terms {
            match term {
                DesignTerm::Continuous { col, .. } => match value_to_num(&pred_cols[*col][i]) {
                    Some(v) if !v.is_nan() => row.push(v),
                    _ => {
                        ok = false;
                        break;
                    }
                },
                DesignTerm::Class { col, levels, .. } => {
                    let v = &pred_cols[*col][i];
                    if v.is_missing() {
                        ok = false;
                        break;
                    }
                    // Reference-cell dummies: 1 for the matching non-reference
                    // level, 0 elsewhere (reference = last level → all zeros).
                    let nref = levels.len() - 1;
                    let pos = levels
                        .iter()
                        .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal);
                    match pos {
                        Some(li) => {
                            for k in 0..nref {
                                row.push(if k == li { 1.0 } else { 0.0 });
                            }
                        }
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
            }
        }
        if !ok {
            continue;
        }

        // Encode response
        let yi: f64 = if *dist == Distribution::Binomial {
            let ev_level = binomial_event_level.as_ref().unwrap();
            if resp_col[i].sas_cmp(ev_level) == std::cmp::Ordering::Equal {
                1.0
            } else {
                0.0
            }
        } else {
            // Numeric response for Poisson and Normal
            match value_to_num(&resp_col[i]) {
                Some(v) if !v.is_nan() => v,
                _ => continue,
            }
        };

        y_vec.push(yi);
        x_mat.push(row);
        freq_vec.push(w);
    }

    (y_vec, x_mat, freq_vec)
}
