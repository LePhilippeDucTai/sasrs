//! Statements globaux : LIBNAME, OPTIONS, TITLEn.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! Appelé par `parser::next_block()` ; le mot-clé de tête est encore dans
//! le stream (peek) ou déjà identifié par l'appelant — convention :
//! l'appelant N'A PAS consommé le mot-clé, `parse_global` le consomme.
//!
//! ## LIBNAME
//! - `libname ref 'chemin' ;`  → `GlobalStmt::Libname` (chemin = littéral
//!   chaîne ; relatif → résolu contre `Session::base_dir` à l'exécution).
//! - `libname ref clear ;`     → `GlobalStmt::LibnameClear`.
//!
//! ## TITLE
//! - `title 'texte' ;` / `titleN 'texte' ;` (N=1..9, suffixe dans
//!   l'ident) ; sans texte → efface. M1 : seul TITLE1 est rendu par le
//!   listing.
//!
//! ## OPTIONS
//! - `options name[=valeur]... ;` → liste brute. L'exécution (executor)
//!   applique `ls=` (40..=256) et ignore le reste avec WARNING
//!   "Option XXX is not yet supported".

use super::{footnote_level, title_level, StatementStream};
use crate::ast::{DatasetRef, GlobalStmt, OdsAction};
use crate::error::{Result, SasError};
use crate::token::{Span, StrSuffix, TokenKind};

/// Parse a global statement (LIBNAME, OPTIONS, or TITLEn).
///
/// The leading keyword token must still be in the stream (not yet consumed);
/// this function consumes it and the closing `;`.
pub fn parse_global(ts: &mut StatementStream) -> Result<GlobalStmt> {
    let head = ts.peek().clone();
    let kw = match head.ident() {
        Some(s) => s.to_ascii_lowercase(),
        None => {
            return Err(SasError::parse(
                "expected LIBNAME, FILENAME, OPTIONS, or TITLE keyword",
                head.span,
            ));
        }
    };

    if kw == "libname" {
        ts.next(); // consume `libname`
        parse_libname(ts)
    } else if kw == "filename" {
        ts.next(); // consume `filename`
        parse_filename(ts)
    } else if kw == "options" {
        ts.next(); // consume `options`
        parse_options(ts)
    } else if kw == "ods" {
        ts.next(); // consume `ods`
        parse_ods_statement(ts)
    } else if let Some(n) = title_level(&kw) {
        ts.next(); // consume `titleN`
        parse_title(ts, n)
    } else if let Some(n) = footnote_level(&kw) {
        ts.next(); // consume `footnoteN`
        parse_footnote(ts, n)
    } else {
        Err(SasError::parse(
            format!(
                "Expected LIBNAME, FILENAME, OPTIONS, ODS, TITLEn, or FOOTNOTEn; got '{}'",
                kw.to_uppercase()
            ),
            head.span,
        ))
    }
}

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
/// Options reconnues (parsées, stockées pour M22.4+) : `FILE='...'`,
/// `STYLE=name`, `OPTIONS=...` (ignorée). `SELECT`/`EXCLUDE` → différés M22.3.
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
        let (file, style) = parse_ods_options(ts)?;
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
        Some(ref a) if a == "select" => {
            return Err(SasError::parse(
                "ODS SELECT is not yet supported (deferred to M22.3)",
                ts.peek().span,
            ));
        }
        Some(ref a) if a == "exclude" => {
            return Err(SasError::parse(
                "ODS EXCLUDE is not yet supported (deferred to M22.3)",
                ts.peek().span,
            ));
        }
        _ => OdsAction::Open,
    };

    let (file, style) = parse_ods_options(ts)?;
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
fn parse_ods_output(ts: &mut StatementStream) -> Result<GlobalStmt> {
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

/// Parse `ODS GRAPHICS ...` (le mot-clé `GRAPHICS` a déjà été consommé) — M29.1.
///
/// Grammaire : `ODS GRAPHICS [ON|OFF] [ / opt=val ... ] ;`
///
/// Options après le `/` :
/// - `WIDTH=nnn` / `HEIGHT=nnn` (pixels)
/// - `IMAGEFMT=PNG|SVG` (ou forme parenthésée `IMAGEFMT=(PNG)`)
/// - `IMAGENAME="fig"` (préfixe de nommage)
/// - `RESET[=index|all]` — parsée puis ignorée (v1)
///
/// Les options sont conservées PAR-STATEMENT (champs `Option`) : c'est leur
/// présence/absence qui pilote la NOTE de log à l'exécution.
fn parse_ods_graphics(ts: &mut StatementStream) -> Result<GlobalStmt> {
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
                    expect_eq(ts, &name)?;
                    let v = parse_dim(ts, &name)?;
                    if name == "width" {
                        width = Some(v);
                    } else {
                        height = Some(v);
                    }
                }
                "imagefmt" | "outputfmt" => {
                    expect_eq(ts, &name)?;
                    imagefmt = Some(parse_imagefmt(ts)?);
                }
                "imagename" => {
                    expect_eq(ts, &name)?;
                    let val_tok = ts.peek().clone();
                    let value = parse_option_value(ts, &val_tok.span)?;
                    imagename = Some(value);
                }
                "reset" => {
                    // `RESET` ou `RESET=index|all` — parsée puis ignorée (v1).
                    if ts.peek().kind == TokenKind::Eq {
                        ts.next(); // consume `=`
                        let val_tok = ts.peek().clone();
                        let _ = parse_option_value(ts, &val_tok.span)?;
                    }
                }
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

/// Helper M29.1 : exige un `=` après un nom d'option ODS GRAPHICS.
fn expect_eq(ts: &mut StatementStream, name: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("ODS GRAPHICS option {} requires a value (e.g. {}=...)", name.to_uppercase(), name.to_uppercase()),
            ts.peek().span,
        ));
    }
    ts.next(); // consume `=`
    Ok(())
}

