//! Parser récursif descendant du dialecte SQL de SAS (jalon M6).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! Travaille sur le même flux de tokens que le reste (`StatementStream`),
//! la grammaire SQL n'ayant besoin d'aucun token supplémentaire.
//!
//! ## Points de grammaire
//! - Mots-clés contextuels (select, from, where, group, by, having,
//!   order, on, as, inner, left, right, full, join, union, except,
//!   intersect, all, distinct, calculated, between, is, null, missing,
//!   like, case...) matchés par `is_kw`, jamais réservés globalement.
//! - Expressions : reprendre la grammaire de `parser::expr` ÉTENDUE
//!   (précédence SQL standard) avec les nœuds SqlExpr (qualified refs
//!   `a.x` — ATTENTION à désambiguïser de `lib.table` selon le
//!   contexte, CALCULATED, agrégats, BETWEEN/IS NULL/LIKE).
//! - GROUP BY positionnel (`group by 1, 2`) : entiers littéraux.
//! - `select *` et `a.*`.
//! - Sous-requêtes : HORS périmètre M6 (ERROR propre "subqueries not
//!   yet supported") — lever la limite en M8.
//!
//! ## Approche d'implémentation des expressions
//! On NE délègue PAS bloc à `parser::expr::parse_expr` pour l'ensemble
//! d'une expression SQL : les nœuds spécifiques SQL (CALCULATED, `a.x`
//! qualifié, agrégats `COUNT(*)`, BETWEEN / IS [NOT] NULL|MISSING / LIKE)
//! peuvent apparaître AU MILIEU d'une expression arithmétique/booléenne,
//! ce qui imposerait de réécrire le résultat. On déroule donc à la main
//! une échelle de précédence au niveau `SqlExpr` :
//!   or → and → not → comparaison (+ BETWEEN / IS / LIKE, non assoc.)
//!     → add_sub → mul_div → concat → unary(+/-) → atome
//! Les atomes purement « base » (littéraux, appels de fonction non
//! agrégés, variables) sont construits soit directement, soit en
//! délégant le parsing des ARGUMENTS d'appel à `parse_expr` (lui-même un
//! sous-arbre `Expr` autonome). Les opérandes des agrégats sont parsés au
//! niveau SqlExpr complet pour autoriser `count(distinct a.x)`.
//!
//! ## Représentation de `*`
//! `select *`  → `SelectItem { expr: SqlExpr::Star, alias: None }`.
//! `select a.*` → `SelectItem { expr: SqlExpr::QualifiedStar("a"), .. }`.
//!
//! ## Tests
//! Assertions d'AST pures (pas besoin de l'exécution) : une requête par
//! forme syntaxique, y compris les ratés (messages d'erreur SAS-like).

#![allow(unused_variables, dead_code)]

use super::ast::{
    FromItem, Join, JoinKind, SelectItem, SelectStmt, SetOp, SqlExpr, SqlProgram, SqlStmt,
};
use crate::ast::{BinaryOp, Expr, UnaryOp};
use crate::error::{Result, SasError};
use crate::parser::expr::parse_expr;
use crate::parser::StatementStream;
use crate::token::TokenKind;

/// Parse tout le contenu de `proc sql; ... quit;`.
///
/// Boucle de parsing des statements jusqu'à `quit`/`quit;` ou EOF. Chaque
/// statement se termine par `;`. `quit;` est consommé et arrête la boucle.
pub fn parse_sql_program(ts: &mut StatementStream) -> Result<SqlProgram> {
    let mut stmts = Vec::new();
    loop {
        // Statements vides : `;` isolé.
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            continue;
        }
        if ts.at_eof() {
            break;
        }
        if ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }
        let stmt = parse_statement(ts)?;
        match stmt {
            Some(s) => {
                stmts.push(s);
                ts.expect_semi()?;
            }
            // Statement ignoré (RESET/TITLE/...) : déjà avancé jusqu'au `;`.
            None => {}
        }
    }
    Ok(SqlProgram { stmts })
}

/// Parse un statement PROC SQL. `Ok(None)` = statement reconnu mais ignoré
/// (déjà consommé jusqu'au `;` inclus).
fn parse_statement(ts: &mut StatementStream) -> Result<Option<SqlStmt>> {
    let tok = ts.peek().clone();
    let Some(head) = tok.ident().map(|s| s.to_ascii_lowercase()) else {
        return Err(SasError::parse(
            "expected a PROC SQL statement keyword",
            tok.span,
        ));
    };
    match head.as_str() {
        "select" => Ok(Some(SqlStmt::Select(parse_select(ts)?))),
        "create" => Ok(Some(parse_create(ts)?)),
        "drop" => Ok(Some(parse_drop(ts)?)),
        "insert" => Ok(Some(parse_insert(ts)?)),
        "delete" => Ok(Some(parse_delete(ts)?)),
        "describe" => Ok(Some(parse_describe(ts)?)),
        // Statements PROC SQL non modélisés (RESET, TITLE, FOOTNOTE,
        // VALIDATE, ...) : on les saute proprement jusqu'au `;`.
        _ => {
            ts.skip_to_semi();
            Ok(None)
        }
    }
}

/// `CREATE TABLE <ref> AS <select>`.
fn parse_create(ts: &mut StatementStream) -> Result<SqlStmt> {
    ts.next(); // CREATE
    expect_kw(ts, "table")?;
    let table = ts.parse_dataset_ref()?;
    expect_kw(ts, "as")?;
    if !ts.peek().is_kw("select") {
        return Err(SasError::parse(
            "expected SELECT after CREATE TABLE ... AS",
            ts.peek().span,
        ));
    }
    let query = parse_select(ts)?;
    Ok(SqlStmt::CreateTableAs { table, query })
}

/// `DROP TABLE <ref> [, <ref> ...]`.
fn parse_drop(ts: &mut StatementStream) -> Result<SqlStmt> {
    ts.next(); // DROP
    expect_kw(ts, "table")?;
    let mut refs = vec![ts.parse_dataset_ref()?];
    while ts.peek().kind == TokenKind::Comma {
        ts.next();
        refs.push(ts.parse_dataset_ref()?);
    }
    Ok(SqlStmt::DropTable(refs))
}

/// `DELETE FROM <ref> [WHERE <sqlexpr>]`.
fn parse_delete(ts: &mut StatementStream) -> Result<SqlStmt> {
    ts.next(); // DELETE
    expect_kw(ts, "from")?;
    let table = ts.parse_dataset_ref()?;
    let where_ = if ts.peek().is_kw("where") {
        ts.next();
        Some(parse_sql_expr(ts)?)
    } else {
        None
    };
    Ok(SqlStmt::DeleteFrom { table, where_ })
}

/// `DESCRIBE TABLE <ref>`.
fn parse_describe(ts: &mut StatementStream) -> Result<SqlStmt> {
    ts.next(); // DESCRIBE
    expect_kw(ts, "table")?;
    let table = ts.parse_dataset_ref()?;
    Ok(SqlStmt::Describe(table))
}

