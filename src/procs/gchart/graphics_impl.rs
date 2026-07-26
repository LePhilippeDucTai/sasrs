//! Rendu PNG/SVG de PROC GCHART (feature `graphics`).
//!
//! MQ9.6 — ces 155 lignes vivaient dans un `mod graphics_impl { … }` INLINE
//! au bas de `gchart.rs`, seul module de proc dans ce cas.

use super::*;
use crate::error::SasError;
use crate::graphics::render::{DrawingSpec, PlotType, draw_to_file};
use crate::missing::value_to_num;
use crate::ods_graphics::ImageFmt;
use crate::procs::common::{self, decode_column};
use crate::value::{Value, VarType};
use std::collections::BTreeMap;

fn category_key(v: &Value) -> String {
    match v {
        Value::Char(s) => s.clone(),
        Value::Num(n) => format!("{n}"),
        Value::Missing(_) => ".".to_string(),
    }
}

/// Agrège les valeurs (catégorie, statistique) selon `chart_type`.
pub fn aggregate(
    ds: &crate::dataset::SasDataset,
    category: &str,
    sumvar: &Option<String>,
    chart_type: ChartType,
) -> Result<Vec<(String, f64)>> {
    let cat_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(category))
        .ok_or_else(|| {
            SasError::runtime(format!("Variable {} not found.", category.to_uppercase()))
        })?;
    let cat_col = decode_column(ds, cat_idx)?;

    match chart_type {
        ChartType::Freq => {
            let mut counts: BTreeMap<String, f64> = BTreeMap::new();
            for v in &cat_col {
                *counts.entry(category_key(v)).or_insert(0.0) += 1.0;
            }
            Ok(counts.into_iter().collect())
        }
        ChartType::Sum | ChartType::Mean => {
            let resp_name = sumvar.as_deref().ok_or_else(|| {
                SasError::runtime("TYPE=SUM/MEAN in PROC GCHART requires SUMVAR=.")
            })?;
            let resp_idx = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(resp_name))
                .ok_or_else(|| {
                    SasError::runtime(format!("Variable {} not found.", resp_name.to_uppercase()))
                })?;
            if ds.vars[resp_idx].ty != VarType::Num {
                return Err(SasError::runtime(format!(
                    "Variable {} must be numeric for SUMVAR= in PROC GCHART.",
                    resp_name.to_uppercase()
                )));
            }
            let resp_col = decode_column(ds, resp_idx)?;
            let mut sums: BTreeMap<String, (f64, f64)> = BTreeMap::new();
            for (cv, rv) in cat_col.iter().zip(resp_col.iter()) {
                let val = value_to_num(rv).unwrap_or(f64::NAN);
                if !val.is_finite() {
                    continue;
                }
                let e = sums.entry(category_key(cv)).or_insert((0.0, 0.0));
                e.0 += val;
                e.1 += 1.0;
            }
            Ok(sums
                .into_iter()
                .map(|(k, (sum, n))| {
                    let v = if matches!(chart_type, ChartType::Mean) && n > 0.0 {
                        sum / n
                    } else {
                        sum
                    };
                    (k, v)
                })
                .collect())
        }
    }
}

pub fn render(ast: &GchartAst, chart: &GchartStmt, session: &mut Session) -> Result<()> {
    let (category, sumvar, chart_type, is_pie) = match chart {
        GchartStmt::VBar {
            category,
            sumvar,
            chart_type,
        }
        | GchartStmt::HBar {
            category,
            sumvar,
            chart_type,
        } => (category, sumvar, *chart_type, false),
        GchartStmt::Pie {
            category,
            sumvar,
            chart_type,
        } => (category, sumvar, *chart_type, true),
    };

    let (ds, _, _) = common::open_input(&ast.data_ref, session)?;

    let x_categorical = aggregate(&ds, category, sumvar, chart_type)?;
    let y_label = match chart_type {
        ChartType::Freq => "Frequency".to_string(),
        ChartType::Sum => format!("SUM of {}", sumvar.as_deref().unwrap_or("")),
        ChartType::Mean => format!("MEAN of {}", sumvar.as_deref().unwrap_or("")),
    };

    let spec = DrawingSpec {
        title: "The GCHART Procedure".to_string(),
        x_label: category.clone(),
        y_label,
        plot_type: if is_pie {
            PlotType::Pie
        } else {
            PlotType::VBar
        },
        data: vec![],
        x_categorical,
    };

    session.graphics_image_count += 1;
    let stem = session
        .ods_graphics
        .file_stem
        .clone()
        .unwrap_or_else(|| "gchart".to_string());
    let fmt = session.ods_graphics.image_format;
    let ext = match fmt {
        ImageFmt::Png => "png",
        ImageFmt::Svg => "svg",
    };
    let name = format!("{}_{}.{}", stem, session.graphics_image_count, ext);
    let path = session.ods_graphics.output_dir.join(&name);

    let (w, h) = draw_to_file(
        &spec,
        &path,
        session.ods_graphics.width,
        session.ods_graphics.height,
        fmt,
    )?;
    session
        .log
        .note(&format!("Output '{}' ({}x{}) written.", name, w, h));
    Ok(())
}
