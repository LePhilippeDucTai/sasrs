use super::super::*;
use super::*;
use polars::df;

#[test]
fn scalar_subquery_in_select_list() {
    // `(select count(*) from t)` est constant pour chaque ligne.
    let mut s = make_session();
    write_people(&mut s);
    let out = run("select name, (select count(*) from t) as n from t;", &mut s);
    assert_eq!(out.height(), 4);
    let ns: Vec<f64> = out
        .column("n")
        .unwrap()
        .cast(&DataType::Float64)
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert!(ns.iter().all(|v| (*v - 4.0).abs() < 1e-9));
}

#[test]
fn scalar_subquery_in_where() {
    // age > avg(age) : moyenne = (10+14+13+11)/4 = 12 → garde 14 et 13.
    let mut s = make_session();
    write_people(&mut s);
    let out = run(
        "select name, age from t where age > (select avg(age) from t);",
        &mut s,
    );
    let ages: Vec<f64> = out
        .column("age")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert_eq!(ages, vec![14.0, 13.0]);
}

#[test]
fn scalar_subquery_empty_is_missing() {
    // Une sous-requête scalaire sans ligne → missing ; `age > .` est faux
    // partout → 0 ligne.
    let mut s = make_session();
    write_people(&mut s);
    let out = run(
        "select name from t where age > (select age from t where age > 100);",
        &mut s,
    );
    assert_eq!(out.height(), 0);
}

#[test]
fn in_subquery_filters() {
    // x IN (select k from keys) : matérialise {1,3}.
    let mut s = make_session();
    let t = df!["x" => [1.0_f64, 2.0, 3.0, 4.0]].unwrap();
    let keys = df!["k" => [1.0_f64, 3.0]].unwrap();
    write_table(&mut s, "T", t, vec![num("x")]);
    write_table(&mut s, "KEYS", keys, vec![num("k")]);
    let out = run("select x from t where x in (select k from keys);", &mut s);
    let xs = nums(&out, "x");
    assert_eq!(xs, vec![1.0, 3.0]);
}

#[test]
fn in_subquery_string_values() {
    // IN sur des valeurs char.
    let mut s = make_session();
    write_people(&mut s);
    let keep = df!["s" => ["F"]].unwrap();
    write_table(&mut s, "KEEP", keep, vec![chr("s", 1)]);
    let out = run(
        "select name from t where sex in (select s from keep);",
        &mut s,
    );
    // Seules Cy et Di sont F.
    assert_eq!(out.height(), 2);
    assert_eq!(sorted_strs(&out, "name"), vec!["Cy", "Di"]);
}

#[test]
fn not_in_subquery_filters() {
    let mut s = make_session();
    let t = df!["x" => [1.0_f64, 2.0, 3.0, 4.0]].unwrap();
    let keys = df!["k" => [1.0_f64, 3.0]].unwrap();
    write_table(&mut s, "T", t, vec![num("x")]);
    write_table(&mut s, "KEYS", keys, vec![num("k")]);
    let out = run(
        "select x from t where x not in (select k from keys);",
        &mut s,
    );
    assert_eq!(nums(&out, "x"), vec![2.0, 4.0]);
}

#[test]
fn not_exists_subquery_inverts() {
    // NOT EXISTS d'une sous-requête vide → vrai → conserve tout.
    let mut s = make_session();
    let t = df!["x" => [1.0_f64, 2.0]].unwrap();
    let other = df!["y" => [9.0_f64]].unwrap();
    write_table(&mut s, "T", t, vec![num("x")]);
    write_table(&mut s, "OTHER", other, vec![num("y")]);
    let out = run(
        "select x from t where not exists (select y from other where y > 100);",
        &mut s,
    );
    assert_eq!(out.height(), 2);
}

#[test]
fn exists_subquery_true_keeps_all() {
    // EXISTS non-corrélé : la sous-requête a des lignes → conserve tout.
    let mut s = make_session();
    let t = df!["x" => [1.0_f64, 2.0, 3.0]].unwrap();
    let other = df!["y" => [9.0_f64]].unwrap();
    write_table(&mut s, "T", t, vec![num("x")]);
    write_table(&mut s, "OTHER", other, vec![num("y")]);
    let out = run(
        "select x from t where exists (select y from other);",
        &mut s,
    );
    assert_eq!(out.height(), 3);
}

#[test]
fn exists_subquery_false_drops_all() {
    // EXISTS non-corrélé faux (sous-requête vide après WHERE) → 0 ligne.
    let mut s = make_session();
    let t = df!["x" => [1.0_f64, 2.0, 3.0]].unwrap();
    let other = df!["y" => [9.0_f64]].unwrap();
    write_table(&mut s, "T", t, vec![num("x")]);
    write_table(&mut s, "OTHER", other, vec![num("y")]);
    let out = run(
        "select x from t where exists (select y from other where y > 100);",
        &mut s,
    );
    assert_eq!(out.height(), 0);
}

#[test]
fn correlated_subquery_errors() {
    // Sous-requête corrélée (référence `t.age` de la requête externe) :
    // erreur documentée.
    let mut s = make_session();
    write_people(&mut s);
    let err = run_err(
        "select name from t where age > \
         (select avg(age) from u where u.sex = t.sex);",
        &mut s,
    );
    assert!(
        err.contains("correlated subqueries are not supported"),
        "got: {err}"
    );
}

