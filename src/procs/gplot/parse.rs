use super::*;

// ───────────────────────── Contract (J02-P6) ─────────────────────────
//
// SAS/GRAPH 9.4 Reference, The GPLOT Procedure, and the SYMBOL / AXIS / LEGEND
// global statements. The options after the `/` of PLOT were skipped up to the
// `;`, the SYMBOL and AXIS sub-options that the engine does not draw were
// dropped, the PROC options skipped token by token (audit d0b4d90).
// CONTRIBUTING §5: a display option that is not rendered is a WARNING; an
// option that creates an output (catalog, image-map data set), BY and the
// plot statements that are not implemented are ERRORs (step rejected).

/// PROC GPLOT statement options limited to the drawn image.
const DISPLAY_PROC_OPTIONS: &[&str] = &["annotate", "anno", "uniform"];

/// PROC GPLOT statement options that create an output: GOUT= (graphics
/// catalog) and IMAGEMAP= (data set).
const OUTPUT_PROC_OPTIONS: &[&str] = &["gout", "imagemap"];

/// Valid PROC GPLOT plot statements that are not implemented.
const UNSUPPORTED_STATEMENTS: &[&str] = &["bubble", "bubble2"];

/// SYMBOL colors the engine draws (mirror of `graphics_impl::color_from_name`);
/// any other COLOR= falls back to the default palette.
const ENGINE_COLORS: &[&str] = &["black", "blue", "green", "orange", "red"];

