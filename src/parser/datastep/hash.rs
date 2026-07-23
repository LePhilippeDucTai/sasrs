//! Objets hash : DECLARE/DCL, hiter, appels de méthode `h.find()`.

use super::*;

/// `declare hash h(opt:val, ...);` / `dcl hash h();` (M17.1) — déclaration
/// d'un objet hash. La tête (`declare`/`dcl`) est déjà en position courante.
/// Après le mot-clé `hash` (seul type d'objet géré ; `hiter` est différé à
/// M17.2 → erreur claire), on lit le nom de l'objet, puis une liste
/// optionnelle d'options `clé:valeur` entre parenthèses (virgules séparatrices).
/// Chaque valeur est un littéral chaîne (`'yes'`) ou numérique, normalisée en
/// `String`. Parenthèses vides (`h()`) ou absentes (`h`) → aucune option.
pub(super) fn parse_declare(ts: &mut StatementStream) -> Result<DsStmt> {
    let decl_tok = ts.peek().clone();
    ts.next(); // `declare` / `dcl`
    // Type de l'objet : seul `hash` est géré en M17.1.
    let obj_type = match ts.peek().ident() {
        Some(s) => s.to_ascii_lowercase(),
        None => {
            return Err(SasError::parse(
                "expected an object type (HASH) after DECLARE",
                ts.peek().span,
            ));
        }
    };
    if obj_type == "hiter" {
        return parse_declare_hiter(ts);
    }
    if obj_type != "hash" {
        return Err(SasError::parse(
            format!(
                "DECLARE of object type {} is not yet implemented (only HASH/HITER).",
                obj_type.to_uppercase()
            ),
            ts.peek().span,
        ));
    }
    ts.next(); // `hash`
    let name = match ts.peek().ident() {
        Some(s) => s.to_string(),
        None => {
            return Err(SasError::parse(
                "expected a hash object name after DECLARE HASH",
                ts.peek().span,
            ));
        }
    };
    ts.next(); // nom de l'objet
    let mut options = Vec::new();
    if ts.peek().kind == TokenKind::LParen {
        ts.next(); // `(`
        if ts.peek().kind != TokenKind::RParen {
            loop {
                // Clé de l'option : un identifiant (`ordered`, `duplicate`,
                // `multidata`, `dataset`...).
                let key = match ts.peek().ident() {
                    Some(s) => s.to_ascii_lowercase(),
                    None => {
                        return Err(SasError::parse(
                            "expected a hash option name",
                            ts.peek().span,
                        ));
                    }
                };
                ts.next(); // clé
                if ts.peek().kind != TokenKind::Colon {
                    return Err(SasError::parse(
                        format!("expected ':' after hash option {}", key.to_uppercase()),
                        ts.peek().span,
                    ));
                }
                ts.next(); // `:`
                // Valeur : un littéral chaîne ou numérique, normalisée en
                // String.
                let val_tok = ts.peek().clone();
                let value = match &val_tok.kind {
                    TokenKind::Str { value, .. } => value.clone(),
                    TokenKind::Num(n) => crate::value::format_best(*n, 12),
                    _ => {
                        return Err(SasError::parse(
                            format!(
                                "expected a literal value for hash option {}",
                                key.to_uppercase()
                            ),
                            val_tok.span,
                        ));
                    }
                };
                ts.next(); // valeur
                options.push((key, value));
                match ts.peek().kind {
                    TokenKind::Comma => {
                        ts.next();
                    }
                    _ => break,
                }
            }
        }
        if ts.peek().kind != TokenKind::RParen {
            return Err(SasError::parse(
                format!(
                    "expected ')' to close the options of hash object {}",
                    name.to_uppercase()
                ),
                ts.peek().span,
            ));
        }
        ts.next(); // `)`
    }
    let _ = decl_tok;
    ts.expect_semi()?;
    Ok(DsStmt::DeclareHash { name, options })
}