/// Helper M29.1 : parse une dimension entière positive (WIDTH=/HEIGHT=).
fn parse_dim(ts: &mut StatementStream, name: &str) -> Result<u32> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Num(f) if *f >= 0.0 && f.fract() == 0.0 => {
            let v = *f as u32;
            ts.next();
            Ok(v)
        }
        _ => Err(SasError::parse(
            format!("ODS GRAPHICS {} requires a positive integer", name.to_uppercase()),
            tok.span,
        )),
    }
}

/// Helper M29.1 : parse un format d'image (PNG|SVG), avec forme parenthésée
/// optionnelle `IMAGEFMT=(PNG)`.
fn parse_imagefmt(ts: &mut StatementStream) -> Result<crate::ods_graphics::ImageFmt> {
    use crate::ods_graphics::ImageFmt;
    let parenthesized = ts.peek().kind == TokenKind::LParen;
    if parenthesized {
        ts.next(); // consume `(`
    }
    let tok = ts.peek().clone();
    let name = match tok.ident() {
        Some(s) => s.to_ascii_lowercase(),
        None => {
            return Err(SasError::parse(
                "IMAGEFMT= requires PNG or SVG",
                tok.span,
            ));
        }
    };
    let fmt = match name.as_str() {
        "png" => ImageFmt::Png,
        "svg" => ImageFmt::Svg,
        other => {
            return Err(SasError::parse(
                format!("IMAGEFMT={} is not supported (use PNG or SVG)", other.to_uppercase()),
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

/// Parse les options d'un statement `ODS` jusqu'au `;` : `FILE=`, `STYLE=`,
/// `OPTIONS=` (ignorée). Renvoie `(file, style)`. Les options inconnues lèvent
/// une erreur de parse (schéma large v1 strict sur les options).
fn parse_ods_options(ts: &mut StatementStream) -> Result<(Option<String>, Option<String>)> {
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
                        format!("ODS option {} requires a value (e.g. {}=...)", name.to_uppercase(), name.to_uppercase()),
                        ts.peek().span,
                    ));
                }
                ts.next(); // consume `=`
                let val_tok = ts.peek().clone();
                let value = parse_option_value(ts, &val_tok.span)?;
                match name.as_str() {
                    "file" => file = Some(value),
                    "style" => style = Some(value),
                    // OPTIONS= : parsée mais ignorée en v1.
                    _ => {}
                }
            }
            other => {
                return Err(SasError::parse(
                    format!("ODS option '{}' is not supported in this build.", other.to_uppercase()),
                    name_tok.span,
                ));
            }
        }
    }

    Ok((file, style))
}

// ── LIBNAME ─────────────────────────────────────────────────────────────────

