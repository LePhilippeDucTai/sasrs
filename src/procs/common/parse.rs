use super::*;

// ───────────────────── couche de parsing partagée (M31.1) ─────────────────────
//
// Combinateurs réutilisables pour le parsing des statements PROC. Ils
// centralisent les boucles d'options / de sous-statements, la résolution
// `expect_eq` / `OUT=` / `DATA=`, le message « Unexpected option » et la
// résolution `_LAST_`, aujourd'hui dupliqués à l'identique dans la quarantaine
// de fichiers `procs/<proc>.rs`. Reproduits VERBATIM depuis `print.rs`
// (boucles) et `sort.rs`/`means.rs` (`expect_eq`, résolution `_LAST_`) afin de
// garantir l'identité octet-à-octet lors de la future migration.
//
// `#[allow(dead_code)]` car AUCUN appelant n'existe encore (M31.1 est purement
// additif) : ces fonctions seront câblées aux procs lors des incréments
// suivants (M31.2+).

/// Pilote la boucle d'options d'un statement PROC, jusqu'au `;` (consommé) ou
/// `Eof`. Pour chaque token de tête, calcule le mot-clé minuscule via
/// `peek().ident()` et délègue à `handle(ts, kw)`.
///
/// - `handle` renvoie `Ok(true)` → option reconnue ET consommée par le
///   handler → on continue. Le pilote NE consomme JAMAIS le mot-clé lui-même.
/// - `handle` renvoie `Ok(false)` (ou le token courant n'est pas un
///   identifiant) → `unknown_option_error(ts, proc_name)`.
///
/// Reproduit EXACTEMENT la boucle d'en-tête de `print.rs` (même flux, même
/// message+span d'erreur). Le handler garde la liberté d'implémenter des
/// branches spécifiques (cf. la branche « stat keyword » de PROC MEANS).
#[allow(dead_code)]
pub fn parse_proc_options<F>(ts: &mut StatementStream, proc_name: &str, mut handle: F) -> Result<()>
where
    F: FnMut(&mut StatementStream, &str) -> Result<bool>,
{
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next(); // consume `;`
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        // Le mot-clé de tête, minusculisé. Un token non-identifiant n'a pas de
        // mot-clé → erreur « Unexpected option » (comme print.rs).
        match ts.peek().ident().map(|s| s.to_ascii_lowercase()) {
            Some(kw) => {
                if !handle(ts, &kw)? {
                    return Err(unknown_option_error(ts, proc_name));
                }
            }
            None => {
                return Err(unknown_option_error(ts, proc_name));
            }
        }
    }
    Ok(())
}

/// Pilote la boucle de sous-statements d'un PROC, jusqu'à `run;`/`quit;`
/// (consommés avec leur `;`) ou `Eof`. Saute les `;` parasites en tête.
///
/// Pour chaque sous-statement, calcule le mot-clé minuscule de tête et délègue
/// à `handle(ts, kw)`. Un `Ok(true)` signifie « sous-statement reconnu et
/// consommé ». Un `Ok(false)` (sous-statement inconnu) déclenche la même
/// récupération que `print.rs` : `skip_to_semi()` puis on continue.
///
/// Reproduit EXACTEMENT la boucle de sous-statements de `print.rs` (y compris
/// la gestion de `run`/`quit` et de leur `;` terminal).
#[allow(dead_code)]
pub fn parse_proc_body<F>(ts: &mut StatementStream, mut handle: F) -> Result<()>
where
    F: FnMut(&mut StatementStream, &str) -> Result<bool>,
{
    loop {
        // Skip stray semicolons
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }

        if ts.peek().kind == TokenKind::Eof {
            break;
        }

        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next(); // consume run/quit
            // consume the `;`
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }

        // Le mot-clé de tête minusculisé ; un token non-identifiant est traité
        // comme un sous-statement inconnu (récupération `skip_to_semi`).
        let kw = ts.peek().ident().map(|s| s.to_ascii_lowercase());
        let recognized = match &kw {
            Some(kw) => handle(ts, kw)?,
            None => false,
        };
        if !recognized {
            // Unknown sub-statement: skip it (recovery, comme print.rs).
            ts.skip_to_semi();
        }
    }
    Ok(())
}

/// Consomme le token courant (le nom d'option) puis exige `=`, avec le MÊME
/// texte/span d'erreur que les procs aujourd'hui (`expected '=' after DATA`).
/// Extrait verbatim de `sort.rs`/`means.rs`.
///
/// `opt` est l'étiquette affichée dans le message (par convention déjà en
/// majuscules côté appelant, ex. « DATA », « OUT »).
#[allow(dead_code)]
pub fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    // Consomme le nom d'option (le mot-clé courant).
    ts.next();
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("expected '=' after {opt}"),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

/// `option = <dataset-ref>` : `expect_eq` puis `parse_dataset_ref()`.
/// Appelé avec le token courant positionné sur le nom d'option (`opt`).
#[allow(dead_code)]
pub fn parse_dataset_opt(ts: &mut StatementStream, opt: &str) -> Result<DatasetRef> {
    expect_eq(ts, opt)?;
    ts.parse_dataset_ref()
}

/// `out = <dataset-ref>` : raccourci de `parse_dataset_opt(ts, "OUT")`.
#[allow(dead_code)]
pub fn parse_out_opt(ts: &mut StatementStream) -> Result<DatasetRef> {
    parse_dataset_opt(ts, "OUT")
}

/// Construit l'erreur « Unexpected option '{BAD}' on PROC {NAME} statement. »
/// EXACTEMENT comme `print.rs`/`sort.rs` : `BAD` = identifiant courant en
/// majuscules (`?` si non-identifiant), `NAME` = `proc_name` (déjà en
/// majuscules par convention d'appel — `print.rs` passe le littéral « PRINT »),
/// span = `ts.peek().span`.
#[allow(dead_code)]
pub fn unknown_option_error(ts: &StatementStream, proc_name: &str) -> SasError {
    let span = ts.peek().span;
    let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
    SasError::parse(
        format!("Unexpected option '{bad}' on PROC {proc_name} statement."),
        span,
    )
}