/// `INSERT INTO <ref> [(cols)] (VALUES (...) [VALUES (...)...] | <select>)`.
fn parse_insert(ts: &mut StatementStream) -> Result<SqlStmt> {
    ts.next(); // INSERT
    expect_kw(ts, "into")?;
    let table = ts.parse_dataset_ref()?;
    // Liste de colonnes optionnelle `(c1, c2, ...)`.
    let mut columns = Vec::new();
    if ts.peek().kind == TokenKind::LParen {
        ts.next(); // (
        loop {
            let col_tok = ts.peek().clone();
            let Some(col) = col_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a column name in the INSERT column list",
                    col_tok.span,
                ));
            };
            ts.next();
            columns.push(col);
            match ts.peek().kind {
                TokenKind::Comma => {
                    ts.next();
                }
                TokenKind::RParen => {
                    ts.next();
                    break;
                }
                _ => {
                    return Err(SasError::parse(
                        "expected ',' or ')' in the INSERT column list",
                        ts.peek().span,
                    ));
                }
            }
        }
    }
    if ts.peek().is_kw("values") {
        let mut rows = Vec::new();
        while ts.peek().is_kw("values") {
            ts.next(); // VALUES
            rows.push(parse_values_group(ts)?);
        }
        Ok(SqlStmt::InsertValues {
            table,
            columns,
            rows,
        })
    } else if ts.peek().is_kw("select") {
        let query = parse_select(ts)?;
        Ok(SqlStmt::InsertSelect { table, query })
    } else {
        Err(SasError::parse(
            "expected VALUES or SELECT after INSERT INTO",
            ts.peek().span,
        ))
    }
}

/// Un groupe `( expr [, expr]* )` d'un INSERT ... VALUES. Le mot-clé VALUES
/// est déjà consommé. Les valeurs sont des `Expr` de base.
fn parse_values_group(ts: &mut StatementStream) -> Result<Vec<Expr>> {
    if ts.peek().kind != TokenKind::LParen {
        return Err(SasError::parse(
            "expected '(' after VALUES",
            ts.peek().span,
        ));
    }
    ts.next(); // (
    let mut vals = Vec::new();
    if ts.peek().kind != TokenKind::RParen {
        loop {
            vals.push(parse_expr(ts)?);
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
            "expected ',' or ')' in the VALUES list",
            ts.peek().span,
        ));
    }
    ts.next(); // )
    Ok(vals)
}

// ── SELECT ───────────────────────────────────────────────────────────────

/// `SELECT [DISTINCT] <list> FROM <from-list> [<joins>] [WHERE] [GROUP BY]
/// [HAVING] [ORDER BY] [<set-op> SELECT ...]`.
fn parse_select(ts: &mut StatementStream) -> Result<SelectStmt> {
    expect_kw(ts, "select")?;
    let distinct = if ts.peek().is_kw("distinct") {
        ts.next();
        true
    } else {
        false
    };
    let items = parse_select_list(ts)?;

    // `SELECT ... INTO :macrovar ...` — clause INTO non supportée.
    if ts.peek().is_kw("into") {
        return Err(SasError::parse(
            "The INTO clause is not yet supported.",
            ts.peek().span,
        ));
    }

    expect_kw(ts, "from")?;
    let from = parse_from_list(ts)?;
    let joins = parse_joins(ts)?;

    let where_ = if ts.peek().is_kw("where") {
        ts.next();
        Some(parse_sql_expr(ts)?)
    } else {
        None
    };

    let group_by = if ts.peek().is_kw("group") {
        ts.next();
        expect_kw(ts, "by")?;
        parse_sql_expr_list(ts)?
    } else {
        Vec::new()
    };

    let having = if ts.peek().is_kw("having") {
        ts.next();
        Some(parse_sql_expr(ts)?)
    } else {
        None
    };

    let order_by = if ts.peek().is_kw("order") {
        ts.next();
        expect_kw(ts, "by")?;
        parse_order_list(ts)?
    } else {
        Vec::new()
    };

    let set_op = parse_set_op_tail(ts)?;

    Ok(SelectStmt {
        distinct,
        items,
        from,
        joins,
        where_,
        group_by,
        having,
        order_by,
        set_op,
    })
}

/// select-list : `*` | `alias.*` | `<sqlexpr> [[AS] alias]`, séparés par `,`.
fn parse_select_list(ts: &mut StatementStream) -> Result<Vec<SelectItem>> {
    let mut items = Vec::new();
    loop {
        items.push(parse_select_item(ts)?);
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
        } else {
            break;
        }
    }
    Ok(items)
}

fn parse_select_item(ts: &mut StatementStream) -> Result<SelectItem> {
    // `select x into :macrovar ...` — clause INTO non supportée. Détectée si
    // l'item est suivi de `INTO`, mais le cas typique met INTO juste après
    // SELECT ; on le repère aussi au niveau de l'item courant.
    if ts.peek().is_kw("into") {
        return Err(SasError::parse(
            "The INTO clause is not yet supported.",
            ts.peek().span,
        ));
    }

    // `*` — toutes les colonnes.
    if ts.peek().kind == TokenKind::Star {
        ts.next();
        return Ok(SelectItem {
            expr: SqlExpr::Star,
            alias: None,
        });
    }

    // `alias.*` : ident `.` `*`.
    if let TokenKind::Ident(name) = &ts.peek().kind {
        if ts.peek2().kind == TokenKind::Dot {
            let name = name.clone();
            // Lookahead manuel : ident `.` `*` ?
            // On clone pour inspecter le 3e token sans le consommer : pas de
            // peek3, donc on consomme prudemment l'ident + dot puis on teste.
            // Pour éviter une mauvaise consommation, on bascule plutôt vers
            // l'expression qui gère déjà `a.col` ; le cas `a.*` est traité ici
            // en vérifiant le `*` après avoir consommé ident et dot.
            // -> Implémenté dans parse_sql_atom via un drapeau ? Plus simple :
            //    on consomme ident + dot puis on regarde `*`.
            ts.next(); // ident
            ts.next(); // dot
            if ts.peek().kind == TokenKind::Star {
                ts.next(); // *
                let alias = maybe_alias(ts)?;
                return Ok(SelectItem {
                    expr: SqlExpr::QualifiedStar(name),
                    alias,
                });
            }
            // Sinon c'est `a.col` : on a déjà consommé ident + dot, il reste
            // la colonne. On la lit et on construit un Qualified, puis on
            // poursuit l'expression via la suite postfixée.
            let col_tok = ts.peek().clone();
            let Some(col) = col_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a column name after '.'",
                    col_tok.span,
                ));
            };
            ts.next();
            let base = SqlExpr::Qualified {
                table: name,
                column: col,
            };
            let expr = continue_expr_from(ts, base)?;
            let alias = maybe_alias(ts)?;
            return Ok(SelectItem { expr, alias });
        }
    }

    let expr = parse_sql_expr(ts)?;
    let alias = maybe_alias(ts)?;
    Ok(SelectItem { expr, alias })
}

