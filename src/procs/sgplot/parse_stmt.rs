use super::*;

/// Parse les options X=var Y=var et les options après `/` d'un statement de
/// tracé à deux variables (SCATTER, SERIES, REG, LOESS). Renvoie
/// `(x, y, group, markerattrs, degree, smooth)`. Les inconnues sont ignorées.
pub(super) fn parse_xy_stmt(
    ts: &mut StatementStream,
) -> Result<(
    String,
    String,
    Option<String>,
    Option<MarkerAttrs>,
    Option<u32>,
    Option<f64>,
)> {
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
pub(super) fn parse_bar_stmt(
    ts: &mut StatementStream,
) -> Result<(String, Option<String>, BarStat)> {
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
pub(super) fn parse_histogram_stmt(
    ts: &mut StatementStream,
) -> Result<(String, Option<f64>, HistScale)> {
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
pub(super) fn parse_axis_stmt(ts: &mut StatementStream) -> Result<AxisOpts> {
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
