use super::*;

/// Find CLASS/VAR/ID column indices and decode the columns.
pub(super) fn resolve_and_decode(
    ds: &crate::dataset::SasDataset,
    ast: &DiscrimAst,
    class_name: &str,
) -> Result<(Vec<Value>, Vec<Vec<Value>>, Option<Vec<Value>>)> {
    let p = ast.var_vars.len();

    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
    };

    let class_idx = find_col(class_name)?;
    let mut var_idxs: Vec<usize> = Vec::with_capacity(p);
    for nm in &ast.var_vars {
        var_idxs.push(find_col(nm)?);
    }
    let id_idx: Option<usize> = match &ast.id_var {
        Some(nm) => Some(find_col(nm)?),
        None => None,
    };

    let class_col = decode_column(ds, class_idx)?;
    let mut var_cols: Vec<Vec<Value>> = Vec::with_capacity(p);
    for &idx in &var_idxs {
        var_cols.push(decode_column(ds, idx)?);
    }
    let id_col: Option<Vec<Value>> = match id_idx {
        Some(i) => Some(decode_column(ds, i)?),
        None => None,
    };
    Ok((class_col, var_cols, id_col))
}

/// Complete observations + sorted class list. SAS sorts classes by formatted
/// value; sas_cmp ordering gives a deterministic class order. Errors if fewer
/// than 2 classes remain.
pub(super) fn collect_complete_obs(
    class_col: &[Value],
    var_cols: &[Vec<Value>],
    n_read: usize,
    p: usize,
) -> Result<(Vec<Value>, Vec<Obs>)> {
    let mut classes: Vec<Value> = Vec::new();
    let mut kept: Vec<Obs> = Vec::new();

    for i in 0..n_read {
        if class_col[i].is_missing() {
            continue;
        }
        let mut x = Vec::with_capacity(p);
        let mut ok = true;
        for vc in var_cols {
            match value_to_num(&vc[i]) {
                Some(v) if !v.is_nan() => x.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let cv = class_col[i].clone();
        if !classes
            .iter()
            .any(|c| c.sas_cmp(&cv) == std::cmp::Ordering::Equal)
        {
            classes.push(cv.clone());
        }
        kept.push(Obs {
            orig_row: i,
            class: cv,
            x,
        });
    }

    classes.sort_by(|a, b| a.sas_cmp(b));

    if classes.len() < 2 {
        return Err(SasError::runtime(
            "PROC DISCRIM requires at least 2 classes with complete observations.",
        ));
    }
    Ok((classes, kept))
}
