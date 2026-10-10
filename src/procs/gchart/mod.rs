//! PROC GCHART — graphique "legacy" SAS/GRAPH (M30.1).
//!
//! PROC GCHART produit des diagrammes en barres (VBAR/HBAR) et des camemberts
//! (PIE) sur l'infrastructure ODS GRAPHICS (M29.1). Il précède les statements
//! VBAR/HBAR de PROC SGPLOT (M29.2).
//!
//! # Modèle d'exécution selon l'état
//!
//! - `ods_graphics.enabled == false` → NOTE de non-activation, EXIT 0.
//! - PIE sans `--features graphics` → NOTE « image deferred » ; avec
//!   `--features graphics`, le camembert est rendu (`gchart_{N}.png`).
//! - VBAR/HBAR sans `--features graphics` → NOTE « image deferred ».
//! - VBAR/HBAR avec `--features graphics` → image `gchart_{N}.png`
//!   (HBAR est dessiné comme un diagramme en barres verticales : le rendu
//!   horizontal n'est pas implémenté ; WARNING depuis J02-P6, levé par J13-P5).
//!
//! # Contrat de support (J02-P6)
//!
//! Les options non dessinées (TYPE=PERCENT|CFREQ|CPERCENT, SUBGROUP=, GROUP=,
//! MIDPOINTS=…), HBAR vertical, les variantes 3D dessinées à plat et les
//! instructions de décoration donnent un WARNING ; BY, GOUT=, IMAGEMAP= et
//! les diagrammes non implémentés une ERROR ; `DATA=` et les variables sont
//! validés dans les deux builds.
//!
//! Contrairement à GPLOT, GCHART itère sur TOUS les statements : un VBAR suivi
//! d'un PIE produit deux images (ou deux « image deferred ») successives.
//!
//! # Invariant build par défaut
//!
//! Le code de rendu est sous `#[cfg(feature = "graphics")]` ; les champs lus
//! uniquement par ce code sont annotés
//! `#[cfg_attr(not(feature = "graphics"), allow(dead_code))]`.

use crate::ast::DatasetRef;
use crate::error::Result;
use crate::ods_graphics::contract;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::expect_ident;
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

// ───────────────────────── Contract (J02-P6) ─────────────────────────
//
// SAS/GRAPH 9.4 Reference, The GCHART Procedure. TYPE=PERCENT|CFREQ|CPERCENT
// fell back to FREQ, SUBGROUP=/GROUP=/MIDPOINTS= and the other chart options
// were swallowed, HBAR was drawn vertically and the 3D charts in two
// dimensions, the PROC options skipped token by token (audit d0b4d90).
// CONTRIBUTING §5: a display option that is not rendered is a WARNING; an
// option that creates an output, BY and the chart statements that are not
// implemented are ERRORs (step rejected).

/// PROC GCHART statement options limited to the drawn image.
const DISPLAY_PROC_OPTIONS: &[&str] = &["annotate", "anno"];

/// PROC GCHART statement options that create an output: GOUT= (graphics
/// catalog) and IMAGEMAP= (data set).
const OUTPUT_PROC_OPTIONS: &[&str] = &["gout", "imagemap"];

/// Valid PROC GCHART chart statements that are not implemented.
const UNSUPPORTED_STATEMENTS: &[&str] = &["block", "donut", "star"];

