use super::*;

// ───────────────────────── Parser ─────────────────────────

/// Lit un nom de variable (identifiant). Erreur propre sinon.
pub(super) fn expect_ident(ts: &mut StatementStream, ctx: &str) -> Result<String> {
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
pub(super) fn parse_plot_stmt(ts: &mut StatementStream) -> Result<GplotStmt> {
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
pub(super) fn read_value(ts: &mut StatementStream) -> Option<String> {
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
pub(super) fn parse_symbol_stmt(ts: &mut StatementStream) -> SymbolDef {
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
pub(super) fn parse_axis_stmt(ts: &mut StatementStream) -> AxisDef {
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
pub(super) fn starts_with_kw(ts: &StatementStream, kw: &str) -> bool {
    ts.peek()
        .ident()
        .map(|s| {
            let lower = s.to_ascii_lowercase();
            lower.starts_with(kw)
                && lower[kw.len()..].chars().all(|c| c.is_ascii_digit())
        })
        .unwrap_or(false)
}
