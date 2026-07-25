use super::super::*;
use super::*;
use crate::parser::StatementStream;
use crate::source::SourceFile;
use crate::sql::parser::parse_sql_program;
use crate::value::VarType;
use polars::df;

#[test]
fn create_table_as_select_writes_and_notes() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql(
        "create table summary as select sex, count(*) as n from t group by sex;",
        &mut s,
    );
    assert!(s.libs.get("WORK").unwrap().exists("SUMMARY"));
    let ds = read_work(&mut s, "SUMMARY");
    assert_eq!(ds.n_obs(), 2);
    assert_eq!(ds.n_vars(), 2);
    // count(*) came back as u32 → must be coerced to f64 num.
    assert!(ds
        .vars
        .iter()
        .all(|v| matches!(v.ty, VarType::Num | VarType::Char)));
    let n_col = ds.df.column("n").unwrap();
    assert_eq!(n_col.dtype(), &DataType::Float64);
    let log = s.log.into_string();
    assert!(
        log.contains("Table WORK.SUMMARY created, with 2 rows and 2 columns."),
        "log: {log}"
    );
    assert_eq!(s.last_dataset.as_deref(), Some("WORK.SUMMARY"));
}

// ── M20.4 : CREATE VIEW ────────────────────────────────────────────────

#[test]
fn create_view_basic() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql(
        "create view v as select name, age from t; select * from v;",
        &mut s,
    );
    // The view is purely in memory (no parquet written).
    assert!(!s.libs.get("WORK").unwrap().exists("V"));
    assert!(s.views.contains_key("V"));
    let listing = s.listing.into_string();
    assert!(listing.contains("Al"), "listing: {listing}");
    assert!(listing.contains("14"), "listing: {listing}");
    let log = s.log.into_string();
    assert!(
        log.contains("SQL view WORK.V has been defined."),
        "log: {log}"
    );
}

#[test]
fn create_view_overwrites() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql("create view v as select name from t;", &mut s);
    run_sql("create view v as select age from t;", &mut s);
    // Redeclared: the second definition wins (age, not name).
    let q = s.views.get("V").unwrap();
    assert_eq!(
        q.items[0].expr,
        crate::sql::ast::SqlExpr::Base(Expr::Var("age".to_string()))
    );
    let log = s.log.into_string();
    assert!(log.contains("redefined"), "log: {log}");
}

#[test]
fn bare_select_renders_to_listing() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql("select name, age from t where age > 12;", &mut s);
    let listing = s.listing.into_string();
    assert!(listing.contains("Bo"), "listing: {listing}");
    assert!(listing.contains("Cy"), "listing: {listing}");
    assert!(listing.contains("14"), "listing: {listing}");
    // No Obs column in SQL SELECT output.
    assert!(!listing.contains("Obs"), "listing: {listing}");
    // Bare SELECT must not set _LAST_.
    assert!(s.last_dataset.is_none());
}

#[test]
fn insert_values_grows_row_count() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql(
        "insert into t (name, sex, age, height) values ('Ed', 'M', 9, 48);",
        &mut s,
    );
    let ds = read_work(&mut s, "T");
    assert_eq!(ds.n_obs(), 5);
    let names: Vec<String> = ds
        .df
        .column("name")
        .unwrap()
        .str()
        .unwrap()
        .iter()
        .map(|o| o.unwrap_or("").to_string())
        .collect();
    assert!(names.contains(&"Ed".to_string()));
    let ages: Vec<f64> = ds
        .df
        .column("age")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert!(ages.contains(&9.0));
    let log = s.log.into_string();
    assert!(
        log.contains("1 rows were inserted into WORK.T."),
        "log: {log}"
    );
}

#[test]
fn insert_values_positional() {
    let mut s = make_session();
    let df = df!["x" => [1.0_f64, 2.0]].unwrap();
    write_table(&mut s, "T", df, vec![num("x")]);
    run_sql("insert into t values (3) values (4);", &mut s);
    let ds = read_work(&mut s, "T");
    assert_eq!(ds.n_obs(), 4);
}