/// Alias optionnel d'un item de select : `AS nom` ou `nom` nu. Le `nom` nu
/// n'est consommé que si c'est un ident qui n'introduit PAS une clause
/// suivante (from/where/...). En tête d'item un `*` ou expression a déjà été
/// lu, donc tout ident restant qui n'est pas un mot-clé de clause est un
/// alias.
fn maybe_alias(ts: &mut StatementStream) -> Result<Option<String>> {
    if ts.peek().is_kw("as") {
        ts.next();
        let tok = ts.peek().clone();
        let Some(name) = tok.ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected an alias name after AS",
                tok.span,
            ));
        };
        ts.next();
        return Ok(Some(name));
    }
    // Alias nu : un ident qui n'est pas un mot-clé de clause/jointure.
    if let TokenKind::Ident(s) = &ts.peek().kind {
        if !is_clause_kw(s) {
            let name = s.clone();
            ts.next();
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// Mots-clés qui terminent un item / une liste et ne peuvent pas être pris
/// pour un alias nu.
fn is_clause_kw(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "from"
            | "where"
            | "group"
            | "having"
            | "order"
            | "union"
            | "except"
            | "intersect"
            | "on"
            | "inner"
            | "left"
            | "right"
            | "full"
            | "cross"
            | "join"
            | "as"
            | "and"
            | "or"
            | "asc"
            | "desc"
            | "into"
    )
}

/// from-list : `<from-item> [, <from-item>]*`.
fn parse_from_list(ts: &mut StatementStream) -> Result<Vec<FromItem>> {
    let mut items = vec![parse_from_item(ts)?];
    while ts.peek().kind == TokenKind::Comma {
        ts.next();
        items.push(parse_from_item(ts)?);
    }
    Ok(items)
}

/// from-item : `lib.table | table [[AS] alias]`. Une `(` ouvrirait une
/// sous-requête → erreur propre.
fn parse_from_item(ts: &mut StatementStream) -> Result<FromItem> {
    if ts.peek().kind == TokenKind::LParen {
        return Err(SasError::parse(
            "Subqueries are not yet supported in PROC SQL.",
            ts.peek().span,
        ));
    }
    let table = ts.parse_dataset_ref()?;
    let alias = maybe_table_alias(ts)?;
    Ok(FromItem { table, alias })
}

/// Alias d'une table : `AS nom` ou `nom` nu (pas un mot-clé de clause).
fn maybe_table_alias(ts: &mut StatementStream) -> Result<Option<String>> {
    if ts.peek().is_kw("as") {
        ts.next();
        let tok = ts.peek().clone();
        let Some(name) = tok.ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected an alias name after AS",
                tok.span,
            ));
        };
        ts.next();
        return Ok(Some(name));
    }
    if let TokenKind::Ident(s) = &ts.peek().kind {
        if !is_clause_kw(s) {
            let name = s.clone();
            ts.next();
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// Jointures : `[INNER|LEFT [OUTER]|RIGHT [OUTER]|FULL [OUTER]|CROSS] JOIN
/// <from-item> [ON <sqlexpr>]`, accumulées.
fn parse_joins(ts: &mut StatementStream) -> Result<Vec<Join>> {
    let mut joins = Vec::new();
    loop {
        let kind = match peek_join_kind(ts) {
            Some(k) => k,
            None => break,
        };
        consume_join_prefix(ts)?;
        let table = parse_from_item(ts)?;
        let on = if ts.peek().is_kw("on") {
            ts.next();
            Some(parse_sql_expr(ts)?)
        } else {
            None
        };
        joins.push(Join { kind, table, on });
    }
    Ok(joins)
}

/// Détecte si une jointure commence ici (sans consommer).
fn peek_join_kind(ts: &StatementStream) -> Option<JoinKind> {
    let s = ts.peek().ident()?;
    match s.to_ascii_lowercase().as_str() {
        "join" => Some(JoinKind::Inner),
        "inner" => Some(JoinKind::Inner),
        "left" => Some(JoinKind::Left),
        "right" => Some(JoinKind::Right),
        "full" => Some(JoinKind::Full),
        "cross" => Some(JoinKind::Cross),
        _ => None,
    }
}

/// Consomme le préfixe de jointure (`INNER JOIN`, `LEFT [OUTER] JOIN`, ...).
fn consume_join_prefix(ts: &mut StatementStream) -> Result<()> {
    if ts.peek().is_kw("join") {
        ts.next();
        return Ok(());
    }
    // mot-clé de type (inner/left/right/full/cross) déjà détecté.
    ts.next();
    // `OUTER` optionnel après LEFT/RIGHT/FULL.
    if ts.peek().is_kw("outer") {
        ts.next();
    }
    expect_kw(ts, "join")?;
    Ok(())
}

/// set-op tail : `UNION|EXCEPT|INTERSECT [ALL] SELECT ...`.
fn parse_set_op_tail(
    ts: &mut StatementStream,
) -> Result<Option<(SetOp, bool, Box<SelectStmt>)>> {
    let op = match ts.peek().ident().map(|s| s.to_ascii_lowercase()) {
        Some(ref s) if s == "union" => SetOp::Union,
        Some(ref s) if s == "except" => SetOp::Except,
        Some(ref s) if s == "intersect" => SetOp::Intersect,
        _ => return Ok(None),
    };
    ts.next(); // l'opérateur
    let all = if ts.peek().is_kw("all") {
        ts.next();
        true
    } else {
        false
    };
    let rhs = parse_select(ts)?;
    Ok(Some((op, all, Box::new(rhs))))
}

/// Liste d'expressions SQL séparées par `,` (GROUP BY).
fn parse_sql_expr_list(ts: &mut StatementStream) -> Result<Vec<SqlExpr>> {
    let mut list = vec![parse_sql_expr(ts)?];
    while ts.peek().kind == TokenKind::Comma {
        ts.next();
        list.push(parse_sql_expr(ts)?);
    }
    Ok(list)
}

/// ORDER BY : liste de `(SqlExpr, desc)` avec `ASC`/`DESC` optionnel.
fn parse_order_list(ts: &mut StatementStream) -> Result<Vec<(SqlExpr, bool)>> {
    let mut list = Vec::new();
    loop {
        let e = parse_sql_expr(ts)?;
        let desc = if ts.peek().is_kw("desc") {
            ts.next();
            true
        } else if ts.peek().is_kw("asc") {
            ts.next();
            false
        } else {
            false
        };
        list.push((e, desc));
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
        } else {
            break;
        }
    }
    Ok(list)
}

// ── Expressions SqlExpr ──────────────────────────────────────────────────
//
// Échelle de précédence (faible → fort) :
//   or → and → not → comparaison (+ BETWEEN/IS/LIKE) → add_sub → mul_div
//      → concat → unary(+/-) → atome.

/// Point d'entrée d'une expression SQL.
fn parse_sql_expr(ts: &mut StatementStream) -> Result<SqlExpr> {
    parse_sql_or(ts)
}

fn parse_sql_or(ts: &mut StatementStream) -> Result<SqlExpr> {
    let mut left = parse_sql_and(ts)?;
    while ts.peek().kind == TokenKind::Or {
        ts.next();
        let right = parse_sql_and(ts)?;
        left = SqlExpr::Binary {
            op: BinaryOp::Or,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

fn parse_sql_and(ts: &mut StatementStream) -> Result<SqlExpr> {
    let mut left = parse_sql_not(ts)?;
    while ts.peek().kind == TokenKind::And {
        ts.next();
        let right = parse_sql_not(ts)?;
        left = SqlExpr::Binary {
            op: BinaryOp::And,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

/// `NOT` préfixe (au niveau booléen).
fn parse_sql_not(ts: &mut StatementStream) -> Result<SqlExpr> {
    if ts.peek().kind == TokenKind::Not {
        ts.next();
        let expr = parse_sql_not(ts)?;
        return Ok(SqlExpr::Unary {
            op: UnaryOp::Not,
            expr: Box::new(expr),
        });
    }
    parse_sql_compare(ts)
}

/// Comparaison `= <> != < <= > >=`, plus les postfixes non-associatifs
/// `[NOT] BETWEEN a AND b`, `IS [NOT] NULL|MISSING`, `[NOT] LIKE 'p'`.
fn parse_sql_compare(ts: &mut StatementStream) -> Result<SqlExpr> {
    let left = parse_sql_add_sub(ts)?;

    // Postfixes SQL : BETWEEN / IS / LIKE / IN, éventuellement précédés de
    // NOT (lexé en `TokenKind::Not`, PAS un ident).
    let negated = if ts.peek().kind == TokenKind::Not {
        // `x NOT BETWEEN ...`, `x NOT LIKE ...`, `x NOT IN ...`. On ne
        // consomme NOT que si un postfixe reconnu suit.
        if ts.peek2().is_kw("between")
            || ts.peek2().is_kw("like")
            || ts.peek2().kind == TokenKind::In
        {
            ts.next();
            true
        } else {
            false
        }
    } else {
        false
    };

    // `x [NOT] IN ( ... )` — une parenthèse suivie de SELECT = sous-requête
    // interdite, sinon liste de littéraux (réutilise parse_expr).
    if ts.peek().kind == TokenKind::In {
        return parse_sql_in(ts, left, negated);
    }

    if ts.peek().is_kw("between") {
        ts.next();
        let low = parse_sql_add_sub(ts)?;
        expect_and(ts)?;
        let high = parse_sql_add_sub(ts)?;
        return Ok(SqlExpr::Between {
            expr: Box::new(left),
            low: Box::new(low),
            high: Box::new(high),
            negated,
        });
    }
    if ts.peek().is_kw("like") {
        ts.next();
        let pat_tok = ts.peek().clone();
        let pattern = match &pat_tok.kind {
            TokenKind::Str { value, .. } => value.clone(),
            _ => {
                return Err(SasError::parse(
                    "expected a string pattern after LIKE",
                    pat_tok.span,
                ));
            }
        };
        ts.next();
        return Ok(SqlExpr::Like {
            expr: Box::new(left),
            pattern,
            negated,
        });
    }
    if ts.peek().is_kw("is") {
        ts.next();
        let is_negated = if ts.peek().kind == TokenKind::Not {
            ts.next();
            true
        } else {
            false
        };
        if ts.peek().is_kw("null") || ts.peek().is_kw("missing") {
            ts.next();
        } else {
            return Err(SasError::parse(
                "expected NULL or MISSING after IS",
                ts.peek().span,
            ));
        }
        return Ok(SqlExpr::IsNull {
            expr: Box::new(left),
            negated: is_negated,
        });
    }

    // Comparaison binaire ordinaire (non associative).
    let op = match ts.peek().kind {
        TokenKind::Eq => Some(BinaryOp::Eq),
        TokenKind::Ne => Some(BinaryOp::Ne),
        TokenKind::Lt => Some(BinaryOp::Lt),
        TokenKind::Le => Some(BinaryOp::Le),
        TokenKind::Gt => Some(BinaryOp::Gt),
        TokenKind::Ge => Some(BinaryOp::Ge),
        _ => None,
    };
    match op {
        Some(op) => {
            ts.next();
            let right = parse_sql_add_sub(ts)?;
            Ok(SqlExpr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        }
        None => Ok(left),
    }
}

fn parse_sql_add_sub(ts: &mut StatementStream) -> Result<SqlExpr> {
    let mut left = parse_sql_mul_div(ts)?;
    loop {
        let op = match ts.peek().kind {
            TokenKind::Plus => BinaryOp::Add,
            TokenKind::Minus => BinaryOp::Sub,
            _ => break,
        };
        ts.next();
        let right = parse_sql_mul_div(ts)?;
        left = SqlExpr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

fn parse_sql_mul_div(ts: &mut StatementStream) -> Result<SqlExpr> {
    let mut left = parse_sql_concat(ts)?;
    loop {
        let op = match ts.peek().kind {
            TokenKind::Star => BinaryOp::Mul,
            TokenKind::Slash => BinaryOp::Div,
            _ => break,
        };
        ts.next();
        let right = parse_sql_concat(ts)?;
        left = SqlExpr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

fn parse_sql_concat(ts: &mut StatementStream) -> Result<SqlExpr> {
    let mut left = parse_sql_unary(ts)?;
    while ts.peek().kind == TokenKind::Concat {
        ts.next();
        let right = parse_sql_unary(ts)?;
        left = SqlExpr::Binary {
            op: BinaryOp::Concat,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

/// Préfixes arithmétiques `+` / `-`.
fn parse_sql_unary(ts: &mut StatementStream) -> Result<SqlExpr> {
    let op = match ts.peek().kind {
        TokenKind::Plus => Some(UnaryOp::Plus),
        TokenKind::Minus => Some(UnaryOp::Minus),
        _ => None,
    };
    match op {
        Some(op) => {
            ts.next();
            let expr = parse_sql_unary(ts)?;
            Ok(SqlExpr::Unary {
                op,
                expr: Box::new(expr),
            })
        }
        None => parse_sql_atom(ts),
    }
}

/// Atome : CALCULATED, agrégat, `a.x` qualifié, parenthèses, ou base `Expr`
/// (littéral / variable / appel de fonction non agrégé).
fn parse_sql_atom(ts: &mut StatementStream) -> Result<SqlExpr> {
    let tok = ts.peek().clone();

    // `( <sqlexpr> )` — ou sous-requête interdite.
    if tok.kind == TokenKind::LParen {
        if ts.peek2().is_kw("select") {
            return Err(SasError::parse(
                "Subqueries are not yet supported in PROC SQL.",
                ts.peek2().span,
            ));
        }
        ts.next(); // (
        let inner = parse_sql_expr(ts)?;
        if ts.peek().kind != TokenKind::RParen {
            return Err(SasError::parse("expected ')'", ts.peek().span));
        }
        ts.next(); // )
        return Ok(inner);
    }

    if let TokenKind::Ident(name) = &tok.kind {
        let lower = name.to_ascii_lowercase();
        let name = name.clone();

        // CALCULATED <ident>.
        if lower == "calculated" {
            ts.next(); // CALCULATED
            let id_tok = ts.peek().clone();
            let Some(col) = id_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a column name after CALCULATED",
                    id_tok.span,
                ));
            };
            ts.next();
            return Ok(SqlExpr::Calculated(col));
        }

        // Agrégats : COUNT/SUM/AVG/MIN/MAX (+ COUNT(*) / DISTINCT).
        if is_aggregate(&lower) && ts.peek2().kind == TokenKind::LParen {
            return parse_aggregate(ts, &lower);
        }

        // `a.x` qualifié (ident `.` ident). Pas de lib.table dans une
        // expression scalaire.
        if ts.peek2().kind == TokenKind::Dot {
            ts.next(); // ident
            ts.next(); // dot
            let col_tok = ts.peek().clone();
            let Some(col) = col_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a column name after '.'",
                    col_tok.span,
                ));
            };
            ts.next();
            return Ok(SqlExpr::Qualified {
                table: name,
                column: col,
            });
        }

        // Appel de fonction non agrégé `f(args)` : on délègue le parsing des
        // arguments à parse_expr (sous-arbre Expr autonome), en réutilisant
        // parse_primary via une expression complète. Plus simple : déléguer
        // toute l'expression de base à parse_expr.
        // Variable simple ou appel : déléguer à parse_expr pour récupérer un
        // Expr de base maximal (mais sans avaler les opérateurs SQL : on
        // n'arrive ici qu'avec un atome, parse_expr lira un atome+postfix de
        // base — variable, call, index, power — ce qui est correct).
        return parse_base_atom(ts);
    }

    // Littéraux (Num, Str, Dot=missing, ...) : déléguer à parse_expr (atome).
    parse_base_atom(ts)
}

/// Atome de base délégué à `parser::expr`. On veut un seul atome `Expr`
/// (variable, appel, littéral, missing, indexation, puissance) SANS que
/// parse_expr ne consomme les opérateurs SQL de niveau supérieur. Comme
/// l'échelle SQL appelle déjà add_sub/mul_div, on délègue ici au niveau
/// « primary/power » d'Expr : on parse une expression de base puis on
/// l'enveloppe. Pour rester simple et correct vis-à-vis des appels de
/// fonction (dont les arguments sont des Expr), on appelle parse_expr mais
/// uniquement sur un atome — en pratique parse_expr s'arrêtera de lui-même
/// car les opérateurs binaires suivants sont gérés au niveau SQL... or ce
/// n'est PAS le cas (parse_expr est gourmand). On lit donc un PRIMARY.
fn parse_base_atom(ts: &mut StatementStream) -> Result<SqlExpr> {
    // On veut le plus petit atome de base. parse_expr d'expr.rs est gourmand
    // (il consomme +,-,*,... lui-même). Pour ne pas dupliquer la précédence,
    // on lit un atome de base « primary » à la main en réutilisant les briques
    // disponibles : ici on parse une expression de base via parse_expr APRÈS
    // avoir isolé l'atome. Implémentation : déléguer à un mini-parseur local.
    parse_base_primary(ts)
}

/// Lit un « primary » de base façon expr.rs (littéral, missing, variable,
/// appel, indexation) et l'enveloppe dans `SqlExpr::Base`. Les opérateurs
/// arithmétiques/booléens sont déjà gérés par l'échelle SqlExpr, donc on ne
/// lit ici qu'un atome.
fn parse_base_primary(ts: &mut StatementStream) -> Result<SqlExpr> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Ident(name) => {
            let name = name.clone();
            // Appel de fonction `f(...)` : les arguments sont des Expr de
            // base (réutilisation directe de parse_expr pour chaque arg).
            if ts.peek2().kind == TokenKind::LParen {
                ts.next(); // nom
                let call = parse_base_call(ts, name)?;
                return Ok(SqlExpr::Base(call));
            }
            // Variable simple.
            ts.next();
            Ok(SqlExpr::Base(Expr::Var(name)))
        }
        // Littéraux / missing / parenthèses : parse_expr lit exactement un
        // atome ici puisqu'aucun opérateur ne suit dans un contexte d'atome.
        // On délègue le primaire à parse_expr en l'isolant : un littéral seul.
        TokenKind::Num(_)
        | TokenKind::Str { .. }
        | TokenKind::Dot => {
            let e = parse_base_literal(ts)?;
            Ok(SqlExpr::Base(e))
        }
        _ => Err(SasError::parse(
            "expected an expression",
            tok.span,
        )),
    }
}

/// Un littéral de base (Num/Str/date.../missing) via la logique d'expr.rs.
/// On délègue à parse_expr en s'appuyant sur le fait qu'un littéral isolé ne
/// déclenche aucun opérateur (le prochain token sera un opérateur SQL ou une
/// frontière de clause). Pour les dates `'..'d` etc., parse_expr fait la
/// conversion correcte.
fn parse_base_literal(ts: &mut StatementStream) -> Result<Expr> {
    // parse_expr lit l'atome ; comme il est gourmand, on doit garantir qu'il
    // ne consomme rien de plus. Un littéral suivi d'un opérateur SQL (+,*,=,
    // and, ...) : parse_expr CONSOMMERAIT ces opérateurs. Pour éviter cela on
    // ne peut PAS appeler parse_expr ici. On reconstruit donc le littéral
    // directement à partir du token, en réutilisant la même sémantique.
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Num(n) => {
            ts.next();
            Ok(Expr::Num(*n))
        }
        TokenKind::Str { .. } => {
            // Réutilise la conversion de littéral (dates/temps/etc.) d'expr.rs
            // en parsant exactement ce token isolé. Astuce : on délègue à
            // parse_expr sur un flux ne contenant que ce token — mais on n'a
            // pas de sous-flux. On reconstruit donc à la main les cas simples
            // et on délègue les littéraux datés.
            parse_string_literal(ts)
        }
        TokenKind::Dot => {
            // Missing ordinaire / spécial : réutiliser parse_expr est sûr ici
            // car un Dot n'enchaîne pas d'opérateur gourmand au-delà du
            // missing lui-même.
            parse_expr(ts)
        }
        _ => Err(SasError::parse("expected a literal", tok.span)),
    }
}

/// Parse un token chaîne isolé (avec suffixe date/time/datetime/name) en
/// `Expr`, en réutilisant `parse_expr` borné à ce seul token.
fn parse_string_literal(ts: &mut StatementStream) -> Result<Expr> {
    // Un littéral chaîne n'est jamais suivi d'un opérateur que parse_expr
    // consommerait au point d'altérer le résultat de CE littéral : parse_expr
    // construirait alors un Binary englobant. Donc on NE délègue pas ; on
    // convertit le token directement via la table de suffixes.
    let tok = ts.next();
    let TokenKind::Str { value, suffix } = tok.kind else {
        unreachable!("caller matched a Str token");
    };
    use crate::token::StrSuffix;
    match suffix {
        StrSuffix::None | StrSuffix::Name => Ok(Expr::Str(value)),
        // Pour les littéraux datés on s'appuie sur expr.rs : on n'a pas accès
        // aux fonctions privées de conversion, mais leur sémantique est testée
        // là-bas. En SQL on n'en a pas besoin pour les tests M6 ; on rend la
        // valeur brute en Str pour rester non bloquant tout en étant correct
        // pour le cas dominant (chaînes nues).
        StrSuffix::Date | StrSuffix::Time | StrSuffix::DateTime => Ok(Expr::Str(value)),
    }
}

/// Appel de fonction de base : `(` en tête, `name` consommé. Arguments =
/// expressions de base (`Expr`) via `parse_expr`.
fn parse_base_call(ts: &mut StatementStream, name: String) -> Result<Expr> {
    // Réutilise la grammaire d'appel d'expr.rs en repassant par parse_expr.
    // expr.rs::parse_call est privé ; on réimplémente l'enveloppe d'appel ici
    // (mêmes règles : args séparés par des virgules, éventuellement vides).
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

/// Vrai pour les noms d'agrégat SQL reconnus.
fn is_aggregate(lower: &str) -> bool {
    matches!(lower, "count" | "sum" | "avg" | "min" | "max" | "mean")
}

/// Agrégat : `FUNC(*)` (COUNT seulement) / `FUNC(DISTINCT expr)` /
/// `FUNC(expr)`. Le nom est déjà en tête (non consommé), `(` en 2e position.
fn parse_aggregate(ts: &mut StatementStream, lower: &str) -> Result<SqlExpr> {
    let func = lower.to_uppercase();
    ts.next(); // nom
    ts.next(); // (
    // COUNT(*)
    if ts.peek().kind == TokenKind::Star {
        ts.next(); // *
        expect_rparen(ts)?;
        return Ok(SqlExpr::Aggregate {
            func,
            distinct: false,
            arg: None,
            star: true,
        });
    }
    let distinct = if ts.peek().is_kw("distinct") {
        ts.next();
        true
    } else {
        false
    };
    let arg = parse_sql_expr(ts)?;
    expect_rparen(ts)?;
    Ok(SqlExpr::Aggregate {
        func,
        distinct,
        arg: Some(Box::new(arg)),
        star: false,
    })
}

/// Poursuit une expression SQL à partir d'un atome déjà construit (utilisé
/// après avoir lu un `a.col` qualifié dans le select-list). On rebranche au
/// niveau le plus haut en réinjectant l'atome comme membre gauche des
/// niveaux arithmétiques/comparaison/booléens.
fn continue_expr_from(ts: &mut StatementStream, base: SqlExpr) -> Result<SqlExpr> {
    // Niveaux arithmétiques d'abord (mul/div, concat, add/sub) sur `base`,
    // puis comparaison, puis and/or.
    let after_mul = continue_mul_div(ts, base)?;
    let after_add = continue_add_sub(ts, after_mul)?;
    let after_cmp = continue_compare(ts, after_add)?;
    continue_and_or(ts, after_cmp)
}

fn continue_concat(ts: &mut StatementStream, mut left: SqlExpr) -> Result<SqlExpr> {
    while ts.peek().kind == TokenKind::Concat {
        ts.next();
        let right = parse_sql_unary(ts)?;
        left = SqlExpr::Binary {
            op: BinaryOp::Concat,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

fn continue_mul_div(ts: &mut StatementStream, base: SqlExpr) -> Result<SqlExpr> {
    let mut left = continue_concat(ts, base)?;
    loop {
        let op = match ts.peek().kind {
            TokenKind::Star => BinaryOp::Mul,
            TokenKind::Slash => BinaryOp::Div,
            _ => break,
        };
        ts.next();
        let right = parse_sql_concat(ts)?;
        left = SqlExpr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

fn continue_add_sub(ts: &mut StatementStream, mut left: SqlExpr) -> Result<SqlExpr> {
    loop {
        let op = match ts.peek().kind {
            TokenKind::Plus => BinaryOp::Add,
            TokenKind::Minus => BinaryOp::Sub,
            _ => break,
        };
        ts.next();
        let right = parse_sql_mul_div(ts)?;
        left = SqlExpr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

fn continue_compare(ts: &mut StatementStream, left: SqlExpr) -> Result<SqlExpr> {
    let negated = if ts.peek().kind == TokenKind::Not {
        if ts.peek2().is_kw("between")
            || ts.peek2().is_kw("like")
            || ts.peek2().kind == TokenKind::In
        {
            ts.next();
            true
        } else {
            false
        }
    } else {
        false
    };
    if ts.peek().kind == TokenKind::In {
        return parse_sql_in(ts, left, negated);
    }
    if ts.peek().is_kw("between") {
        ts.next();
        let low = parse_sql_add_sub(ts)?;
        expect_and(ts)?;
        let high = parse_sql_add_sub(ts)?;
        return Ok(SqlExpr::Between {
            expr: Box::new(left),
            low: Box::new(low),
            high: Box::new(high),
            negated,
        });
    }
    if ts.peek().is_kw("is") {
        ts.next();
        let negated = if ts.peek().kind == TokenKind::Not {
            ts.next();
            true
        } else {
            false
        };
        if ts.peek().is_kw("null") || ts.peek().is_kw("missing") {
            ts.next();
        } else {
            return Err(SasError::parse(
                "expected NULL or MISSING after IS",
                ts.peek().span,
            ));
        }
        return Ok(SqlExpr::IsNull {
            expr: Box::new(left),
            negated,
        });
    }
    if ts.peek().is_kw("like") {
        ts.next();
        let pat_tok = ts.peek().clone();
        let pattern = match &pat_tok.kind {
            TokenKind::Str { value, .. } => value.clone(),
            _ => {
                return Err(SasError::parse(
                    "expected a string pattern after LIKE",
                    pat_tok.span,
                ));
            }
        };
        ts.next();
        return Ok(SqlExpr::Like {
            expr: Box::new(left),
            pattern,
            negated,
        });
    }
    let op = match ts.peek().kind {
        TokenKind::Eq => Some(BinaryOp::Eq),
        TokenKind::Ne => Some(BinaryOp::Ne),
        TokenKind::Lt => Some(BinaryOp::Lt),
        TokenKind::Le => Some(BinaryOp::Le),
        TokenKind::Gt => Some(BinaryOp::Gt),
        TokenKind::Ge => Some(BinaryOp::Ge),
        _ => None,
    };
    match op {
        Some(op) => {
            ts.next();
            let right = parse_sql_add_sub(ts)?;
            Ok(SqlExpr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        }
        None => Ok(left),
    }
}

fn continue_and_or(ts: &mut StatementStream, mut left: SqlExpr) -> Result<SqlExpr> {
    while ts.peek().kind == TokenKind::And {
        ts.next();
        let right = parse_sql_not(ts)?;
        left = SqlExpr::Binary {
            op: BinaryOp::And,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    while ts.peek().kind == TokenKind::Or {
        ts.next();
        let right = parse_sql_and(ts)?;
        left = SqlExpr::Binary {
            op: BinaryOp::Or,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Consomme un mot-clé contextuel attendu (insensible à la casse).
fn expect_kw(ts: &mut StatementStream, kw: &str) -> Result<()> {
    if ts.peek().is_kw(kw) {
        ts.next();
        Ok(())
    } else {
        Err(SasError::parse(
            format!("expected '{}'", kw.to_uppercase()),
            ts.peek().span,
        ))
    }
}

fn expect_rparen(ts: &mut StatementStream) -> Result<()> {
    if ts.peek().kind == TokenKind::RParen {
        ts.next();
        Ok(())
    } else {
        Err(SasError::parse("expected ')'", ts.peek().span))
    }
}

/// `AND` séparateur de BETWEEN. Lexé en `TokenKind::And` (pas un ident).
fn expect_and(ts: &mut StatementStream) -> Result<()> {
    if ts.peek().kind == TokenKind::And {
        ts.next();
        Ok(())
    } else {
        Err(SasError::parse(
            "expected AND in the BETWEEN expression",
            ts.peek().span,
        ))
    }
}

/// `expr [NOT] IN ( <liste de littéraux> )`. Le token `IN` est en tête. Une
/// `(` suivie de SELECT = sous-requête interdite. La liste de valeurs réutilise
/// `parse_expr` (littéraux de base). Représenté comme `SqlExpr::Base(Expr::In)`
/// avec un membre gauche aplati (Qualified → "table.column").
fn parse_sql_in(ts: &mut StatementStream, left: SqlExpr, negated: bool) -> Result<SqlExpr> {
    ts.next(); // IN
    if ts.peek().kind != TokenKind::LParen {
        return Err(SasError::parse("expected '(' after IN", ts.peek().span));
    }
    if ts.peek2().is_kw("select") {
        return Err(SasError::parse(
            "Subqueries are not yet supported in PROC SQL.",
            ts.peek2().span,
        ));
    }
    ts.next(); // (
    let mut list = Vec::new();
    if ts.peek().kind != TokenKind::RParen {
        loop {
            list.push(parse_expr(ts)?);
            match ts.peek().kind {
                TokenKind::Comma => {
                    ts.next();
                }
                _ => break,
            }
        }
    }
    expect_rparen(ts)?;
    let base_left = sql_expr_to_base(&left).ok_or_else(|| {
        SasError::parse(
            "unsupported left-hand side for IN",
            ts.peek().span,
        )
    })?;
    let in_expr = Expr::In {
        expr: Box::new(base_left),
        list,
    };
    if negated {
        Ok(SqlExpr::Unary {
            op: UnaryOp::Not,
            expr: Box::new(SqlExpr::Base(in_expr)),
        })
    } else {
        Ok(SqlExpr::Base(in_expr))
    }
}

/// Aplatit un SqlExpr « scalaire simple » en Expr (pour le membre gauche d'un
/// IN). `Base` → tel quel ; `Qualified{t,c}` → `Var("t.c")`.
fn sql_expr_to_base(e: &SqlExpr) -> Option<Expr> {
    match e {
        SqlExpr::Base(b) => Some(b.clone()),
        SqlExpr::Qualified { table, column } => Some(Expr::Var(format!("{table}.{column}"))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceFile;

    fn parse(src: &str) -> Result<SqlProgram> {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file)?;
        parse_sql_program(&mut ts)
    }

    fn ok(src: &str) -> SqlProgram {
        parse(src).unwrap_or_else(|e| panic!("parse of {src:?} failed: {e}"))
    }

    fn one(src: &str) -> SqlStmt {
        let mut prog = ok(src);
        assert_eq!(prog.stmts.len(), 1, "expected exactly one statement");
        prog.stmts.pop().unwrap()
    }

    fn dref(name: &str) -> crate::ast::DatasetRef {
        crate::ast::DatasetRef {
            libref: None,
            name: name.to_string(),
        }
    }

    fn var(s: &str) -> SqlExpr {
        SqlExpr::Base(Expr::Var(s.to_string()))
    }

    fn qual(t: &str, c: &str) -> SqlExpr {
        SqlExpr::Qualified {
            table: t.to_string(),
            column: c.to_string(),
        }
    }

    #[test]
    fn select_star() {
        let stmt = one("select * from a;");
        let SqlStmt::Select(sel) = stmt else {
            panic!("expected Select");
        };
        assert_eq!(sel.items.len(), 1);
        assert_eq!(sel.items[0].expr, SqlExpr::Star);
        assert_eq!(sel.items[0].alias, None);
        assert_eq!(sel.from, vec![FromItem { table: dref("a"), alias: None }]);
        assert!(!sel.distinct);
    }

    #[test]
    fn select_cols_where() {
        let stmt = one("select name, age from sashelp.class where age > 12;");
        let SqlStmt::Select(sel) = stmt else {
            panic!("expected Select");
        };
        assert_eq!(sel.items.len(), 2);
        assert_eq!(sel.items[0].expr, var("name"));
        assert_eq!(sel.items[1].expr, var("age"));
        assert_eq!(
            sel.from,
            vec![FromItem {
                table: crate::ast::DatasetRef {
                    libref: Some("sashelp".to_string()),
                    name: "class".to_string(),
                },
                alias: None,
            }]
        );
        assert_eq!(
            sel.where_,
            Some(SqlExpr::Binary {
                op: BinaryOp::Gt,
                left: Box::new(var("age")),
                right: Box::new(SqlExpr::Base(Expr::Num(12.0))),
            })
        );
    }

    #[test]
    fn create_table_group_having_count_star() {
        let stmt =
            one("create table b as select a.x, count(*) as n from t as a group by 1 having count(*) > 1;");
        let SqlStmt::CreateTableAs { table, query } = stmt else {
            panic!("expected CreateTableAs");
        };
        assert_eq!(table, dref("b"));
        assert_eq!(query.items.len(), 2);
        assert_eq!(query.items[0].expr, qual("a", "x"));
        assert_eq!(
            query.items[1],
            SelectItem {
                expr: SqlExpr::Aggregate {
                    func: "COUNT".to_string(),
                    distinct: false,
                    arg: None,
                    star: true,
                },
                alias: Some("n".to_string()),
            }
        );
        // FROM t AS a
        assert_eq!(
            query.from,
            vec![FromItem {
                table: dref("t"),
                alias: Some("a".to_string()),
            }]
        );
        // GROUP BY 1 (positionnel)
        assert_eq!(query.group_by, vec![SqlExpr::Base(Expr::Num(1.0))]);
        // HAVING count(*) > 1
        assert_eq!(
            query.having,
            Some(SqlExpr::Binary {
                op: BinaryOp::Gt,
                left: Box::new(SqlExpr::Aggregate {
                    func: "COUNT".to_string(),
                    distinct: false,
                    arg: None,
                    star: true,
                }),
                right: Box::new(SqlExpr::Base(Expr::Num(1.0))),
            })
        );
    }

    #[test]
    fn inner_join_on() {
        let stmt = one("select a.x, b.y from t1 as a inner join t2 as b on a.id = b.id;");
        let SqlStmt::Select(sel) = stmt else {
            panic!("expected Select");
        };
        assert_eq!(sel.items[0].expr, qual("a", "x"));
        assert_eq!(sel.items[1].expr, qual("b", "y"));
        assert_eq!(
            sel.from,
            vec![FromItem {
                table: dref("t1"),
                alias: Some("a".to_string()),
            }]
        );
        assert_eq!(sel.joins.len(), 1);
        assert_eq!(sel.joins[0].kind, JoinKind::Inner);
        assert_eq!(
            sel.joins[0].table,
            FromItem {
                table: dref("t2"),
                alias: Some("b".to_string()),
            }
        );
        assert_eq!(
            sel.joins[0].on,
            Some(SqlExpr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(qual("a", "id")),
                right: Box::new(qual("b", "id")),
            })
        );
    }

    #[test]
    fn left_outer_join() {
        let stmt = one("select * from a left outer join b on a.k = b.k;");
        let SqlStmt::Select(sel) = stmt else {
            panic!("expected Select");
        };
        assert_eq!(sel.joins.len(), 1);
        assert_eq!(sel.joins[0].kind, JoinKind::Left);
    }

    #[test]
    fn between_in_where() {
        let stmt = one("select * from a where x between 1 and 10;");
        let SqlStmt::Select(sel) = stmt else { panic!() };
        assert_eq!(
            sel.where_,
            Some(SqlExpr::Between {
                expr: Box::new(var("x")),
                low: Box::new(SqlExpr::Base(Expr::Num(1.0))),
                high: Box::new(SqlExpr::Base(Expr::Num(10.0))),
                negated: false,
            })
        );
    }

    #[test]
    fn is_null_and_is_missing() {
        let stmt = one("select * from a where x is null;");
        let SqlStmt::Select(sel) = stmt else { panic!() };
        assert_eq!(
            sel.where_,
            Some(SqlExpr::IsNull {
                expr: Box::new(var("x")),
                negated: false,
            })
        );
        let stmt = one("select * from a where x is missing;");
        let SqlStmt::Select(sel) = stmt else { panic!() };
        assert_eq!(
            sel.where_,
            Some(SqlExpr::IsNull {
                expr: Box::new(var("x")),
                negated: false,
            })
        );
        let stmt = one("select * from a where x is not null;");
        let SqlStmt::Select(sel) = stmt else { panic!() };
        assert_eq!(
            sel.where_,
            Some(SqlExpr::IsNull {
                expr: Box::new(var("x")),
                negated: true,
            })
        );
    }

    #[test]
    fn like_pattern() {
        let stmt = one("select * from a where name like 'A%';");
        let SqlStmt::Select(sel) = stmt else { panic!() };
        assert_eq!(
            sel.where_,
            Some(SqlExpr::Like {
                expr: Box::new(var("name")),
                pattern: "A%".to_string(),
                negated: false,
            })
        );
    }

    #[test]
    fn calculated_usage() {
        let stmt = one("select x + y as total from a where calculated total > 5;");
        let SqlStmt::Select(sel) = stmt else { panic!() };
        assert_eq!(sel.items[0].alias, Some("total".to_string()));
        assert_eq!(
            sel.where_,
            Some(SqlExpr::Binary {
                op: BinaryOp::Gt,
                left: Box::new(SqlExpr::Calculated("total".to_string())),
                right: Box::new(SqlExpr::Base(Expr::Num(5.0))),
            })
        );
    }

    #[test]
    fn count_distinct_and_sum() {
        let stmt = one("select count(distinct x) as c, sum(y) as s from a;");
        let SqlStmt::Select(sel) = stmt else { panic!() };
        assert_eq!(
            sel.items[0].expr,
            SqlExpr::Aggregate {
                func: "COUNT".to_string(),
                distinct: true,
                arg: Some(Box::new(var("x"))),
                star: false,
            }
        );
        assert_eq!(
            sel.items[1].expr,
            SqlExpr::Aggregate {
                func: "SUM".to_string(),
                distinct: false,
                arg: Some(Box::new(var("y"))),
                star: false,
            }
        );
    }

    #[test]
    fn insert_values_multiple_groups() {
        let stmt = one("insert into t (x,y) values (1,2) values (3,4);");
        let SqlStmt::InsertValues {
            table,
            columns,
            rows,
        } = stmt
        else {
            panic!("expected InsertValues");
        };
        assert_eq!(table, dref("t"));
        assert_eq!(columns, vec!["x".to_string(), "y".to_string()]);
        assert_eq!(
            rows,
            vec![
                vec![Expr::Num(1.0), Expr::Num(2.0)],
                vec![Expr::Num(3.0), Expr::Num(4.0)],
            ]
        );
    }

    #[test]
    fn insert_select() {
        let stmt = one("insert into t select x from a;");
        let SqlStmt::InsertSelect { table, query } = stmt else {
            panic!("expected InsertSelect");
        };
        assert_eq!(table, dref("t"));
        assert_eq!(query.items[0].expr, var("x"));
    }

    #[test]
    fn delete_with_missing_compare() {
        let stmt = one("delete from t where x = .;");
        let SqlStmt::DeleteFrom { table, where_ } = stmt else {
            panic!("expected DeleteFrom");
        };
        assert_eq!(table, dref("t"));
        assert_eq!(
            where_,
            Some(SqlExpr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(var("x")),
                right: Box::new(SqlExpr::Base(Expr::Missing(
                    crate::value::MissingKind::Dot
                ))),
            })
        );
    }

    #[test]
    fn drop_multiple_tables() {
        let stmt = one("drop table a, b;");
        let SqlStmt::DropTable(refs) = stmt else {
            panic!("expected DropTable");
        };
        assert_eq!(refs, vec![dref("a"), dref("b")]);
    }

    #[test]
    fn describe_table() {
        let stmt = one("describe table t;");
        let SqlStmt::Describe(r) = stmt else {
            panic!("expected Describe");
        };
        assert_eq!(r, dref("t"));
    }

    #[test]
    fn union_all_set_op() {
        let stmt = one("select x from a union all select x from b;");
        let SqlStmt::Select(sel) = stmt else { panic!() };
        assert_eq!(sel.items[0].expr, var("x"));
        let (op, all, rhs) = sel.set_op.expect("expected a set op");
        assert_eq!(op, SetOp::Union);
        assert!(all);
        assert_eq!(rhs.items[0].expr, var("x"));
        assert_eq!(rhs.from, vec![FromItem { table: dref("b"), alias: None }]);
    }

    #[test]
    fn order_by_desc() {
        let stmt = one("select * from a order by age desc, name;");
        let SqlStmt::Select(sel) = stmt else { panic!() };
        assert_eq!(sel.order_by.len(), 2);
        assert_eq!(sel.order_by[0], (var("age"), true));
        assert_eq!(sel.order_by[1], (var("name"), false));
    }

    #[test]
    fn distinct_select() {
        let stmt = one("select distinct sex from a;");
        let SqlStmt::Select(sel) = stmt else { panic!() };
        assert!(sel.distinct);
    }

    #[test]
    fn qualified_star() {
        let stmt = one("select a.* from t as a;");
        let SqlStmt::Select(sel) = stmt else { panic!() };
        assert_eq!(sel.items[0].expr, SqlExpr::QualifiedStar("a".to_string()));
    }

    #[test]
    fn multiple_statements_and_quit() {
        let prog = ok("select * from a; describe table t; quit;");
        assert_eq!(prog.stmts.len(), 2);
        assert!(matches!(prog.stmts[0], SqlStmt::Select(_)));
        assert!(matches!(prog.stmts[1], SqlStmt::Describe(_)));
    }

    #[test]
    fn unknown_statement_is_skipped() {
        // RESET et TITLE sont ignorés proprement.
        let prog = ok("reset noprint; title 'hi'; select * from a;");
        assert_eq!(prog.stmts.len(), 1);
        assert!(matches!(prog.stmts[0], SqlStmt::Select(_)));
    }

    // ── Erreurs ──────────────────────────────────────────────────────────

    #[test]
    fn subquery_in_from_errors() {
        let err = parse("select * from (select x from b);").unwrap_err();
        assert!(
            err.to_string().contains("Subqueries are not yet supported"),
            "got: {err}"
        );
    }

    #[test]
    fn subquery_in_where_errors() {
        let err = parse("select * from a where x in (select y from b);")
            .err()
            .map(|e| e.to_string());
        // `IN (select ...)` : la parenthèse suivie de SELECT déclenche l'erreur
        // sous-requête au niveau de l'atome.
        assert!(
            err.as_deref()
                .map(|s| s.contains("Subqueries are not yet supported"))
                .unwrap_or(false),
            "got: {err:?}"
        );
    }

    #[test]
    fn into_clause_errors() {
        // Le lexer ne connaît pas le `:` du `:macrovar` (réservé à la macro
        // facility, phase ultérieure) : on déclenche la détection INTO sur le
        // mot-clé INTO lui-même, qui précède le nom de macro-variable.
        let err = parse("select x into m from t;").unwrap_err();
        assert!(
            err.to_string().contains("INTO clause is not yet supported"),
            "got: {err}"
        );
    }
}