fn parse_libname(ts: &mut StatementStream) -> Result<GlobalStmt> {
    // Expect the libref identifier.
    let ref_tok = ts.peek().clone();
    let libref = match ref_tok.ident() {
        Some(s) => s.to_string(),
        None => {
            return Err(SasError::parse(
                "LIBNAME requires a libref (1–8 character identifier)",
                ref_tok.span,
            ));
        }
    };

    // Validate: libref must be ≤ 8 characters.
    if libref.len() > 8 {
        return Err(SasError::parse(
            format!(
                "The libref {} exceeds the maximum of 8 characters.",
                libref.to_uppercase()
            ),
            ref_tok.span,
        ));
    }
    ts.next(); // consume libref

    // Peek at what follows: `clear` keyword, a string literal (path), or
    // an engine identifier followed by a string literal.
    let next_tok = ts.peek().clone();

    // `libname ref clear ;`
    if next_tok.is_kw("clear") {
        ts.next(); // consume `clear`
        ts.expect_semi()?;
        return Ok(GlobalStmt::LibnameClear { libref });
    }

    // `libname ref 'path' ;`  — no engine
    if let TokenKind::Str { value, suffix } = &next_tok.kind {
        if *suffix != StrSuffix::None {
            return Err(SasError::parse(
                "LIBNAME path must be a plain string literal (no date/time suffix)",
                next_tok.span,
            ));
        }
        let path = value.clone();
        ts.next(); // consume the string literal
        ts.expect_semi()?;
        return Ok(GlobalStmt::Libname { libref, engine: None, path });
    }

    // `libname ref <engine> 'path' ;`  — engine identifier before the path.
    // Known engines: CSV, XLSX, EXCEL, PARQUET, BASE, V9 (plus any identifier
    // is accepted and uppercased; the executor emits an error for unknowns).
    if let TokenKind::Ident(eng) = &next_tok.kind {
        let engine = eng.to_ascii_uppercase();
        ts.next(); // consume the engine identifier

        // Now expect the path string literal.
        let path_tok = ts.peek().clone();
        match &path_tok.kind {
            TokenKind::Str { value, suffix } => {
                if *suffix != StrSuffix::None {
                    return Err(SasError::parse(
                        "LIBNAME path must be a plain string literal (no date/time suffix)",
                        path_tok.span,
                    ));
                }
                let path = value.clone();
                ts.next(); // consume the string literal
                ts.expect_semi()?;
                return Ok(GlobalStmt::Libname { libref, engine: Some(engine), path });
            }
            _ => {
                return Err(SasError::parse(
                    format!(
                        "Expected a quoted path after engine {} for libref {}; \
                         got an unexpected token.",
                        engine,
                        libref.to_uppercase()
                    ),
                    path_tok.span,
                ));
            }
        }
    }

    Err(SasError::parse(
        format!(
            "Expected a quoted path or CLEAR after libref {}; \
             got an unexpected token.",
            libref.to_uppercase()
        ),
        next_tok.span,
    ))
}

// ── FILENAME (M35.2) ─────────────────────────────────────────────────────────

/// Parse `FILENAME fileref <chemin|device> ;` (le mot-clé `FILENAME` est déjà
/// consommé) — M35.2, forme MINIMALE.
///
/// Formes reconnues :
/// - `FILENAME ref 'chemin' ;` / `FILENAME ref "chemin" ;` : chemin entre
///   guillemets → enregistré tel quel (résolu à l'exécution).
/// - `FILENAME ref chemin ;` : un identifiant nu est traité comme un chemin
///   (fichier ou répertoire), SAUF s'il s'agit d'un mot-clé device reconnu.
/// - `FILENAME ref TEMP|PIPE|URL|DUMMY|… [...] ;` : device/options →
///   accepté-et-ignoré (`path = None`, `device = Some(...)`), une NOTE est
///   émise à l'exécution. Le reste de la ligne (options) est consommé sans
///   interprétation.
///
/// On reste volontairement permissif : tout token résiduel jusqu'au `;` est
/// consommé (les options éventuelles d'un `FILENAME ref 'p' lrecl=...;` sont
/// ignorées) afin de ne JAMAIS produire d'erreur de parse sur un `FILENAME`.
fn parse_filename(ts: &mut StatementStream) -> Result<GlobalStmt> {
    // Nom du fileref (identifiant).
    let ref_tok = ts.peek().clone();
    let fileref = match ref_tok.ident() {
        Some(s) => s.to_string(),
        None => {
            return Err(SasError::parse(
                "FILENAME requires a fileref (1–8 character identifier)",
                ref_tok.span,
            ));
        }
    };
    ts.next(); // consume fileref

    // Token suivant : chemin entre guillemets, identifiant (device ou chemin nu),
    // ou directement `;` (forme dégénérée acceptée comme no-op).
    let next_tok = ts.peek().clone();
    let mut path: Option<String> = None;
    let mut device: Option<String> = None;

    match &next_tok.kind {
        TokenKind::Semi => {
            // `FILENAME ref ;` — rien à enregistrer (no-op).
        }
        TokenKind::Str { value, suffix } => {
            if *suffix != StrSuffix::None {
                return Err(SasError::parse(
                    "FILENAME path must be a plain string literal (no date/time suffix)",
                    next_tok.span,
                ));
            }
            path = Some(value.clone());
            ts.next(); // consume the string literal
        }
        TokenKind::Ident(id) => {
            // Mots-clés device connus → accepté-et-ignoré (pas de chemin).
            let upper = id.to_ascii_uppercase();
            const DEVICES: &[&str] = &[
                "TEMP", "PIPE", "URL", "DUMMY", "TERMINAL", "PLOTTER", "PRINTER",
                "TAPE", "DISK", "CATALOG", "FTP", "SOCKET", "EMAIL", "CLIPBRD",
                "ZIP", "HADOOP", "SFTP", "WEBDAV", "JMS",
            ];
            if DEVICES.contains(&upper.as_str()) {
                device = Some(upper);
            } else {
                // Identifiant nu traité comme chemin (fichier ou répertoire).
                path = Some(id.clone());
            }
            ts.next(); // consume the identifier
        }
        _ => {
            return Err(SasError::parse(
                format!(
                    "Expected a quoted path, a path, or a device after fileref {}.",
                    fileref.to_uppercase()
                ),
                next_tok.span,
            ));
        }
    }

    // Consommer tout résidu (options device, lrecl=, etc.) jusqu'au `;`.
    while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
        ts.next();
    }
    ts.expect_semi()?;
    Ok(GlobalStmt::Filename { fileref, path, device })
}

