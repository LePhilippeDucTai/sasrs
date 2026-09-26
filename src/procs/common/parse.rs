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

/// An unimplemented statement that can change data, statistics or control flow.
/// Returning an error prevents execution of the incomplete PROC AST.
pub fn unsupported_statement(proc_name: &str, statement: &str) -> SasError {
    SasError::runtime(format!(
        "The {} statement is not supported in PROC {}; it can affect results and cannot be ignored.",
        statement.to_ascii_uppercase(),
        proc_name.to_ascii_uppercase()
    ))
}

/// Text of the warning queued by the parser and emitted by the executor.
pub fn ignored_display_statement(proc_name: &str, statement: &str) -> String {
    format!(
        "The {} statement is ignored in PROC {}; display customization is not supported.",
        statement.to_ascii_uppercase(),
        proc_name.to_ascii_uppercase()
    )
}

/// Shared fallback for statements a PROC does not implement. Called with the
/// statement keyword still current; implemented statements always take priority.
pub fn unhandled_proc_statement(ts: &mut StatementStream, proc_name: &str) -> Result<()> {
    let token = ts.peek().clone();
    let kw = token.ident().unwrap_or("?").to_ascii_lowercase();
    match kw.as_str() {
        "format" | "label" | "attrib" => {
            // ATTRIB LENGTH/INFORMAT can change stored values. Do not classify
            // such a request as a display-only customization.
            if kw == "attrib" {
                let mut n = 1;
                while !matches!(ts.peek_nth(n).kind, TokenKind::Semi | TokenKind::Eof) {
                    if (ts.peek_nth(n).is_kw("length") || ts.peek_nth(n).is_kw("informat"))
                        && ts.peek_nth(n + 1).kind == TokenKind::Eq
                    {
                        return Err(unsupported_statement(proc_name, "ATTRIB"));
                    }
                    n += 1;
                }
            }
            ts.warn_ignored_display(ignored_display_statement(proc_name, &kw));
            ts.skip_to_semi();
            Ok(())
        }
        "by" | "weight" | "freq" | "output" | "id" | "where" | "class" | "estimate"
        | "contrast" | "lsmeans" | "reweight" | "refit" => {
            Err(unsupported_statement(proc_name, &kw))
        }
        _ => Err(SasError::parse(
            format!(
                "180-322: Statement '{}' is not valid or it is used out of proper order in PROC {}.",
                kw.to_ascii_uppercase(),
                proc_name.to_ascii_uppercase()
            ),
            token.span,
        )),
    }
}

/// Consume comments and global statements in both shared and run-group loops.
pub fn parse_proc_inert_or_global(ts: &mut StatementStream) -> Result<bool> {
    if matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Star) {
        ts.skip_to_semi();
        return Ok(true);
    }
    ts.parse_proc_global()
}

/// Drive PROC statements through RUN/QUIT/EOF or an implicit DATA/PROC boundary.
/// A handler returns true only for an implemented, consumed statement. Unknown
/// statements are errors; only explicitly display-only requests may be skipped.
pub fn parse_proc_body<F>(ts: &mut StatementStream, proc_name: &str, mut handle: F) -> Result<()>
where
    F: FnMut(&mut StatementStream, &str) -> Result<bool>,
{
    loop {
        if parse_proc_inert_or_global(ts)? {
            continue;
        }
        if ts.at_eof() || ts.peek().is_kw("data") || ts.peek().is_kw("proc") {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next();
            ts.expect_semi()?;
            break;
        }
        let kw = ts.peek().ident().map(|s| s.to_ascii_lowercase());
        let recognized = match &kw {
            Some(kw) => handle(ts, kw)?,
            None => false,
        };
        if !recognized {
            unhandled_proc_statement(ts, proc_name)?;
        }
    }
    Ok(())
}

/// Consomme le nom d'option courant PUIS le `=` qui le suit.
///
/// MQ8.5 — s'appelait `expect_eq`, sous le même nom que quatre copies locales
/// au contrat INVERSE (elles laissent le nom d'option au flux). Migrer une
/// proc de l'une à l'autre avalait donc un token de plus, silencieusement.
/// Le nom dit maintenant lequel des deux contrats on prend ; le message et le
/// span d'erreur sont inchangés (`expected '=' after DATA`).
///
/// `opt` est l'étiquette affichée dans le message (par convention déjà en
/// majuscules côté appelant, ex. « DATA », « OUT »).
pub fn consume_option_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    // Consomme le nom d'option (le mot-clé courant).
    ts.next();
    expect_eq(ts, opt)
}

/// Exige un `=` sur le token COURANT et le consomme ; le nom d'option a déjà
/// été consommé par l'appelant. C'est le contrat majoritaire (MEANS, SGPLOT,
/// IMPORT, EXPORT l'écrivaient chacun de leur côté).
///
/// Voir [`consume_option_eq`] pour la variante qui consomme aussi le nom.
pub fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("expected '=' after {opt}"),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

/// `option = <dataset-ref>` : `consume_option_eq` puis `parse_dataset_ref()`.
/// Appelé avec le token courant positionné sur le NOM D'OPTION (`opt`).
pub fn parse_dataset_opt(ts: &mut StatementStream, opt: &str) -> Result<DatasetRef> {
    consume_option_eq(ts, opt)?;
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

/// Consomme et JETTE la valeur d'une option non gérée (`= valeur` ou
/// `= (…)`), afin que le parsing du reste du statement reste synchronisé.
/// Sans effet si le token courant n'est pas un `=` (option booléenne).
///
/// MQ9.2 — les procs graphiques acceptent SILENCIEUSEMENT les options
/// qu'elles ne rendent pas (politique documentée : la colonne « non couvert »
/// du README les liste, elles ne sont pas des erreurs). Les six sites qui
/// écrivaient `let _ = parse_paren_attrs(ts)` / `let _ = read_value(ts)`
/// exprimaient donc bien une intention — mais rien ne le disait, et un
/// `let _ =` isolé est indiscernable d'une erreur avalée par mégarde. Le nom
/// de cette fonction porte l'intention. Elle utilise en prime
/// `skip_balanced_parens`, qui gère l'imbrication, là où `parse_paren_attrs`
/// s'arrêtait à la première `)` et désynchronisait sur `opt=(a=(1 2))`.
pub fn skip_option_value(ts: &mut StatementStream) {
    if ts.peek().kind != TokenKind::Eq {
        return;
    }
    ts.next();
    if ts.peek().kind == TokenKind::LParen {
        ts.skip_balanced_parens();
    } else {
        read_value(ts);
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
