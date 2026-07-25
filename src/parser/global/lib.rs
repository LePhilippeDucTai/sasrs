use super::*;

// ── LIBNAME ─────────────────────────────────────────────────────────────────

pub(super) fn parse_libname(ts: &mut StatementStream) -> Result<GlobalStmt> {
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
pub(super) fn parse_filename(ts: &mut StatementStream) -> Result<GlobalStmt> {
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
