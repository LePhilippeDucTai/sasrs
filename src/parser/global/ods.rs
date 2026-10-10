use super::*;

// ── ODS ──────────────────────────────────────────────────────────────────────

/// Parse a statement `ODS` (Output Delivery System), schéma large v1.
///
/// Le mot-clé `ODS` a déjà été consommé par l'appelant. Formes reconnues :
/// - `ODS LISTING ;`                 → ouvre le listing texte (défaut)
/// - `ODS HTML ;`                    → ouvre la destination HTML
/// - `ODS RTF|PDF|EXCEL ;`           → stubs (parse no-op, rendu différé M23)
/// - `ODS HTML CLOSE ;`              → ferme la destination HTML
/// - `ODS CLOSE ;` / `ODS CLOSE name ;` → ferme la destination (courante / nommée)
/// - `ODS _ALL_ CLOSE ;`             → ferme tout (traité comme CLOSE générique)
///
/// Options reconnues : `FILE='...'`, `STYLE=name` (stockée, non appliquée),
/// `OPTIONS=...` (non appliquée) — J02-P6 : STYLE= et OPTIONS= donnent un
/// WARNING d'affichage. `ODS [dest] SELECT/EXCLUDE ...;` → liste de sélection
/// ODS (M38.4, `parse_ods_select`).
pub fn parse_ods_statement(ts: &mut StatementStream) -> Result<GlobalStmt> {
    // `ODS ;` nu : no-op accepté.
    if ts.peek().kind == TokenKind::Semi {
        ts.expect_semi()?;
        return Ok(GlobalStmt::Ods {
            destination: "listing".to_string(),
            action: OdsAction::Open,
            file: None,
            style: None,
        });
    }

    // Premier mot : soit un verbe global (`CLOSE`), soit un nom de destination.
    let first_tok = ts.peek().clone();
    let first = match first_tok.ident() {
        Some(s) => s.to_ascii_lowercase(),
        None => {
            return Err(SasError::parse(
                "ODS requires a destination name or a CLOSE keyword",
                first_tok.span,
            ));
        }
    };

    // `ODS OUTPUT ...` — capture de tables ODS vers des datasets (M22.3).
    if first == "output" {
        ts.next(); // consume `output`
        return parse_ods_output(ts);
    }

    // `ODS GRAPHICS ...` — infra de génération d'images (M29.1).
    if first == "graphics" {
        ts.next(); // consume `graphics`
        return parse_ods_graphics(ts);
    }

    // `ODS SELECT ...` / `ODS EXCLUDE ...` — liste de sélection ODS (M38.4).
    if first == "select" || first == "exclude" {
        ts.next(); // consume `select`/`exclude`
        return parse_ods_select(ts, first == "exclude");
    }

    // `ODS CLOSE [name] ;` — verbe en tête, destination optionnelle après.
    if first == "close" {
        ts.next(); // consume `close`
        let dest = if let Some(name) = ts.peek().ident() {
            let d = name.to_ascii_lowercase();
            ts.next();
            d
        } else {
            // `ODS CLOSE ;` — ferme la destination courante (alias listing).
            "listing".to_string()
        };
        let (file, style) = parse_ods_options(ts, &dest)?;
        ts.expect_semi()?;
        return Ok(GlobalStmt::Ods {
            destination: dest,
            action: OdsAction::Close,
            file,
            style,
        });
    }

    // Sinon, `first` est un nom de destination : listing / html / rtf / pdf /
    // excel / _all_ / autre.
    let destination = first;
    ts.next(); // consume destination name

    // Action suivant la destination : CLOSE / SELECT / EXCLUDE / (défaut OPEN).
    let action = match ts.peek().ident().map(|s| s.to_ascii_lowercase()) {
        Some(ref a) if a == "close" => {
            ts.next();
            OdsAction::Close
        }
        Some(ref a) if a == "open" => {
            ts.next();
            OdsAction::Open
        }
        Some(ref a) if a == "select" || a == "exclude" => {
            // M38.4 — `ODS <dest> SELECT/EXCLUDE ...` : la qualification de
            // destination est acceptée mais la liste s'applique GLOBALEMENT
            // (divergence documentée : sasrs route toute la sortie vers la
            // destination courante unique, « global » et « par destination »
            // coïncident donc en pratique).
            let exclude = a == "exclude";
            ts.next(); // consume `select`/`exclude`
            return parse_ods_select(ts, exclude);
        }
        _ => OdsAction::Open,
    };

    let (file, style) = parse_ods_options(ts, &destination)?;
    ts.expect_semi()?;
    Ok(GlobalStmt::Ods {
        destination,
        action,
        file,
        style,
    })
}