// ── TITLE ────────────────────────────────────────────────────────────────────

fn parse_title(ts: &mut StatementStream, n: u8) -> Result<GlobalStmt> {
    // `title ;` or `titleN ;` — no text, clears the title.
    if ts.peek().kind == TokenKind::Semi {
        ts.expect_semi()?;
        return Ok(GlobalStmt::Title { n, text: None });
    }

    // Only a quoted string literal is accepted in M1.
    //
    // Note: SAS itself accepts unquoted text after TITLE (e.g. `title My Report;`),
    // but our M1 parser intentionally restricts this to quoted string literals only.
    // Unquoted text after TITLE is an error here; this keeps the AST unambiguous and
    // avoids complex multi-token text concatenation. Callers should quote their titles.
    let text_tok = ts.peek().clone();
    match &text_tok.kind {
        TokenKind::Str { value, suffix } => {
            if *suffix != StrSuffix::None {
                return Err(SasError::parse(
                    "TITLE text must be a plain string literal (no date/time suffix)",
                    text_tok.span,
                ));
            }
            let text = value.clone();
            ts.next(); // consume the string literal
            ts.expect_semi()?;
            Ok(GlobalStmt::Title { n, text: Some(text) })
        }
        _ => {
            // Unquoted text or any non-string token after TITLE.
            Err(SasError::parse(
                "TITLE text must be a quoted string literal, e.g. title 'My Report';",
                text_tok.span,
            ))
        }
    }
}

// ── FOOTNOTE ───────────────────────────────────────────────────────────────

/// Parse `FOOTNOTEn ['texte'];`. Même grammaire que TITLE : soit une chaîne
/// littérale simple, soit rien (efface le niveau). Le niveau `n` (1..9) est déjà
/// extrait par l'appelant.
fn parse_footnote(ts: &mut StatementStream, n: u8) -> Result<GlobalStmt> {
    // `footnote ;` or `footnoteN ;` — no text, clears the footnote.
    if ts.peek().kind == TokenKind::Semi {
        ts.expect_semi()?;
        return Ok(GlobalStmt::Footnote { n, text: None });
    }

    let text_tok = ts.peek().clone();
    match &text_tok.kind {
        TokenKind::Str { value, suffix } => {
            if *suffix != StrSuffix::None {
                return Err(SasError::parse(
                    "FOOTNOTE text must be a plain string literal (no date/time suffix)",
                    text_tok.span,
                ));
            }
            let text = value.clone();
            ts.next(); // consume the string literal
            ts.expect_semi()?;
            Ok(GlobalStmt::Footnote { n, text: Some(text) })
        }
        _ => Err(SasError::parse(
            "FOOTNOTE text must be a quoted string literal, e.g. footnote 'My Note';",
            text_tok.span,
        )),
    }
}

// ── OPTIONS ──────────────────────────────────────────────────────────────────

