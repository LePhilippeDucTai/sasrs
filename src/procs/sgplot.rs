//! PROC SGPLOT — graphique statistique (M29.2).
//!
//! PROC SGPLOT lit un dataset et produit UNE image (PNG ou SVG) via
//! l'infrastructure ODS GRAPHICS (M29.1, module [`crate::graphics::render`]).
//!
//! # Modèle d'exécution selon l'état
//!
//! - `ods_graphics.enabled == false` → NOTE de non-activation, EXIT 0.
//! - `enabled == true` mais build par défaut (sans `--features graphics`) →
//!   NOTE « image deferred », EXIT 0.
//! - `enabled == true` + `--features graphics` → l'image est matérialisée et la
//!   NOTE « Output '...' (WxH) written. » est émise.
//!
//! Les fonctions complexes (LOESS, DENSITY) et le traitement BY-group sont
//! PARSÉS sans erreur mais DIFFÉRÉS à l'exécution (NOTE seulement), de sorte que
//! la grammaire reste tolérante sans bloquer le programme.
//!
//! # Invariant build par défaut
//!
//! Tout le code de génération d'image est sous `#[cfg(feature = "graphics")]`.
//! Les champs de l'AST qui ne sont consultés que par ce code sont annotés
//! `#[cfg_attr(not(feature = "graphics"), allow(dead_code))]` pour préserver
//! l'invariant « 0 warning » du build par défaut.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::procs::common;
use crate::session::Session;
use crate::token::TokenKind;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct SgplotAst {
    /// `DATA=` ; `None` → `_LAST_`.
    pub data_ref: Option<DatasetRef>,
    /// Statements de tracé (SCATTER, SERIES, …). v1 n'en honore que le premier.
    pub plot_stmts: Vec<SgplotStmt>,
    /// Options XAXIS (parsées ; appliquées partiellement en v1).
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub xaxis: Option<AxisOpts>,
    /// Options YAXIS.
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub yaxis: Option<AxisOpts>,
    /// `BY var` — traitement par groupe (différé en v1).
    pub by_var: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SgplotStmt {
    Scatter {
        x: String,
        y: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        group: Option<String>,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        markerattrs: Option<MarkerAttrs>,
    },
    Series {
        x: String,
        y: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        group: Option<String>,
    },
    VBar {
        category: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        response: Option<String>,
        stat: BarStat,
    },
    HBar {
        category: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        response: Option<String>,
        stat: BarStat,
    },
    Histogram {
        var: String,
        binwidth: Option<f64>,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        scale: HistScale,
    },
    Density {
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        var: String,
    },
    VBox {
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        category: Option<String>,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        response: String,
    },
    Reg {
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        x: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        y: String,
        degree: u32,
    },
    Loess {
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        x: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        y: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        smooth: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarStat {
    Freq,
    Sum,
    Mean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistScale {
    Count,
    Percent,
    Proportion,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AxisOpts {
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub label: Option<String>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub values_min: Option<f64>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub values_max: Option<f64>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub type_: Option<AxisType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisType {
    Linear,
    Log,
    Discrete,
}

/// MARKERATTRS=(SYMBOL= COLOR= SIZE=) — parsé puis ignoré en v1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerAttrs {
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub symbol: Option<String>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub color: Option<String>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub size: Option<String>,
}

// ───────────────────────── Parser helpers ─────────────────────────

fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("expected '=' after {opt}"),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

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

/// Lit une valeur numérique littérale. Erreur propre sinon.
fn expect_number(ts: &mut StatementStream, ctx: &str) -> Result<f64> {
    match ts.peek().kind {
        TokenKind::Num(f) => {
            ts.next();
            Ok(f)
        }
        _ => Err(SasError::parse(
            format!("expected a number {ctx}"),
            ts.peek().span,
        )),
    }
}

/// Lit une valeur de chaîne (string littérale ou identifiant).
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

/// Parse une liste parenthésée d'options `name=value` (MARKERATTRS, LINEATTRS).
/// Consomme `( ... )`. Renvoie les paires sous forme brute (UPPERCASE name).
fn parse_paren_attrs(ts: &mut StatementStream) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if ts.peek().kind != TokenKind::LParen {
        return out;
    }
    ts.next(); // (
    loop {
        match &ts.peek().kind {
            TokenKind::RParen => {
                ts.next();
                break;
            }
            TokenKind::Eof | TokenKind::Semi => break,
            _ => {}
        }
        let name = match ts.peek().ident().map(|s| s.to_ascii_lowercase()) {
            Some(n) => {
                ts.next();
                n
            }
            None => {
                ts.next();
                continue;
            }
        };
        if ts.peek().kind == TokenKind::Eq {
            ts.next();
            if let Some(v) = read_value(ts) {
                out.push((name, v));
            }
        }
    }
    out
}

/// Parse les options X=var Y=var et les options après `/` d'un statement de
/// tracé à deux variables (SCATTER, SERIES, REG, LOESS). Renvoie
/// `(x, y, group, markerattrs, degree, smooth)`. Les inconnues sont ignorées.
fn parse_xy_stmt(
    ts: &mut StatementStream,
) -> Result<(String, String, Option<String>, Option<MarkerAttrs>, Option<u32>, Option<f64>)> {
    let mut x: Option<String> = None;
    let mut y: Option<String> = None;
    let mut group: Option<String> = None;
    let mut markerattrs: Option<MarkerAttrs> = None;
    let mut degree: Option<u32> = None;
    let mut smooth: Option<f64> = None;

    // Args avant le `/`.
    while ts.peek().kind != TokenKind::Semi
        && ts.peek().kind != TokenKind::Slash
        && ts.peek().kind != TokenKind::Eof
    {
        let name = match ts.peek().ident().map(|s| s.to_ascii_lowercase()) {
            Some(n) => n,
            None => {
                ts.next();
                continue;
            }
        };
        match name.as_str() {
            "x" => {
                common::expect_eq(ts, "X")?;
                x = Some(expect_ident(ts, "after X=")?);
            }
            "y" => {
                common::expect_eq(ts, "Y")?;
                y = Some(expect_ident(ts, "after Y=")?);
            }
            _ => {
                ts.next();
            }
        }
    }

    // Options après le `/`.
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
            ts.next(); // consume option name
            match name.as_str() {
                "group" => {
                    expect_eq(ts, "GROUP")?;
                    group = Some(expect_ident(ts, "after GROUP=")?);
                }
                "markerattrs" => {
                    expect_eq(ts, "MARKERATTRS")?;
                    let attrs = parse_paren_attrs(ts);
                    let mut m = MarkerAttrs {
                        symbol: None,
                        color: None,
                        size: None,
                    };
                    for (k, v) in attrs {
                        match k.as_str() {
                            "symbol" => m.symbol = Some(v),
                            "color" => m.color = Some(v),
                            "size" => m.size = Some(v),
                            _ => {}
                        }
                    }
                    markerattrs = Some(m);
                }
                "lineattrs" => {
                    expect_eq(ts, "LINEATTRS")?;
                    let _ = parse_paren_attrs(ts);
                }
                "degree" => {
                    expect_eq(ts, "DEGREE")?;
                    degree = Some(expect_number(ts, "after DEGREE=")? as u32);
                }
                "smooth" => {
                    expect_eq(ts, "SMOOTH")?;
                    smooth = Some(expect_number(ts, "after SMOOTH=")?);
                }
                // Options à valeur (ex. NAME=) : consommer `= valeur` si présents.
                _ => {
                    if ts.peek().kind == TokenKind::Eq {
                        ts.next();
                        // Valeur simple ou parenthésée.
                        if ts.peek().kind == TokenKind::LParen {
                            let _ = parse_paren_attrs(ts);
                        } else {
                            let _ = read_value(ts);
                        }
                    }
                    // Sinon : flag booléen (NOAUTOLEGEND, …) — déjà consommé.
                }
            }
        }
    }

    let x = x.ok_or_else(|| SasError::parse("missing X= in plot statement", ts.peek().span))?;
    let y = y.ok_or_else(|| SasError::parse("missing Y= in plot statement", ts.peek().span))?;
    Ok((x, y, group, markerattrs, degree, smooth))
}

/// Parse un statement de barres (VBAR/HBAR) : `vbar category / response= stat=`.
fn parse_bar_stmt(ts: &mut StatementStream) -> Result<(String, Option<String>, BarStat)> {
    let category = expect_ident(ts, "after VBAR/HBAR")?;
    let mut response: Option<String> = None;
    let mut stat = BarStat::Freq;
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
                "response" => {
                    expect_eq(ts, "RESPONSE")?;
                    response = Some(expect_ident(ts, "after RESPONSE=")?);
                }
                "stat" => {
                    expect_eq(ts, "STAT")?;
                    let s = expect_ident(ts, "after STAT=")?;
                    stat = match s.to_ascii_lowercase().as_str() {
                        "sum" => BarStat::Sum,
                        "mean" => BarStat::Mean,
                        _ => BarStat::Freq,
                    };
                }
                _ => {
                    if ts.peek().kind == TokenKind::Eq {
                        ts.next();
                        if ts.peek().kind == TokenKind::LParen {
                            let _ = parse_paren_attrs(ts);
                        } else {
                            let _ = read_value(ts);
                        }
                    }
                }
            }
        }
    }
    Ok((category, response, stat))
}