#[test]
fn insert_select_appends() {
    let mut s = make_session();
    let a = df!["x" => [1.0_f64, 2.0]].unwrap();
    let b = df!["x" => [10.0_f64, 20.0, 30.0]].unwrap();
    write_table(&mut s, "A", a, vec![num("x")]);
    write_table(&mut s, "B", b, vec![num("x")]);
    run_sql("insert into a select x from b;", &mut s);
    let ds = read_work(&mut s, "A");
    assert_eq!(ds.n_obs(), 5);
}

// ── M20.4 : subqueries in INSERT ────────────────────────────────────────

#[test]
fn insert_select_with_scalar_subquery() {
    let mut s = make_session();
    write_people(&mut s);
    // target table: two num columns.
    let tgt = df!["n" => [0.0_f64], "a" => [0.0_f64]].unwrap();
    write_table(&mut s, "DEST", tgt, vec![num("n"), num("a")]);
    run_sql(
        "insert into dest select (select count(*) from t) as n, age as a from t where age = 10;",
        &mut s,
    );
    let ds = read_work(&mut s, "DEST");
    // started with 1 row, inserted 1 (Al, age 10) → 2 rows.
    assert_eq!(ds.n_obs(), 2);
    let ns: Vec<f64> = ds
        .df
        .column("n")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    // the scalar subquery COUNT(*) over t = 4.
    assert!(ns.contains(&4.0), "ns: {ns:?}");
}

#[test]
fn insert_select_with_in_subquery() {
    let mut s = make_session();
    write_people(&mut s);
    let dest = df!["name" => ["zz"], "sex" => ["M"], "age" => [0.0_f64], "height" => [0.0_f64]]
        .unwrap();
    write_table(
        &mut s,
        "DEST",
        dest,
        vec![chr("name", 8), chr("sex", 1), num("age"), num("height")],
    );
    run_sql(
        "insert into dest select * from t where age in (select age from t where sex = 'F');",
        &mut s,
    );
    let ds = read_work(&mut s, "DEST");
    // F rows of t: Cy(13), Di(11) → two inserted, plus the seed row = 3.
    assert_eq!(ds.n_obs(), 3);
}

#[test]
fn insert_select_with_exists() {
    let mut s = make_session();
    write_people(&mut s);
    let dest = df!["name" => ["zz"], "sex" => ["M"], "age" => [0.0_f64], "height" => [0.0_f64]]
        .unwrap();
    write_table(
        &mut s,
        "DEST",
        dest,
        vec![chr("name", 8), chr("sex", 1), num("age"), num("height")],
    );
    // EXISTS true → all rows of t pass; non-correlated.
    run_sql(
        "insert into dest select * from t where exists (select * from t where sex = 'F');",
        &mut s,
    );
    let ds = read_work(&mut s, "DEST");
    // all 4 rows of t inserted + seed = 5.
    assert_eq!(ds.n_obs(), 5);
}

#[test]
fn insert_select_with_union_subquery() {
    let mut s = make_session();
    let a = df!["x" => [1.0_f64, 2.0]].unwrap();
    let b = df!["x" => [2.0_f64, 3.0]].unwrap();
    write_table(&mut s, "A", a, vec![num("x")]);
    write_table(&mut s, "B", b, vec![num("x")]);
    let dest = df!["x" => [0.0_f64]].unwrap();
    write_table(&mut s, "DEST", dest, vec![num("x")]);
    run_sql(
        "insert into dest select * from (select x from a union select x from b) as u;",
        &mut s,
    );
    let ds = read_work(&mut s, "DEST");
    // union distinct of {1,2} and {2,3} = {1,2,3} → 3 rows + seed = 4.
    assert_eq!(ds.n_obs(), 4);
}

#[test]
fn delete_with_where_removes_rows() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql("delete from t where age > 12;", &mut s);
    let ds = read_work(&mut s, "T");
    assert_eq!(ds.n_obs(), 2);
    let ages: Vec<f64> = ds
        .df
        .column("age")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert!(ages.iter().all(|a| *a <= 12.0));
    let log = s.log.into_string();
    assert!(
        log.contains("2 rows were deleted from WORK.T."),
        "log: {log}"
    );
}

