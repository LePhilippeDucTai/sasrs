//! PROC GCHART — graphique "legacy" SAS/GRAPH (M30.1).
//!
//! PROC GCHART produit des diagrammes en barres (VBAR/HBAR) et des camemberts
//! (PIE) sur l'infrastructure ODS GRAPHICS (M29.1). Il précède les statements
//! VBAR/HBAR de PROC SGPLOT (M29.2).
//!
//! # Modèle d'exécution selon l'état
//!
//! - `ods_graphics.enabled == false` → NOTE de non-activation, EXIT 0.
//! - PIE → toujours différé (NOTE "PIE chart deferred in PROC GCHART.").
//! - VBAR/HBAR sans `--features graphics` → NOTE « image deferred ».
//! - VBAR/HBAR avec `--features graphics` → image `gchart_{N}.png`.
//!
//! Contrairement à GPLOT, GCHART itère sur TOUS les statements : un VBAR suivi
//! d'un PIE produit une image (ou un « image deferred ») PUIS la NOTE de
//! différé du PIE.
//!
//! # Invariant build par défaut
//!
//! Le code de rendu est sous `#[cfg(feature = "graphics")]` ; les champs lus
//! uniquement par ce code sont annotés
//! `#[cfg_attr(not(feature = "graphics"), allow(dead_code))]`.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct GchartAst {
    /// `DATA=` ; `None` → `_LAST_`.
    pub data_ref: Option<DatasetRef>,
    /// Statements de diagramme dans l'ordre d'apparition.
    pub charts: Vec<GchartStmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GchartStmt {
    VBar {
        category: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        sumvar: Option<String>,
        chart_type: ChartType,
    },
    HBar {
        category: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        sumvar: Option<String>,
        chart_type: ChartType,
    },
    /// PIE : différé (NOTE) en v1.
    Pie {
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        category: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartType {
    Freq,
    Sum,
    Mean,
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

/// Lit une valeur (string, ident ou nombre) après un `=`.
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

/// Parse les options après `/` d'un statement VBAR/HBAR :
/// `sumvar=var`, `type=freq|sum|mean`. Renvoie `(sumvar, chart_type)`.
///
/// Règle SAS : `SUMVAR=` sans `TYPE=` implique `TYPE=SUM`.
fn parse_bar_options(ts: &mut StatementStream) -> Result<(Option<String>, ChartType)> {
    let mut sumvar: Option<String> = None;
    let mut explicit_type: Option<ChartType> = None;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
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
                "sumvar" => {
                    if ts.peek().kind == TokenKind::Eq {
                        ts.next();
                    }
                    sumvar = Some(expect_ident(ts, "after SUMVAR=")?);
                }
                "type" => {
                    if ts.peek().kind == TokenKind::Eq {
                        ts.next();
                    }
                    let t = expect_ident(ts, "after TYPE=")?;
                    explicit_type = Some(match t.to_ascii_lowercase().as_str() {
                        "sum" => ChartType::Sum,
                        "mean" => ChartType::Mean,
                        _ => ChartType::Freq,
                    });
                }
                _ => {
                    if ts.peek().kind == TokenKind::Eq {
                        ts.next();
                        let _ = read_value(ts);
                    }
                }
            }
        }
    }

    let chart_type = explicit_type.unwrap_or(if sumvar.is_some() {
        ChartType::Sum
    } else {
        ChartType::Freq
    });
    Ok((sumvar, chart_type))
}

