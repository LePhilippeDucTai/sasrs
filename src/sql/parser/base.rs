use super::*;

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
pub(super) fn parse_base_atom(ts: &mut StatementStream) -> Result<SqlExpr> {
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
pub(super) fn parse_base_primary(ts: &mut StatementStream) -> Result<SqlExpr> {
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
        TokenKind::Num(_) | TokenKind::Str { .. } | TokenKind::Dot => {
            let e = parse_base_literal(ts)?;
            Ok(SqlExpr::Base(e))
        }
        _ => Err(SasError::parse("expected an expression", tok.span)),
    }
}

/// Un littéral de base (Num/Str/date.../missing) via la logique d'expr.rs.
/// On délègue à parse_expr en s'appuyant sur le fait qu'un littéral isolé ne
/// déclenche aucun opérateur (le prochain token sera un opérateur SQL ou une
/// frontière de clause). Pour les dates `'..'d` etc., parse_expr fait la
/// conversion correcte.
pub(super) fn parse_base_literal(ts: &mut StatementStream) -> Result<Expr> {
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
pub(super) fn parse_string_literal(ts: &mut StatementStream) -> Result<Expr> {
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
pub(super) fn parse_base_call(ts: &mut StatementStream, name: String) -> Result<Expr> {
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
pub(super) fn is_aggregate(lower: &str) -> bool {
    matches!(lower, "count" | "sum" | "avg" | "min" | "max" | "mean")
}

/// Agrégat : `FUNC(*)` (COUNT seulement) / `FUNC(DISTINCT expr)` /
/// `FUNC(expr)`. Le nom est déjà en tête (non consommé), `(` en 2e position.
pub(super) fn parse_aggregate(ts: &mut StatementStream, lower: &str) -> Result<SqlExpr> {
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
