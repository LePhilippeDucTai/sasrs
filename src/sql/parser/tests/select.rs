use super::*;

#[test]
fn select_star() {
    let stmt = one("select * from a;");
    let SqlStmt::Select(sel) = stmt else {
        panic!("expected Select");
    };
    assert_eq!(sel.items.len(), 1);
    assert_eq!(sel.items[0].expr, SqlExpr::Star);
    assert_eq!(sel.items[0].alias, None);
    assert_eq!(
        sel.from,
        vec![FromItem {
            table: dref("a"),
            alias: None,
            subquery: None
        }]
    );
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
            subquery: None,
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
    let stmt = one(
        "create table b as select a.x, count(*) as n from t as a group by 1 having count(*) > 1;",
    );
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
            subquery: None,
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

// ── M20.4 ──────────────────────────────────────────────────────────────

#[test]
fn create_view_parses() {
    let stmt = one("create view v as select x from t where x > 1;");
    let SqlStmt::CreateView { name, query } = stmt else {
        panic!("expected CreateView");
    };
    assert_eq!(name, dref("v"));
    assert_eq!(query.items.len(), 1);
    assert_eq!(query.items[0].expr, var("x"));
    assert!(query.where_.is_some());
}

#[test]
fn create_table_still_parses_as_table() {
    // Le discriminant table/view ne doit pas casser CREATE TABLE.
    let stmt = one("create table b as select x from t;");
    assert!(matches!(stmt, SqlStmt::CreateTableAs { .. }));
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
            subquery: None,
        }]
    );
    assert_eq!(sel.joins.len(), 1);
    assert_eq!(sel.joins[0].kind, JoinKind::Inner);
    assert_eq!(
        sel.joins[0].table,
        FromItem {
            table: dref("t2"),
            alias: Some("b".to_string()),
            subquery: None,
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
            right: Box::new(SqlExpr::Base(Expr::Missing(crate::value::MissingKind::Dot))),
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
fn drop_view_parses() {
    let stmt = one("drop view v, w;");
    let SqlStmt::DropView(refs) = stmt else {
        panic!("expected DropView");
    };
    assert_eq!(refs, vec![dref("v"), dref("w")]);
}

#[test]
fn drop_table_still_parses() {
    let stmt = one("drop table t;");
    assert!(matches!(stmt, SqlStmt::DropTable(_)));
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
    assert_eq!(
        rhs.from,
        vec![FromItem {
            table: dref("b"),
            alias: None,
            subquery: None
        }]
    );
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
fn subquery_in_from_parses() {
    // M20.4 : `FROM (SELECT ...) [AS] alias` est désormais supporté.
    let stmt = one("select * from (select x from b) as u;");
    let SqlStmt::Select(sel) = stmt else { panic!() };
    assert_eq!(sel.from.len(), 1);
    assert_eq!(sel.from[0].alias.as_deref(), Some("u"));
    let sub = sel.from[0].subquery.as_ref().expect("FROM subquery");
    assert_eq!(sub.items.len(), 1);
    assert_eq!(sub.items[0].expr, var("x"));
}

#[test]
fn subquery_in_where_parses() {
    // M20.2 : `x IN (SELECT ...)` parse en `SqlExpr::InSubquery`.
    let stmt = one("select * from a where x in (select y from b);");
    let SqlStmt::Select(sel) = stmt else { panic!() };
    let SqlExpr::InSubquery {
        expr,
        query,
        negated,
    } = sel.where_.unwrap()
    else {
        panic!("expected InSubquery");
    };
    assert_eq!(*expr, var("x"));
    assert!(!negated);
    assert_eq!(query.items[0].expr, var("y"));
    assert_eq!(
        query.from,
        vec![FromItem {
            table: dref("b"),
            alias: None,
            subquery: None
        }]
    );
}

#[test]
fn scalar_subquery_parses() {
    // M20.2 : `(SELECT ...)` en position scalaire dans le select-list.
    let stmt = one("select (select count(*) from b) as n from a;");
    let SqlStmt::Select(sel) = stmt else { panic!() };
    assert_eq!(sel.items[0].alias, Some("n".to_string()));
    let SqlExpr::Subquery(q) = &sel.items[0].expr else {
        panic!("expected Subquery, got {:?}", sel.items[0].expr);
    };
    assert_eq!(
        q.items[0].expr,
        SqlExpr::Aggregate {
            func: "COUNT".to_string(),
            distinct: false,
            arg: None,
            star: true,
        }
    );
}