/// True for `prefix` or `prefixN` (`legend`, `legend2`, `pattern12`…).
fn is_numbered(kw: &str, prefix: &str) -> bool {
    kw.strip_prefix(prefix)
        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

/// SAS/GRAPH global statements valid in the step that only decorate the
/// graph (LEGENDn, PATTERNn, GOPTIONS, NOTE): display WARNING. They used to
/// be rejected as « 180-322 … not valid ».
fn is_display_statement(kw: &str) -> bool {
    kw == "goptions" || kw == "note" || is_numbered(kw, "legend") || is_numbered(kw, "pattern")
}

// ───────────────────────── Parser ─────────────────────────

/// Parse un statement PLOT : `y*x`, `y*x=group`, ou `(y1 y2)*x`.
///
/// Le mot-clé `plot` a déjà été consommé (`stmt` : PLOT ou PLOT2, pour les
/// diagnostics). Consomme jusqu'au `;` exclu. J02-P6 : les options après `/`
/// (OVERLAY, HAXIS=, VAXIS=, LEGEND=, HREF=, VREF=…) étaient sautées jusqu'au
/// `;` ; aucune n'est rendue : un WARNING par option. Une deuxième requête de
/// tracé dans le même statement (`plot y1*x y2*x;`) était rejetée comme
/// instruction « 180-322 » : ERROR du contrat.
pub(super) fn parse_plot_stmt(ts: &mut StatementStream, stmt: &str) -> Result<GplotStmt> {
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

    // Une deuxième requête de tracé avant `/` ou `;`.
    if matches!(ts.peek().kind, TokenKind::Ident(_) | TokenKind::LParen) {
        return Err(SasError::parse(
            format!(
                "A {stmt} statement with more than one plot request is not supported in PROC \
                 GPLOT; it can affect results and cannot be ignored{}.",
                contract::planned("J14-P3")
            ),
            ts.peek().span,
        ));
    }

    // Options après `/` : aucune n'est rendue (WARNING par option). Le moteur
    // applique le premier AXIS du step à l'axe horizontal et le deuxième à
    // l'axe vertical, quels que soient HAXIS=/VAXIS= : le WARNING le dit.
    if ts.peek().kind == TokenKind::Slash {
        ts.next(); // /
        while !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
            let Some(name) = ts.peek().ident().map(|s| s.to_ascii_uppercase()) else {
                return Err(contract::expected_option(ts, "GPLOT", stmt));
            };
            if (name == "HAXIS" || name == "VAXIS") && ts.peek2().kind == TokenKind::Eq {
                ts.warn_ignored_display(format!(
                    "The {name}= option of the {stmt} statement is ignored in PROC GPLOT: the \
                     first AXIS statement of the step is applied to the horizontal axis and the \
                     second to the vertical axis."
                ));
                ts.next();
                contract::skip_option_args(ts);
            } else {
                contract::warn_option(ts, "GPLOT", Some(stmt));
            }
        }
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

    // Options du statement PROC GPLOT, jusqu'au `;` (J02-P6 : plus de saut
    // silencieux ; option inconnue → « Unexpected option »).
    common::parse_proc_options(ts, "GPLOT", |ts, kw| {
        if kw == "data" {
            data_ref = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if DISPLAY_PROC_OPTIONS.contains(&kw) {
            contract::warn_option(ts, "GPLOT", None);
        } else if OUTPUT_PROC_OPTIONS.contains(&kw) {
            return Err(contract::unsupported_option(
                "GPLOT",
                &contract::option_label(ts),
                ts.peek().span,
            ));
        } else {
            return Ok(false);
        }
        Ok(true)
    })?;

    let mut plots: Vec<GplotStmt> = Vec::new();
    let mut symbols: Vec<SymbolDef> = Vec::new();
    let mut axes: Vec<AxisDef> = Vec::new();

    common::parse_proc_body(ts, "GPLOT", |ts, kw| {
        if kw == "plot" || kw == "plot2" {
            let stmt = kw.to_ascii_uppercase();
            ts.next();
            plots.push(parse_plot_stmt(ts, &stmt)?);
            ts.expect_semi()?;
        } else if is_numbered(kw, "symbol") {
            let stmt = kw.to_ascii_uppercase();
            ts.next();
            symbols.push(parse_symbol_stmt(ts, &stmt)?);
            ts.expect_semi()?;
        } else if is_numbered(kw, "axis") {
            let stmt = kw.to_ascii_uppercase();
            ts.next();
            axes.push(parse_axis_stmt(ts, &stmt)?);
            ts.expect_semi()?;
        } else if kw == "by" {
            // Same contract ERROR as SGPLOT/GCHART/PLOT until J13-P4.
            return Err(contract::by_not_supported("GPLOT", ts.peek().span));
        } else if UNSUPPORTED_STATEMENTS.contains(&kw) {
            return Err(contract::unsupported_statement(
                "GPLOT",
                kw,
                None,
                ts.peek().span,
            ));
        } else if is_display_statement(kw) {
            ts.warn_ignored_display(common::ignored_display_statement("GPLOT", kw));
            ts.skip_to_semi();
        } else {
            return Ok(false);
        }
        Ok(true)
    })?;

    Ok(GplotAst {
        data_ref,
        plots,
        symbols,
        axes,
    })
}

/// Parse un SYMBOLn (`stmt` : SYMBOL1…) : suite d'options `name=value`
/// jusqu'au `;` (non consommé).
///
/// Le moteur dessine INTERPOL=JOIN (ligne) ou NONE (marqueurs), la présence
/// d'un VALUE= et cinq couleurs (BLACK, BLUE, GREEN, ORANGE, RED). J02-P6 :
/// une autre interpolation (SPLINE, RL, SM…), un symbole VALUE= (toujours
/// dessiné avec le marqueur par défaut), une autre couleur (remplacée par la
/// palette) et les autres options (HEIGHT=, WIDTH=, LINE=, REPEAT=, FONT=…)
/// étaient abandonnés en silence : WARNING.
pub(super) fn parse_symbol_stmt(ts: &mut StatementStream, stmt: &str) -> Result<SymbolDef> {
    let mut def = SymbolDef::default();
    while !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
        let Some(name) = ts.peek().ident().map(str::to_string) else {
            return Err(contract::expected_option(ts, "GPLOT", stmt));
        };
        // `i`/`v`/`c` sont les abréviations SAS de interpol/value/color.
        let canon = match name.to_ascii_lowercase().as_str() {
            "i" | "interpol" => "interpol",
            "v" | "value" => "value",
            "c" | "color" => "color",
            _ => "",
        };
        let value = match &ts.peek_nth(2).kind {
            TokenKind::Ident(s) | TokenKind::Str { value: s, .. }
                if !canon.is_empty() && ts.peek2().kind == TokenKind::Eq =>
            {
                Some(s.clone())
            }
            _ => None,
        };
        let Some(value) = value else {
            contract::warn_option(ts, "GPLOT", Some(stmt));
            continue;
        };
        let label = format!(
            "{}={}",
            name.to_ascii_uppercase(),
            value.to_ascii_uppercase()
        );
        ts.next(); // name
        ts.next(); // =
        ts.next(); // value
        match canon {
            "interpol" => {
                if !(value.eq_ignore_ascii_case("join") || value.eq_ignore_ascii_case("none")) {
                    ts.warn_ignored_display(contract::ignored_option("GPLOT", Some(stmt), &label));
                }
                def.interpol = Some(value);
            }
            "value" => {
                ts.warn_ignored_display(format!(
                    "The {label} option of the {stmt} statement is not honored in PROC GPLOT: \
                     markers use the default symbol."
                ));
                def.value = Some(value);
            }
            _ => {
                if !ENGINE_COLORS.contains(&value.to_ascii_lowercase().as_str()) {
                    ts.warn_ignored_display(contract::ignored_option("GPLOT", Some(stmt), &label));
                }
                def.color = Some(value);
            }
        }
    }
    Ok(def)
}

/// Parse un AXISn (`stmt` : AXIS1…) : `order=(min to max [by step])`,
/// `label=('..')` ; consomme jusqu'au `;` exclu.
///
/// J02-P6 : ORDER= n'est rendu que par ses deux premiers nombres, pris comme
/// bornes de l'axe (graduations et valeurs suivantes perdues, signe d'un
/// nombre négatif perdu) ; LABEL= ne garde que son premier texte (attributs
/// ANGLE=, HEIGHT=, FONT=… et lignes suivantes perdus ; LABEL=NONE dessinait
/// le texte « none ») ; les autres options (MAJOR=, MINOR=, VALUE=, STYLE=…)
/// étaient sautées : WARNING.
pub(super) fn parse_axis_stmt(ts: &mut StatementStream, stmt: &str) -> Result<AxisDef> {
    let mut def = AxisDef::default();
    while !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
        let Some(name) = ts.peek().ident().map(|s| s.to_ascii_lowercase()) else {
            return Err(contract::expected_option(ts, "GPLOT", stmt));
        };
        let has_eq = ts.peek2().kind == TokenKind::Eq;
        let value_kind = ts.peek_nth(2).kind.clone();
        match name.as_str() {
            "order" if has_eq && value_kind == TokenKind::LParen => {
                ts.next(); // order
                ts.next(); // =
                let nums = contract::value_list_numbers(ts);
                if let Some(&mn) = nums.first() {
                    def.order_min = Some(mn);
                }
                if nums.len() >= 2 {
                    // `order=(min to max [by step])` : 1er nombre = min, 2e = max.
                    def.order_max = Some(nums[1]);
                }
                ts.warn_ignored_display(contract::partial_value_list(
                    "GPLOT", stmt, "ORDER=", &nums,
                ));
            }
            "label" if has_eq && value_kind == TokenKind::LParen => {
                ts.next(); // label
                ts.next(); // =
                // label=('text' …) : prendre le premier texte ; tout autre
                // élément (attribut, texte suivant) est perdu.
                ts.next(); // (
                let mut lab: Option<String> = None;
                let mut dropped = false;
                let mut depth = 1usize;
                loop {
                    match ts.peek().kind.clone() {
                        TokenKind::Semi | TokenKind::Eof => break,
                        TokenKind::RParen => {
                            depth -= 1;
                            ts.next();
                            if depth == 0 {
                                break;
                            }
                        }
                        TokenKind::Str { value, .. } if lab.is_none() && depth == 1 => {
                            lab = Some(value);
                            ts.next();
                        }
                        kind => {
                            if kind == TokenKind::LParen {
                                depth += 1;
                            }
                            dropped = true;
                            ts.next();
                        }
                    }
                }
                if dropped {
                    ts.warn_ignored_display(format!(
                        "The LABEL= attributes of the {stmt} statement are ignored in PROC \
                         GPLOT; only the first label text is drawn."
                    ));
                }
                def.label = lab;
            }
            "label"
                if has_eq && matches!(value_kind, TokenKind::Ident(_) | TokenKind::Str { .. }) =>
            {
                ts.next(); // label
                ts.next(); // =
                let text = read_value(ts);
                if text
                    .as_deref()
                    .is_some_and(|t| t.eq_ignore_ascii_case("none"))
                    && matches!(value_kind, TokenKind::Ident(_))
                {
                    ts.warn_ignored_display(format!(
                        "The LABEL=NONE option of the {stmt} statement is not honored in PROC \
                         GPLOT: the text NONE is drawn as the axis label."
                    ));
                }
                def.label = text;
            }
            _ => contract::warn_option(ts, "GPLOT", Some(stmt)),
        }
    }
    Ok(def)
}