/// Parse un statement HISTOGRAM : `histogram var / binwidth= scale=`.
fn parse_histogram_stmt(ts: &mut StatementStream) -> Result<(String, Option<f64>, HistScale)> {
    let var = expect_ident(ts, "after HISTOGRAM")?;
    let mut binwidth: Option<f64> = None;
    let mut scale = HistScale::Count;
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
                "binwidth" => {
                    expect_eq(ts, "BINWIDTH")?;
                    binwidth = Some(expect_number(ts, "after BINWIDTH=")?);
                }
                "scale" => {
                    expect_eq(ts, "SCALE")?;
                    let s = expect_ident(ts, "after SCALE=")?;
                    scale = match s.to_ascii_lowercase().as_str() {
                        "percent" => HistScale::Percent,
                        "proportion" => HistScale::Proportion,
                        _ => HistScale::Count,
                    };
                }
                _ => {
                    if ts.peek().kind == TokenKind::Eq {
                        ts.next();
                        if ts.peek().kind == TokenKind::LParen {
                            let _ = parse_paren_attrs(ts);
                        } else {
                            let _ = read_value(ts);
                        }
                    }
                }
            }
        }
    }
    Ok((var, binwidth, scale))
}

/// Parse un statement AXIS (XAXIS/YAXIS) : `xaxis label='..' values=(..) type=`.
fn parse_axis_stmt(ts: &mut StatementStream) -> Result<AxisOpts> {
    let mut opts = AxisOpts {
        label: None,
        values_min: None,
        values_max: None,
        type_: None,
    };
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
            "label" => {
                expect_eq(ts, "LABEL")?;
                opts.label = read_value(ts);
            }
            "type" => {
                expect_eq(ts, "TYPE")?;
                let t = expect_ident(ts, "after TYPE=")?;
                opts.type_ = Some(match t.to_ascii_lowercase().as_str() {
                    "log" => AxisType::Log,
                    "discrete" => AxisType::Discrete,
                    _ => AxisType::Linear,
                });
            }
            "values" => {
                expect_eq(ts, "VALUES")?;
                // VALUES=(min to max by step) ou (v1 v2 ...).
                if ts.peek().kind == TokenKind::LParen {
                    ts.next();
                    let mut nums: Vec<f64> = Vec::new();
                    while ts.peek().kind != TokenKind::RParen && ts.peek().kind != TokenKind::Eof {
                        match ts.peek().kind {
                            TokenKind::Num(f) => {
                                nums.push(f);
                                ts.next();
                            }
                            _ => {
                                // `to`, `by` ou autres mots-clés : on ignore mais
                                // on garde la trace min/max via les nombres lus.
                                ts.next();
                            }
                        }
                    }
                    if ts.peek().kind == TokenKind::RParen {
                        ts.next();
                    }
                    if let Some(&mn) = nums.first() {
                        opts.values_min = Some(mn);
                    }
                    if nums.len() >= 2 {
                        // 2e nombre = max pour (min to max [by step]).
                        opts.values_max = Some(nums[1]);
                    }
                }
            }
            _ => {
                if ts.peek().kind == TokenKind::Eq {
                    ts.next();
                    if ts.peek().kind == TokenKind::LParen {
                        // Sauter le bloc parenthésé.
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
    Ok(opts)
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC SGPLOT. Appelé APRÈS consommation de `proc sgplot`.
pub fn parse(ts: &mut StatementStream) -> Result<SgplotAst> {
    let mut data_ref: Option<DatasetRef> = None;

    // Options du statement PROC SGPLOT, jusqu'au `;`.
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            data_ref = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else {
            ts.next(); // ignorer les options PROC inconnues
        }
    }

    let mut plot_stmts: Vec<SgplotStmt> = Vec::new();
    let mut xaxis: Option<AxisOpts> = None;
    let mut yaxis: Option<AxisOpts> = None;
    let mut by_var: Option<String> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "scatter" => {
                ts.next();
                let (x, y, group, markerattrs, _, _) = parse_xy_stmt(ts)?;
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::Scatter {
                    x,
                    y,
                    group,
                    markerattrs,
                });
                true
            }
            "series" => {
                ts.next();
                let (x, y, group, _, _, _) = parse_xy_stmt(ts)?;
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::Series { x, y, group });
                true
            }
            "reg" => {
                ts.next();
                let (x, y, _, _, degree, _) = parse_xy_stmt(ts)?;
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::Reg {
                    x,
                    y,
                    degree: degree.unwrap_or(1),
                });
                true
            }
            "loess" => {
                ts.next();
                let (x, y, _, _, _, smooth) = parse_xy_stmt(ts)?;
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::Loess {
                    x,
                    y,
                    smooth: smooth.unwrap_or(0.5),
                });
                true
            }
            "vbar" => {
                ts.next();
                let (category, response, stat) = parse_bar_stmt(ts)?;
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::VBar {
                    category,
                    response,
                    stat,
                });
                true
            }
            "hbar" => {
                ts.next();
                let (category, response, stat) = parse_bar_stmt(ts)?;
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::HBar {
                    category,
                    response,
                    stat,
                });
                true
            }
            "histogram" => {
                ts.next();
                let (var, binwidth, scale) = parse_histogram_stmt(ts)?;
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::Histogram {
                    var,
                    binwidth,
                    scale,
                });
                true
            }
            "density" => {
                ts.next();
                let var = expect_ident(ts, "after DENSITY")?;
                ts.skip_to_semi();
                plot_stmts.push(SgplotStmt::Density { var });
                true
            }
            "vbox" => {
                ts.next();
                let response = expect_ident(ts, "after VBOX")?;
                let mut category: Option<String> = None;
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
                        if name == "category" {
                            expect_eq(ts, "CATEGORY")?;
                            category = Some(expect_ident(ts, "after CATEGORY=")?);
                        } else if ts.peek().kind == TokenKind::Eq {
                            ts.next();
                            let _ = read_value(ts);
                        }
                    }
                }
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::VBox { category, response });
                true
            }
            "xaxis" => {
                ts.next();
                xaxis = Some(parse_axis_stmt(ts)?);
                ts.expect_semi()?;
                true
            }
            "yaxis" => {
                ts.next();
                yaxis = Some(parse_axis_stmt(ts)?);
                ts.expect_semi()?;
                true
            }
            "by" => {
                ts.next();
                by_var = ts.peek().ident().map(str::to_string);
                ts.skip_to_semi();
                true
            }
            _ => false,
        })
    })?;

    Ok(SgplotAst {
        data_ref,
        plot_stmts,
        xaxis,
        yaxis,
        by_var,
    })
}

