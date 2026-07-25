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


mod literal;
mod primary;

pub(crate) use literal::*;
pub(crate) use primary::*;

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

#[cfg(test)]
mod tests;