/// Parse `ODS OUTPUT ...` (le mot-clé `OUTPUT` a déjà été consommé) — M22.3.
///
/// Formes reconnues :
/// - `ODS OUTPUT table=ds [table2=ds2 ...] ;` → liste de mappings
///   (nom de table ODS → dataset cible). Le nom de table ODS est conservé tel
///   quel ici (la mise en UPPERCASE est faite à l'exécution, le matching étant
///   insensible à la casse).
/// - `ODS OUTPUT CLOSE ;` → purge tous les mappings.
pub(super) fn parse_ods_output(ts: &mut StatementStream) -> Result<GlobalStmt> {
    // `ODS OUTPUT CLOSE ;` — désactive la capture.
    if ts.peek().ident().map(|s| s.eq_ignore_ascii_case("close")) == Some(true)
        && ts.peek2().kind != TokenKind::Eq
    {
        ts.next(); // consume `close`
        ts.expect_semi()?;
        return Ok(GlobalStmt::OdsOutput {
            mappings: Vec::new(),
            close: true,
        });
    }

    let mut mappings: Vec<(String, DatasetRef)> = Vec::new();
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }
        let name_tok = ts.peek().clone();
        let table = match name_tok.ident() {
            Some(s) => s.to_string(),
            None => {
                return Err(SasError::parse(
                    "Expected an ODS table name (e.g. Summary=ds) in ODS OUTPUT",
                    name_tok.span,
                ));
            }
        };
        ts.next(); // consume table name
        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "ODS OUTPUT requires '<ods-table>=<dataset>'",
                ts.peek().span,
            ));
        }
        ts.next(); // consume `=`
        let dref = ts.parse_dataset_ref()?;
        mappings.push((table, dref));
    }

    if mappings.is_empty() {
        return Err(SasError::parse(
            "ODS OUTPUT requires at least one '<ods-table>=<dataset>' mapping or CLOSE",
            ts.peek().span,
        ));
    }

    ts.expect_semi()?;
    Ok(GlobalStmt::OdsOutput {
        mappings,
        close: false,
    })
}

/// Parse la liste d'un `ODS SELECT`/`ODS EXCLUDE` (le verbe a déjà été
/// consommé) — M38.4.
///
/// Grammaire : `ODS [dest] SELECT|EXCLUDE nom1 nom2 … ;` ou
/// `ODS [dest] SELECT|EXCLUDE ALL|NONE ;`. Les mots-clés `ALL`/`NONE` doivent
/// être SEULS (comme dans SAS, où ils remplacent la liste entière). La forme
/// SAS `nom(PERSIST)` n'est pas supportée dans ce build (le cycle de vie
/// par défaut — liste nominative consommée à la fin du step suivant — est
/// implémenté dans `session::ods_select`) : une parenthèse lève une erreur de
/// parse explicite.
pub(super) fn parse_ods_select(ts: &mut StatementStream, exclude: bool) -> Result<GlobalStmt> {
    let stmt_name = if exclude { "ODS EXCLUDE" } else { "ODS SELECT" };
    let mut items: Vec<String> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi | TokenKind::Eof => break,
            TokenKind::LParen => {
                return Err(SasError::parse(
                    format!("{stmt_name} (PERSIST) is not supported in this build."),
                    tok.span,
                ));
            }
            _ => match tok.ident() {
                Some(name) => {
                    items.push(name.to_string());
                    ts.next();
                }
                None => {
                    return Err(SasError::parse(
                        format!("{stmt_name} expects output object names, ALL, or NONE"),
                        tok.span,
                    ));
                }
            },
        }
    }
    if items.is_empty() {
        return Err(SasError::parse(
            format!("{stmt_name} requires at least one output object name, ALL, or NONE"),
            ts.peek().span,
        ));
    }
    // ALL/NONE remplacent la liste ENTIÈRE : ils ne se combinent pas avec des
    // noms d'objets (même règle que SAS).
    if items.len() > 1
        && items
            .iter()
            .any(|i| i.eq_ignore_ascii_case("all") || i.eq_ignore_ascii_case("none"))
    {
        return Err(SasError::parse(
            format!("{stmt_name} ALL/NONE cannot be combined with output object names"),
            ts.peek().span,
        ));
    }
    ts.expect_semi()?;
    Ok(GlobalStmt::OdsSelect { exclude, items })
}