/// `declare hiter hi('h');` (M17.2) — déclaration d'un itérateur de hash. La
/// tête `declare`/`dcl` est consommée, `hiter` est en position courante. On lit
/// le nom de l'itérateur puis, entre parenthèses, un littéral chaîne nommant
/// l'objet hash à parcourir.
fn parse_declare_hiter(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `hiter`
    let name = match ts.peek().ident() {
        Some(s) => s.to_string(),
        None => {
            return Err(SasError::parse(
                "expected an iterator name after DECLARE HITER",
                ts.peek().span,
            ));
        }
    };
    ts.next(); // nom de l'itérateur
    if ts.peek().kind != TokenKind::LParen {
        return Err(SasError::parse(
            "expected '(' with the hash object name after DECLARE HITER",
            ts.peek().span,
        ));
    }
    ts.next(); // `(`
    let hash_name = match &ts.peek().kind {
        TokenKind::Str { value, .. } => value.clone(),
        _ => {
            return Err(SasError::parse(
                "expected a quoted hash object name for DECLARE HITER",
                ts.peek().span,
            ));
        }
    };
    ts.next(); // chaîne
    if ts.peek().kind != TokenKind::RParen {
        return Err(SasError::parse(
            "expected ')' to close DECLARE HITER",
            ts.peek().span,
        ));
    }
    ts.next(); // `)`
    ts.expect_semi()?;
    Ok(DsStmt::DeclareHiter { name, hash_name })
}

/// `h.method(args);` (M17.1) — appel de méthode d'objet hash. La forme
/// `ident . ident (` est détectée par `parse_statement` AVANT le dispatch par
/// mot-clé. Les arguments sont des expressions séparées par des virgules
/// (liste vide autorisée pour `defineDone()`). La validation de la méthode est
/// faite à l'exécution.
pub(super) fn parse_hash_method(ts: &mut StatementStream) -> Result<DsStmt> {
    let object = ts
        .peek()
        .ident()
        .expect("matched an Ident head in parse_statement")
        .to_string();
    ts.next(); // objet
    ts.next(); // `.`
    let method = ts
        .peek()
        .ident()
        .expect("matched an Ident after '.' in parse_statement")
        .to_string();
    ts.next(); // méthode
    // `(` garanti par le motif de détection.
    ts.next(); // `(`
    let args = parse_hash_args(ts, &object, &method)?;
    ts.expect_semi()?;
    Ok(DsStmt::HashMethod(Box::new(crate::ast::HashMethodCall {
        object,
        method,
        args,
    })))
}

/// Parse la liste d'arguments d'un appel de méthode hash (la `(` est déjà
/// consommée ; consomme la `)`). Chaque argument est positionnel (une
/// expression : `'k'`, `1`) ou nommé (`key: expr`, `data: expr`,
/// `dataset: 'lib.tab'`). Liste vide autorisée.
pub(crate) fn parse_hash_args(
    ts: &mut StatementStream,
    object: &str,
    method: &str,
) -> Result<Vec<crate::ast::HashArg>> {
    use crate::ast::HashArg;
    let mut args = Vec::new();
    if ts.peek().kind != TokenKind::RParen {
        loop {
            // Argument nommé `name : expr` ? (ident suivi d'un `:`).
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                if ts.peek2().kind == TokenKind::Colon {
                    ts.next(); // nom
                    ts.next(); // `:`
                    let value = super::expr::parse_expr(ts)?;
                    args.push(HashArg::Named(name.to_ascii_lowercase(), value));
                } else {
                    args.push(HashArg::Positional(super::expr::parse_expr(ts)?));
                }
            } else {
                args.push(HashArg::Positional(super::expr::parse_expr(ts)?));
            }
            match ts.peek().kind {
                TokenKind::Comma => {
                    ts.next();
                }
                _ => break,
            }
        }
    }
    if ts.peek().kind != TokenKind::RParen {
        return Err(SasError::parse(
            format!(
                "expected ')' to close the arguments of {}.{}",
                object.to_uppercase(),
                method
            ),
            ts.peek().span,
        ));
    }
    ts.next(); // `)`
    Ok(args)
}