fn parse_options(ts: &mut StatementStream) -> Result<GlobalStmt> {
    let mut opts: Vec<(String, Option<String>)> = Vec::new();

    // Collect `name` or `name=value` pairs until `;`.
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }

        // The option name must be an identifier.
        let name_tok = ts.peek().clone();
        let name = match name_tok.ident() {
            Some(s) => s.to_string(),
            None => {
                return Err(SasError::parse(
                    "Expected an option name (identifier) in OPTIONS statement",
                    name_tok.span,
                ));
            }
        };
        ts.next(); // consume the name

        // Check for an `=` (value follows) or just a flag.
        if ts.peek().kind == TokenKind::Eq {
            ts.next(); // consume `=`
            // FMTSEARCH= and MISSING= accept a parenthesised list `(a b c)`.
            // We handle this here rather than in `parse_option_value` to avoid
            // accepting `(` universally for all options.
            let name_lc = name.to_ascii_lowercase();
            if (name_lc == "fmtsearch" || name_lc == "missing")
                && ts.peek().kind == TokenKind::LParen
            {
                let value = parse_paren_list(ts)?;
                opts.push((name, Some(value)));
            } else {
                let val_tok = ts.peek().clone();
                let value = parse_option_value(ts, &val_tok.span)?;
                opts.push((name, Some(value)));
            }
        } else {
            // Boolean flag: `nocenter`, `center`, etc.
            opts.push((name, None));
        }
    }

    ts.expect_semi()?;
    Ok(GlobalStmt::Options(opts))
}

/// Parse a parenthesised list of identifiers for OPTIONS values such as
/// `FMTSEARCH=(lib1 lib2)` and `MISSING=(. .)`.
/// The leading `(` must still be in the stream; it is consumed here.
/// Returns the identifiers joined by spaces (e.g. `"lib1 lib2"`).
fn parse_paren_list(ts: &mut StatementStream) -> Result<String> {
    let lparen = ts.peek().clone();
    ts.next(); // consume `(`
    let mut items: Vec<String> = Vec::new();
    loop {
        match ts.peek().kind {
            TokenKind::RParen | TokenKind::Semi | TokenKind::Eof => break,
            _ => {}
        }
        let tok = ts.peek().clone();
        match tok.ident() {
            Some(s) => {
                items.push(s.to_string());
                ts.next();
            }
            None => {
                return Err(SasError::parse(
                    "Expected an identifier or ')' inside parenthesised OPTIONS value",
                    tok.span,
                ));
            }
        }
    }
    if ts.peek().kind != TokenKind::RParen {
        return Err(SasError::parse(
            "Expected ')' to close parenthesised OPTIONS value",
            lparen.span,
        ));
    }
    ts.next(); // consume `)`
    Ok(items.join(" "))
}

