use super::execute::stmt_kind;
use super::*;
use crate::graphics::render::{
    draw_to_file_ext, Decorations, DrawingSpec, Overlay, PlotType, SeriesColor,
};
use crate::missing::value_to_num;
use crate::ods_graphics::ImageFmt;
use crate::procs::common::decode_column;
use crate::value::VarType;

/// Courbe LOESS : lissage local pondéré (poids tricube, ajustement linéaire
/// local) sur `npoints` abscisses réparties sur la plage des x. `frac` est la
/// fraction de points dans la fenêtre locale (SMOOTH=, défaut ~0.5).
///
/// Renvoie une liste `(x, yhat)` triée par x croissant. Si trop peu de points
/// ou plage nulle, renvoie un vecteur vide.
pub fn loess_curve(
    xs: &[f64],
    ys: &[f64],
    frac: f64,
    npoints: usize,
) -> Vec<(f64, f64)> {
    // Paires finies, triées par x.
    let mut pts: Vec<(f64, f64)> = xs
        .iter()
        .zip(ys.iter())
        .filter(|(a, b)| a.is_finite() && b.is_finite())
        .map(|(a, b)| (*a, *b))
        .collect();
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let n = pts.len();
    if n < 2 {
        return Vec::new();
    }
    let x_min = pts[0].0;
    let x_max = pts[n - 1].0;
    if (x_max - x_min).abs() < f64::EPSILON {
        return Vec::new();
    }
    let frac = frac.clamp(0.05, 1.0);
    // Taille de fenêtre (au moins 2 voisins).
    let win = ((frac * n as f64).ceil() as usize).clamp(2, n);

    let np = npoints.max(2);
    let mut out = Vec::with_capacity(np);
    for i in 0..np {
        let x0 = x_min + (x_max - x_min) * i as f64 / (np - 1) as f64;
        // Distances à x0, prendre les `win` plus proches.
        let mut dist: Vec<f64> = pts.iter().map(|(x, _)| (x - x0).abs()).collect();
        let mut sorted = dist.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let dmax = sorted[win - 1].max(f64::EPSILON);
        // Poids tricube ; régression linéaire locale pondérée.
        let (mut sw, mut swx, mut swy, mut swxx, mut swxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (k, (x, y)) in pts.iter().enumerate() {
            let d = dist[k] / dmax;
            if d >= 1.0 {
                continue;
            }
            let w = {
                let t = 1.0 - d * d * d;
                t * t * t
            };
            sw += w;
            swx += w * x;
            swy += w * y;
            swxx += w * x * x;
            swxy += w * x * y;
        }
        // Résolution du système 2x2 pour (a, b) : y = a + b x.
        let denom = sw * swxx - swx * swx;
        let yhat = if denom.abs() > 1e-12 {
            let b = (sw * swxy - swx * swy) / denom;
            let a = (swy - b * swx) / sw;
            a + b * x0
        } else if sw > 0.0 {
            swy / sw
        } else {
            f64::NAN
        };
        dist.clear();
        if yhat.is_finite() {
            out.push((x0, yhat));
        }
    }
    out
}

/// Densité normale (gaussienne) ajustée par moyenne/écart-type des données,
/// évaluée sur `npoints` abscisses couvrant la plage [min, max] élargie.
pub fn normal_density_curve(xs: &[f64], npoints: usize) -> Vec<(f64, f64)> {
    let vals: Vec<f64> = xs.iter().copied().filter(|v| v.is_finite()).collect();
    let n = vals.len();
    if n < 2 {
        return Vec::new();
    }
    let mean = vals.iter().sum::<f64>() / n as f64;
    let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    let sd = var.sqrt();
    if !(sd > 0.0) {
        return Vec::new();
    }
    let x_min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let x_max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    // Élargir un peu pour englober les queues.
    let lo = x_min - 0.5 * sd;
    let hi = x_max + 0.5 * sd;
    let np = npoints.max(2);
    let inv = 1.0 / (sd * (std::f64::consts::TAU).sqrt());
    (0..np)
        .map(|i| {
            let x = lo + (hi - lo) * i as f64 / (np - 1) as f64;
            let z = (x - mean) / sd;
            let pdf = inv * (-0.5 * z * z).exp();
            (x, pdf)
        })
        .collect()
}

/// Densité par noyau gaussien (KDE), bande passante de Silverman, évaluée sur
/// `npoints` abscisses.
pub fn kernel_density_curve(xs: &[f64], npoints: usize) -> Vec<(f64, f64)> {
    let vals: Vec<f64> = xs.iter().copied().filter(|v| v.is_finite()).collect();
    let n = vals.len();
    if n < 2 {
        return Vec::new();
    }
    let mean = vals.iter().sum::<f64>() / n as f64;
    let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    let sd = var.sqrt();
    if !(sd > 0.0) {
        return Vec::new();
    }
    // Bande passante de Silverman (règle du pouce).
    let h = 1.06 * sd * (n as f64).powf(-0.2);
    let h = if h > 0.0 { h } else { sd };
    let x_min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let x_max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let lo = x_min - 3.0 * h;
    let hi = x_max + 3.0 * h;
    let np = npoints.max(2);
    let inv = 1.0 / ((n as f64) * h * (std::f64::consts::TAU).sqrt());
    (0..np)
        .map(|i| {
            let x = lo + (hi - lo) * i as f64 / (np - 1) as f64;
            let mut s = 0.0;
            for &v in &vals {
                let z = (x - v) / h;
                s += (-0.5 * z * z).exp();
            }
            (x, inv * s)
        })
        .collect()
}

/// Extrait une colonne numérique par nom (erreur propre si absente / non num).
fn numeric_column(
    ds: &crate::dataset::SasDataset,
    name: &str,
) -> Result<Vec<f64>> {
    let idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            SasError::runtime(format!("Variable {} not found.", name.to_uppercase()))
        })?;
    if ds.vars[idx].ty != VarType::Num {
        return Err(SasError::runtime(format!(
            "Variable {} must be numeric for PROC SGPLOT.",
            name.to_uppercase()
        )));
    }
    let col = decode_column(ds, idx)?;
    Ok(col
        .iter()
        .map(|v| value_to_num(v).unwrap_or(f64::NAN))
        .collect())
}