/// Parse PROC GCHART. Appelé APRÈS consommation de `proc gchart`.
pub fn parse(ts: &mut StatementStream) -> Result<GchartAst> {
    let mut data_ref: Option<DatasetRef> = None;

    // Options du statement PROC GCHART, jusqu'au `;`.
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
            ts.next();
        }
    }

    let mut charts: Vec<GchartStmt> = Vec::new();

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

        if ts.peek().is_kw("vbar") || ts.peek().is_kw("vbar3d") {
            ts.next();
            let category = expect_ident(ts, "after VBAR")?;
            let (sumvar, chart_type) = parse_bar_options(ts)?;
            ts.expect_semi()?;
            charts.push(GchartStmt::VBar {
                category,
                sumvar,
                chart_type,
            });
        } else if ts.peek().is_kw("hbar") || ts.peek().is_kw("hbar3d") {
            ts.next();
            let category = expect_ident(ts, "after HBAR")?;
            let (sumvar, chart_type) = parse_bar_options(ts)?;
            ts.expect_semi()?;
            charts.push(GchartStmt::HBar {
                category,
                sumvar,
                chart_type,
            });
        } else if ts.peek().is_kw("pie") || ts.peek().is_kw("pie3d") {
            ts.next();
            let category = expect_ident(ts, "after PIE")?;
            ts.skip_to_semi();
            charts.push(GchartStmt::Pie { category });
        } else {
            ts.skip_to_semi();
        }
    }

    Ok(GchartAst { data_ref, charts })
}

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &GchartAst, session: &mut Session) -> Result<()> {
    // 1) ODS GRAPHICS non activé → NOTE de non-activation, EXIT 0.
    if !session.ods_graphics.enabled {
        session.log.note(
            "ODS GRAPHICS is not enabled. Use \"ods graphics on;\" before PROC GCHART to generate images.",
        );
        return Ok(());
    }

    // 2) Aucun statement : rien à dessiner.
    if ast.charts.is_empty() {
        session
            .log
            .note("No chart statement found in PROC GCHART; nothing to plot.");
        return Ok(());
    }

    // 3) Itérer sur TOUS les statements (contrairement à GPLOT).
    for chart in &ast.charts {
        match chart {
            GchartStmt::Pie { .. } => {
                session.log.note("PIE chart deferred in PROC GCHART.");
            }
            GchartStmt::VBar { .. } | GchartStmt::HBar { .. } => {
                #[cfg(not(feature = "graphics"))]
                {
                    session.log.note(
                        "ODS GRAPHICS: image deferred (compile with --features graphics).",
                    );
                }
                #[cfg(feature = "graphics")]
                {
                    graphics_impl::render(ast, chart, session)?;
                }
            }
        }
    }

    Ok(())
}

// ───────────────────────── Rendu (feature graphics) ─────────────────────────

#[cfg(feature = "graphics")]
mod graphics_impl {
    use super::*;
    use crate::graphics::render::{draw_to_file, DrawingSpec, PlotType};
    use crate::missing::value_to_num;
    use crate::ods_graphics::ImageFmt;
    use crate::procs::common::decode_column;
    use crate::value::{Value, VarType};
    use std::collections::BTreeMap;

