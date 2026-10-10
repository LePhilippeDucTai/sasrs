//! PROC GPLOT — graphique "legacy" SAS/GRAPH (M30.1).
//!
//! PROC GPLOT est le prédécesseur de PROC SGPLOT (M29.2). Il s'appuie sur la
//! même infrastructure ODS GRAPHICS (M29.1, module [`crate::graphics::render`])
//! avec une syntaxe différente : le statement `PLOT y*x` (ou `y*x=group`).
//!
//! # Modèle d'exécution selon l'état
//!
//! - `ods_graphics.enabled == false` → NOTE de non-activation, EXIT 0.
//! - `enabled == true` mais build par défaut (sans `--features graphics`) →
//!   NOTE « image deferred », EXIT 0.
//! - `enabled == true` + `--features graphics` → l'image est matérialisée
//!   (`gplot_{N}.png`) et la NOTE « Output '...' (WxH) written. » est émise.
//!
//! v1 ne rend que le PREMIER statement PLOT (NOTE pour les suivants). Les
//! statements SYMBOLn et AXISn présents dans le bloc PROC sont honorés sous
//! `--features graphics` (INTERPOL=JOIN, présence de VALUE=, cinq couleurs
//! COLOR=, bornes de ORDER=, premier texte de LABEL=).
//!
//! # Contrat de support (J02-P6)
//!
//! Les options après `/`, les sous-options SYMBOL/AXIS non dessinées et les
//! options d'affichage du statement PROC donnent un WARNING (elles étaient
//! sautées en silence) ; BY, GOUT=, IMAGEMAP= et les tracés non implémentés
//! une ERROR ; `DATA=` et les variables sont validés dans les deux builds ;
//! l'ordre texte des niveaux numériques de `=z` est signalé (WARNING).
//!
//! # Invariant build par défaut
//!
//! Tout le code de génération d'image est sous `#[cfg(feature = "graphics")]`.
//! Les champs de l'AST consultés uniquement par ce code sont annotés
//! `#[cfg_attr(not(feature = "graphics"), allow(dead_code))]` pour préserver
//! l'invariant « 0 warning » du build par défaut.

use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use crate::missing::value_to_num;
use crate::ods_graphics::contract;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::{expect_ident, read_value};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::VarType;

mod parse;

pub use parse::parse;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct GplotAst {
    /// `DATA=` ; `None` → `_LAST_`.
    pub data_ref: Option<DatasetRef>,
    /// Statements PLOT (le premier est rendu ; M34.11 superpose ses séries).
    pub plots: Vec<GplotStmt>,
    /// Statements SYMBOLn dans l'ordre (SYMBOL1, SYMBOL2, …).
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub symbols: Vec<SymbolDef>,
    /// Statements AXISn dans l'ordre.
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub axes: Vec<AxisDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GplotStmt {
    /// `plot y*x` / `plot y*x=group` / `plot (y1 y2)*x`.
    Plot {
        y_vars: Vec<String>,
        x_var: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        group_var: Option<String>,
    },
}

/// Définition d'un SYMBOLn : `interpol=` (JOIN→ligne), `value=` (marqueur),
/// `color=`. Les attributs non dessinés (HEIGHT, WIDTH, LINE, REPEAT…) donnent
/// un WARNING au parsing (J02-P6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SymbolDef {
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub interpol: Option<String>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub value: Option<String>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub color: Option<String>,
}

/// Définition d'un AXISn : `order=(min to max)` et `label=`. Le reste donne un
/// WARNING au parsing (J02-P6).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AxisDef {
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub order_min: Option<f64>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub order_max: Option<f64>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub label: Option<String>,
}

// ───────────────────────── Execute ─────────────────────────

/// Variables named by the PLOT statements, for the `DATA=` check.
fn referenced_vars(ast: &GplotAst) -> Vec<&str> {
    let mut vars: Vec<&str> = Vec::new();
    for GplotStmt::Plot {
        y_vars,
        x_var,
        group_var,
    } in &ast.plots
    {
        vars.extend(y_vars.iter().map(String::as_str));
        vars.push(x_var);
        vars.extend(group_var.as_deref());
    }
    vars
}