/// Parse `ODS GRAPHICS ...` (le mot-clé `GRAPHICS` a déjà été consommé) — M29.1.
///
/// Grammaire : `ODS GRAPHICS [ON|OFF] [ / opt=val ... ] ;`
///
/// Options après le `/` :
/// - `WIDTH=nnn` / `HEIGHT=nnn` (pixels)
/// - `IMAGEFMT=PNG|SVG` (ou forme parenthésée `IMAGEFMT=(PNG)`)
/// - `IMAGENAME="fig"` (préfixe de nommage)
/// - `RESET[=option|ALL]` — voir `parse_reset` (J02-P6)
///
/// Les options sont conservées PAR-STATEMENT (champs `Option`) : c'est leur
/// présence/absence qui pilote la NOTE de log à l'exécution. Elles
/// s'appliquent dans l'ordre du source (`RESET WIDTH=640` → 640).
pub(super) fn parse_ods_graphics(ts: &mut StatementStream) -> Result<GlobalStmt> {
    use crate::ast::{OdsGraphicsStmt, OdsGraphicsToggle};
    use crate::ods_graphics::ImageFmt;

    // ON / OFF optionnel en tête (avant un éventuel `/`).
    let toggle = match ts.peek().ident().map(|s| s.to_ascii_lowercase()) {
        Some(ref a) if a == "on" => {
            ts.next();
            OdsGraphicsToggle::On
        }
        Some(ref a) if a == "off" => {
            ts.next();
            OdsGraphicsToggle::Off
        }
        _ => OdsGraphicsToggle::None,
    };

    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut imagefmt: Option<ImageFmt> = None;
    let mut imagename: Option<String> = None;

    // Options optionnelles après un `/`.
    if ts.peek().kind == TokenKind::Slash {
        ts.next(); // consume `/`
        loop {
            if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                break;
            }
            let name_tok = ts.peek().clone();
            let name = match name_tok.ident() {
                Some(s) => s.to_ascii_lowercase(),
                None => {
                    return Err(SasError::parse(
                        "Expected an ODS GRAPHICS option (WIDTH=, HEIGHT=, IMAGEFMT=, IMAGENAME=, RESET) or ';'",
                        name_tok.span,
                    ));
                }
            };
            ts.next(); // consume option name

            match name.as_str() {
                "width" | "height" => {
                    expect_ods_graphics_eq(ts, &name)?;
                    let v = parse_dim(ts, &name)?;
                    if name == "width" {
                        width = Some(v);
                    } else {
                        height = Some(v);
                    }
                }
                "imagefmt" | "outputfmt" => {
                    expect_ods_graphics_eq(ts, &name)?;
                    imagefmt = Some(parse_imagefmt(ts)?);
                }
                "imagename" => {
                    expect_ods_graphics_eq(ts, &name)?;
                    let val_tok = ts.peek().clone();
                    let value = parse_option_value(ts, &val_tok.span)?;
                    imagename = Some(value);
                }
                "reset" => parse_reset(ts, &mut width, &mut height, &mut imagefmt)?,
                other => {
                    return Err(SasError::parse(
                        format!(
                            "ODS GRAPHICS option '{}' is not supported in this build.",
                            other.to_uppercase()
                        ),
                        name_tok.span,
                    ));
                }
            }
        }
    }

    ts.expect_semi()?;
    Ok(GlobalStmt::OdsGraphics(OdsGraphicsStmt {
        toggle,
        width,
        height,
        imagefmt,
        imagename,
    }))
}

