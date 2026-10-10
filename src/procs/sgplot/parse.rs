use super::*;

// ───────────────────────── Contract (J02-P6) ─────────────────────────
//
// SAS 9.4 ODS Graphics: Procedures Guide, The SGPLOT Procedure, Syntax. The
// options the image engine does not render used to be skipped token by token,
// unknown PROC options included, and BY was noted then ignored (audit
// d0b4d90). CONTRIBUTING §5: a display option that is not rendered is a
// WARNING; an option that creates an output file, the BY statement and the
// plot statements that are not implemented are ERRORs (step rejected).

/// PROC SGPLOT statement options limited to the drawn image.
const DISPLAY_PROC_OPTIONS: &[&str] = &[
    "aspect",
    "cycleattrs",
    "dattrmap",
    "des",
    "description",
    "noautolegend",
    "noborder",
    "nocycleattrs",
    "noopaque",
    "nosubpixel",
    "nowall",
    "objectlabel",
    "pad",
    "pctlevel",
    "pctndec",
    "rattrmap",
    "sganno",
    "subpixel",
    "uniform",
];

/// Valid PROC SGPLOT plot statements that are not implemented: ignoring one
/// would drop a layer of the requested graph, or leave nothing to draw.
const UNSUPPORTED_STATEMENTS: &[&str] = &[
    "band",
    "block",
    "bubble",
    "dot",
    "ellipse",
    "ellipseparm",
    "fringe",
    "hbarbasic",
    "hbarparm",
    "hbox",
    "heatmap",
    "heatmapparm",
    "highlow",
    "hline",
    "lineparm",
    "needle",
    "pbspline",
    "polygon",
    "spline",
    "step",
    "text",
    "vbarbasic",
    "vbarparm",
    "vector",
    "vline",
    "waterfall",
    "xaxistable",
    "yaxistable",
];

/// Valid statements limited to the decoration of the graph (legends, insets,
/// reference and drop lines, style attributes, secondary axes): display
/// WARNING, the graph is drawn without them.
const DISPLAY_STATEMENTS: &[&str] = &[
    "dropline",
    "gradlegend",
    "inset",
    "keylegend",
    "legenditem",
    "refline",
    "styleattrs",
    "symbolchar",
    "symbolimage",
    "x2axis",
    "y2axis",
];

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC SGPLOT. Appelé APRÈS consommation de `proc sgplot`.
pub fn parse(ts: &mut StatementStream) -> Result<SgplotAst> {
    let mut data_ref: Option<DatasetRef> = None;

    // Options du statement PROC SGPLOT, jusqu'au `;`. J02-P6 : une option
    // inconnue est une ERROR (« Unexpected option »), une option d'affichage
    // un WARNING ; plus aucun saut silencieux.
    common::parse_proc_options(ts, "SGPLOT", |ts, kw| {
        if kw == "data" {
            data_ref = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if DISPLAY_PROC_OPTIONS.contains(&kw) {
            contract::warn_option(ts, "SGPLOT", None);
        } else if kw == "tmplout" {
            // Writes the generated GTL template to a file: an output.
            return Err(contract::unsupported_option(
                "SGPLOT",
                "TMPLOUT=",
                ts.peek().span,
            ));
        } else {
            return Ok(false);
        }
        Ok(true)
    })?;

    let mut plot_stmts: Vec<SgplotStmt> = Vec::new();
    let mut xaxis: Option<AxisOpts> = None;
    let mut yaxis: Option<AxisOpts> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, "SGPLOT", |ts, kw| {
        Ok(match kw {
            "scatter" => {
                ts.next();
                let (x, y, group, markerattrs, _, _) = parse_xy_stmt(ts, kw)?;
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
                let (x, y, group, _, _, _) = parse_xy_stmt(ts, kw)?;
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::Series { x, y, group });
                true
            }
            "reg" => {
                ts.next();
                let (x, y, _, _, degree, _) = parse_xy_stmt(ts, kw)?;
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
                let (x, y, _, _, _, smooth) = parse_xy_stmt(ts, kw)?;
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
                let (category, response, stat) = parse_bar_stmt(ts, kw)?;
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
                let (category, response, stat) = parse_bar_stmt(ts, kw)?;
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
                let (var, kernel) = parse_density_stmt(ts)?;
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::Density { var, kernel });
                true
            }
            "vbox" => {
                ts.next();
                let (category, response) = parse_vbox_stmt(ts)?;
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::VBox { category, response });
                true
            }
            "xaxis" => {
                ts.next();
                xaxis = Some(parse_axis_stmt(ts, kw)?);
                ts.expect_semi()?;
                true
            }
            "yaxis" => {
                ts.next();
                yaxis = Some(parse_axis_stmt(ts, kw)?);
                ts.expect_semi()?;
                true
            }
            // J02-P6 — BY was noted (« BY-group processing deferred »), no
            // image, code 0: same contract ERROR as GPLOT/GCHART/PLOT.
            "by" => return Err(contract::by_not_supported("SGPLOT", ts.peek().span)),
            _ if UNSUPPORTED_STATEMENTS.contains(&kw) => {
                let unit = (kw == "hbox").then_some("J13-P3");
                return Err(contract::unsupported_statement(
                    "SGPLOT",
                    kw,
                    unit,
                    ts.peek().span,
                ));
            }
            _ if DISPLAY_STATEMENTS.contains(&kw) => {
                ts.warn_ignored_display(common::ignored_display_statement("SGPLOT", kw));
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
    })
}