/// Finite values of a numeric column (`None` for a missing value); `None`
/// when the variable is absent or character.
fn numeric_values(ds: &SasDataset, name: &str) -> Result<Option<Vec<Option<f64>>>> {
    let Some(idx) = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(name))
    else {
        return Ok(None);
    };
    if ds.vars[idx].ty != VarType::Num {
        return Ok(None);
    }
    let col = common::decode_column(ds, idx)?;
    Ok(Some(
        col.iter()
            .map(|v| value_to_num(v).filter(|f| f.is_finite()))
            .collect(),
    ))
}

/// J02-P6 — `plot y*x=z` : the engine orders the series of the `z` levels by
/// their TEXT (`format!("{n}")` keys of a `BTreeMap` in
/// `graphics_impl::build_series`), so a numeric `z` with the levels 2 and 10
/// draws 10 first and gives it SYMBOL1. SAS orders the levels by value. The
/// fix belongs to the rendering code (`cfg(feature = "graphics")`, J13-P2);
/// until then this layer, common to both builds, warns when the two orders
/// differ (levels of the observations the engine plots: x and y present).
fn warn_text_ordered_levels(
    ds: &SasDataset,
    stmt: &GplotStmt,
    session: &mut Session,
) -> Result<()> {
    let GplotStmt::Plot {
        y_vars,
        x_var,
        group_var,
    } = stmt;
    let (Some(group), Some(y)) = (group_var, y_vars.first()) else {
        return Ok(());
    };
    let (Some(zs), Some(xs), Some(ys)) = (
        numeric_values(ds, group)?,
        numeric_values(ds, x_var)?,
        numeric_values(ds, y)?,
    ) else {
        return Ok(());
    };
    let mut levels: Vec<f64> = Vec::new();
    for ((z, x), y) in zs.iter().zip(&xs).zip(&ys) {
        if let (Some(z), Some(_), Some(_)) = (z, x, y)
            && !levels.contains(z)
        {
            levels.push(*z);
        }
    }
    levels.sort_by(|a, b| format!("{a}").cmp(&format!("{b}")));
    if let Some(pair) = levels.windows(2).find(|w| w[0] > w[1]) {
        session.log.warning(&format!(
            "The levels of the numeric group variable {} are ordered as text, not by value, in \
             PROC GPLOT ({} is drawn before {}); SYMBOL definitions and colors follow that \
             order{}.",
            group.to_ascii_uppercase(),
            pair[0],
            pair[1],
            contract::planned("J13-P2")
        ));
    }
    Ok(())
}

pub fn execute(ast: &GplotAst, session: &mut Session) -> Result<()> {
    // 0) J02-P6 — DATA= et variables validées dans les deux builds : le build
    //    par défaut n'ouvrait jamais la table (« image deferred », code 0,
    //    même pour une table ou une variable absente).
    let ds = contract::open_checked(&ast.data_ref, session, &referenced_vars(ast))?;

    // 1) ODS GRAPHICS non activé → NOTE de non-activation, EXIT 0.
    if !session.ods_graphics.enabled {
        session.log.note(
            "ODS GRAPHICS is not enabled. Use \"ods graphics on;\" before PROC GPLOT to generate images.",
        );
        return Ok(());
    }

    // 2) Aucun statement PLOT : rien à dessiner.
    let first = match ast.plots.first() {
        Some(s) => s,
        None => {
            session
                .log
                .note("No PLOT statement found in PROC GPLOT; nothing to plot.");
            return Ok(());
        }
    };

    // 3) v1 : un seul plot par image — prévenir si plusieurs.
    if ast.plots.len() > 1 {
        session.log.note(&format!(
            "PROC GPLOT v1 renders only the first PLOT statement; {} additional statement(s) ignored.",
            ast.plots.len() - 1
        ));
    }

    // 4) J02-P6 — ordre des niveaux numériques de `=z` (rendu : texte).
    warn_text_ordered_levels(&ds, first, session)?;

    // 5) Génération de l'image.
    #[cfg(not(feature = "graphics"))]
    {
        let _ = first;
        session
            .log
            .note("ODS GRAPHICS: image deferred (compile with --features graphics).");
        Ok(())
    }

    #[cfg(feature = "graphics")]
    {
        graphics_impl::render(ast, first, session)
    }
}