    /// Résout DATA= ou _LAST_.
    fn resolve_input(ast: &GchartAst, session: &Session) -> Result<DatasetRef> {
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

    fn category_key(v: &Value) -> String {
        match v {
            Value::Char(s) => s.clone(),
            Value::Num(n) => format!("{n}"),
            Value::Missing(_) => ".".to_string(),
        }
    }

    /// Agrège les valeurs (catégorie, statistique) selon `chart_type`.
    fn aggregate(
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
                        SasError::runtime(format!(
                            "Variable {} not found.",
                            resp_name.to_uppercase()
                        ))
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
        let (category, sumvar, chart_type) = match chart {
            GchartStmt::VBar {
                category,
                sumvar,
                chart_type,
            }
            | GchartStmt::HBar {
                category,
                sumvar,
                chart_type,
            } => (category, sumvar, *chart_type),
            GchartStmt::Pie { .. } => return Ok(()),
        };

        let in_ref = resolve_input(ast, session)?;
        let in_libref = in_ref.libref_or_work();
        let in_table = in_ref.name.to_uppercase();
        let provider = session.libs.get(&in_libref)?;
        let (ds, notes) = provider.read(&in_table)?;
        for note in notes {
            session.log.forward(&note);
        }

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
            plot_type: PlotType::VBar,
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
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceFile;

    fn parse_gchart(src: &str) -> Result<GchartAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // gchart
        parse(&mut ts)
    }

    #[allow(dead_code)]
    fn make_session() -> Session {
        Session::new(None, std::env::temp_dir(), true).unwrap()
    }

    // ── Parse tests ──────────────────────────────────────────────────────

    #[test]
    fn parse_vbar_freq() {
        let ast = parse_gchart("proc gchart data=a; vbar category; run;").unwrap();
        assert_eq!(ast.charts.len(), 1);
        match &ast.charts[0] {
            GchartStmt::VBar {
                category,
                sumvar,
                chart_type,
            } => {
                assert_eq!(category, "category");
                assert!(sumvar.is_none());
                assert_eq!(*chart_type, ChartType::Freq);
            }
            other => panic!("expected VBar, got {other:?}"),
        }
    }

    #[test]
    fn parse_vbar_sumvar_implies_sum() {
        let ast = parse_gchart("proc gchart data=a; vbar category / sumvar=count; run;").unwrap();
        match &ast.charts[0] {
            GchartStmt::VBar {
                sumvar,
                chart_type,
                ..
            } => {
                assert_eq!(sumvar.as_deref(), Some("count"));
                assert_eq!(*chart_type, ChartType::Sum);
            }
            other => panic!("expected VBar, got {other:?}"),
        }
    }

    #[test]
    fn parse_vbar_type_mean() {
        let ast = parse_gchart("proc gchart data=a; vbar category / type=mean; run;").unwrap();
        match &ast.charts[0] {
            GchartStmt::VBar { chart_type, .. } => assert_eq!(*chart_type, ChartType::Mean),
            other => panic!("expected VBar, got {other:?}"),
        }
    }

    #[test]
    fn parse_hbar() {
        let ast = parse_gchart("proc gchart data=a; hbar category; run;").unwrap();
        match &ast.charts[0] {
            GchartStmt::HBar { category, .. } => assert_eq!(category, "category"),
            other => panic!("expected HBar, got {other:?}"),
        }
    }

    #[test]
    fn parse_pie() {
        let ast = parse_gchart("proc gchart data=a; pie category; run;").unwrap();
        assert!(matches!(ast.charts[0], GchartStmt::Pie { .. }));
    }

    // ── Execute tests (default build) ────────────────────────────────────

    #[test]
    fn execute_without_ods_on_notes_not_enabled() {
        let mut session = make_session();
        let ast = parse_gchart("proc gchart data=a; vbar category; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(
            log.contains("ODS GRAPHICS is not enabled") && log.contains("PROC GCHART"),
            "log: {log}"
        );
    }

    #[test]
    fn execute_pie_defers() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        let ast = parse_gchart("proc gchart data=a; pie category; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("PIE chart deferred in PROC GCHART."), "log: {log}");
    }

    #[cfg(not(feature = "graphics"))]
    #[test]
    fn execute_vbar_with_ods_on_no_feature_defers() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        let ast = parse_gchart("proc gchart data=a; vbar category; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("image deferred"), "log: {log}");
    }

    // ── Execute tests (feature graphics) ─────────────────────────────────

    #[cfg(feature = "graphics")]
    fn write_cats(session: &mut Session, table: &str) {
        use crate::dataset::{SasDataset, VarMeta};
        use crate::value::VarType;
        use polars::df;
        let df = df![
            "category" => ["A", "B", "C", "D"],
            "count" => [10.0_f64, 25.0, 15.0, 30.0]
        ]
        .unwrap();
        let vars = vec![
            VarMeta {
                name: "category".into(),
                ty: VarType::Char,
                length: 1,
                format: None,
                label: None,
            },
            VarMeta {
                name: "count".into(),
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
    fn execute_vbar_with_graphics_writes_image() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        session.ods_graphics.output_dir = std::env::temp_dir();
        session.ods_graphics.file_stem = Some("gcharttest_single".into());
        write_cats(&mut session, "CATS");
        let ast =
            parse_gchart("proc gchart data=work.cats; vbar category / sumvar=count; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("written"), "log: {log}");
        let p = std::env::temp_dir().join("gcharttest_single_1.png");
        assert!(p.exists(), "image not created: {p:?}");
        assert!(p.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&p);
    }
}
