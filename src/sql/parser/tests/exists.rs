use super::*;

#[test]
fn exists_subquery_parses() {
    // M20.2 : `EXISTS (SELECT ...)` et `NOT EXISTS (...)`.
    let stmt = one("select * from a where exists (select 1 from b);");
    let SqlStmt::Select(sel) = stmt else { panic!() };
    let SqlExpr::Exists { query, negated } = sel.where_.unwrap() else {
        panic!("expected Exists");
    };
    assert!(!negated);
    assert_eq!(query.from, vec![FromItem { table: dref("b"), alias: None, subquery: None }]);

    let stmt = one("select * from a where not exists (select 1 from b);");
    let SqlStmt::Select(sel) = stmt else { panic!() };
    // `NOT EXISTS` → Unary(Not, Exists).
    let SqlExpr::Unary { op, expr } = sel.where_.unwrap() else {
        panic!("expected Unary(Not, Exists)");
    };
    assert_eq!(op, UnaryOp::Not);
    assert!(matches!(*expr, SqlExpr::Exists { negated: false, .. }));
}

#[test]
fn not_in_subquery_parses() {
    // M20.2 : `x NOT IN (SELECT ...)` → InSubquery { negated: true }.
    let stmt = one("select * from a where x not in (select y from b);");
    let SqlStmt::Select(sel) = stmt else { panic!() };
    let SqlExpr::InSubquery { negated, .. } = sel.where_.unwrap() else {
        panic!("expected InSubquery");
    };
    assert!(negated);
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

#[test]
fn update_single_column_no_where() {
    let stmt = one("update t set x = 1;");
    let SqlStmt::Update {
        table,
        assignments,
        where_,
    } = stmt
    else {
        panic!("expected Update");
    };
    assert_eq!(table, dref("t"));
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].0, "x");
    assert_eq!(assignments[0].1, SqlExpr::Base(Expr::Num(1.0)));
    assert!(where_.is_none());
}

#[test]
fn update_multiple_columns_with_where() {
    let stmt = one("update t set x = x + 1, y = 'z' where x > 5;");
    let SqlStmt::Update {
        assignments,
        where_,
        ..
    } = stmt
    else {
        panic!("expected Update");
    };
    assert_eq!(assignments.len(), 2);
    assert_eq!(assignments[0].0, "x");
    assert_eq!(assignments[1].0, "y");
    assert!(where_.is_some());
}

#[test]
fn update_requires_set() {
    let err = parse("update t where x > 1;").unwrap_err();
    assert!(err.to_string().contains("SET"), "got: {err}");
}