// ───────────────────────── Rendu (feature graphics) ─────────────────────────

#[cfg(feature = "graphics")]
pub(crate) mod graphics_impl {
    use super::*;
    use crate::graphics::render::{
        Decorations, DrawingSpec, Overlay, PlotType, SeriesColor, draw_to_file_ext, palette,
    };
    use crate::missing::value_to_num;
    use crate::ods_graphics::ImageFmt;
    use crate::procs::common::{self, decode_column};
    use crate::value::{Value, VarType};

    /// Extrait une colonne numérique par nom (erreur propre si absente / non num).
    fn numeric_column(ds: &crate::dataset::SasDataset, name: &str) -> Result<Vec<f64>> {
        let idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", name.to_uppercase()))
            })?;
        if ds.vars[idx].ty != VarType::Num {
            return Err(SasError::runtime(format!(
                "Variable {} must be numeric for PROC GPLOT.",
                name.to_uppercase()
            )));
        }
        let col = decode_column(ds, idx)?;
        Ok(col
            .iter()
            .map(|v| value_to_num(v).unwrap_or(f64::NAN))
            .collect())
    }

    /// Traduit un nom de couleur SAS vers la palette logique.
    pub fn color_from_name(name: &str) -> Option<SeriesColor> {
        match name.to_ascii_lowercase().as_str() {
            "blue" => Some(SeriesColor::Blue),
            "red" => Some(SeriesColor::Red),
            "green" => Some(SeriesColor::Green),
            "orange" => Some(SeriesColor::Orange),
            "black" => Some(SeriesColor::Black),
            _ => None,
        }
    }

    /// Décide (ligne ?, marqueur ?) à partir d'un SYMBOLn éventuel. Par défaut
    /// SAS/GRAPH : marqueurs (pas de jointure). INTERPOL=JOIN → ligne.
    pub fn line_marker(sym: Option<&SymbolDef>) -> (bool, bool) {
        match sym {
            Some(s) => {
                let join = s
                    .interpol
                    .as_deref()
                    .map(|i| i.eq_ignore_ascii_case("join"))
                    .unwrap_or(false);
                let has_value = s.value.is_some();
                if join {
                    (true, has_value)
                } else {
                    // Pas de JOIN : marqueurs (toujours, même sans VALUE= explicite).
                    (false, true)
                }
            }
            None => (false, true),
        }
    }

    /// Une série prête à tracer : points `(x, y)`, couleur, ligne (SYMBOL
    /// `INTERPOL=JOIN`), marqueur (SYMBOL `VALUE=`).
    type PlotSeries = (Vec<(f64, f64)>, SeriesColor, bool, bool);

    /// Construit la liste des séries (label, data, color, line, marker) à tracer
    /// pour un statement PLOT, en honorant SYMBOLn et `=group`.
    pub fn build_series(
        ds: &crate::dataset::SasDataset,
        stmt: &GplotStmt,
        symbols: &[SymbolDef],
    ) -> Result<Vec<PlotSeries>> {
        let GplotStmt::Plot {
            y_vars,
            x_var,
            group_var,
        } = stmt;
        let xs = numeric_column(ds, x_var)?;
        let mut out = Vec::new();

        if let Some(g) = group_var {
            // PLOT y*x=group : une série par niveau de groupe.
            let y = y_vars
                .first()
                .ok_or_else(|| SasError::runtime("PLOT statement has no Y variable."))?;
            let ys = numeric_column(ds, y)?;
            let gidx = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(g))
                .ok_or_else(|| {
                    SasError::runtime(format!("Variable {} not found.", g.to_uppercase()))
                })?;
            let gcol = decode_column(ds, gidx)?;
            use std::collections::BTreeMap;
            let mut groups: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
            for ((gx, x), yv) in gcol.iter().zip(xs.iter()).zip(ys.iter()) {
                if !x.is_finite() || !yv.is_finite() {
                    continue;
                }
                let key = match gx {
                    Value::Char(s) => s.clone(),
                    Value::Num(n) => format!("{n}"),
                    Value::Missing(_) => ".".to_string(),
                };
                groups.entry(key).or_default().push((*x, *yv));
            }
            for (i, (_k, data)) in groups.into_iter().enumerate() {
                let sym = symbols.get(i);
                let (line, marker) = line_marker(sym);
                let color = sym
                    .and_then(|s| s.color.as_deref())
                    .and_then(color_from_name)
                    .unwrap_or_else(|| palette(i));
                out.push((data, color, line, marker));
            }
        } else {
            // PLOT (y1 y2 ...)*x : une série par variable Y.
            for (i, y) in y_vars.iter().enumerate() {
                let ys = numeric_column(ds, y)?;
                let data: Vec<(f64, f64)> = xs
                    .iter()
                    .zip(ys.iter())
                    .filter(|(a, b)| a.is_finite() && b.is_finite())
                    .map(|(a, b)| (*a, *b))
                    .collect();
                let sym = symbols.get(i);
                let (line, marker) = line_marker(sym);
                let color = sym
                    .and_then(|s| s.color.as_deref())
                    .and_then(color_from_name)
                    .unwrap_or_else(|| palette(i));
                out.push((data, color, line, marker));
            }
        }
        Ok(out)
    }

    pub fn render(ast: &GplotAst, stmt: &GplotStmt, session: &mut Session) -> Result<()> {
        let GplotStmt::Plot { y_vars, x_var, .. } = stmt;
        let y_var = y_vars
            .first()
            .ok_or_else(|| SasError::runtime("PLOT statement has no Y variable."))?
            .clone();

        // Lire les données.
        let (ds, _, _) = common::open_input(&ast.data_ref, session)?;

        let series = build_series(&ds, stmt, &ast.symbols)?;

        // Libellés d'axe : AXIS1 LABEL= pour X, AXIS2 LABEL= pour Y (convention
        // courante GPLOT). Sinon nom de variable.
        let x_label = ast
            .axes
            .first()
            .and_then(|a| a.label.clone())
            .unwrap_or_else(|| x_var.clone());
        let y_label = ast
            .axes
            .get(1)
            .and_then(|a| a.label.clone())
            .unwrap_or_else(|| y_var.clone());

        // Toutes les séries (y compris la 1re) sont rendues en overlays pour
        // honorer la couleur SYMBOL de CHACUNE (la série primaire du DrawingSpec
        // est toujours bleue côté render.rs). Le DrawingSpec ne porte que les
        // axes ; ses données primaires restent vides.
        let spec = DrawingSpec::new("The GPLOT Procedure", x_label, y_label, PlotType::Scatter);

        let mut overlays: Vec<Overlay> = Vec::new();
        for (data, color, line, marker) in series.into_iter() {
            overlays.push(Overlay {
                data,
                color,
                line,
                marker,
            });
        }

        // Bornes d'axes : AXIS1 ORDER= → X, AXIS2 ORDER= → Y.
        let x_range = ast
            .axes
            .first()
            .and_then(|a| match (a.order_min, a.order_max) {
                (Some(lo), Some(hi)) => Some((lo, hi)),
                _ => None,
            });
        let y_range = ast
            .axes
            .get(1)
            .and_then(|a| match (a.order_min, a.order_max) {
                (Some(lo), Some(hi)) => Some((lo, hi)),
                _ => None,
            });

        let deco = Decorations {
            overlays,
            x_range,
            y_range,
        };

        // Nommage séquentiel : préfixe IMAGENAME= sinon "gplot".
        session.graphics_image_count += 1;
        let stem = session
            .ods_graphics
            .file_stem
            .clone()
            .unwrap_or_else(|| "gplot".to_string());
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
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;

#[cfg(test)]
mod contract_tests;
