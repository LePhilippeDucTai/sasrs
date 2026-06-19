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
    /// Statements PLOT. v1 n'en honore que le premier.
    pub plots: Vec<GplotStmt>,
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
        } else if ts.peek().is_kw("symbol")
            || ts.peek().is_kw("axis")
            || starts_with_kw(ts, "symbol")
            || starts_with_kw(ts, "axis")
        {
            // SYMBOL/AXIS dans le bloc : parsé sans erreur, ignoré en v1.
            ts.skip_to_semi();
        } else {
            ts.skip_to_semi();
        }
    }

    Ok(GplotAst { data_ref, plots })
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
mod graphics_impl {
    use super::*;
    use crate::graphics::render::{draw_to_file, DrawingSpec, PlotType};
    use crate::missing::value_to_num;
    use crate::ods_graphics::ImageFmt;
    use crate::procs::common::decode_column;
    use crate::value::VarType;

    /// Résout DATA= ou _LAST_ (calqué sur sgplot.rs).
    fn resolve_input(ast: &GplotAst, session: &Session) -> Result<DatasetRef> {
        match &ast.data_ref {
            Some(r) => Ok(r.clone()),
            None => {
                let last = session.last_dataset.clone().ok_or_else(|| {
                    SasError::runtime("There is no default input data set (_LAST_ is undefined).")
                })?;
                let parts: Vec<&str> = last.splitn(2, '.').collect();
                if parts.len() == 2 {
                    Ok(DatasetRef {
                        libref: Some(parts[0].to_string()),
                        name: parts[1].to_string(),
                    })
                } else {
                    Ok(DatasetRef {
                        libref: None,
                        name: last,
                    })
                }
            }
        }
    }

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

    pub fn render(ast: &GplotAst, stmt: &GplotStmt, session: &mut Session) -> Result<()> {
        let GplotStmt::Plot { y_vars, x_var, .. } = stmt;
        let y_var = y_vars
            .first()
            .ok_or_else(|| SasError::runtime("PLOT statement has no Y variable."))?;

        // Lire les données.
        let in_ref = resolve_input(ast, session)?;
        let in_libref = in_ref.libref_or_work();
        let in_table = in_ref.name.to_uppercase();
        let provider = session.libs.get(&in_libref)?;
        let (ds, notes) = provider.read(&in_table)?;
        for note in notes {
            session.log.forward(&note);
        }

        let xs = numeric_column(&ds, x_var)?;
        let ys = numeric_column(&ds, y_var)?;
        let data: Vec<(f64, f64)> = xs
            .iter()
            .zip(ys.iter())
            .filter(|(a, b)| a.is_finite() && b.is_finite())
            .map(|(a, b)| (*a, *b))
            .collect();

        let spec = DrawingSpec {
            title: "The GPLOT Procedure".to_string(),
            x_label: x_var.clone(),
            y_label: y_var.clone(),
            plot_type: PlotType::Scatter,
            data,
            x_categorical: vec![],
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
}
