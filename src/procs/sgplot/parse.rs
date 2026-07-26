use super::*;

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
                // Options après `/` : TYPE=KERNEL|NORMAL (KERNEL/NORMAL aussi
                // acceptés en mots-clés nus). Reste ignoré.
                let mut kernel = false;
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
                            "kernel" => kernel = true,
                            "normal" => kernel = false,
                            "type" => {
                                if ts.peek().kind == TokenKind::Eq {
                                    ts.next();
                                }
                                if let Some(v) = read_value(ts) {
                                    kernel = v.eq_ignore_ascii_case("kernel");
                                }
                            }
                            _ => common::skip_option_value(ts),
                        }
                    }
                }
                ts.expect_semi()?;
                plot_stmts.push(SgplotStmt::Density { var, kernel });
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
                        } else {
                            common::skip_option_value(ts);
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