#[test]
fn delete_all_rows() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql("delete from t;", &mut s);
    let ds = read_work(&mut s, "T");
    assert_eq!(ds.n_obs(), 0);
    assert_eq!(ds.n_vars(), 4);
}

#[test]
fn drop_table_removes_it() {
    let mut s = make_session();
    write_people(&mut s);
    assert!(s.libs.get("WORK").unwrap().exists("T"));
    run_sql("drop table t;", &mut s);
    assert!(!s.libs.get("WORK").unwrap().exists("T"));
    let log = s.log.into_string();
    assert!(
        log.contains("Table WORK.T has been dropped."),
        "log: {log}"
    );
}

#[test]
fn drop_missing_table_errors_in_log() {
    let mut s = make_session();
    run_sql("drop table nope;", &mut s);
    let log = s.log.into_string();
    assert!(
        log.contains("Table WORK.NOPE does not exist."),
        "log: {log}"
    );
}

#[test]
fn drop_view() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql("create view v as select name from t;", &mut s);
    assert!(s.views.contains_key("V"));
    run_sql("drop view v;", &mut s);
    assert!(!s.views.contains_key("V"));
    let log = s.log.into_string();
    assert!(
        log.contains("View WORK.V has been dropped."),
        "log: {log}"
    );
}

#[test]
fn drop_view_via_drop_table() {
    // DROP TABLE on a view name removes the view (shared logic).
    let mut s = make_session();
    write_people(&mut s);
    run_sql("create view v as select name from t;", &mut s);
    run_sql("drop table v;", &mut s);
    assert!(!s.views.contains_key("V"));
}

#[test]
fn describe_writes_table_definition_to_log() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql("describe table t;", &mut s);
    let log = s.log.into_string();
    assert!(log.contains("WORK.T"), "log: {log}");
    assert!(log.contains("create table"), "log: {log}");
    // char column should show its declared length.
    assert!(log.contains("char("), "log: {log}");
}

#[test]
fn view_from_qualified_table() {
    // The view's own FROM uses an explicit `work.t` qualifier.
    let mut s = make_session();
    write_people(&mut s);
    run_sql(
        "create view v as select name, age from work.t where age >= 13; \
         select name from v;",
        &mut s,
    );
    let listing = s.listing.into_string();
    assert!(listing.contains("Bo"), "listing: {listing}");
    assert!(listing.contains("Cy"), "listing: {listing}");
    assert!(!listing.contains("Al"), "listing: {listing}");
}

#[test]
fn view_reference_missing() {
    // The view body references a nonexistent table → error when used.
    let mut s = make_session();
    run_sql("create view v as select x from nonexistent;", &mut s);
    // The CREATE itself is lazy (no error yet).
    let file = SourceFile::new("select * from v;");
    let mut ts = StatementStream::new(&file).unwrap();
    let prog = parse_sql_program(&mut ts).unwrap();
    let err = execute(&prog, &mut s).unwrap_err();
    assert!(!err.to_string().is_empty(), "expected an error");
}

#[test]
fn view_select_from_missing() {
    // Selecting from a view that was never defined → clean error
    // (treated as a missing table).
    let mut s = make_session();
    let file = SourceFile::new("select * from ghost;");
    let mut ts = StatementStream::new(&file).unwrap();
    let prog = parse_sql_program(&mut ts).unwrap();
    let err = execute(&prog, &mut s).unwrap_err();
    assert!(!err.to_string().is_empty(), "expected an error");
}

#[test]
fn view_referencing_view() {
    // Nested views: v2 selects from v1.
    let mut s = make_session();
    write_people(&mut s);
    run_sql(
        "create view v1 as select name, age from t where age >= 12; \
         create view v2 as select name from v1; \
         select name from v2;",
        &mut s,
    );
    let listing = s.listing.into_string();
    assert!(listing.contains("Bo"), "listing: {listing}");
    assert!(listing.contains("Cy"), "listing: {listing}");
    assert!(!listing.contains("Al"), "listing: {listing}");
}
