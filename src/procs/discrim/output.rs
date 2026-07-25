use super::*;

/// SAS shows the class level value as the "Variable" column in Class Level
/// Information (a valid SAS name derived from the formatted value).
pub(super) fn make_class_var_name(label: &str) -> String {
    // SAS builds a name like `_A` for value "A". For numeric / messy values it
    // prefixes with an underscore. Keep it simple and SAS-like.
    let trimmed = label.trim();
    if trimmed.is_empty() {
        "_".to_string()
    } else if trimmed.chars().next().map(|c| c.is_alphabetic()).unwrap_or(false) {
        trimmed.to_string()
    } else {
        format!("_{trimmed}")
    }
}

/// Print a labeled square matrix with variable names on rows and columns.
pub(super) fn write_matrix(session: &mut Session, var_names: &[String], mat: &[Vec<f64>]) {
    let p = var_names.len();
    let mut headers: Vec<String> = vec![String::new()];
    let mut aligns: Vec<Align> = vec![Align::Left];
    for nm in var_names {
        headers.push(nm.clone());
        aligns.push(Align::Right);
    }
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(p);
    for i in 0..p {
        let mut row = vec![var_names[i].clone()];
        for j in 0..p {
            row.push(fmt4(mat[i][j]));
        }
        rows.push(row);
    }
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Build and write the OUT= dataset: input columns + `_FROM_`, `_INTO_`,
/// and one `_<k>` posterior column per class.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_out_dataset(
    _ast: &DiscrimAst,
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    model: &LdaModel,
    var_cols: &[Vec<Value>],
    class_col: &[Value],
    out_ref: &DatasetRef,
    n_read: usize,
) -> Result<()> {
    use crate::dataset::{SasDataset, VarMeta};
    use crate::value::VarType;
    use polars::prelude::*;

    let p = model.p;
    let g = model.n_groups;

    let mut from_vals: Vec<Option<String>> = Vec::with_capacity(n_read);
    let mut into_vals: Vec<Option<String>> = Vec::with_capacity(n_read);
    let mut post_cols: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(n_read); g];

    for i in 0..n_read {
        // Build x; if any var missing or class missing, row is not classified.
        let mut x = Vec::with_capacity(p);
        let mut ok = !class_col[i].is_missing();
        for vc in var_cols {
            match value_to_num(&vc[i]) {
                Some(v) if !v.is_nan() => x.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            let into = model.classify(&x);
            let post = model.posteriors(&x);
            from_vals.push(Some(value_label(&class_col[i])));
            into_vals.push(Some(model.class_labels[into].clone()));
            for k in 0..g {
                post_cols[k].push(Some(post[k]));
            }
        } else {
            from_vals.push(if class_col[i].is_missing() {
                None
            } else {
                Some(value_label(&class_col[i]))
            });
            into_vals.push(None);
            for k in 0..g {
                post_cols[k].push(None);
            }
        }
    }

    let mut out_df = ds.df.clone();
    out_df
        .with_column(Series::new("_FROM_".into(), from_vals))
        .and_then(|df| df.with_column(Series::new("_INTO_".into(), into_vals)))
        .map_err(|e| SasError::runtime(format!("DISCRIM OUT= build failed: {e}")))?;
    for k in 0..g {
        let col_name = format!("_{}", model.class_labels[k]);
        out_df
            .with_column(Series::new(col_name.into(), post_cols[k].clone()))
            .map_err(|e| SasError::runtime(format!("DISCRIM OUT= build failed: {e}")))?;
    }

    let mut vars = ds.vars.clone();
    vars.push(VarMeta {
        name: "_FROM_".into(),
        ty: VarType::Char,
        length: 32,
        format: None,
        label: None,
    });
    vars.push(VarMeta {
        name: "_INTO_".into(),
        ty: VarType::Char,
        length: 32,
        format: None,
        label: None,
    });
    for k in 0..g {
        vars.push(VarMeta {
            name: format!("_{}", model.class_labels[k]),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        });
    }

    let out_ds = SasDataset { df: out_df, vars };
    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
    let out_display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars = out_ds.vars.len();
    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(out_display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        out_display, n_rows, n_vars
    ));
    Ok(())
}