/// ODS GRAPHICS options that RESET= accepts but that sasrs does not model
/// (SAS 9.4 ODS User's Guide, ODS GRAPHICS statement, RESET= option): their
/// value is always the default, so resetting them is a no-op.
const RESET_NOOP_OPTIONS: &[&str] = &[
    "antialias",
    "antialiasmax",
    "attrpriority",
    "border",
    "byline",
    "dataskinmax",
    "discretemax",
    "drilltarget",
    "groupmax",
    "imagemap",
    "labelmax",
    "labelplacement",
    "legendareamax",
    "loessobsmax",
    "nbinsmax",
    "nxybinsmax",
    "panelcellmax",
    "scale",
    "scalemarkers",
    "stackdepthmax",
    "subpixel",
    "tipmax",
];

/// `RESET`, `RESET=ALL` or `RESET=option` (the `RESET` keyword consumed).
///
/// SAS 9.4 ODS User's Guide, ODS GRAPHICS statement
/// (https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/odsug/p0kroq43yu0lspn16hk1u4c65lti.htm):
/// RESET (or RESET=ALL) « resets all options to their defaults »,
/// RESET=option resets one option; INDEX « resets the index counter that is
/// appended to static image files ». J02-P6 — RESET used to be parsed then
/// ignored. WIDTH=, HEIGHT= and OUTPUTFMT= (alias IMAGEFMT=) are now reset by
/// setting their default value in this statement. The IMAGENAME= prefix and
/// the image index cannot be reset through `OdsGraphicsStmt` (its definition,
/// `src/ast/global.rs`, has no field for them): WARNING. Any other name is an
/// ERROR, like an unknown ODS GRAPHICS option.
fn parse_reset(
    ts: &mut StatementStream,
    width: &mut Option<u32>,
    height: &mut Option<u32>,
    imagefmt: &mut Option<crate::ods_graphics::ImageFmt>,
) -> Result<()> {
    use crate::ods_graphics::{DEFAULT_HEIGHT, DEFAULT_WIDTH, ImageFmt};

    let target = if ts.peek().kind == TokenKind::Eq {
        ts.next(); // consume `=`
        let tok = ts.peek().clone();
        let Some(name) = tok.ident().map(|s| s.to_ascii_lowercase()) else {
            return Err(SasError::parse(
                "RESET= requires ALL or the name of an ODS GRAPHICS option",
                tok.span,
            ));
        };
        ts.next();
        (name, tok.span)
    } else {
        ("all".to_string(), ts.peek().span)
    };
    match target.0.as_str() {
        "all" => {
            *width = Some(DEFAULT_WIDTH);
            *height = Some(DEFAULT_HEIGHT);
            *imagefmt = Some(ImageFmt::default());
            ts.warn_ignored_display(
                "The RESET option of the ODS GRAPHICS statement does not reset the IMAGENAME= \
                 prefix and the image index in this build; WIDTH=, HEIGHT= and OUTPUTFMT= are \
                 reset to their defaults."
                    .to_string(),
            );
        }
        "width" => *width = Some(DEFAULT_WIDTH),
        "height" => *height = Some(DEFAULT_HEIGHT),
        "outputfmt" | "imagefmt" => *imagefmt = Some(ImageFmt::default()),
        "imagename" => ts.warn_ignored_display(
            "RESET=IMAGENAME is not supported in this build; the IMAGENAME= prefix is kept."
                .to_string(),
        ),
        "index" => {
            // `INDEX(n)` sets the next index: not supported either.
            ts.skip_balanced_parens();
            ts.warn_ignored_display(
                "RESET=INDEX is not supported in this build; the image index is not reset."
                    .to_string(),
            );
        }
        other if RESET_NOOP_OPTIONS.contains(&other) => {}
        other => {
            return Err(SasError::parse(
                format!(
                    "RESET={} is not a valid ODS GRAPHICS option.",
                    other.to_ascii_uppercase()
                ),
                target.1,
            ));
        }
    }
    Ok(())
}

