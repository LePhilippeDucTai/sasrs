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
//! statements globaux SYMBOL et AXIS, lorsqu'ils apparaissent DANS le bloc
//! PROC, sont parsés sans erreur puis ignorés (NOTE de différé).
//!
//! # Invariant build par défaut
//!
//! Tout le code de génération d'image est sous `#[cfg(feature = "graphics")]`.
//! Les champs de l'AST consultés uniquement par ce code sont annotés
//! `#[cfg_attr(not(feature = "graphics"), allow(dead_code))]` pour préserver
//! l'invariant « 0 warning » du build par défaut.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;

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
/// `color=`. Attributs non interprétés (HEIGHT, WIDTH, LINE, REPEAT…) ignorés.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SymbolDef {
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub interpol: Option<String>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub value: Option<String>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub color: Option<String>,
}

/// Définition d'un AXISn : `order=(min to max)` et `label=`. Le reste est ignoré.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AxisDef {
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub order_min: Option<f64>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub order_max: Option<f64>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub label: Option<String>,
}

// ───────────────────────── Parser ─────────────────────────

/// Lit un nom de variable (identifiant). Erreur propre sinon.
fn expect_ident(ts: &mut StatementStream, ctx: &str) -> Result<String> {
    match ts.peek().ident().map(str::to_string) {
        Some(s) => {
            ts.next();
            Ok(s)
        }
        None => Err(SasError::parse(
            format!("expected an identifier {ctx}"),
            ts.peek().span,
        )),
    }
}

/// Parse un statement PLOT : `y*x`, `y*x=group`, ou `(y1 y2)*x`.
///
/// Le mot-clé `plot` a déjà été consommé. Consomme jusqu'au `;` exclu (mais
/// pas le `;`). Les options après un éventuel `/` sont ignorées en v1.
fn parse_plot_stmt(ts: &mut StatementStream) -> Result<GplotStmt> {
    // Membre gauche : un identifiant, ou une liste parenthésée `(y1 y2 ...)`.
    let mut y_vars: Vec<String> = Vec::new();
    if ts.peek().kind == TokenKind::LParen {
        ts.next(); // (
        while ts.peek().kind != TokenKind::RParen && ts.peek().kind != TokenKind::Eof {
            match ts.peek().ident().map(str::to_string) {
                Some(s) => {
                    ts.next();
                    y_vars.push(s);
                }
                None => {
                    ts.next();
                }
            }
        }
        if ts.peek().kind == TokenKind::RParen {
            ts.next(); // )
        }
        if y_vars.is_empty() {
            return Err(SasError::parse(
                "PLOT statement requires at least one Y variable",
                ts.peek().span,
            ));
        }
    } else {
        y_vars.push(expect_ident(ts, "for Y variable in PLOT")?);
    }

    // Séparateur `*`.
    if ts.peek().kind != TokenKind::Star {
        return Err(SasError::parse(
            "expected '*' between Y and X in PLOT statement (y*x)",
            ts.peek().span,
        ));
    }
    ts.next(); // *

    // Membre droit : la variable X.
    let x_var = expect_ident(ts, "for X variable in PLOT")?;

    // `=group` optionnel.
    let mut group_var: Option<String> = None;
    if ts.peek().kind == TokenKind::Eq {
        ts.next(); // =
        group_var = Some(expect_ident(ts, "for GROUP variable in PLOT (y*x=group)")?);
    }

    // Options après `/` : ignorées en v1, on saute jusqu'au `;`.
    if ts.peek().kind == TokenKind::Slash {
        ts.skip_to_semi();
        // skip_to_semi a consommé le `;` : on renvoie tel quel et l'appelant
        // n'attend PAS de `;` supplémentaire. Pour rester homogène avec le
        // chemin sans `/`, on signale le `;` déjà consommé via un drapeau.
        return Ok(GplotStmt::Plot {
            y_vars,
            x_var,
            group_var,
        });
    }

    Ok(GplotStmt::Plot {
        y_vars,
        x_var,
        group_var,
    })
}