// ───────────────────────── Execute ─────────────────────────

/// Nom lisible d'un statement de tracé, pour les NOTE.
fn stmt_kind(stmt: &SgplotStmt) -> &'static str {
    match stmt {
        SgplotStmt::Scatter { .. } => "SCATTER",
        SgplotStmt::Series { .. } => "SERIES",
        SgplotStmt::VBar { .. } => "VBAR",
        SgplotStmt::HBar { .. } => "HBAR",
        SgplotStmt::Histogram { .. } => "HISTOGRAM",
        SgplotStmt::Density { .. } => "DENSITY",
        SgplotStmt::VBox { .. } => "VBOX",
        SgplotStmt::Reg { .. } => "REG",
        SgplotStmt::Loess { .. } => "LOESS",
    }
}

pub fn execute(ast: &SgplotAst, session: &mut Session) -> Result<()> {
    // 1) ODS GRAPHICS non activé → NOTE de non-activation, EXIT 0.
    if !session.ods_graphics.enabled {
        session.log.note(
            "ODS GRAPHICS is not enabled. Use \"ods graphics on;\" before PROC SGPLOT to generate images.",
        );
        return Ok(());
    }

    // 2) Aucun statement de tracé : rien à dessiner.
    let first = match ast.plot_stmts.first() {
        Some(s) => s,
        None => {
            session
                .log
                .note("No plot statement found in PROC SGPLOT; nothing to plot.");
            return Ok(());
        }
    };

    // 3) BY-group → différé (NOTE), même sous --features graphics.
    if ast.by_var.is_some() {
        session
            .log
            .note("BY-group processing deferred in PROC SGPLOT.");
        return Ok(());
    }

    // 4) Fonctions différées sur le PREMIER statement (avant le gate feature
    //    pour que la NOTE soit testable dans le build par défaut).
    match first {
        SgplotStmt::Loess { .. } => {
            session
                .log
                .note("LOESS plot deferred (not yet implemented in PROC SGPLOT).");
            return Ok(());
        }
        SgplotStmt::Density { .. } => {
            session
                .log
                .note("DENSITY plot deferred (not yet implemented in PROC SGPLOT).");
            return Ok(());
        }
        _ => {}
    }

    // 5) v1 : un seul plot par image — prévenir si plusieurs.
    if ast.plot_stmts.len() > 1 {
        session.log.note(&format!(
            "PROC SGPLOT v1 renders only the first plot statement ({}); {} additional statement(s) ignored.",
            stmt_kind(first),
            ast.plot_stmts.len() - 1
        ));
    }

    // 6) Génération de l'image.
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

    /// Construit le DrawingSpec depuis le premier statement de tracé.
    fn build_spec(
        ds: &crate::dataset::SasDataset,
        stmt: &SgplotStmt,
        ast: &SgplotAst,
        title: String,
    ) -> Result<DrawingSpec> {
        // Libellés d'axes : XAXIS/YAXIS LABEL= sinon le nom de variable.
        let x_axis_label = ast.xaxis.as_ref().and_then(|a| a.label.clone());
        let y_axis_label = ast.yaxis.as_ref().and_then(|a| a.label.clone());

        match stmt {
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
                Ok(DrawingSpec {
                    title,
                    x_label: x_axis_label.unwrap_or_else(|| x.clone()),
                    y_label: y_axis_label.unwrap_or_else(|| y.clone()),
                    plot_type,
                    data,
                    x_categorical: vec![],
                })
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
                Ok(DrawingSpec {
                    title,
                    x_label: x_axis_label.unwrap_or_else(|| var.clone()),
                    y_label: y_axis_label.unwrap_or_else(|| "Frequency".to_string()),
                    plot_type: PlotType::Histogram { bins },
                    data,
                    x_categorical: vec![],
                })
            }
            SgplotStmt::VBar { category, .. } => {
                // VBAR : agrégat FREQ par catégorie (v1 : compte les occurrences).
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
                let x_categorical: Vec<(String, f64)> = counts.into_iter().collect();
                Ok(DrawingSpec {
                    title,
                    x_label: x_axis_label.unwrap_or_else(|| category.clone()),
                    y_label: y_axis_label.unwrap_or_else(|| "Frequency".to_string()),
                    plot_type: PlotType::VBar,
                    data: vec![],
                    x_categorical,
                })
            }
            // Types non encore rendus par l'infra render.rs : géré en amont.
            _ => Err(SasError::runtime(format!(
                "{} plot not yet rendered in PROC SGPLOT.",
                stmt_kind(stmt)
            ))),
        }
    }

    pub fn render(ast: &SgplotAst, stmt: &SgplotStmt, session: &mut Session) -> Result<()> {
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
        let in_ref = common::resolve_last_dataset(&ast.data_ref, session)?;
        let in_libref = in_ref.libref_or_work();
        let in_table = in_ref.name.to_uppercase();
        let provider = session.libs.get(&in_libref)?;
        let (ds, notes) = provider.read(&in_table)?;
        for note in notes {
            session.log.forward(&note);
        }

        let spec = build_spec(&ds, stmt, ast, "The SGPlot Procedure".to_string())?;

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

    fn parse_sgplot(src: &str) -> Result<SgplotAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // sgplot
        parse(&mut ts)
    }

    #[allow(dead_code)]
    fn make_session() -> Session {
        Session::new(None, std::env::temp_dir(), true).unwrap()
    }

    // ── Parse tests ──────────────────────────────────────────────────────

    #[test]
    fn parse_scatter() {
        let ast = parse_sgplot("proc sgplot data=a; scatter x=age y=height; run;").unwrap();
        assert_eq!(ast.plot_stmts.len(), 1);
        match &ast.plot_stmts[0] {
            SgplotStmt::Scatter { x, y, group, .. } => {
                assert_eq!(x, "age");
                assert_eq!(y, "height");
                assert!(group.is_none());
            }
            other => panic!("expected Scatter, got {other:?}"),
        }
    }

    #[test]
    fn parse_scatter_with_group_and_markerattrs() {
        let ast = parse_sgplot(
            "proc sgplot data=a; scatter x=age y=height / group=sex markerattrs=(symbol=circlefilled color=red); run;",
        )
        .unwrap();
        match &ast.plot_stmts[0] {
            SgplotStmt::Scatter {
                group, markerattrs, ..
            } => {
                assert_eq!(group.as_deref(), Some("sex"));
                let m = markerattrs.as_ref().unwrap();
                assert_eq!(m.symbol.as_deref(), Some("circlefilled"));
                assert_eq!(m.color.as_deref(), Some("red"));
            }
            other => panic!("expected Scatter, got {other:?}"),
        }
    }

    #[test]
    fn parse_series() {
        let ast = parse_sgplot("proc sgplot data=a; series x=time y=value; run;").unwrap();
        match &ast.plot_stmts[0] {
            SgplotStmt::Series { x, y, .. } => {
                assert_eq!(x, "time");
                assert_eq!(y, "value");
            }
            other => panic!("expected Series, got {other:?}"),
        }
    }

    #[test]
    fn parse_vbar() {
        let ast =
            parse_sgplot("proc sgplot data=a; vbar category / response=n stat=sum; run;").unwrap();
        match &ast.plot_stmts[0] {
            SgplotStmt::VBar {
                category,
                response,
                stat,
            } => {
                assert_eq!(category, "category");
                assert_eq!(response.as_deref(), Some("n"));
                assert_eq!(*stat, BarStat::Sum);
            }
            other => panic!("expected VBar, got {other:?}"),
        }
    }

    #[test]
    fn parse_hbar_default_stat_freq() {
        let ast = parse_sgplot("proc sgplot data=a; hbar category / response=amount; run;").unwrap();
        match &ast.plot_stmts[0] {
            SgplotStmt::HBar { stat, .. } => assert_eq!(*stat, BarStat::Freq),
            other => panic!("expected HBar, got {other:?}"),
        }
    }

    #[test]
    fn parse_histogram() {
        let ast = parse_sgplot("proc sgplot data=a; histogram height / binwidth=10; run;").unwrap();
        match &ast.plot_stmts[0] {
            SgplotStmt::Histogram { var, binwidth, .. } => {
                assert_eq!(var, "height");
                assert_eq!(*binwidth, Some(10.0));
            }
            other => panic!("expected Histogram, got {other:?}"),
        }
    }

    #[test]
    fn parse_xaxis_yaxis() {
        let ast = parse_sgplot(
            "proc sgplot data=a; scatter x=age y=h; xaxis label='Age'; yaxis type=log; run;",
        )
        .unwrap();
        let x = ast.xaxis.as_ref().unwrap();
        assert_eq!(x.label.as_deref(), Some("Age"));
        let y = ast.yaxis.as_ref().unwrap();
        assert_eq!(y.type_, Some(AxisType::Log));
    }

    #[test]
    fn parse_xaxis_values_range() {
        let ast = parse_sgplot(
            "proc sgplot data=a; scatter x=age y=h; xaxis values=(0 to 100 by 10); run;",
        )
        .unwrap();
        let x = ast.xaxis.as_ref().unwrap();
        assert_eq!(x.values_min, Some(0.0));
        assert_eq!(x.values_max, Some(100.0));
    }

    #[test]
    fn parse_reg_default_degree() {
        let ast = parse_sgplot("proc sgplot data=a; reg x=age y=height; run;").unwrap();
        match &ast.plot_stmts[0] {
            SgplotStmt::Reg { degree, .. } => assert_eq!(*degree, 1),
            other => panic!("expected Reg, got {other:?}"),
        }
    }

    #[test]
    fn parse_reg_degree2() {
        let ast = parse_sgplot("proc sgplot data=a; reg x=age y=height / degree=2; run;").unwrap();
        match &ast.plot_stmts[0] {
            SgplotStmt::Reg { degree, .. } => assert_eq!(*degree, 2),
            other => panic!("expected Reg, got {other:?}"),
        }
    }

    #[test]
    fn parse_loess() {
        let ast =
            parse_sgplot("proc sgplot data=a; loess x=age y=height / smooth=0.5; run;").unwrap();
        match &ast.plot_stmts[0] {
            SgplotStmt::Loess { smooth, .. } => assert_eq!(*smooth, 0.5),
            other => panic!("expected Loess, got {other:?}"),
        }
    }

    #[test]
    fn parse_density() {
        let ast = parse_sgplot("proc sgplot data=a; density height / kernel; run;").unwrap();
        assert!(matches!(ast.plot_stmts[0], SgplotStmt::Density { .. }));
    }

    #[test]
    fn parse_vbox() {
        let ast =
            parse_sgplot("proc sgplot data=a; vbox response / category=group; run;").unwrap();
        match &ast.plot_stmts[0] {
            SgplotStmt::VBox { category, response } => {
                assert_eq!(response, "response");
                assert_eq!(category.as_deref(), Some("group"));
            }
            other => panic!("expected VBox, got {other:?}"),
        }
    }

    #[test]
    fn parse_by() {
        let ast = parse_sgplot("proc sgplot data=a; by sex; scatter x=age y=h; run;").unwrap();
        assert_eq!(ast.by_var.as_deref(), Some("sex"));
    }

    // ── Execute tests (default build) ────────────────────────────────────

    #[test]
    fn execute_without_ods_on_notes_not_enabled() {
        let mut session = make_session();
        let ast = parse_sgplot("proc sgplot data=a; scatter x=age y=h; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(
            log.contains("ODS GRAPHICS is not enabled"),
            "log: {log}"
        );
    }

    #[cfg(not(feature = "graphics"))]
    #[test]
    fn execute_with_ods_on_no_feature_defers() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        let ast = parse_sgplot("proc sgplot data=a; scatter x=age y=h; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("image deferred"), "log: {log}");
    }

    #[test]
    fn execute_loess_defers() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        let ast =
            parse_sgplot("proc sgplot data=a; loess x=age y=h / smooth=0.5; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("LOESS plot deferred"), "log: {log}");
    }

    #[test]
    fn execute_density_defers() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        let ast = parse_sgplot("proc sgplot data=a; density h; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("DENSITY plot deferred"), "log: {log}");
    }

    #[test]
    fn execute_by_defers() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        let ast = parse_sgplot("proc sgplot data=a; by sex; scatter x=age y=h; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("BY-group processing deferred"), "log: {log}");
    }

    // ── Execute tests (feature graphics) ─────────────────────────────────

    #[cfg(feature = "graphics")]
    fn write_heights(session: &mut Session, table: &str) {
        use crate::dataset::{SasDataset, VarMeta};
        use crate::value::VarType;
        use polars::df;
        let df = df![
            "age" => [10.0_f64, 12.0, 14.0, 16.0, 18.0],
            "height" => [140.0_f64, 150.0, 158.0, 165.0, 170.0]
        ]
        .unwrap();
        let vars = vec![
            VarMeta {
                name: "age".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
            VarMeta {
                name: "height".into(),
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
        session.ods_graphics.file_stem = Some("sgtest_single".into());
        write_heights(&mut session, "H");
        let ast =
            parse_sgplot("proc sgplot data=work.h; scatter x=age y=height; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("written"), "log: {log}");
        let p = std::env::temp_dir().join("sgtest_single_1.png");
        assert!(p.exists(), "image not created: {p:?}");
        assert!(p.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&p);
    }

    #[cfg(feature = "graphics")]
    #[test]
    fn execute_sequential_naming() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        session.ods_graphics.output_dir = std::env::temp_dir();
        session.ods_graphics.file_stem = Some("sgtest_seq".into());
        write_heights(&mut session, "H");
        let ast =
            parse_sgplot("proc sgplot data=work.h; scatter x=age y=height; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        execute(&ast, &mut session).unwrap();
        let p1 = std::env::temp_dir().join("sgtest_seq_1.png");
        let p2 = std::env::temp_dir().join("sgtest_seq_2.png");
        assert!(p1.exists(), "first image missing");
        assert!(p2.exists(), "second image missing");
        let _ = std::fs::remove_file(&p1);
        let _ = std::fs::remove_file(&p2);
    }

    #[cfg(feature = "graphics")]
    #[test]
    fn execute_missing_column_errors() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        session.ods_graphics.output_dir = std::env::temp_dir();
        write_heights(&mut session, "H");
        let ast =
            parse_sgplot("proc sgplot data=work.h; scatter x=nonexistent y=height; run;").unwrap();
        let res = execute(&ast, &mut session);
        assert!(res.is_err(), "expected error for missing column");
        let msg = res.err().unwrap().to_string();
        assert!(msg.contains("NONEXISTENT"), "msg: {msg}");
    }
}