/// Parse the value token after `=` in an OPTIONS pair.
/// Accepts: identifier, integer or float number, plain string literal.
fn parse_option_value(ts: &mut StatementStream, _span: &Span) -> Result<String> {
    let val_tok = ts.peek().clone();
    match &val_tok.kind {
        TokenKind::Ident(s) => {
            let s = s.clone();
            ts.next();
            Ok(s)
        }
        TokenKind::Num(f) => {
            let f = *f;
            ts.next();
            // Format integers without a trailing ".0" for readability.
            if f.fract() == 0.0 && f.abs() < 1e15 {
                Ok(format!("{}", f as i64))
            } else {
                Ok(format!("{}", f))
            }
        }
        TokenKind::Str { value, suffix } => {
            if *suffix != StrSuffix::None {
                return Err(SasError::parse(
                    "OPTIONS value must be a plain string literal (no date/time suffix)",
                    val_tok.span,
                ));
            }
            let v = value.clone();
            ts.next();
            Ok(v)
        }
        _ => Err(SasError::parse(
            "Expected an identifier, number, or quoted string as OPTIONS value",
            val_tok.span,
        )),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::GlobalStmt;
    use crate::source::SourceFile;

    fn parse(src: &str) -> Result<GlobalStmt> {
        let sf = SourceFile::new(src);
        let mut ts = StatementStream::new(&sf).unwrap();
        parse_global(&mut ts)
    }

    // ── LIBNAME ──────────────────────────────────────────────────────────────

    #[test]
    fn libname_with_path() {
        let stmt = parse("libname mylib '/data/sas';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "mylib".into(),
                engine: None,
                path: "/data/sas".into(),
            }
        );
    }

    #[test]
    fn libname_relative_path() {
        let stmt = parse("libname outlib 'output/results';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "outlib".into(),
                engine: None,
                path: "output/results".into(),
            }
        );
    }

    #[test]
    fn libname_with_csv_engine() {
        let stmt = parse("libname csvlib csv '/data/csv';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "csvlib".into(),
                engine: Some("CSV".into()),
                path: "/data/csv".into(),
            }
        );
    }

    #[test]
    fn libname_with_xlsx_engine() {
        let stmt = parse("libname xl xlsx '/data/xl';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "xl".into(),
                engine: Some("XLSX".into()),
                path: "/data/xl".into(),
            }
        );
    }

    #[test]
    fn libname_with_parquet_engine() {
        let stmt = parse("libname pq parquet '/data/pq';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "pq".into(),
                engine: Some("PARQUET".into()),
                path: "/data/pq".into(),
            }
        );
    }

    #[test]
    fn libname_engine_is_uppercased() {
        let stmt = parse("libname x Csv '/tmp';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "x".into(),
                engine: Some("CSV".into()),
                path: "/tmp".into(),
            }
        );
    }

    #[test]
    fn libname_clear() {
        let stmt = parse("libname mylib clear;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::LibnameClear {
                libref: "mylib".into(),
            }
        );
    }

    #[test]
    fn libname_clear_case_insensitive() {
        let stmt = parse("LIBNAME MYLIB CLEAR;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::LibnameClear {
                libref: "MYLIB".into(),
            }
        );
    }

    #[test]
    fn libname_libref_too_long_is_error() {
        // "toolonglib" = 10 characters — must error.
        let err = parse("libname toolonglib '/path';").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("libref") || msg.contains("8"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn libname_missing_libref_is_error() {
        // `libname '/path';` — no libref identifier.
        let err = parse("libname '/path';").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("libref"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn libname_missing_path_is_error() {
        // `libname mylib 123;` — path is not a string literal.
        let err = parse("libname mylib 123;").unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    // ── FILENAME (M35.2) ─────────────────────────────────────────────────────

    #[test]
    fn filename_quoted_path() {
        let stmt = parse("filename inc '/tmp/x.sas';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Filename {
                fileref: "inc".into(),
                path: Some("/tmp/x.sas".into()),
                device: None,
            }
        );
    }

    #[test]
    fn filename_bare_path() {
        // Un identifiant nu (non-device) est traité comme chemin.
        let stmt = parse("filename inc myfile;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Filename {
                fileref: "inc".into(),
                path: Some("myfile".into()),
                device: None,
            }
        );
    }

    #[test]
    fn filename_device_temp_ignored() {
        let stmt = parse("filename tmp TEMP;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Filename {
                fileref: "tmp".into(),
                path: None,
                device: Some("TEMP".into()),
            }
        );
    }

    #[test]
    fn filename_options_after_path_ignored() {
        // Résidu d'options après le chemin → consommé sans erreur.
        let stmt = parse("filename inc '/tmp/x.sas' lrecl=256;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Filename {
                fileref: "inc".into(),
                path: Some("/tmp/x.sas".into()),
                device: None,
            }
        );
    }

    // ── TITLE ────────────────────────────────────────────────────────────────

    #[test]
    fn title_simple() {
        let stmt = parse("title 'My Report';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Title {
                n: 1,
                text: Some("My Report".into()),
            }
        );
    }

    #[test]
    fn title_uppercase() {
        let stmt = parse("TITLE 'My Report';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Title {
                n: 1,
                text: Some("My Report".into()),
            }
        );
    }

    #[test]
    fn title3() {
        let stmt = parse("title3 'Section Header';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Title {
                n: 3,
                text: Some("Section Header".into()),
            }
        );
    }

    #[test]
    fn title9() {
        let stmt = parse("title9 'Footer';").unwrap();
        assert_eq!(stmt, GlobalStmt::Title { n: 9, text: Some("Footer".into()) });
    }

    #[test]
    fn title_without_text_clears() {
        // `title;` — no text, clears title 1.
        let stmt = parse("title;").unwrap();
        assert_eq!(stmt, GlobalStmt::Title { n: 1, text: None });
    }

    #[test]
    fn title5_without_text_clears() {
        let stmt = parse("title5;").unwrap();
        assert_eq!(stmt, GlobalStmt::Title { n: 5, text: None });
    }

    #[test]
    fn title_unquoted_text_is_error() {
        // SAS accepts unquoted text but our M1 parser requires a quoted literal.
        // `title foo;` must return a parse error.
        let err = parse("title foo;").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("quoted") || msg.to_lowercase().contains("string"),
            "expected an error about quoted string, got: {msg}"
        );
    }

    // ── FOOTNOTE ───────────────────────────────────────────────────────────

    #[test]
    fn footnote_simple() {
        let stmt = parse("footnote 'My Note';").unwrap();
        assert_eq!(stmt, GlobalStmt::Footnote { n: 1, text: Some("My Note".into()) });
    }

    #[test]
    fn footnote3() {
        let stmt = parse("footnote3 'Third';").unwrap();
        assert_eq!(stmt, GlobalStmt::Footnote { n: 3, text: Some("Third".into()) });
    }

    #[test]
    fn footnote_without_text_clears() {
        let stmt = parse("footnote;").unwrap();
        assert_eq!(stmt, GlobalStmt::Footnote { n: 1, text: None });
    }

    #[test]
    fn footnote5_without_text_clears() {
        let stmt = parse("footnote5;").unwrap();
        assert_eq!(stmt, GlobalStmt::Footnote { n: 5, text: None });
    }

    #[test]
    fn footnote_unquoted_text_is_error() {
        let err = parse("footnote foo;").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("quoted") || msg.to_lowercase().contains("string"),
            "expected an error about quoted string, got: {msg}"
        );
    }

    // ── OPTIONS ──────────────────────────────────────────────────────────────

    #[test]
    fn options_ls_and_nocenter() {
        let stmt = parse("options ls=80 nocenter;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Options(vec![
                ("ls".into(), Some("80".into())),
                ("nocenter".into(), None),
            ])
        );
    }

    #[test]
    fn options_string_value() {
        // FMTSEARCH= now accepts a parenthesised list — this must parse successfully.
        let stmt = parse("options fmtsearch=(mylib work);").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Options(vec![("fmtsearch".into(), Some("mylib work".into()))])
        );

        // `(` in any other options value (not FMTSEARCH/MISSING) is still an error.
        let err = parse("options notes=(yes);").unwrap_err();
        let _ = err; // just verify no panic

        // A proper string value:
        let stmt = parse("options label='My value';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Options(vec![("label".into(), Some("My value".into()))])
        );
    }

    #[test]
    fn options_empty_is_ok() {
        // `options;` — empty list is accepted (no-op per spec).
        let stmt = parse("options;").unwrap();
        assert_eq!(stmt, GlobalStmt::Options(vec![]));
    }

    #[test]
    fn options_multiple_flags_and_values() {
        let stmt = parse("options center ps=60 linesize=132 nodate;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Options(vec![
                ("center".into(), None),
                ("ps".into(), Some("60".into())),
                ("linesize".into(), Some("132".into())),
                ("nodate".into(), None),
            ])
        );
    }

    #[test]
    fn options_float_value() {
        // A float value should be formatted without trailing `.0` for integers.
        let stmt = parse("options decimals=2.5;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Options(vec![("decimals".into(), Some("2.5".into()))])
        );
    }

    // ── ODS ──────────────────────────────────────────────────────────────────

    #[test]
    fn parse_ods_listing() {
        let stmt = parse("ODS LISTING ;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Ods {
                destination: "listing".into(),
                action: OdsAction::Open,
                file: None,
                style: None,
            }
        );
    }

    #[test]
    fn parse_ods_html_open() {
        let stmt = parse("ods html;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Ods {
                destination: "html".into(),
                action: OdsAction::Open,
                file: None,
                style: None,
            }
        );
    }

    #[test]
    fn parse_ods_html_with_file() {
        let stmt = parse("ODS HTML FILE='out.html';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Ods {
                destination: "html".into(),
                action: OdsAction::Open,
                file: Some("out.html".into()),
                style: None,
            }
        );
    }

    #[test]
    fn parse_ods_html_with_file_and_style() {
        let stmt = parse("ods html file='r.html' style=journal;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Ods {
                destination: "html".into(),
                action: OdsAction::Open,
                file: Some("r.html".into()),
                style: Some("journal".into()),
            }
        );
    }

    #[test]
    fn parse_ods_rtf_pdf_excel_stubs() {
        for (src, dest) in [
            ("ods rtf;", "rtf"),
            ("ods pdf;", "pdf"),
            ("ods excel;", "excel"),
        ] {
            let stmt = parse(src).unwrap();
            assert_eq!(
                stmt,
                GlobalStmt::Ods {
                    destination: dest.into(),
                    action: OdsAction::Open,
                    file: None,
                    style: None,
                }
            );
        }
    }

    #[test]
    fn parse_ods_close_destination_after_name() {
        let stmt = parse("ODS HTML CLOSE;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Ods {
                destination: "html".into(),
                action: OdsAction::Close,
                file: None,
                style: None,
            }
        );
    }

    #[test]
    fn parse_ods_close_verb_with_name() {
        let stmt = parse("ods close html;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Ods {
                destination: "html".into(),
                action: OdsAction::Close,
                file: None,
                style: None,
            }
        );
    }

    #[test]
    fn parse_ods_close_bare_defaults_listing() {
        let stmt = parse("ODS CLOSE;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Ods {
                destination: "listing".into(),
                action: OdsAction::Close,
                file: None,
                style: None,
            }
        );
    }

    #[test]
    fn parse_ods_select_is_deferred_error() {
        let err = parse("ods html select foo;").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("select") && msg.contains("M22.3"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn parse_ods_unknown_option_is_error() {
        let err = parse("ods html bogus=1;").unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn parse_ods_case_insensitive() {
        let stmt = parse("Ods Html Close ;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Ods {
                destination: "html".into(),
                action: OdsAction::Close,
                file: None,
                style: None,
            }
        );
    }

    // ── ODS OUTPUT (M22.3) ───────────────────────────────────────────────────

    #[test]
    fn parse_ods_output_single_mapping() {
        let stmt = parse("ods output Summary=out;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::OdsOutput {
                mappings: vec![(
                    "Summary".into(),
                    DatasetRef {
                        libref: None,
                        name: "out".into(),
                    }
                )],
                close: false,
            }
        );
    }

    #[test]
    fn parse_ods_output_two_mappings() {
        let stmt = parse("ods output a=x b=y;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::OdsOutput {
                mappings: vec![
                    ("a".into(), DatasetRef { libref: None, name: "x".into() }),
                    ("b".into(), DatasetRef { libref: None, name: "y".into() }),
                ],
                close: false,
            }
        );
    }

    #[test]
    fn parse_ods_output_qualified_target() {
        let stmt = parse("ods output OneWayFreqs=work.freq_out;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::OdsOutput {
                mappings: vec![(
                    "OneWayFreqs".into(),
                    DatasetRef {
                        libref: Some("work".into()),
                        name: "freq_out".into(),
                    }
                )],
                close: false,
            }
        );
    }

    #[test]
    fn parse_ods_output_close() {
        let stmt = parse("ods output close;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::OdsOutput {
                mappings: vec![],
                close: true,
            }
        );
    }

    #[test]
    fn parse_ods_output_requires_equals() {
        let err = parse("ods output Summary;").unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    // ── ODS GRAPHICS (M29.1) ─────────────────────────────────────────────────

    use crate::ast::{OdsGraphicsStmt, OdsGraphicsToggle};
    use crate::ods_graphics::ImageFmt;

    fn graphics_stmt(src: &str) -> OdsGraphicsStmt {
        match parse(src).unwrap() {
            GlobalStmt::OdsGraphics(s) => s,
            other => panic!("expected OdsGraphics, got {other:?}"),
        }
    }

    #[test]
    fn parse_ods_graphics_on() {
        let s = graphics_stmt("ods graphics on;");
        assert_eq!(s.toggle, OdsGraphicsToggle::On);
        assert_eq!(s.width, None);
        assert_eq!(s.height, None);
        assert_eq!(s.imagefmt, None);
    }

    #[test]
    fn parse_ods_graphics_off() {
        let s = graphics_stmt("ODS GRAPHICS OFF;");
        assert_eq!(s.toggle, OdsGraphicsToggle::Off);
    }

    #[test]
    fn parse_ods_graphics_on_with_dims() {
        let s = graphics_stmt("ods graphics on / width=1000 height=700;");
        assert_eq!(s.toggle, OdsGraphicsToggle::On);
        assert_eq!(s.width, Some(1000));
        assert_eq!(s.height, Some(700));
    }

    #[test]
    fn parse_ods_graphics_imagefmt_svg() {
        let s = graphics_stmt("ods graphics on / imagefmt=svg;");
        assert_eq!(s.imagefmt, Some(ImageFmt::Svg));
    }

    #[test]
    fn parse_ods_graphics_imagefmt_png_parenthesized() {
        let s = graphics_stmt("ods graphics on / imagefmt=(png);");
        assert_eq!(s.imagefmt, Some(ImageFmt::Png));
    }

    #[test]
    fn parse_ods_graphics_imagename_and_reset() {
        let s = graphics_stmt("ods graphics / imagename=\"myfig\" reset=index;");
        assert_eq!(s.toggle, OdsGraphicsToggle::None);
        assert_eq!(s.imagename.as_deref(), Some("myfig"));
    }

    #[test]
    fn parse_ods_graphics_reset_bare() {
        let s = graphics_stmt("ods graphics on / reset width=640;");
        assert_eq!(s.toggle, OdsGraphicsToggle::On);
        assert_eq!(s.width, Some(640));
    }
}
