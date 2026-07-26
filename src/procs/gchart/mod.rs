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
use crate::error::Result;
use crate::parser::StatementStream;
use crate::procs::common::{expect_ident, read_value};
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
    /// PIE cat / SUMVAR= TYPE=FREQ|SUM|MEAN.
    Pie {
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        category: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        sumvar: Option<String>,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        chart_type: ChartType,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartType {
    Freq,
    Sum,
    Mean,
}

// ───────────────────────── Parser ─────────────────────────

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
            let (sumvar, chart_type) = parse_bar_options(ts)?;
            ts.expect_semi()?;
            charts.push(GchartStmt::Pie {
                category,
                sumvar,
                chart_type,
            });
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
                // PIE : différé dans le build par défaut, rendu sous --features
                // graphics (M34.11). NOTE par défaut byte-identique.
                #[cfg(not(feature = "graphics"))]
                {
                    session.log.note("PIE chart deferred in PROC GCHART.");
                }
                #[cfg(feature = "graphics")]
                {
                    graphics_impl::render(ast, chart, session)?;
                }
            }
            GchartStmt::VBar { .. } | GchartStmt::HBar { .. } => {
                #[cfg(not(feature = "graphics"))]
                {
                    session
                        .log
                        .note("ODS GRAPHICS: image deferred (compile with --features graphics).");
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
pub(crate) mod graphics_impl;
// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
