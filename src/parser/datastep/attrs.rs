//! Statements d'attributs : RETAIN, LENGTH, FORMAT, LABEL, ATTRIB.

use super::*;

/// `retain [v [init]]... ;` — la liste peut être vide (`retain;` = toutes
/// les variables du PDV). Chaque nom peut être suivi d'une valeur initiale
/// LITTÉRALE : nombre (avec `-` unaire, replié en `Expr::Num` négatif),
/// chaîne, ou missing (`.` / `.a`.. / `._`, adjacence vérifiée par spans
/// comme dans le parser d'expressions).
pub(super) fn parse_retain(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `retain`
    let mut items: Vec<(String, Option<Expr>)> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                ts.next();
                return Ok(DsStmt::Retain(items));
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                validate_sas_name(&name, tok.span)?;
                ts.next();
                let init = parse_retain_init(ts)?;
                items.push((name, init));
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name in the RETAIN statement",
                    tok.span,
                ));
            }
        }
    }
}

/// Valeur initiale optionnelle d'un élément de RETAIN : un littéral, ou
/// rien (le token suivant est alors un autre nom ou le `;`).
fn parse_retain_init(ts: &mut StatementStream) -> Result<Option<Expr>> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Num(n) => {
            let n = *n;
            let num_end = tok.span.end;
            ts.next();
            // Forme `21710d` / `21710dt` / `43200t` : un littéral numérique
            // immédiatement suivi (spans jointifs) d'un suffixe d/t/dt. La
            // VALEUR est déjà le nombre SAS (date/datetime/time) ; le suffixe
            // est un marqueur de type sans effet sur la constante. On le
            // consomme s'il est présent et adjacent.
            if let TokenKind::Ident(s) = &ts.peek().kind {
                let lower = s.to_ascii_lowercase();
                if ts.peek().span.start == num_end && matches!(lower.as_str(), "d" | "t" | "dt") {
                    ts.next(); // suffixe
                }
            }
            Ok(Some(Expr::Num(n)))
        }
        TokenKind::Minus => {
            // `-5` : moins unaire sur littéral numérique, replié.
            ts.next(); // `-`
            let num_tok = ts.peek().clone();
            let TokenKind::Num(n) = num_tok.kind else {
                return Err(SasError::parse(
                    "expected a numeric literal after '-' in the RETAIN statement",
                    num_tok.span,
                ));
            };
            ts.next();
            Ok(Some(Expr::Num(-n)))
        }
        TokenKind::Str { value, suffix } => {
            // Littéral simple (chaîne) OU littéral date/heure/datetime
            // (`'01JAN2020'd`, `'14:30:00't`, `'01JAN2020 14:30:00'dt`),
            // converti en sa valeur SAS numérique (M16.3).
            let value = value.clone();
            let suffix = *suffix;
            let span = tok.span;
            ts.next();
            Ok(Some(super::expr::literal_from_string(
                &value, suffix, span,
            )?))
        }
        TokenKind::Dot => {
            // `.` seul, ou missing spécial `.a`.. / `._` si l'ident d'UNE
            // lettre/`_` est ADJACENT (spans jointifs, comme expr.rs).
            let dot_end = tok.span.end;
            ts.next(); // `.`
            if let TokenKind::Ident(s) = &ts.peek().kind {
                if ts.peek().span.start == dot_end && s.chars().count() == 1 {
                    if let Some(kind) = MissingKind::from_letter(s.chars().next().unwrap()) {
                        ts.next();
                        return Ok(Some(Expr::Missing(kind)));
                    }
                }
            }
            Ok(Some(Expr::Missing(MissingKind::Dot)))
        }
        // Pas de littéral : l'élément n'a pas de valeur initiale.
        _ => Ok(None),
    }
}