/// Parse PROC GPLOT. Appelé APRÈS consommation de `proc gplot`.
pub fn parse(ts: &mut StatementStream) -> Result<GplotAst> {
    let mut data_ref: Option<DatasetRef> = None;

    // Options du statement PROC GPLOT, jusqu'au `;`.
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            ts.next();
            if ts.peek().kind == TokenKind::Eq {
                ts.next();
            }
            data_ref = Some(ts.parse_dataset_ref()?);
        } else {
            ts.next(); // ignorer les options PROC inconnues
        }
    }

    let mut plots: Vec<GplotStmt> = Vec::new();
    let mut symbols: Vec<SymbolDef> = Vec::new();
    let mut axes: Vec<AxisDef> = Vec::new();

    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }

        if ts.peek().is_kw("plot") || ts.peek().is_kw("plot2") {
            ts.next();
            // parse_plot_stmt peut avoir consommé le `;` (chemin avec `/`).
            // On note la position pour savoir s'il reste un `;`.
            let stmt = parse_plot_stmt(ts)?;
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            plots.push(stmt);
        } else if ts.peek().is_kw("symbol") || starts_with_kw(ts, "symbol") {
            ts.next();
            symbols.push(parse_symbol_stmt(ts));
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
        } else if ts.peek().is_kw("axis") || starts_with_kw(ts, "axis") {
            ts.next();
            axes.push(parse_axis_stmt(ts));
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
        } else {
            ts.skip_to_semi();
        }
    }

    Ok(GplotAst {
        data_ref,
        plots,
        symbols,
        axes,
    })
}

/// Lit une valeur (string, ident ou nombre) — pour `color=`, `value=`, etc.
fn read_value(ts: &mut StatementStream) -> Option<String> {
    match &ts.peek().kind {
        TokenKind::Str { value, .. } => {
            let v = value.clone();
            ts.next();
            Some(v)
        }
        TokenKind::Ident(s) => {
            let v = s.clone();
            ts.next();
            Some(v)
        }
        TokenKind::Num(f) => {
            let f = *f;
            ts.next();
            Some(if f.fract() == 0.0 {
                format!("{}", f as i64)
            } else {
                format!("{f}")
            })
        }
        _ => None,
    }
}

/// Parse un SYMBOLn : suite d'options `name=value` jusqu'au `;` (non consommé).
fn parse_symbol_stmt(ts: &mut StatementStream) -> SymbolDef {
    let mut def = SymbolDef::default();
    while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
        let name = match ts.peek().ident().map(|s| s.to_ascii_lowercase()) {
            Some(n) => n,
            None => {
                ts.next();
                continue;
            }
        };
        ts.next();
        // `i`/`v`/`c` sont les abréviations SAS de interpol/value/color.
        let canon = match name.as_str() {
            "i" => "interpol",
            "v" => "value",
            "c" => "color",
            other => other,
        };
        if ts.peek().kind == TokenKind::Eq {
            ts.next();
            let val = read_value(ts);
            match canon {
                "interpol" => def.interpol = val,
                "value" => def.value = val,
                "color" => def.color = val,
                _ => {}
            }
        }
    }
    def
}

