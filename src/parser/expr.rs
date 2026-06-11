//! Parser d'expressions SAS (Pratt / precedence climbing), partagé par
//! l'étape DATA, les WHERE et (en partie) PROC SQL.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! ## Précédence SAS (du plus FORT au plus FAIBLE liage) — attention,
//! SAS est inhabituel :
//! 1. `**` (associatif à DROITE), préfixes `+` `-` `NOT` — oui, NOT lie
//!    très fort en SAS : `not x = 1` ≡ `(not x) = 1` !
//! 2. `*` `/`
//! 3. `+` `-` (binaires)
//! 4. `||` (concaténation)
//! 5. comparaisons `=` `ne` `<` `<=` `>` `>=` et `IN (v1, v2, ...)`
//!    (non associatives ; pas de chaînage a<b<c en M1)
//! 6. `AND`
//! 7. `OR`
//!
//! ## Primaires
//! - `TokenKind::Num` → `Expr::Num`
//! - `TokenKind::Str` : suffixe `None`→`Expr::Str` ; `Date`→ jours depuis
//!   1960-01-01 (parser `ddMONyyyy`, ex. `01jan2020`, via chrono ;
//!   `'...'d` invalide → erreur de parse) ; `Time`→ secondes depuis
//!   minuit (`hh:mm[:ss]`) ; `DateTime`→ secondes depuis 1960
//!   (`ddMONyyyy:hh:mm:ss`) ; `Name`→ `Expr::Str`.
//! - `Dot` → `Expr::Missing(Dot)` ; `Dot` immédiatement suivi d'un ident
//!   d'une lettre ou `_` (adjacent : vérifier les spans !) → missing
//!   spécial `.A`..`.Z` / `._`.
//! - `Ident` suivi de `(` → `Expr::Call { name, args }` (args séparés par
//!   des virgules, éventuellement vides : `today()`).
//! - `Ident` sinon → `Expr::Var`.
//! - `( expr )`.
//!
//! ## IN
//! `expr IN ( item [, item]* )` ; items = littéraux num/str (M1).
//! Produit `Expr::In { expr, list }`.
//!
//! ## Tests unitaires à écrire
//! - `2**3**2` = 512 (droite-associatif) une fois évalué.
//! - `not x = 1` parse comme `(not x) = 1`.
//! - `'01jan1960'd` → Num(0.0) ; `'02jan1960'd` → Num(1.0).
//! - `.a` adjacent → Missing(Letter(0)) ; `. a` (espace) → erreur ou
//!   Missing puis Var (le contexte appelant tranchera) — choisir Missing
//!   ordinaire si non adjacent.
//!
//! ## Note d'implémentation — précédence unaire vs `**`
//! On implémente un escalier de fonctions de precedence climbing, du plus
//! faible au plus fort liage : `or` → `and` → `compare` (+ IN, non
//! associatif) → `concat` → `add_sub` → `mul_div` → `unary` → `power` →
//! `primary`. Deux subtilités SAS encodées par la STRUCTURE de l'escalier :
//!   * `**` est plus fort que le moins unaire : `unary` parse son opérande
//!     en appelant `power`, donc `-2**2` se lit `-(2**2)` = -4.
//!   * `**` est associatif à droite : la récursion droite de `power`
//!     appelle de nouveau `unary` (donc `power`), d'où `2**3**2` =
//!     `2**(3**2)`. Le membre gauche d'un `**` ne peut pas lui-même être un
//!     préfixe (il sort de `primary`), ce qui est conforme à SAS.

use super::StatementStream;
use crate::ast::{BinaryOp, Expr, UnaryOp};
use crate::error::{Result, SasError};
use crate::token::{Span, StrSuffix, TokenKind};
use crate::value::MissingKind;
use chrono::{NaiveDate, NaiveTime};

/// Jour SAS 0 = 1960-01-01.
fn sas_epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(1960, 1, 1).expect("1960-01-01 is a valid date")
}