/// `length v1 v2 $ 20 v3 5;` — suites répétables de « noms... [$] n » ; le
/// `$` s'applique au groupe de noms qui précède le nombre. La validation
/// des PLAGES de longueur (char 1..=32767, num 3..=8) est faite à la
/// compilation ; ici on exige seulement un entier positif.
pub(super) fn parse_length(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `length`
    let mut items: Vec<(String, LengthSpec)> = Vec::new();
    let mut group: Vec<String> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Ident(name) => {
                let name = name.clone();
                validate_sas_name(&name, tok.span)?;
                ts.next();
                group.push(name);
            }
            TokenKind::Dollar | TokenKind::Num(_) => {
                let is_char = tok.kind == TokenKind::Dollar;
                if is_char {
                    ts.next(); // `$`
                }
                let num_tok = ts.peek().clone();
                let TokenKind::Num(n) = num_tok.kind else {
                    return Err(SasError::parse(
                        "expected a length after '$' in the LENGTH statement",
                        num_tok.span,
                    ));
                };
                if group.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name before the length in the LENGTH statement",
                        tok.span,
                    ));
                }
                if n.fract() != 0.0 || n < 1.0 {
                    return Err(SasError::parse(
                        "the length in a LENGTH statement must be a positive integer",
                        num_tok.span,
                    ));
                }
                ts.next(); // le nombre
                let spec = LengthSpec {
                    char: is_char,
                    len: n as usize,
                };
                for name in group.drain(..) {
                    items.push((name, spec));
                }
            }
            TokenKind::Semi => {
                // Noms restés sans longueur → erreur AVANT de consommer le
                // `;` (la resynchronisation de l'appelant le consommera).
                if !group.is_empty() {
                    return Err(SasError::parse(
                        "expected a length in the LENGTH statement",
                        tok.span,
                    ));
                }
                if items.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the LENGTH statement",
                        tok.span,
                    ));
                }
                ts.next();
                return Ok(DsStmt::Length(items));
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name or a length in the LENGTH statement",
                    tok.span,
                ));
            }
        }
    }
}

/// `format weight height 8.2 name $char10.;` (M4) — suites répétables de
/// « noms... token-de-format ». Chaque groupe associe une liste d'un-ou-
/// plusieurs noms de variables au token de format qui le suit (lu via
/// `read_format_token`, robuste au découpage du lexer). L'application aux
/// variables (et la validation du token) est faite à la compilation.
pub(super) fn parse_format(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `format`
    let mut groups: Vec<(Vec<String>, String)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                if !names.is_empty() {
                    return Err(SasError::parse(
                        "expected a format after the variable name(s) in the FORMAT statement",
                        tok.span,
                    ));
                }
                if groups.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the FORMAT statement",
                        tok.span,
                    ));
                }
                ts.next();
                return Ok(DsStmt::Format(groups));
            }
            // Un Ident est SOIT un nom de variable, SOIT le début d'un token
            // de format (ex. `date9.` se lexe `date9` + `.`). On tranche par
            // l'adjacence : si le token suivant touche cet Ident et est un
            // morceau de format (`$`/nombre/`.`), l'Ident ouvre le format.
            TokenKind::Ident(name) if !ident_begins_format(ts) => {
                let name = name.clone();
                validate_sas_name(&name, tok.span)?;
                ts.next();
                names.push(name);
            }
            // Token de format : `$`/nombre/`.` en tête, ou un Ident adjacent
            // à un tel morceau. Clôt le groupe de noms courant.
            _ => {
                if names.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the FORMAT statement",
                        tok.span,
                    ));
                }
                let token = super::expr::read_format_token(ts)?;
                groups.push((std::mem::take(&mut names), token));
            }
        }
    }
}

/// Vrai si le token courant (un Ident) ouvre un token de format plutôt qu'un
/// nom de variable : le token SUIVANT est ADJACENT (aucun espace) et est un
/// morceau de format (`$`, un nombre ou `.`). Ainsi `date9.` (Ident `date9`
/// collé à `.`) est un format, alors que `weight 8.2` (espace) garde
/// `weight` comme nom.
pub(super) fn ident_begins_format(ts: &StatementStream) -> bool {
    let cur = ts.peek();
    let next = ts.peek2();
    next.span.start == cur.span.end
        && matches!(
            next.kind,
            TokenKind::Dollar | TokenKind::Num(_) | TokenKind::Dot
        )
}

/// `label weight='Body Weight' name='Pupil';` (M4) — paires
/// `ident = 'libellé'` jusqu'au `;`. La valeur est un littéral chaîne.
pub(super) fn parse_label(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `label`
    let mut pairs: Vec<(String, String)> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                if pairs.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the LABEL statement",
                        tok.span,
                    ));
                }
                ts.next();
                return Ok(DsStmt::Label(pairs));
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                validate_sas_name(&name, tok.span)?;
                ts.next();
                if ts.peek().kind != TokenKind::Eq {
                    return Err(SasError::parse(
                        "expected '=' after the variable name in the LABEL statement",
                        ts.peek().span,
                    ));
                }
                ts.next(); // `=`
                let label = expect_string_literal(ts, "LABEL")?;
                pairs.push((name, label));
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name in the LABEL statement",
                    tok.span,
                ));
            }
        }
    }
}