/// Construit le DrawingSpec depuis le statement PRIMAIRE (scatter / series /
/// histogram / vbar). Les statements LOESS / DENSITY / REG deviennent des
/// overlays ajoutés ensuite par [`build_overlays`].
fn build_primary_spec(
    ds: &crate::dataset::SasDataset,
    stmt: &SgplotStmt,
    ast: &SgplotAst,
    title: String,
) -> Result<DrawingSpec> {
    let x_axis_label = ast.xaxis.as_ref().and_then(|a| a.label.clone());
    let y_axis_label = ast.yaxis.as_ref().and_then(|a| a.label.clone());

    let spec = match stmt {
        SgplotStmt::Scatter { x, y, .. } | SgplotStmt::Series { x, y, .. } => {
            let xs = numeric_column(ds, x)?;
            let ys = numeric_column(ds, y)?;
            let data: Vec<(f64, f64)> = xs
                .iter()
                .zip(ys.iter())
                .filter(|(a, b)| a.is_finite() && b.is_finite())
                .map(|(a, b)| (*a, *b))
                .collect();
            let plot_type = if matches!(stmt, SgplotStmt::Series { .. }) {
                PlotType::Series
            } else {
                PlotType::Scatter
            };
            let mut s = DrawingSpec::new(
                title,
                x_axis_label.unwrap_or_else(|| x.clone()),
                y_axis_label.unwrap_or_else(|| y.clone()),
                plot_type,
            );
            s.data = data;
            s
        }
        // LOESS / DENSITY seuls (pas de scatter/histogram parent) : on les
        // rend en SERIES sur leur propre courbe.
        SgplotStmt::Loess { x, y, smooth } => {
            let xs = numeric_column(ds, x)?;
            let ys = numeric_column(ds, y)?;
            let curve = loess_curve(&xs, &ys, *smooth, 100);
            let mut s = DrawingSpec::new(
                title,
                x_axis_label.unwrap_or_else(|| x.clone()),
                y_axis_label.unwrap_or_else(|| y.clone()),
                PlotType::Series,
            );
            s.data = curve;
            s
        }
        SgplotStmt::Density { var, kernel } => {
            let xs = numeric_column(ds, var)?;
            let curve = if *kernel {
                kernel_density_curve(&xs, 100)
            } else {
                normal_density_curve(&xs, 100)
            };
            let mut s = DrawingSpec::new(
                title,
                x_axis_label.unwrap_or_else(|| var.clone()),
                y_axis_label.unwrap_or_else(|| "Density".to_string()),
                PlotType::Series,
            );
            s.data = curve;
            s
        }
        SgplotStmt::Histogram { var, binwidth, .. } => {
            let xs: Vec<f64> = numeric_column(ds, var)?
                .into_iter()
                .filter(|v| v.is_finite())
                .collect();
            let data: Vec<(f64, f64)> = xs.iter().map(|v| (*v, 0.0)).collect();
            let bins = match binwidth {
                Some(bw) if *bw > 0.0 && !xs.is_empty() => {
                    let mn = xs.iter().cloned().fold(f64::INFINITY, f64::min);
                    let mx = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    (((mx - mn) / bw).ceil() as usize).max(1)
                }
                _ => 10,
            };
            let mut s = DrawingSpec::new(
                title,
                x_axis_label.unwrap_or_else(|| var.clone()),
                y_axis_label.unwrap_or_else(|| "Frequency".to_string()),
                PlotType::Histogram { bins },
            );
            s.data = data;
            s
        }
        SgplotStmt::VBar { category, .. } => {
            let idx = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(category))
                .ok_or_else(|| {
                    SasError::runtime(format!("Variable {} not found.", category.to_uppercase()))
                })?;
            let col = decode_column(ds, idx)?;
            use std::collections::BTreeMap;
            let mut counts: BTreeMap<String, f64> = BTreeMap::new();
            for v in &col {
                let key = match v {
                    crate::value::Value::Char(s) => s.clone(),
                    crate::value::Value::Num(n) => format!("{n}"),
                    crate::value::Value::Missing(_) => ".".to_string(),
                };
                *counts.entry(key).or_insert(0.0) += 1.0;
            }
            let mut s = DrawingSpec::new(
                title,
                x_axis_label.unwrap_or_else(|| category.clone()),
                y_axis_label.unwrap_or_else(|| "Frequency".to_string()),
                PlotType::VBar,
            );
            s.x_categorical = counts.into_iter().collect();
            s
        }
        _ => {
            return Err(SasError::runtime(format!(
                "{} plot not yet rendered in PROC SGPLOT.",
                stmt_kind(stmt)
            )))
        }
    };
    Ok(spec)
}