/// Helper M29.1 : parse une dimension entière positive (WIDTH=/HEIGHT=).
pub(super) fn parse_dim(ts: &mut StatementStream, name: &str) -> Result<u32> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Num(f) if *f >= 0.0 && f.fract() == 0.0 => {
            let v = *f as u32;
            ts.next();
            Ok(v)
        }
        _ => Err(SasError::parse(
            format!(
                "ODS GRAPHICS {} requires a positive integer",
                name.to_uppercase()
            ),
            tok.span,
        )),
    }
}

/// Helper M29.1 : parse un format d'image (PNG|SVG), avec forme parenthésée
/// optionnelle `IMAGEFMT=(PNG)`.
pub(super) fn parse_imagefmt(ts: &mut StatementStream) -> Result<crate::ods_graphics::ImageFmt> {
    use crate::ods_graphics::ImageFmt;
    let parenthesized = ts.peek().kind == TokenKind::LParen;
    if parenthesized {
        ts.next(); // consume `(`
    }
    let tok = ts.peek().clone();
    let name = match tok.ident() {
        Some(s) => s.to_ascii_lowercase(),
        None => {
            return Err(SasError::parse("IMAGEFMT= requires PNG or SVG", tok.span));
        }
    };
    let fmt = match name.as_str() {
        "png" => ImageFmt::Png,
        "svg" => ImageFmt::Svg,
        other => {
            return Err(SasError::parse(
                format!(
                    "IMAGEFMT={} is not supported (use PNG or SVG)",
                    other.to_uppercase()
                ),
                tok.span,
            ));
        }
    };
    ts.next(); // consume format ident
    if parenthesized {
        if ts.peek().kind != TokenKind::RParen {
            return Err(SasError::parse(
                "expected ')' after IMAGEFMT=(...)",
                ts.peek().span,
            ));
        }
        ts.next(); // consume `)`
    }
    Ok(fmt)
}

/// Parse les options d'un statement `ODS <dest>` jusqu'au `;` : `FILE=`,
/// `STYLE=`, `OPTIONS=`. Renvoie `(file, style)`. Les options inconnues lèvent
/// une erreur de parse (schéma large v1 strict sur les options).
///
/// J02-P6 — STYLE= (stocké dans l'AST mais appliqué par aucune destination)
/// et OPTIONS= étaient ignorées sans diagnostic : WARNING d'affichage.
pub(super) fn parse_ods_options(
    ts: &mut StatementStream,
    dest: &str,
) -> Result<(Option<String>, Option<String>)> {
    let mut file: Option<String> = None;
    let mut style: Option<String> = None;

    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }
        let name_tok = ts.peek().clone();
        let name = match name_tok.ident() {
            Some(s) => s.to_ascii_lowercase(),
            None => {
                return Err(SasError::parse(
                    "Expected an ODS option name (FILE=, STYLE=, ...) or ';'",
                    name_tok.span,
                ));
            }
        };
        ts.next(); // consume option name

        match name.as_str() {
            "file" | "style" | "options" => {
                // Toutes ces options attendent `= valeur`.
                if ts.peek().kind != TokenKind::Eq {
                    return Err(SasError::parse(
                        format!(
                            "ODS option {} requires a value (e.g. {}=...)",
                            name.to_uppercase(),
                            name.to_uppercase()
                        ),
                        ts.peek().span,
                    ));
                }
                ts.next(); // consume `=`
                let val_tok = ts.peek().clone();
                let value = parse_option_value(ts, &val_tok.span)?;
                if name != "file" {
                    ts.warn_ignored_display(format!(
                        "The {}= option of the ODS {} statement is ignored in this build; \
                         display customization is not supported.",
                        name.to_ascii_uppercase(),
                        dest.to_ascii_uppercase()
                    ));
                }
                match name.as_str() {
                    "file" => file = Some(value),
                    "style" => style = Some(value),
                    _ => {}
                }
            }
            other => {
                return Err(SasError::parse(
                    format!(
                        "ODS option '{}' is not supported in this build.",
                        other.to_uppercase()
                    ),
                    name_tok.span,
                ));
            }
        }
    }

    Ok((file, style))
}