/// `attrib weight format=8.2 label='Body Weight';` (M4) — un item par
/// groupe de variables (un ou plusieurs noms) suivi d'options
/// `format=<token>`, `label='...'`, `length=[$]n`. `length=` est parsé mais
/// NON appliqué en M4 (simplification documentée). Un nouveau nom de
/// variable (sans `=`) après des options clôt l'item courant.
pub(super) fn parse_attrib(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `attrib`
    let mut items: Vec<AttribItem> = Vec::new();
    let mut vars: Vec<String> = Vec::new();
    let mut format: Option<String> = None;
    let mut label: Option<String> = None;
    let mut length: Option<LengthSpec> = None;
    let flush = |vars: &mut Vec<String>,
                 format: &mut Option<String>,
                 label: &mut Option<String>,
                 length: &mut Option<LengthSpec>,
                 items: &mut Vec<AttribItem>| {
        if !vars.is_empty() {
            items.push(AttribItem {
                vars: std::mem::take(vars),
                format: format.take(),
                label: label.take(),
                length: length.take(),
            });
        }
    };
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                flush(&mut vars, &mut format, &mut label, &mut length, &mut items);
                if items.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the ATTRIB statement",
                        tok.span,
                    ));
                }
                ts.next();
                return Ok(DsStmt::Attrib(items));
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                let lower = name.to_ascii_lowercase();
                // Une option `format=/label=/length=` : le mot-clé est suivi
                // d'un `=`. On consomme l'ident puis on inspecte le `=`.
                if matches!(lower.as_str(), "format" | "label" | "length") {
                    // Sauvegarde du span pour les messages d'erreur.
                    let kw_span = tok.span;
                    ts.next(); // mot-clé d'option
                    if ts.peek().kind != TokenKind::Eq {
                        return Err(SasError::parse(
                            format!(
                                "expected '=' after {} in the ATTRIB statement",
                                lower.to_uppercase()
                            ),
                            ts.peek().span,
                        ));
                    }
                    ts.next(); // `=`
                    if vars.is_empty() {
                        return Err(SasError::parse(
                            "expected a variable name before the attributes in the ATTRIB statement",
                            kw_span,
                        ));
                    }
                    match lower.as_str() {
                        "format" => format = Some(super::expr::read_format_token(ts)?),
                        "label" => label = Some(expect_string_literal(ts, "ATTRIB")?),
                        "length" => length = Some(parse_attrib_length(ts)?),
                        _ => unreachable!(),
                    }
                } else {
                    // Un nom de variable : s'il commence un nouvel item (des
                    // attributs ont déjà été lus), on flush l'item précédent.
                    validate_sas_name(&name, tok.span)?;
                    if format.is_some() || label.is_some() || length.is_some() {
                        flush(&mut vars, &mut format, &mut label, &mut length, &mut items);
                    }
                    ts.next();
                    vars.push(name);
                }
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name or an attribute in the ATTRIB statement",
                    tok.span,
                ));
            }
        }
    }
}

/// `length=[$]n` pour ATTRIB : `$` optionnel (caractère), puis un entier
/// positif.
fn parse_attrib_length(ts: &mut StatementStream) -> Result<LengthSpec> {
    let is_char = ts.peek().kind == TokenKind::Dollar;
    if is_char {
        ts.next(); // `$`
    }
    let num_tok = ts.peek().clone();
    let TokenKind::Num(n) = num_tok.kind else {
        return Err(SasError::parse(
            "expected a length after LENGTH= in the ATTRIB statement",
            num_tok.span,
        ));
    };
    if n.fract() != 0.0 || n < 1.0 {
        return Err(SasError::parse(
            "the length in an ATTRIB statement must be a positive integer",
            num_tok.span,
        ));
    }
    ts.next();
    Ok(LengthSpec {
        char: is_char,
        len: n as usize,
    })
}

/// Lit un littéral chaîne simple (`'...'` / `"..."`) et renvoie sa valeur.
/// Les suffixes datés ne sont pas acceptés comme libellés.
fn expect_string_literal(ts: &mut StatementStream, stmt: &str) -> Result<String> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Str {
            value,
            suffix: StrSuffix::None | StrSuffix::Name,
        } => {
            let s = value.clone();
            ts.next();
            Ok(s)
        }
        _ => Err(SasError::parse(
            format!("expected a quoted string in the {stmt} statement"),
            tok.span,
        )),
    }
}