#[test]
fn nested_non_correlated_subquery() {
    // Sous-requête à deux niveaux, toutes non-corrélées.
    let mut s = make_session();
    let t = df!["x" => [1.0_f64, 2.0, 3.0, 4.0]].unwrap();
    let a = df!["k" => [2.0_f64, 3.0, 4.0]].unwrap();
    let b = df!["m" => [2.0_f64, 3.0]].unwrap();
    write_table(&mut s, "T", t, vec![num("x")]);
    write_table(&mut s, "A", a, vec![num("k")]);
    write_table(&mut s, "B", b, vec![num("m")]);
    // x IN (k IN (m)) → A∩B sur la valeur = {2,3} → filtre T à {2,3}.
    let out = run(
        "select x from t where x in \
         (select k from a where k in (select m from b));",
        &mut s,
    );
    assert_eq!(nums(&out, "x"), vec![2.0, 3.0]);
}

#[test]
fn dictionary_tables_lists_datasets() {
    let mut s = make_session();
    write_people(&mut s); // WORK.T
    write_table(
        &mut s,
        "U",
        df!["a" => [1.0_f64, 2.0]].unwrap(),
        vec![num("a")],
    );
    let out = run(
        "select libname, memname, nobs, nvar from dictionary.tables \
         order by memname;",
        &mut s,
    );
    assert_eq!(strs(&out, "memname"), vec!["T", "U"]);
    assert_eq!(strs(&out, "libname"), vec!["WORK", "WORK"]);
    // T : 4 lignes / 4 variables ; U : 2 lignes / 1 variable.
    let nobs: Vec<f64> = out
        .column("nobs")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    let nvar: Vec<f64> = out
        .column("nvar")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert_eq!(nobs, vec![4.0, 2.0]);
    assert_eq!(nvar, vec![4.0, 1.0]);
}

#[test]
fn dictionary_columns_lists_variables() {
    let mut s = make_session();
    write_people(&mut s); // name char(8), sex char(1), age num, height num
    let out = run(
        "select name, type, length, varnum from dictionary.columns \
         where memname = 'T' order by varnum;",
        &mut s,
    );
    assert_eq!(strs(&out, "name"), vec!["name", "sex", "age", "height"]);
    assert_eq!(strs(&out, "type"), vec!["char", "char", "num", "num"]);
    let length: Vec<f64> = out
        .column("length")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert_eq!(length, vec![8.0, 1.0, 8.0, 8.0]);
    let varnum: Vec<f64> = out
        .column("varnum")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert_eq!(varnum, vec![1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn dictionary_macros_lists_globals() {
    let mut s = make_session();
    s.macro_engine
        .set_symbol_global("MYVAR", "hello".to_string());
    let out = run(
        "select scope, name, value from dictionary.macros \
         where name = 'MYVAR';",
        &mut s,
    );
    assert_eq!(out.height(), 1);
    assert_eq!(strs(&out, "scope"), vec!["GLOBAL"]);
    assert_eq!(strs(&out, "name"), vec!["MYVAR"]);
    assert_eq!(strs(&out, "value"), vec!["hello"]);
}

#[test]
fn dictionary_macros_automatic_scope() {
    let mut s = make_session();
    // Variables automatiques amorcées (SYSVER, etc.) → scope AUTOMATIC.
    let out = run(
        "select scope, name from dictionary.macros where name = 'SYSVER';",
        &mut s,
    );
    assert_eq!(out.height(), 1);
    assert_eq!(strs(&out, "scope"), vec!["AUTOMATIC"]);
}

#[test]
fn dictionary_where_filter() {
    let mut s = make_session();
    write_people(&mut s); // T : age 10..14
    let out = run(
        "select name, type from dictionary.columns \
         where memname = 'T' and type = 'num' order by name;",
        &mut s,
    );
    // Seules age et height sont numériques.
    assert_eq!(strs(&out, "name"), vec!["age", "height"]);
}

#[test]
fn dictionary_columns_column_order() {
    let mut s = make_session();
    write_people(&mut s);
    // SELECT * doit respecter l'ordre canonique des colonnes dictionary.
    let out = run("select * from dictionary.columns;", &mut s);
    let names: Vec<&str> = out.get_column_names().iter().map(|n| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "libname", "memname", "name", "type", "length", "npos", "varnum", "label", "format",
            "informat",
        ]
    );
}

#[test]
fn sashelp_vcolumn_alias() {
    let mut s = make_session();
    write_people(&mut s);
    // sashelp.vcolumn doit produire exactement les mêmes données que
    // DICTIONARY.COLUMNS.
    let a = run(
        "select name, type from sashelp.vcolumn where memname = 'T' \
         order by varnum;",
        &mut s,
    );
    let b = run(
        "select name, type from dictionary.columns where memname = 'T' \
         order by varnum;",
        &mut s,
    );
    assert_eq!(strs(&a, "name"), strs(&b, "name"));
    assert_eq!(strs(&a, "type"), strs(&b, "type"));
    assert_eq!(strs(&a, "name"), vec!["name", "sex", "age", "height"]);
}

#[test]
fn sashelp_vtable_alias() {
    let mut s = make_session();
    write_people(&mut s);
    let out = run(
        "select memname from sashelp.vtable order by memname;",
        &mut s,
    );
    assert_eq!(strs(&out, "memname"), vec!["T"]);
}