/// True for `prefix` or `prefixN` (`axis1`, `legend2`, `pattern12`…).
fn is_numbered(kw: &str, prefix: &str) -> bool {
    kw.strip_prefix(prefix)
        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

/// SAS/GRAPH global statements valid in the step that only decorate the
/// chart (AXISn, LEGENDn, PATTERNn, GOPTIONS, NOTE): display WARNING. They
/// used to be rejected as « 180-322 … not valid ».
fn is_display_statement(kw: &str) -> bool {
    kw == "goptions"
        || kw == "note"
        || is_numbered(kw, "axis")
        || is_numbered(kw, "legend")
        || is_numbered(kw, "pattern")
}

// ───────────────────────── Parser ─────────────────────────

/// Parse les options après `/` d'un statement VBAR/HBAR/PIE (`stmt` : nom du
/// statement pour les diagnostics) : `sumvar=var`, `type=freq|sum|mean`.
/// Renvoie `(sumvar, chart_type)`.
///
/// Règle SAS : `SUMVAR=` sans `TYPE=` implique `TYPE=SUM`. J02-P6 :
/// TYPE=PERCENT|CFREQ|CPERCENT (et une valeur inconnue) retombaient en
/// silence sur FREQ, SUBGROUP=, GROUP=, MIDPOINTS= et les autres options
/// étaient avalés : WARNING par option (le diagramme FREQ est dessiné).
fn parse_bar_options(ts: &mut StatementStream, stmt: &str) -> Result<(Option<String>, ChartType)> {
    let mut sumvar: Option<String> = None;
    let mut explicit_type: Option<ChartType> = None;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let name = match ts.peek().ident().map(|s| s.to_ascii_lowercase()) {
                Some(n) => n,
                None => return Err(contract::expected_option(ts, "GCHART", stmt)),
            };
            match name.as_str() {
                "sumvar" if ts.peek2().kind == TokenKind::Eq => {
                    ts.next();
                    ts.next();
                    sumvar = Some(expect_ident(ts, "after SUMVAR=")?);
                }
                "type" if ts.peek2().kind == TokenKind::Eq => {
                    ts.next();
                    ts.next();
                    let t = expect_ident(ts, "after TYPE=")?;
                    explicit_type = Some(match t.to_ascii_lowercase().as_str() {
                        "freq" => ChartType::Freq,
                        "sum" => ChartType::Sum,
                        "mean" => ChartType::Mean,
                        _ => {
                            ts.warn_ignored_display(contract::ignored_option(
                                "GCHART",
                                Some(stmt),
                                &format!("TYPE={}", t.to_ascii_uppercase()),
                            ));
                            ChartType::Freq
                        }
                    });
                }
                _ => contract::warn_option(ts, "GCHART", Some(stmt)),
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

    // Options du statement PROC GCHART, jusqu'au `;` (J02-P6 : plus de saut
    // silencieux ; option inconnue → « Unexpected option »).
    common::parse_proc_options(ts, "GCHART", |ts, kw| {
        if kw == "data" {
            data_ref = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if DISPLAY_PROC_OPTIONS.contains(&kw) {
            contract::warn_option(ts, "GCHART", None);
        } else if OUTPUT_PROC_OPTIONS.contains(&kw) {
            return Err(contract::unsupported_option(
                "GCHART",
                &contract::option_label(ts),
                ts.peek().span,
            ));
        } else {
            return Ok(false);
        }
        Ok(true)
    })?;

    let mut charts: Vec<GchartStmt> = Vec::new();

    common::parse_proc_body(ts, "GCHART", |ts, kw| {
        let stmt = kw.to_ascii_uppercase();
        let chart = match kw {
            "vbar" | "vbar3d" | "hbar" | "hbar3d" | "pie" | "pie3d" => {
                if let Some(flat) = stmt.strip_suffix("3D") {
                    // J02-P6 — the 3D chart was drawn flat without a word.
                    ts.warn_ignored_display(format!(
                        "The {stmt} statement is drawn as a two-dimensional {flat} chart in \
                         PROC GCHART; display customization is not supported."
                    ));
                }
                if kw.starts_with("hbar") {
                    // J02-P6 — the engine draws vertical bars.
                    ts.warn_ignored_display(format!(
                        "The {stmt} statement is drawn as a vertical bar chart in PROC GCHART; \
                         horizontal bars are not supported{}.",
                        contract::planned("J13-P5")
                    ));
                }
                ts.next();
                let category = expect_ident(ts, &format!("after {stmt}"))?;
                let (sumvar, chart_type) = parse_bar_options(ts, &stmt)?;
                ts.expect_semi()?;
                match kw {
                    "vbar" | "vbar3d" => GchartStmt::VBar {
                        category,
                        sumvar,
                        chart_type,
                    },
                    "hbar" | "hbar3d" => GchartStmt::HBar {
                        category,
                        sumvar,
                        chart_type,
                    },
                    _ => GchartStmt::Pie {
                        category,
                        sumvar,
                        chart_type,
                    },
                }
            }
            // Same contract ERROR as SGPLOT/GPLOT/PLOT until J13-P4.
            "by" => return Err(contract::by_not_supported("GCHART", ts.peek().span)),
            _ if UNSUPPORTED_STATEMENTS.contains(&kw) => {
                return Err(contract::unsupported_statement(
                    "GCHART",
                    kw,
                    None,
                    ts.peek().span,
                ));
            }
            _ if is_display_statement(kw) => {
                ts.warn_ignored_display(common::ignored_display_statement("GCHART", kw));
                ts.skip_to_semi();
                return Ok(true);
            }
            _ => return Ok(false),
        };
        charts.push(chart);
        Ok(true)
    })?;

    Ok(GchartAst { data_ref, charts })
}

// ───────────────────────── Execute ─────────────────────────

/// Variables named by the chart statements, for the `DATA=` check.
fn referenced_vars(ast: &GchartAst) -> Vec<&str> {
    let mut vars: Vec<&str> = Vec::new();
    for chart in &ast.charts {
        let (GchartStmt::VBar {
            category, sumvar, ..
        }
        | GchartStmt::HBar {
            category, sumvar, ..
        }
        | GchartStmt::Pie {
            category, sumvar, ..
        }) = chart;
        vars.push(category);
        vars.extend(sumvar.as_deref());
    }
    vars
}

pub fn execute(ast: &GchartAst, session: &mut Session) -> Result<()> {
    // 0) J02-P6 — DATA= et variables validées dans les deux builds : le build
    //    par défaut n'ouvrait jamais la table (« image deferred », code 0,
    //    même pour une table ou une variable absente).
    contract::open_checked(&ast.data_ref, session, &referenced_vars(ast))?;

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

#[cfg(test)]
mod contract_tests;
