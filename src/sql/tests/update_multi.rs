use super::super::*;
use super::*;
use crate::parser::StatementStream;
use crate::source::SourceFile;
use crate::sql::parser::parse_sql_program;
use crate::testkit::*;

#[test]
fn update_basic() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql("update t set age = 0;", &mut s);
    let ds = read_work(&mut s, "T");
    assert!(ages(&ds).iter().all(|a| *a == 0.0));
    let log = s.log.into_string();
    assert!(log.contains("4 rows were updated in WORK.T."), "log: {log}");
}

#[test]
fn update_multiple_columns() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql("update t set age = 99, height = 100;", &mut s);
    let ds = read_work(&mut s, "T");
    assert!(ages(&ds).iter().all(|a| *a == 99.0));
    let heights: Vec<f64> = ds
        .df
        .column("height")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert!(heights.iter().all(|h| *h == 100.0));
}

#[test]
fn update_with_where() {
    let mut s = make_session();
    write_people(&mut s);
    run_sql("update t set age = 100 where age > 12;", &mut s);
    let ds = read_work(&mut s, "T");
    // Only Bo(14) and Cy(13) updated → two rows are 100, others unchanged.
    let a = ages(&ds);
    assert_eq!(a.iter().filter(|x| **x == 100.0).count(), 2);
    assert!(a.contains(&10.0)); // Al unchanged
    assert!(a.contains(&11.0)); // Di unchanged
    let log = s.log.into_string();
    assert!(log.contains("2 rows were updated in WORK.T."), "log: {log}");
}

#[test]
fn update_self_reference() {
    // UPDATE based on another column / the same column.
    let mut s = make_session();
    write_people(&mut s);
    run_sql("update t set age = age + 1;", &mut s);
    let ds = read_work(&mut s, "T");
    let a = ages(&ds);
    assert!(a.contains(&11.0)); // 10+1
    assert!(a.contains(&15.0)); // 14+1
}

#[test]
fn update_unknown_column() {
    let mut s = make_session();
    write_people(&mut s);
    let file = SourceFile::new("update t set nope = 1;");
    let mut ts = StatementStream::new(&file).unwrap();
    let prog = parse_sql_program(&mut ts).unwrap();
    let err = execute(&prog, &mut s).unwrap_err();
    assert!(
        err.to_string().contains("NOPE") || err.to_string().contains("could not be found"),
        "got: {err}"
    );
}

#[test]
fn update_nonexistent_table() {
    let mut s = make_session();
    let file = SourceFile::new("update ghost set x = 1;");
    let mut ts = StatementStream::new(&file).unwrap();
    let prog = parse_sql_program(&mut ts).unwrap();
    let err = execute(&prog, &mut s).unwrap_err();
    assert!(err.to_string().contains("does not exist"), "got: {err}");
}

#[test]
fn update_type_coercion() {
    let mut s = make_session();
    write_people(&mut s);
    // num ← char numeric string → parsed to number; char ← num → BEST12.
    run_sql("update t set age = '21', sex = 5;", &mut s);
    let ds = read_work(&mut s, "T");
    // age stays numeric 21 (char "21" coerced via numeric assignment path).
    let a = ages(&ds);
    assert!(a.iter().all(|x| *x == 21.0), "ages: {a:?}");
    // sex is char(1): num 5 → "5" then truncated to its length.
    let sexes: Vec<String> = ds
        .df
        .column("sex")
        .unwrap()
        .str()
        .unwrap()
        .iter()
        .map(|o| o.unwrap_or("").to_string())
        .collect();
    assert!(sexes.iter().all(|x| x == "5"), "sexes: {sexes:?}");
}

#[test]
fn update_all_rows() {
    // Explicit expectation: UPDATE without WHERE touches every row.
    let mut s = make_session();
    write_people(&mut s);
    run_sql("update t set height = 1;", &mut s);
    let ds = read_work(&mut s, "T");
    let heights: Vec<f64> = ds
        .df
        .column("height")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert_eq!(heights.len(), 4);
    assert!(heights.iter().all(|h| *h == 1.0));
    let log = s.log.into_string();
    assert!(log.contains("4 rows were updated in WORK.T."), "log: {log}");
}

#[test]
fn update_missing_values() {
    // Assigning a missing keeps it missing (no spurious conversion).
    let mut s = make_session();
    write_people(&mut s);
    run_sql("update t set age = . where age = 10;", &mut s);
    let ds = read_work(&mut s, "T");
    let col = ds.df.column("age").unwrap().f64().unwrap();
    let n_missing = col.iter().filter(|o| o.is_none()).count();
    assert_eq!(n_missing, 1, "exactly one age should now be missing");
}

#[test]
fn multi_statement_program() {
    let mut s = make_session();
    write_people(&mut s);
    // create, then select (listing), then drop — all in one program.
    run_sql(
        "create table big as select * from t where age >= 12; \
         select name from big; \
         drop table big;",
        &mut s,
    );
    // big was dropped at the end.
    assert!(!s.libs.get("WORK").unwrap().exists("BIG"));
    let listing = s.listing.take_string();
    // selected names of those with age >= 12 (Bo, Cy).
    assert!(listing.contains("Bo"), "listing: {listing}");
    assert!(listing.contains("Cy"), "listing: {listing}");
    let log = s.log.into_string();
    assert!(
        log.contains("Table WORK.BIG created, with 2 rows and"),
        "log: {log}"
    );
    assert!(
        log.contains("Table WORK.BIG has been dropped."),
        "log: {log}"
    );
}