/// Parse un AXISn : `order=(min to max [by step])`, `label=('..')`. Le reste est
/// ignoré (sauté proprement, y compris les blocs parenthésés).
fn parse_axis_stmt(ts: &mut StatementStream) -> AxisDef {
    let mut def = AxisDef::default();
    while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
        let name = match ts.peek().ident().map(|s| s.to_ascii_lowercase()) {
            Some(n) => n,
            None => {
                ts.next();
                continue;
            }
        };
        ts.next();
        match name.as_str() {
            "order" => {
                if ts.peek().kind == TokenKind::Eq {
                    ts.next();
                }
                if ts.peek().kind == TokenKind::LParen {
                    ts.next();
                    let mut nums: Vec<f64> = Vec::new();
                    while ts.peek().kind != TokenKind::RParen && ts.peek().kind != TokenKind::Eof {
                        if let TokenKind::Num(f) = ts.peek().kind {
                            nums.push(f);
                        }
                        ts.next();
                    }
                    if ts.peek().kind == TokenKind::RParen {
                        ts.next();
                    }
                    // `order=(min to max [by step])` : 1er nombre = min, 2e = max
                    // (le 3e éventuel est le pas, ignoré).
                    if let Some(&mn) = nums.first() {
                        def.order_min = Some(mn);
                    }
                    if nums.len() >= 2 {
                        def.order_max = Some(nums[1]);
                    }
                }
            }
            "label" => {
                if ts.peek().kind == TokenKind::Eq {
                    ts.next();
                }
                if ts.peek().kind == TokenKind::LParen {
                    // label=('text') : prendre la première chaîne.
                    ts.next();
                    let mut lab: Option<String> = None;
                    while ts.peek().kind != TokenKind::RParen && ts.peek().kind != TokenKind::Eof {
                        if lab.is_none() {
                            if let TokenKind::Str { value, .. } = &ts.peek().kind {
                                lab = Some(value.clone());
                            }
                        }
                        ts.next();
                    }
                    if ts.peek().kind == TokenKind::RParen {
                        ts.next();
                    }
                    def.label = lab;
                } else {
                    def.label = read_value(ts);
                }
            }
            _ => {
                if ts.peek().kind == TokenKind::Eq {
                    ts.next();
                    if ts.peek().kind == TokenKind::LParen {
                        let mut depth = 0;
                        loop {
                            match ts.peek().kind {
                                TokenKind::LParen => {
                                    depth += 1;
                                    ts.next();
                                }
                                TokenKind::RParen => {
                                    depth -= 1;
                                    ts.next();
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                TokenKind::Eof | TokenKind::Semi => break,
                                _ => {
                                    ts.next();
                                }
                            }
                        }
                    } else {
                        let _ = read_value(ts);
                    }
                }
            }
        }
    }
    def
}

