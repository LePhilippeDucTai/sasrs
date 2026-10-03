//! J01-P3 (issue #15) : WHERE equi en jointure comma-FROM et remerge
//! GROUP BY (colonne source non agrégée résolue contre la source).

use super::*;
use crate::testkit::*;
use polars::df;

/// Valeurs numeriques d'une colonne DANS L'ORDRE des lignes (le helper
/// partage `nums` trie — or l'ordre SAS compte ici : jointure et GROUP BY).
fn ordered_nums(df: &DataFrame, col: &str) -> Vec<f64> {
    df.column(col)
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect()
}

fn write_customers_orders(session: &mut Session) {
    let customers = df![
        "id"   => [1.0_f64, 2.0, 3.0],
        "name" => ["Alice", "Bob", "Carol"],
    ]
    .unwrap();
    write_table(
        session,
        "CUSTOMERS",
        customers,
        vec![num("id"), chr("name", 8)],
    );
    let orders = df![
        "id"     => [1.0_f64, 3.0, 3.0],
        "amount" => [30.0_f64, 10.0, 20.0],
    ]
    .unwrap();
    write_table(session, "ORDERS", orders, vec![num("id"), num("amount")]);
}

fn write_sales(session: &mut Session) {
    let sales = df![
        "region" => ["North", "South", "North"],
        "amount" => [10.0_f64, 30.0, 40.0],
    ]
    .unwrap();
    write_table(
        session,
        "SALES",
        sales,
        vec![chr("region", 8), num("amount")],
    );
}

/// `from a, b where a.k = b.k` = jointure interne (anti-produit
/// cartésien), pas un cross join filtré.
#[test]
fn comma_from_where_equi_is_inner_join() {
    let mut s = make_session();
    write_customers_orders(&mut s);
    let out = run(
        "select c.name, o.amount from customers as c, orders as o where c.id = o.id;",
        &mut s,
    );
    // 3 appariements (Bob n'a pas de commande) — PAS 9 lignes.
    assert_eq!(out.height(), 3);
    assert_eq!(
        strs(&out, "name"),
        vec![
            "Alice".to_string(),
            "Carol".to_string(),
            "Carol".to_string()
        ]
    );
    assert_eq!(ordered_nums(&out, "amount"), vec![30.0, 10.0, 20.0]);
}

/// Conjoint non-equi du WHERE : reste un filtre après jointure.
#[test]
fn comma_from_where_equi_plus_residual_filter() {
    let mut s = make_session();
    write_customers_orders(&mut s);
    let out = run(
        "select c.name, o.amount from customers as c, orders as o
         where c.id = o.id and o.amount > 15;",
        &mut s,
    );
    assert_eq!(out.height(), 2);
    assert_eq!(
        strs(&out, "name"),
        vec!["Alice".to_string(), "Carol".to_string()]
    );
    assert_eq!(ordered_nums(&out, "amount"), vec![30.0, 20.0]);
}

/// Issue #15 : colonne source non agrégée dans un GROUP BY → remerge,
/// l'agrégat par groupe est remis sur chaque ligne (ratio à la somme du
/// groupe), groupes triés par clé, NOTE SAS émise.
#[test]
fn group_by_remerge_bare_source_column() {
    let mut s = make_session();
    write_sales(&mut s);
    let out = run(
        "select region, amount, amount / sum(amount) as share from sales group by region;",
        &mut s,
    );
    assert_eq!(
        strs(&out, "region"),
        vec![
            "North".to_string(),
            "North".to_string(),
            "South".to_string()
        ]
    );
    assert_eq!(ordered_nums(&out, "amount"), vec![10.0, 40.0, 30.0]);
    let shares = ordered_nums(&out, "share");
    assert!((shares[0] - 0.2).abs() < 1e-9);
    assert!((shares[1] - 0.8).abs() < 1e-9);
    assert!((shares[2] - 1.0).abs() < 1e-9);
    let log = s.log.into_string();
    assert!(
        log.contains(
            "The query requires remerging summary statistics back with the original data."
        ),
        "log: {log}"
    );
}

/// GROUP BY sans remerge (clés + agrégats seuls) : PAS de NOTE.
#[test]
fn group_by_no_remerge_no_note() {
    let mut s = make_session();
    write_sales(&mut s);
    let out = run(
        "select region, sum(amount) as total from sales group by region;",
        &mut s,
    );
    assert_eq!(
        strs(&out, "region"),
        vec!["North".to_string(), "South".to_string()]
    );
    assert_eq!(ordered_nums(&out, "total"), vec![50.0, 30.0]);
    let log = s.log.into_string();
    assert!(!log.contains("remerging summary statistics"), "log: {log}");
}