/// Bornes d'axe forcées par XAXIS/YAXIS VALUES=.
fn axis_ranges(ast: &SgplotAst) -> (Option<(f64, f64)>, Option<(f64, f64)>) {
    let x = ast.xaxis.as_ref().and_then(|ax| {
        match (ax.values_min, ax.values_max) {
            (Some(lo), Some(hi)) => Some((lo, hi)),
            _ => None,
        }
    });
    let y = ast.yaxis.as_ref().and_then(|ax| {
        match (ax.values_min, ax.values_max) {
            (Some(lo), Some(hi)) => Some((lo, hi)),
            _ => None,
        }
    });
    (x, y)
}

/// Construit les overlays (LOESS, DENSITY, REG droite) à superposer, en
/// excluant le statement primaire déjà tracé.
fn build_overlays(
    ds: &crate::dataset::SasDataset,
    ast: &SgplotAst,
    primary: usize,
) -> Result<Vec<Overlay>> {
    let mut overlays = Vec::new();
    let mut ci = 1usize;
    for (i, stmt) in ast.plot_stmts.iter().enumerate() {
        if i == primary {
            continue;
        }
        match stmt {
            SgplotStmt::Loess { x, y, smooth } => {
                let xs = numeric_column(ds, x)?;
                let ys = numeric_column(ds, y)?;
                let curve = loess_curve(&xs, &ys, *smooth, 100);
                if !curve.is_empty() {
                    overlays.push(Overlay {
                        data: curve,
                        color: crate::graphics::render::palette(ci),
                        line: true,
                        marker: false,
                    });
                    ci += 1;
                }
            }
            SgplotStmt::Density { var, kernel } => {
                let xs = numeric_column(ds, var)?;
                let curve = if *kernel {
                    kernel_density_curve(&xs, 100)
                } else {
                    normal_density_curve(&xs, 100)
                };
                if !curve.is_empty() {
                    overlays.push(Overlay {
                        data: curve,
                        color: SeriesColor::Red,
                        line: true,
                        marker: false,
                    });
                    ci += 1;
                }
            }
            SgplotStmt::Series { x, y, .. } | SgplotStmt::Scatter { x, y, .. } => {
                // Série additionnelle superposée.
                let xs = numeric_column(ds, x)?;
                let ys = numeric_column(ds, y)?;
                let data: Vec<(f64, f64)> = xs
                    .iter()
                    .zip(ys.iter())
                    .filter(|(a, b)| a.is_finite() && b.is_finite())
                    .map(|(a, b)| (*a, *b))
                    .collect();
                if !data.is_empty() {
                    let line = matches!(stmt, SgplotStmt::Series { .. });
                    overlays.push(Overlay {
                        data,
                        color: crate::graphics::render::palette(ci),
                        line,
                        marker: !line,
                    });
                    ci += 1;
                }
            }
            _ => {}
        }
    }
    Ok(overlays)
}

