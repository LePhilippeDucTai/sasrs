use super::*;

// ───────────────────── couche de parsing partagée (M31.1) ─────────────────────
//
// Combinateurs réutilisables pour le parsing des statements PROC : boucles
// d'options et de sous-statements, résolution `OUT=`/`DATA=`, message
// « Unexpected option ». Ils ont été extraits VERBATIM de `print.rs` (boucles)
// et de `sort.rs`/`means.rs` (`expect_eq`, résolution `_LAST_`) pour garantir
// l'identité octet-à-octet lors de la migration des procs.
//
// La migration n'est PAS terminée : `parse_proc_options` n'est adopté que par
// une partie des procs, les autres réimplémentent encore la même boucle.

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
pub fn parse_dataset_opt(ts: &mut StatementStream, opt: &str) -> Result<DatasetRef> {
    expect_eq(ts, opt)?;
    ts.parse_dataset_ref()
}

/// `out = <dataset-ref>` : raccourci de `parse_dataset_opt(ts, "OUT")`.
pub fn parse_out_opt(ts: &mut StatementStream) -> Result<DatasetRef> {
    parse_dataset_opt(ts, "OUT")
}

/// Construit l'erreur « Unexpected option '{BAD}' on PROC {NAME} statement. »
/// EXACTEMENT comme `print.rs`/`sort.rs` : `BAD` = identifiant courant en
/// majuscules (`?` si non-identifiant), `NAME` = `proc_name` (déjà en
/// majuscules par convention d'appel — `print.rs` passe le littéral « PRINT »),
/// span = `ts.peek().span`.
pub fn unknown_option_error(ts: &StatementStream, proc_name: &str) -> SasError {
    let span = ts.peek().span;
    let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
    SasError::parse(
        format!("Unexpected option '{bad}' on PROC {proc_name} statement."),
        span,
    )
}

/// Lit un identifiant et le consomme ; erreur de parsing sinon. `ctx` complète
/// le message (« expected an identifier {ctx} »), p.ex. `"after PLOT"` ou
/// `"after OUT="`.
///
/// MQ8.4 — unifie les 4 copies identiques de gplot / gchart / plot / sgplot et
/// celle de transpose (dont le message est reconstruit au site d'appel).
pub fn expect_ident(ts: &mut StatementStream, ctx: &str) -> Result<String> {
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

/// Lit la valeur d'une option de proc graphique : chaîne, identifiant ou
/// nombre (rendu sans partie décimale quand elle est nulle). `None` — sans
/// rien consommer — si le token courant n'est aucun des trois.
///
/// MQ8.4 — unifie les 3 copies identiques de gplot / gchart / sgplot.
pub fn read_value(ts: &mut StatementStream) -> Option<String> {
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

/// Valeur d'option `opt=<chaîne|identifiant>` : consomme le token et le rend,
/// erreur « expected a value after {opt}= » sinon. Le token courant EST la
/// valeur (le `=` a déjà été consommé).
///
/// MQ8.4 — unifie les copies identiques d'IMPORT et EXPORT.
pub fn parse_string_or_ident(ts: &mut StatementStream, opt: &str) -> Result<String> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Str { value, .. } => {
            let s = value.clone();
            ts.next();
            Ok(s)
        }
        TokenKind::Ident(s) => {
            let s = s.clone();
            ts.next();
            Ok(s)
        }
        _ => Err(SasError::parse(
            format!("expected a value after {opt}="),
            tok.span,
        )),
    }
}