pub fn parse_expr(ts: &mut StatementStream) -> Result<Expr> {
    parse_or(ts)
}

/// Niveau le plus faible : `OR` (associatif à gauche).
fn parse_or(ts: &mut StatementStream) -> Result<Expr> {
    let mut left = parse_and(ts)?;
    while ts.peek().kind == TokenKind::Or {
        ts.next();
        let right = parse_and(ts)?;
        left = Expr::Binary {
            op: BinaryOp::Or,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

/// `AND` (associatif à gauche).
fn parse_and(ts: &mut StatementStream) -> Result<Expr> {
    let mut left = parse_compare(ts)?;
    while ts.peek().kind == TokenKind::And {
        ts.next();
        let right = parse_compare(ts)?;
        left = Expr::Binary {
            op: BinaryOp::And,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

/// Comparaisons et `IN`, NON associatifs : au plus un opérateur à ce niveau
/// (pas de chaînage `a < b < c` en M1).
fn parse_compare(ts: &mut StatementStream) -> Result<Expr> {
    let left = parse_concat(ts)?;
    let op = match ts.peek().kind {
        TokenKind::Eq => Some(BinaryOp::Eq),
        TokenKind::Ne => Some(BinaryOp::Ne),
        TokenKind::Lt => Some(BinaryOp::Lt),
        TokenKind::Le => Some(BinaryOp::Le),
        TokenKind::Gt => Some(BinaryOp::Gt),
        TokenKind::Ge => Some(BinaryOp::Ge),
        TokenKind::In => return parse_in(ts, left),
        _ => None,
    };
    match op {
        Some(op) => {
            ts.next();
            let right = parse_concat(ts)?;
            Ok(Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        }
        None => Ok(left),
    }
}

/// `expr IN ( item [, item]* )` — items littéraux num/str (M1). `IN` déjà
/// en tête de stream.
fn parse_in(ts: &mut StatementStream, left: Expr) -> Result<Expr> {
    ts.next(); // IN
    if ts.peek().kind != TokenKind::LParen {
        return Err(SasError::parse(
            "expected '(' after IN",
            ts.peek().span,
        ));
    }
    ts.next(); // (
    let mut list = Vec::new();
    if ts.peek().kind != TokenKind::RParen {
        loop {
            list.push(parse_in_item(ts)?);
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
            "expected ',' or ')' in IN list",
            ts.peek().span,
        ));
    }
    ts.next(); // )
    Ok(Expr::In {
        expr: Box::new(left),
        list,
    })
}

/// Un item de liste `IN` : littéral numérique ou chaîne (y compris littéraux
/// datés). Le moins unaire est toléré devant un nombre (`in (-1, 2)`).
fn parse_in_item(ts: &mut StatementStream) -> Result<Expr> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Num(n) => {
            ts.next();
            Ok(Expr::Num(*n))
        }
        TokenKind::Minus => {
            ts.next();
            let inner = ts.peek().clone();
            if let TokenKind::Num(n) = inner.kind {
                ts.next();
                Ok(Expr::Num(-n))
            } else {
                Err(SasError::parse(
                    "expected a numeric literal after '-' in IN list",
                    inner.span,
                ))
            }
        }
        TokenKind::Str { value, suffix } => {
            ts.next();
            literal_from_string(value, *suffix, tok.span)
        }
        _ => Err(SasError::parse(
            "IN list items must be numeric or character literals",
            tok.span,
        )),
    }
}

/// `||` concaténation (associatif à gauche).
fn parse_concat(ts: &mut StatementStream) -> Result<Expr> {
    let mut left = parse_add_sub(ts)?;
    while ts.peek().kind == TokenKind::Concat {
        ts.next();
        let right = parse_add_sub(ts)?;
        left = Expr::Binary {
            op: BinaryOp::Concat,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

/// `+` `-` binaires (associatifs à gauche).
fn parse_add_sub(ts: &mut StatementStream) -> Result<Expr> {
    let mut left = parse_mul_div(ts)?;
    loop {
        let op = match ts.peek().kind {
            TokenKind::Plus => BinaryOp::Add,
            TokenKind::Minus => BinaryOp::Sub,
            _ => break,
        };
        ts.next();
        let right = parse_mul_div(ts)?;
        left = Expr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

/// `*` `/` (associatifs à gauche).
fn parse_mul_div(ts: &mut StatementStream) -> Result<Expr> {
    let mut left = parse_unary(ts)?;
    loop {
        let op = match ts.peek().kind {
            TokenKind::Star => BinaryOp::Mul,
            TokenKind::Slash => BinaryOp::Div,
            _ => break,
        };
        ts.next();
        let right = parse_unary(ts)?;
        left = Expr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

/// Préfixes `+` `-` `NOT`. Leur opérande est parsé au niveau `power`, ce qui
/// rend `**` plus fort que le moins unaire (`-2**2` = `-(2**2)`).
fn parse_unary(ts: &mut StatementStream) -> Result<Expr> {
    let op = match ts.peek().kind {
        TokenKind::Plus => Some(UnaryOp::Plus),
        TokenKind::Minus => Some(UnaryOp::Minus),
        TokenKind::Not => Some(UnaryOp::Not),
        _ => None,
    };
    match op {
        Some(op) => {
            ts.next();
            let expr = parse_unary(ts)?;
            Ok(Expr::Unary {
                op,
                expr: Box::new(expr),
            })
        }
        None => parse_power(ts),
    }
}

/// `**` associatif à DROITE. Le membre gauche sort de `primary` (ne peut pas
/// être un préfixe) ; le membre droit retourne dans `unary` (donc admet un
/// préfixe et un nouveau `**`), d'où `2**3**2` = `2**(3**2)`.
fn parse_power(ts: &mut StatementStream) -> Result<Expr> {
    let base = parse_primary(ts)?;
    if ts.peek().kind == TokenKind::Power {
        ts.next();
        let exp = parse_unary(ts)?;
        Ok(Expr::Binary {
            op: BinaryOp::Power,
            left: Box::new(base),
            right: Box::new(exp),
        })
    } else {
        Ok(base)
    }
}

/// Primaires : littéraux, missings, variables/appels, parenthèses.
fn parse_primary(ts: &mut StatementStream) -> Result<Expr> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Num(n) => {
            ts.next();
            Ok(Expr::Num(*n))
        }
        TokenKind::Str { value, suffix } => {
            ts.next();
            literal_from_string(value, *suffix, tok.span)
        }
        TokenKind::Dot => parse_dot(ts),
        TokenKind::LParen => {
            ts.next();
            let inner = parse_expr(ts)?;
            if ts.peek().kind != TokenKind::RParen {
                return Err(SasError::parse(
                    "expected ')'",
                    ts.peek().span,
                ));
            }
            ts.next();
            Ok(inner)
        }
        TokenKind::Ident(name) => {
            let name = name.clone();
            ts.next();
            if ts.peek().kind == TokenKind::LParen {
                parse_call(ts, name)
            } else {
                Ok(Expr::Var(name))
            }
        }
        _ => Err(SasError::parse(
            "expected an expression",
            tok.span,
        )),
    }
}

/// `Dot` en tête : missing ordinaire, ou missing spécial `.A`..`.Z` / `._`
/// si un ident d'UNE lettre/`_` lui est immédiatement adjacent (spans
/// jointifs). Sinon, le `Dot` seul est consommé et l'ident reste à parser
/// par l'appelant.
fn parse_dot(ts: &mut StatementStream) -> Result<Expr> {
    let dot_tok = ts.peek().clone();
    let dot_end = dot_tok.span.end;
    ts.next(); // consomme le '.'
    let next = ts.peek().clone();
    if let TokenKind::Ident(s) = &next.kind {
        // Adjacent : aucun espace entre le '.' et l'ident.
        if next.span.start == dot_end && s.chars().count() == 1 {
            let c = s.chars().next().unwrap();
            if let Some(kind) = MissingKind::from_letter(c) {
                ts.next(); // consomme l'ident de la lettre
                return Ok(Expr::Missing(kind));
            }
        }
    }
    Ok(Expr::Missing(MissingKind::Dot))
}

/// Appel de fonction : `(` déjà en tête de stream, `name` déjà consommé.
fn parse_call(ts: &mut StatementStream, name: String) -> Result<Expr> {
    ts.next(); // (
    let mut args = Vec::new();
    if ts.peek().kind != TokenKind::RParen {
        loop {
            args.push(parse_expr(ts)?);
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
            format!("expected ',' or ')' in call to {}", name.to_uppercase()),
            ts.peek().span,
        ));
    }
    ts.next(); // )
    Ok(Expr::Call { name, args })
}

/// Convertit un littéral chaîne (avec son suffixe) en `Expr`.
fn literal_from_string(value: &str, suffix: StrSuffix, span: Span) -> Result<Expr> {
    match suffix {
        StrSuffix::None | StrSuffix::Name => Ok(Expr::Str(value.to_string())),
        StrSuffix::Date => Ok(Expr::Num(parse_date_literal(value, span)?)),
        StrSuffix::Time => Ok(Expr::Num(parse_time_literal(value, span)?)),
        StrSuffix::DateTime => Ok(Expr::Num(parse_datetime_literal(value, span)?)),
    }
}

/// `ddMONyyyy` (insensible à la casse) → NaiveDate.
fn parse_date_ddmonyyyy(s: &str, span: Span) -> Result<NaiveDate> {
    let bytes = s.as_bytes();
    // Au minimum d + mmm + yyyy. Le jour fait 1 ou 2 chiffres.
    // On découpe : digits | 3 lettres | digits.
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let day_str = &s[..i];
    if day_str.is_empty() || i + 3 > s.len() {
        return Err(SasError::parse(
            format!("invalid date literal '{s}'"),
            span,
        ));
    }
    let mon_str = &s[i..i + 3];
    let year_str = &s[i + 3..];
    let day: u32 = day_str.parse().map_err(|_| {
        SasError::parse(format!("invalid date literal '{s}'"), span)
    })?;
    let month = match mon_str.to_ascii_lowercase().as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => {
            return Err(SasError::parse(
                format!("invalid month in date literal '{s}'"),
                span,
            ));
        }
    };
    if year_str.is_empty() || !year_str.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SasError::parse(
            format!("invalid year in date literal '{s}'"),
            span,
        ));
    }
    let year: i32 = year_str.parse().map_err(|_| {
        SasError::parse(format!("invalid date literal '{s}'"), span)
    })?;
    NaiveDate::from_ymd_opt(year, month, day).ok_or_else(|| {
        SasError::parse(format!("invalid date literal '{s}'"), span)
    })
}

/// `'ddMONyyyy'd` → jours depuis 1960-01-01 (f64).
fn parse_date_literal(s: &str, span: Span) -> Result<f64> {
    let date = parse_date_ddmonyyyy(s.trim(), span)?;
    let days = date.signed_duration_since(sas_epoch()).num_days();
    Ok(days as f64)
}

/// `hh:mm[:ss]` → secondes depuis minuit (f64).
fn parse_time_literal(s: &str, span: Span) -> Result<f64> {
    let s = s.trim();
    let mut parts = s.split(':');
    let h = parts.next();
    let m = parts.next();
    let sec = parts.next();
    if parts.next().is_some() {
        return Err(SasError::parse(
            format!("invalid time literal '{s}'"),
            span,
        ));
    }
    let (Some(h), Some(m)) = (h, m) else {
        return Err(SasError::parse(
            format!("invalid time literal '{s}'"),
            span,
        ));
    };
    let parse_int = |p: &str| -> Result<u32> {
        p.trim()
            .parse::<u32>()
            .map_err(|_| SasError::parse(format!("invalid time literal '{s}'"), span))
    };
    let hh = parse_int(h)?;
    let mm = parse_int(m)?;
    let ss = match sec {
        Some(p) => parse_int(p)?,
        None => 0,
    };
    if mm >= 60 || ss >= 60 {
        return Err(SasError::parse(
            format!("invalid time literal '{s}'"),
            span,
        ));
    }
    // Validation des composantes via NaiveTime (heures < 24 — SAS tolère
    // davantage, mais on reste strict pour les littéraux M1).
    if NaiveTime::from_hms_opt(hh, mm, ss).is_none() {
        return Err(SasError::parse(
            format!("invalid time literal '{s}'"),
            span,
        ));
    }
    Ok((hh * 3600 + mm * 60 + ss) as f64)
}

/// `'ddMONyyyy:hh:mm:ss'dt` → secondes depuis 1960-01-01T00:00:00 (f64).
fn parse_datetime_literal(s: &str, span: Span) -> Result<f64> {
    let s = s.trim();
    let Some((date_part, time_part)) = s.split_once(':') else {
        return Err(SasError::parse(
            format!("invalid datetime literal '{s}'"),
            span,
        ));
    };
    let date = parse_date_ddmonyyyy(date_part, span)?;
    let secs_in_day = parse_time_literal(time_part, span)?;
    let days = date.signed_duration_since(sas_epoch()).num_days();
    Ok(days as f64 * 86400.0 + secs_in_day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceFile;

    /// Parse une expression complète et vérifie qu'elle consomme tout
    /// jusqu'à EOF.
    fn parse(src: &str) -> Result<Expr> {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file)?;
        let expr = parse_expr(&mut ts)?;
        Ok(expr)
    }

    fn ok(src: &str) -> Expr {
        parse(src).unwrap_or_else(|e| panic!("parse of {src:?} failed: {e}"))
    }

    // Helpers de construction pour des assertions de STRUCTURE lisibles.
    fn bin(op: BinaryOp, l: Expr, r: Expr) -> Expr {
        Expr::Binary {
            op,
            left: Box::new(l),
            right: Box::new(r),
        }
    }
    fn un(op: UnaryOp, e: Expr) -> Expr {
        Expr::Unary {
            op,
            expr: Box::new(e),
        }
    }
    fn num(n: f64) -> Expr {
        Expr::Num(n)
    }
    fn var(s: &str) -> Expr {
        Expr::Var(s.to_string())
    }

    #[test]
    fn power_is_right_associative_structure() {
        // 2**3**2 = 2**(3**2), pas (2**3)**2.
        let e = ok("2 ** 3 ** 2");
        assert_eq!(
            e,
            bin(
                BinaryOp::Power,
                num(2.0),
                bin(BinaryOp::Power, num(3.0), num(2.0))
            )
        );
    }

    #[test]
    fn power_binds_tighter_than_unary_minus() {
        // -2**2 = -(2**2).
        let e = ok("-2 ** 2");
        assert_eq!(
            e,
            un(UnaryOp::Minus, bin(BinaryOp::Power, num(2.0), num(2.0)))
        );
    }

    #[test]
    fn not_binds_tighter_than_comparison() {
        // not x = 1  ≡  (not x) = 1.
        let e = ok("not x = 1");
        assert_eq!(
            e,
            bin(
                BinaryOp::Eq,
                un(UnaryOp::Not, var("x")),
                num(1.0)
            )
        );
    }

    #[test]
    fn arithmetic_precedence() {
        // 1 + 2 * 3  ≡  1 + (2*3).
        let e = ok("1 + 2 * 3");
        assert_eq!(
            e,
            bin(BinaryOp::Add, num(1.0), bin(BinaryOp::Mul, num(2.0), num(3.0)))
        );
        // 2 * 3 + 1  ≡  (2*3) + 1.
        let e = ok("2 * 3 + 1");
        assert_eq!(
            e,
            bin(BinaryOp::Add, bin(BinaryOp::Mul, num(2.0), num(3.0)), num(1.0))
        );
    }

    #[test]
    fn add_sub_left_associative() {
        // 10 - 3 - 2 ≡ (10-3)-2.
        let e = ok("10 - 3 - 2");
        assert_eq!(
            e,
            bin(BinaryOp::Sub, bin(BinaryOp::Sub, num(10.0), num(3.0)), num(2.0))
        );
    }

    #[test]
    fn comparison_vs_arithmetic() {
        // a + b = c  ≡  (a+b) = c (arithmetic binds tighter than compare).
        let e = ok("a + b = c");
        assert_eq!(
            e,
            bin(
                BinaryOp::Eq,
                bin(BinaryOp::Add, var("a"), var("b")),
                var("c")
            )
        );
    }

    #[test]
    fn and_or_precedence() {
        // a or b and c  ≡  a or (b and c).
        let e = ok("a or b and c");
        assert_eq!(
            e,
            bin(
                BinaryOp::Or,
                var("a"),
                bin(BinaryOp::And, var("b"), var("c"))
            )
        );
    }

    #[test]
    fn concat_precedence() {
        // a || b = c  ≡  (a||b) = c (concat binds tighter than compare).
        let e = ok("a || b = c");
        assert_eq!(
            e,
            bin(
                BinaryOp::Eq,
                bin(BinaryOp::Concat, var("a"), var("b")),
                var("c")
            )
        );
        // 1 + 2 || 3  ≡  (1+2) || 3 (add binds tighter than concat).
        let e = ok("1 + 2 || 3");
        assert_eq!(
            e,
            bin(
                BinaryOp::Concat,
                bin(BinaryOp::Add, num(1.0), num(2.0)),
                num(3.0)
            )
        );
    }

    #[test]
    fn parentheses_override_precedence() {
        // (1 + 2) * 3.
        let e = ok("(1 + 2) * 3");
        assert_eq!(
            e,
            bin(BinaryOp::Mul, bin(BinaryOp::Add, num(1.0), num(2.0)), num(3.0))
        );
    }

    #[test]
    fn date_literal_epoch() {
        assert_eq!(ok("'01jan1960'd"), num(0.0));
        assert_eq!(ok("'02jan1960'd"), num(1.0));
        assert_eq!(ok("'01JAN2020'd"), num(21915.0));
    }

    #[test]
    fn time_literal_seconds_from_midnight() {
        assert_eq!(ok("'12:30't"), num(12.0 * 3600.0 + 30.0 * 60.0));
        assert_eq!(
            ok("'12:30:45't"),
            num(12.0 * 3600.0 + 30.0 * 60.0 + 45.0)
        );
    }

    #[test]
    fn datetime_literal_seconds_from_1960() {
        // 1960-01-02 12:00:00 = 1 day + 12h.
        assert_eq!(
            ok("'02jan1960:12:00:00'dt"),
            num(86400.0 + 12.0 * 3600.0)
        );
        // Epoch itself.
        assert_eq!(ok("'01jan1960:00:00:00'dt"), num(0.0));
    }

    #[test]
    fn invalid_date_literal_errors() {
        assert!(parse("'32jan2020'd").is_err());
        assert!(parse("'01xxx2020'd").is_err());
        assert!(parse("'notadate'd").is_err());
        assert!(parse("'29feb2021'd").is_err()); // 2021 non bissextile
    }

    #[test]
    fn invalid_time_literal_errors() {
        assert!(parse("'12:99't").is_err());
        assert!(parse("'noon't").is_err());
    }

    #[test]
    fn string_literal_none_and_name() {
        assert_eq!(ok("'hello'"), Expr::Str("hello".to_string()));
        assert_eq!(ok("'my var'n"), Expr::Str("my var".to_string()));
    }

    #[test]
    fn dot_alone_is_ordinary_missing() {
        assert_eq!(ok("."), Expr::Missing(MissingKind::Dot));
    }

    #[test]
    fn dot_adjacent_letter_is_special_missing() {
        // `.a` (jointif) → Missing(Letter(0)).
        assert_eq!(ok(".a"), Expr::Missing(MissingKind::Letter(0)));
        assert_eq!(ok(".Z"), Expr::Missing(MissingKind::Letter(25)));
        assert_eq!(ok("._"), Expr::Missing(MissingKind::Underscore));
    }

    #[test]
    fn dot_non_adjacent_is_ordinary_missing() {
        // `. a` (espace) → Missing(Dot) ; l'ident `a` reste pour l'appelant.
        let file = SourceFile::new(". a");
        let mut ts = StatementStream::new(&file).unwrap();
        let e = parse_expr(&mut ts).unwrap();
        assert_eq!(e, Expr::Missing(MissingKind::Dot));
        // L'ident `a` n'a pas été consommé.
        assert!(ts.peek().is_kw("a"));
    }

    #[test]
    fn dot_followed_by_multiletter_ident_is_ordinary_missing() {
        // `.ab` n'est pas un missing spécial (ident de >1 lettre).
        let file = SourceFile::new(".ab");
        let mut ts = StatementStream::new(&file).unwrap();
        let e = parse_expr(&mut ts).unwrap();
        assert_eq!(e, Expr::Missing(MissingKind::Dot));
        assert!(ts.peek().is_kw("ab"));
    }

    #[test]
    fn variable_reference() {
        assert_eq!(ok("age"), var("age"));
    }

    #[test]
    fn function_call_zero_args() {
        assert_eq!(
            ok("today()"),
            Expr::Call {
                name: "today".to_string(),
                args: vec![]
            }
        );
    }

    #[test]
    fn function_call_one_arg() {
        assert_eq!(
            ok("abs(x)"),
            Expr::Call {
                name: "abs".to_string(),
                args: vec![var("x")]
            }
        );
    }

    #[test]
    fn function_call_two_args() {
        assert_eq!(
            ok("sum(a, b)"),
            Expr::Call {
                name: "sum".to_string(),
                args: vec![var("a"), var("b")]
            }
        );
        // Arguments composés.
        assert_eq!(
            ok("max(a + 1, b * 2)"),
            Expr::Call {
                name: "max".to_string(),
                args: vec![
                    bin(BinaryOp::Add, var("a"), num(1.0)),
                    bin(BinaryOp::Mul, var("b"), num(2.0)),
                ]
            }
        );
    }

    #[test]
    fn in_operator_num_and_str() {
        let e = ok("x in (1, 2, 3)");
        assert_eq!(
            e,
            Expr::In {
                expr: Box::new(var("x")),
                list: vec![num(1.0), num(2.0), num(3.0)],
            }
        );
        let e = ok("sex in ('M', 'F')");
        assert_eq!(
            e,
            Expr::In {
                expr: Box::new(var("sex")),
                list: vec![
                    Expr::Str("M".to_string()),
                    Expr::Str("F".to_string())
                ],
            }
        );
    }

    #[test]
    fn in_with_negative_and_single_item() {
        let e = ok("x in (-1)");
        assert_eq!(
            e,
            Expr::In {
                expr: Box::new(var("x")),
                list: vec![num(-1.0)],
            }
        );
    }

    #[test]
    fn in_binds_at_comparison_level() {
        // a and x in (1, 2)  ≡  a and (x in (1,2)).
        let e = ok("a and x in (1, 2)");
        assert_eq!(
            e,
            bin(
                BinaryOp::And,
                var("a"),
                Expr::In {
                    expr: Box::new(var("x")),
                    list: vec![num(1.0), num(2.0)],
                }
            )
        );
    }

    #[test]
    fn unmatched_paren_errors() {
        assert!(parse("(1 + 2").is_err());
    }

    #[test]
    fn empty_input_errors() {
        assert!(parse("").is_err());
    }
}