/// Vrai si le token courant est un identifiant dont le préfixe (sans suffixe
/// numérique éventuel) correspond à `kw` — pour `symbol1`, `axis2`, etc.
fn starts_with_kw(ts: &StatementStream, kw: &str) -> bool {
    ts.peek()
        .ident()
        .map(|s| {
            let lower = s.to_ascii_lowercase();
            lower.starts_with(kw)
                && lower[kw.len()..].chars().all(|c| c.is_ascii_digit())
        })
        .unwrap_or(false)
}

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &GplotAst, session: &mut Session) -> Result<()> {
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

    // 4) Génération de l'image.
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
        draw_to_file_ext, palette, Decorations, DrawingSpec, Overlay, PlotType, SeriesColor,
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

    /// Construit la liste des séries (label, data, color, line, marker) à tracer
    /// pour un statement PLOT, en honorant SYMBOLn et `=group`.
    pub fn build_series(
        ds: &crate::dataset::SasDataset,
        stmt: &GplotStmt,
        symbols: &[SymbolDef],
    ) -> Result<Vec<(Vec<(f64, f64)>, SeriesColor, bool, bool)>> {
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
        let in_ref = common::resolve_last_dataset(&ast.data_ref, session)?;
        let in_libref = in_ref.libref_or_work();
        let in_table = in_ref.name.to_uppercase();
        let provider = session.libs.get(&in_libref)?;
        let (ds, notes) = provider.read(&in_table)?;
        for note in notes {
            session.log.forward(&note);
        }

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
        let spec = DrawingSpec::new(
            "The GPLOT Procedure",
            x_label,
            y_label,
            PlotType::Scatter,
        );

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
        let x_range = ast.axes.first().and_then(|a| match (a.order_min, a.order_max) {
            (Some(lo), Some(hi)) => Some((lo, hi)),
            _ => None,
        });
        let y_range = ast.axes.get(1).and_then(|a| match (a.order_min, a.order_max) {
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
mod tests {
    use super::*;
    use crate::source::SourceFile;

    fn parse_gplot(src: &str) -> Result<GplotAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // gplot
        parse(&mut ts)
    }

    #[allow(dead_code)]
    fn make_session() -> Session {
        Session::new(None, std::env::temp_dir(), true).unwrap()
    }

    // ── Parse tests ──────────────────────────────────────────────────────

    #[test]
    fn parse_plot_simple() {
        let ast = parse_gplot("proc gplot data=a; plot y*x; run;").unwrap();
        assert_eq!(ast.plots.len(), 1);
        match &ast.plots[0] {
            GplotStmt::Plot {
                y_vars,
                x_var,
                group_var,
            } => {
                assert_eq!(y_vars, &vec!["y".to_string()]);
                assert_eq!(x_var, "x");
                assert!(group_var.is_none());
            }
        }
    }

    #[test]
    fn parse_plot_with_group() {
        let ast = parse_gplot("proc gplot data=a; plot y*x=group; run;").unwrap();
        match &ast.plots[0] {
            GplotStmt::Plot {
                y_vars,
                x_var,
                group_var,
            } => {
                assert_eq!(y_vars, &vec!["y".to_string()]);
                assert_eq!(x_var, "x");
                assert_eq!(group_var.as_deref(), Some("group"));
            }
        }
    }

    #[test]
    fn parse_plot_multiple_y() {
        let ast = parse_gplot("proc gplot data=a; plot (y1 y2)*x; run;").unwrap();
        match &ast.plots[0] {
            GplotStmt::Plot { y_vars, x_var, .. } => {
                assert_eq!(y_vars, &vec!["y1".to_string(), "y2".to_string()]);
                assert_eq!(x_var, "x");
            }
        }
    }

    #[test]
    fn parse_symbol_axis_ignored() {
        // SYMBOL/AXIS dans le bloc sont parsés sans erreur et ignorés.
        let ast = parse_gplot(
            "proc gplot data=a; symbol1 color=blue value=dot; axis1 label=('T'); plot y*x; run;",
        )
        .unwrap();
        assert_eq!(ast.plots.len(), 1);
    }

    // ── Execute tests (default build) ────────────────────────────────────

    #[test]
    fn execute_without_ods_on_notes_not_enabled() {
        let mut session = make_session();
        let ast = parse_gplot("proc gplot data=a; plot y*x; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(
            log.contains("ODS GRAPHICS is not enabled")
                && log.contains("PROC GPLOT"),
            "log: {log}"
        );
    }

    #[cfg(not(feature = "graphics"))]
    #[test]
    fn execute_with_ods_on_no_feature_defers() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        let ast = parse_gplot("proc gplot data=a; plot y*x; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("image deferred"), "log: {log}");
    }

    // ── Execute tests (feature graphics) ─────────────────────────────────

    #[cfg(feature = "graphics")]
    fn write_xy(session: &mut Session, table: &str) {
        use crate::dataset::{SasDataset, VarMeta};
        use crate::value::VarType;
        use polars::df;
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
            "y" => [2.0_f64, 4.0, 3.0, 5.0, 6.0]
        ]
        .unwrap();
        let vars = vec![
            VarMeta {
                name: "x".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
            VarMeta {
                name: "y".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    }

    #[cfg(feature = "graphics")]
    #[test]
    fn execute_with_graphics_writes_image() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        session.ods_graphics.output_dir = std::env::temp_dir();
        session.ods_graphics.file_stem = Some("gplottest_single".into());
        write_xy(&mut session, "XY");
        let ast = parse_gplot("proc gplot data=work.xy; plot y*x; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("written"), "log: {log}");
        let p = std::env::temp_dir().join("gplottest_single_1.png");
        assert!(p.exists(), "image not created: {p:?}");
        assert!(p.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&p);
    }

    // ── M34.11 : multi-séries + SYMBOL/AXIS ──────────────────────────────

    #[cfg(feature = "graphics")]
    fn write_multi(session: &mut Session, table: &str) {
        use crate::dataset::{SasDataset, VarMeta};
        use crate::value::VarType;
        use polars::df;
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0],
            "y1" => [1.0_f64, 2.0, 3.0, 4.0],
            "y2" => [4.0_f64, 3.0, 2.0, 1.0],
            "g" => ["a", "b", "a", "b"]
        ]
        .unwrap();
        let mk = |n: &str, t: VarType, l: usize| VarMeta {
            name: n.into(),
            ty: t,
            length: l,
            format: None,
            label: None,
        };
        let vars = vec![
            mk("x", VarType::Num, 8),
            mk("y1", VarType::Num, 8),
            mk("y2", VarType::Num, 8),
            mk("g", VarType::Char, 1),
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    }

    #[cfg(feature = "graphics")]
    #[test]
    fn build_series_two_y_vars_makes_two_series() {
        use crate::dataset::{SasDataset, VarMeta};
        use crate::value::VarType;
        use polars::df;
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0],
            "y1" => [1.0_f64, 2.0, 3.0],
            "y2" => [3.0_f64, 2.0, 1.0]
        ]
        .unwrap();
        let mk = |n: &str| VarMeta {
            name: n.into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        };
        let ds = SasDataset {
            df,
            vars: vec![mk("x"), mk("y1"), mk("y2")],
        };
        let ast = parse_gplot("proc gplot data=work.m; plot (y1 y2)*x; run;").unwrap();
        let series =
            graphics_impl::build_series(&ds, &ast.plots[0], &ast.symbols).unwrap();
        assert_eq!(series.len(), 2, "expected one series per Y var");
        assert_eq!(series[0].0.len(), 3);
        assert_eq!(series[1].0.len(), 3);
    }

    #[cfg(feature = "graphics")]
    #[test]
    fn build_series_group_makes_one_series_per_level() {
        use crate::dataset::{SasDataset, VarMeta};
        use crate::value::VarType;
        use polars::df;
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0],
            "y" => [1.0_f64, 2.0, 3.0, 4.0],
            "g" => ["a", "b", "a", "b"]
        ]
        .unwrap();
        let mk = |n: &str, t: VarType| VarMeta {
            name: n.into(),
            ty: t,
            length: 8,
            format: None,
            label: None,
        };
        let ds = SasDataset {
            df,
            vars: vec![
                mk("x", VarType::Num),
                mk("y", VarType::Num),
                mk("g", VarType::Char),
            ],
        };
        let ast = parse_gplot("proc gplot data=work.m; plot y*x=g; run;").unwrap();
        let series =
            graphics_impl::build_series(&ds, &ast.plots[0], &ast.symbols).unwrap();
        assert_eq!(series.len(), 2, "expected one series per group level");
        // 2 niveaux (a, b) avec 2 points chacun.
        assert_eq!(series[0].0.len(), 2);
        assert_eq!(series[1].0.len(), 2);
    }

    #[cfg(feature = "graphics")]
    #[test]
    fn symbol_interpol_join_makes_line() {
        use crate::graphics::render::SeriesColor;
        let sym = SymbolDef {
            interpol: Some("join".into()),
            value: None,
            color: Some("red".into()),
        };
        let (line, _marker) = graphics_impl::line_marker(Some(&sym));
        assert!(line, "INTERPOL=JOIN should yield a line");
        assert_eq!(
            graphics_impl::color_from_name("red"),
            Some(SeriesColor::Red)
        );
        // Sans SYMBOL : marqueurs, pas de ligne.
        let (line0, marker0) = graphics_impl::line_marker(None);
        assert!(!line0 && marker0);
    }

    #[cfg(feature = "graphics")]
    #[test]
    fn execute_multi_series_writes_image() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        session.ods_graphics.output_dir = std::env::temp_dir();
        session.ods_graphics.file_stem = Some("gplottest_multi".into());
        write_multi(&mut session, "M");
        let ast = parse_gplot(
            "proc gplot data=work.m; symbol1 interpol=join color=blue; symbol2 interpol=join color=red; axis1 order=(0 to 5) label=('X axis'); plot (y1 y2)*x; run;",
        )
        .unwrap();
        assert_eq!(ast.symbols.len(), 2);
        assert_eq!(ast.axes.len(), 1);
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("written"), "log: {log}");
        let p = std::env::temp_dir().join("gplottest_multi_1.png");
        assert!(p.exists(), "image not created: {p:?}");
        assert!(p.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parse_symbol_axis_captured() {
        let ast = parse_gplot(
            "proc gplot data=a; symbol1 i=join v=dot c=blue; axis1 order=(0 to 100 by 10) label=('Time'); plot y*x; run;",
        )
        .unwrap();
        assert_eq!(ast.symbols.len(), 1);
        assert_eq!(ast.symbols[0].interpol.as_deref(), Some("join"));
        assert_eq!(ast.symbols[0].value.as_deref(), Some("dot"));
        assert_eq!(ast.symbols[0].color.as_deref(), Some("blue"));
        assert_eq!(ast.axes.len(), 1);
        assert_eq!(ast.axes[0].order_min, Some(0.0));
        assert_eq!(ast.axes[0].order_max, Some(100.0));
        assert_eq!(ast.axes[0].label.as_deref(), Some("Time"));
    }
}