/// Indice du statement primaire : le premier qui n'est PAS une simple courbe
/// (LOESS/DENSITY). Si tous sont des courbes, on prend le premier.
fn primary_index(ast: &SgplotAst) -> usize {
    ast.plot_stmts
        .iter()
        .position(|s| {
            matches!(
                s,
                SgplotStmt::Scatter { .. }
                    | SgplotStmt::Series { .. }
                    | SgplotStmt::Histogram { .. }
                    | SgplotStmt::VBar { .. }
            )
        })
        .unwrap_or(0)
}

pub fn render(ast: &SgplotAst, session: &mut Session) -> Result<()> {
    let primary = primary_index(ast);
    let stmt = &ast.plot_stmts[primary];

    // Types non supportés par render.rs (HBAR, VBOX, REG) → NOTE, pas d'erreur.
    if matches!(
        stmt,
        SgplotStmt::HBar { .. } | SgplotStmt::VBox { .. } | SgplotStmt::Reg { .. }
    ) {
        session.log.note(&format!(
            "{} plot deferred (not yet rendered in PROC SGPLOT).",
            stmt_kind(stmt)
        ));
        return Ok(());
    }

    // Lire les données.
    let (ds, _, _) = common::open_input(&ast.data_ref, session)?;

    let spec = build_primary_spec(&ds, stmt, ast, "The SGPlot Procedure".to_string())?;
    let (x_range, y_range) = axis_ranges(ast);
    let deco = Decorations {
        overlays: build_overlays(&ds, ast, primary)?,
        x_range,
        y_range,
    };

    // Nommage séquentiel : préfixe IMAGENAME= sinon "sgplot".
    session.graphics_image_count += 1;
    let stem = session
        .ods_graphics
        .file_stem
        .clone()
        .unwrap_or_else(|| "sgplot".to_string());
    let fmt = session.ods_graphics.image_format;
    let ext = match fmt {
        ImageFmt::Png => "png",
        ImageFmt::Svg => "svg",
    };
    let name = format!("{}_{}.{}", stem, session.graphics_image_count, ext);
    let path = session.ods_graphics.output_dir.join(&name);

    let (w, h) = draw_to_file_ext(
        &spec,
        &deco,
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
