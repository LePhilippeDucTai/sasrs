use super::*;
use crate::missing::encode_special;
use crate::testkit::*;
use crate::value::MissingKind;
use polars::df;

#[test]
fn where_filter_numeric() {
    let mut s = make_session();
    write_people(&mut s);
    let df = run("select name, age from t where age > 12;", &mut s);
    assert_eq!(df.height(), 2);
    let ages: Vec<f64> = df
        .column("age")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert_eq!(ages, vec![14.0, 13.0]);
}

#[test]
fn where_equals_missing_is_null() {
    let mut s = make_session();
    let df = df![
        "x" => [Some(1.0_f64), None, Some(3.0)],
        "y" => [10.0_f64, 20.0, 30.0],
    ]
    .unwrap();
    write_table(&mut s, "T", df, vec![num("x"), num("y")]);
    let out = run("select y from t where x = .;", &mut s);
    assert_eq!(out.height(), 1);
    let ys: Vec<f64> = out
        .column("y")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert_eq!(ys, vec![20.0]);
}

#[test]
fn where_special_missing_normalized_to_null() {
    // Une colonne contient un missing spécial (.A) : `x = .` doit le
    // capturer (normalisation NaN-payload → null avant comparaison).
    let mut s = make_session();
    let df = df![
        "x" => [Some(1.0_f64), Some(encode_special(MissingKind::Letter(0))), Some(3.0)],
        "y" => [10.0_f64, 20.0, 30.0],
    ]
    .unwrap();
    write_table(&mut s, "T", df, vec![num("x"), num("y")]);
    let out = run("select y from t where x = .;", &mut s);
    let ys: Vec<f64> = out
        .column("y")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert_eq!(ys, vec![20.0]);
}

#[test]
fn group_by_aggregates() {
    let mut s = make_session();
    write_people(&mut s);
    let out = run(
        "select sex, count(*) as n, avg(height) as a from t group by sex;",
        &mut s,
    );
    assert_eq!(out.height(), 2);
    // Vérifie les valeurs par sexe.
    let sexes: Vec<String> = out
        .column("sex")
        .unwrap()
        .str()
        .unwrap()
        .iter()
        .map(|o| o.unwrap().to_string())
        .collect();
    let ns: Vec<u32> = out
        .column("n")
        .unwrap()
        .u32()
        .unwrap()
        .into_no_null_iter()
        .collect();
    // Chaque groupe a 2 lignes.
    for (i, sx) in sexes.iter().enumerate() {
        assert_eq!(ns[i], 2, "sex {sx}");
    }
    // avg(height) : F = (55+52)/2 = 53.5 ; M = (50+60)/2 = 55.
    let avgs: Vec<f64> = out
        .column("a")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    for (i, sx) in sexes.iter().enumerate() {
        if sx == "F" {
            assert!((avgs[i] - 53.5).abs() < 1e-9);
        } else {
            assert!((avgs[i] - 55.0).abs() < 1e-9);
        }
    }
}

#[test]
fn remerge_grand_total_and_note() {
    let mut s = make_session();
    write_people(&mut s);
    let out = run("select name, max(height) as mx from t;", &mut s);
    // Une ligne par observation d'origine, mx constant = 60.
    assert_eq!(out.height(), 4);
    let mxs: Vec<f64> = out
        .column("mx")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert!(mxs.iter().all(|v| (*v - 60.0).abs() < 1e-9));
    let log = s.log.into_string();
    assert!(
        log.contains(
            "The query requires remerging summary statistics back with the original data."
        ),
        "log: {log}"
    );
}

#[test]
fn order_by_missing_first() {
    let mut s = make_session();
    let df = df![
        "x" => [Some(3.0_f64), None, Some(1.0), Some(2.0)],
    ]
    .unwrap();
    write_table(&mut s, "T", df, vec![num("x")]);
    let out = run("select x from t order by x;", &mut s);
    let col = out.column("x").unwrap().f64().unwrap();
    // null en premier, puis 1, 2, 3.
    assert_eq!(col.get(0), None);
    assert_eq!(col.get(1), Some(1.0));
    assert_eq!(col.get(2), Some(2.0));
    assert_eq!(col.get(3), Some(3.0));
}

#[test]
fn order_by_descending() {
    let mut s = make_session();
    write_people(&mut s);
    let out = run("select age from t order by age desc;", &mut s);
    let ages: Vec<f64> = out
        .column("age")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert_eq!(ages, vec![14.0, 13.0, 11.0, 10.0]);
}

#[test]
fn join_with_missing_key_matches() {
    // join_nulls(true) : les clés missing s'apparient.
    let mut s = make_session();
    let left = df![
        "k" => [Some(1.0_f64), None, Some(2.0)],
        "a" => [10.0_f64, 20.0, 30.0],
    ]
    .unwrap();
    write_table(&mut s, "L", left, vec![num("k"), num("a")]);
    let right = df![
        "k" => [Some(1.0_f64), None],
        "b" => [100.0_f64, 200.0],
    ]
    .unwrap();
    write_table(&mut s, "R", right, vec![num("k"), num("b")]);
    let out = run("select l.a, r.b from l inner join r on l.k = r.k;", &mut s);
    // k=1 (a=10,b=100) et k=null (a=20,b=200) → 2 lignes.
    assert_eq!(out.height(), 2);
    let bs: Vec<f64> = out
        .column("b")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert!(bs.contains(&100.0) && bs.contains(&200.0));
}

#[test]
fn distinct_dedups_rows() {
    let mut s = make_session();
    let df = df![
        "x" => [1.0_f64, 1.0, 2.0, 2.0, 2.0],
    ]
    .unwrap();
    write_table(&mut s, "T", df, vec![num("x")]);
    let out = run("select distinct x from t;", &mut s);
    assert_eq!(out.height(), 2);
}

#[test]
fn select_star() {
    let mut s = make_session();
    write_people(&mut s);
    let out = run("select * from t;", &mut s);
    assert_eq!(out.width(), 4);
    assert_eq!(out.height(), 4);
}

#[test]
fn calculated_reexpands_alias() {
    let mut s = make_session();
    write_people(&mut s);
    // bmi-like : alias `dbl` = age*2, puis CALCULATED dbl + 1.
    let out = run(
        "select age*2 as dbl, calculated dbl + 1 as plus from t order by age;",
        &mut s,
    );
    let dbl: Vec<f64> = out
        .column("dbl")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    let plus: Vec<f64> = out
        .column("plus")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    for (d, p) in dbl.iter().zip(plus.iter()) {
        assert!((p - (d + 1.0)).abs() < 1e-9);
    }
}

#[test]
fn union_all_and_distinct() {
    let mut s = make_session();
    let a = df!["x" => [1.0_f64, 2.0]].unwrap();
    let b = df!["x" => [2.0_f64, 3.0]].unwrap();
    write_table(&mut s, "A", a, vec![num("x")]);
    write_table(&mut s, "B", b, vec![num("x")]);
    let all = run("select x from a union all select x from b;", &mut s);
    assert_eq!(all.height(), 4);
    let uniq = run("select x from a union select x from b;", &mut s);
    assert_eq!(uniq.height(), 3);
}

#[test]
fn like_pattern_match() {
    let mut s = make_session();
    let df = df![
        "name" => ["Alice", "Bob", "Albert", "Carol"],
    ]
    .unwrap();
    write_table(&mut s, "T", df, vec![chr("name", 8)]);
    let out = run("select name from t where name like 'Al%';", &mut s);
    let names: Vec<String> = out
        .column("name")
        .unwrap()
        .str()
        .unwrap()
        .iter()
        .map(|o| o.unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["Alice".to_string(), "Albert".to_string()]);
}

#[test]
fn like_prefix_percent() {
    let mut s = make_session();
    write_words(&mut s);
    let out = run("select w from w where w like 'ab%';", &mut s);
    // ab* : abc, abx, abcd (pas ABC — sensible à la casse).
    assert_eq!(sorted_strs(&out, "w"), vec!["abc", "abcd", "abx"]);
}

#[test]
fn like_suffix_percent() {
    let mut s = make_session();
    write_words(&mut s);
    let out = run("select w from w where w like '%bc';", &mut s);
    // *bc : abc, xbc (a_c ne finit pas par bc).
    assert_eq!(sorted_strs(&out, "w"), vec!["abc", "xbc"]);
}

#[test]
fn like_contains_percent() {
    let mut s = make_session();
    write_words(&mut s);
    let out = run("select w from w where w like '%b%';", &mut s);
    // contient b : abc, abx, xbc, abcd (pas axc, ABC, a_c).
    assert_eq!(sorted_strs(&out, "w"), vec!["abc", "abcd", "abx", "xbc"]);
}

#[test]
fn like_underscore_single_char() {
    let mut s = make_session();
    write_words(&mut s);
    // 'a_c' : a, un caractère QUELCONQUE, c → abc, axc, a_c (3 lettres).
    // Pas abcd (4 lettres), pas xbc, pas ABC.
    let out = run("select w from w where w like 'a_c';", &mut s);
    assert_eq!(sorted_strs(&out, "w"), vec!["a_c", "abc", "axc"]);
}

#[test]
fn like_underscore_is_literal_one_char() {
    // Vérifie que `_` matche un seul caractère, pas zéro ni plusieurs.
    let mut s = make_session();
    let df = df!["w" => ["ac", "abc", "abbc"]].unwrap();
    write_table(&mut s, "W", df, vec![chr("w", 8)]);
    let out = run("select w from w where w like 'a_c';", &mut s);
    // Seul "abc" (a + 1 char + c).
    assert_eq!(sorted_strs(&out, "w"), vec!["abc"]);
}

#[test]
fn like_exact_no_wildcard() {
    let mut s = make_session();
    write_words(&mut s);
    let out = run("select w from w where w like 'abc';", &mut s);
    assert_eq!(sorted_strs(&out, "w"), vec!["abc"]);
}

#[test]
fn like_internal_percent_and_underscore() {
    // Motif mixte : 'a%c_' avec un `%` interne et un `_` final.
    let mut s = make_session();
    let df = df!["w" => ["abcd", "ac1", "abxcZ", "abc", "axxxcc"]].unwrap();
    write_table(&mut s, "W", df, vec![chr("w", 8)]);
    let out = run("select w from w where w like 'a%c_';", &mut s);
    // a, n'importe quoi, c, puis exactement 1 char :
    //   abcd (a|b|c|d ✓), ac1 (a||c|1 ✓), abxcZ (a|bx|c|Z ✓),
    //   axxxcc (a|xxx|c|c ✓). Pas abc (rien après c).
    assert_eq!(
        sorted_strs(&out, "w"),
        vec!["abcd", "abxcZ", "ac1", "axxxcc"]
    );
}

#[test]
fn like_case_sensitive() {
    // SAS LIKE est sensible à la casse (pas d'upcase implicite).
    let mut s = make_session();
    write_words(&mut s);
    let out = run("select w from w where w like 'ABC';", &mut s);
    assert_eq!(sorted_strs(&out, "w"), vec!["ABC"]);
}

#[test]
fn like_missing_never_matches() {
    let mut s = make_session();
    let df = df!["w" => [Some("abc"), None, Some("axc")]].unwrap();
    write_table(&mut s, "W", df, vec![chr("w", 8)]);
    // 'a%c' matche abc et axc ; le null ne matche jamais.
    let out = run("select w from w where w like 'a%c';", &mut s);
    assert_eq!(out.height(), 2);
    // Même un motif "tout" (%) exclut les missing.
    let out2 = run("select w from w where w like '%';", &mut s);
    assert_eq!(out2.height(), 2);
}

#[test]
fn like_compared_with_equals() {
    // LIKE 'abc' (sans joker) ≡ = 'abc'.
    let mut s = make_session();
    write_words(&mut s);
    let like = run("select w from w where w like 'abc';", &mut s);
    let eq = run("select w from w where w = 'abc';", &mut s);
    assert_eq!(sorted_strs(&like, "w"), sorted_strs(&eq, "w"));
}

// ---- M42.2 : CONTAINS / SOUNDS LIKE -----------------------------------

#[test]
fn contains_predicate_matches_substring() {
    let mut s = make_session();
    write_words(&mut s);
    let out = run("select w from w where w contains 'b';", &mut s);
    // Contient 'b' (sensible à la casse, comme INDEX) : abc, abx, xbc, abcd.
    // Pas axc, a_c (pas de 'b'), pas ABC ('B' majuscule ≠ 'b').
    assert_eq!(sorted_strs(&out, "w"), vec!["abc", "abcd", "abx", "xbc"]);
}

#[test]
fn not_contains_predicate_excludes_matches() {
    let mut s = make_session();
    write_words(&mut s);
    let out = run("select w from w where w not contains 'b';", &mut s);
    assert_eq!(sorted_strs(&out, "w"), vec!["ABC", "a_c", "axc"]);
}

#[test]
fn contains_empty_pattern_never_matches() {
    // Oracle/SAS : CONTAINS '' ≡ INDEX(expr, '') > 0 ≡ 0 > 0 ≡ faux (INDEX
    // ne trouve jamais une sous-chaîne vide).
    let mut s = make_session();
    write_words(&mut s);
    let out = run("select w from w where w contains '';", &mut s);
    assert_eq!(out.height(), 0);
}

#[test]
fn contains_missing_never_matches() {
    let mut s = make_session();
    let df = df!["w" => [Some("abc"), None, Some("axc")]].unwrap();
    write_table(&mut s, "W", df, vec![chr("w", 8)]);
    let out = run("select w from w where w contains 'a';", &mut s);
    assert_eq!(out.height(), 2);
}

#[test]
fn sounds_like_matches_soundex_code() {
    let mut s = make_session();
    let df = df!["name" => ["Robert", "Rupert", "Rubin", "Ashcraft"]].unwrap();
    write_table(&mut s, "T", df, vec![chr("name", 12)]);
    let out = run(
        "select name from t where name sounds like 'Robert';",
        &mut s,
    );
    // "Robert" et "Rupert" partagent le code Soundex R163 ; "Rubin" (R150)
    // et "Ashcraft" (A261) n'y matchent pas.
    assert_eq!(sorted_strs(&out, "name"), vec!["Robert", "Rupert"]);
}

#[test]
fn not_sounds_like_excludes_matches() {
    let mut s = make_session();
    let df = df!["name" => ["Robert", "Rupert", "Rubin", "Ashcraft"]].unwrap();
    write_table(&mut s, "T", df, vec![chr("name", 12)]);
    let out = run(
        "select name from t where name not sounds like 'Robert';",
        &mut s,
    );
    assert_eq!(sorted_strs(&out, "name"), vec!["Ashcraft", "Rubin"]);
}

#[test]
fn soundex_worked_examples() {
    // Exemples de référence standard (Knuth / Soundex classique) — voir la
    // doc-comment de `soundex` dans `sql/plan/expr.rs`.
    assert_eq!(soundex("Robert"), "R163");
    assert_eq!(soundex("Rupert"), "R163");
    assert_eq!(soundex("Ashcraft"), "A261");
    assert_eq!(soundex("Tymczak"), "T522");
    assert_eq!(soundex("Pfister"), "P123");
    // Entrée vide (ou sans aucune lettre) → "0000" (cas limite documenté).
    assert_eq!(soundex(""), "0000");
    assert_eq!(soundex("123"), "0000");
}

#[test]
fn having_filters_groups() {
    let mut s = make_session();
    write_people(&mut s);
    let out = run(
        "select sex, count(*) as n from t group by sex having count(*) > 1;",
        &mut s,
    );
    // Les deux groupes ont 2 → tous passent.
    assert_eq!(out.height(), 2);
}

#[test]
fn between_filter() {
    let mut s = make_session();
    write_people(&mut s);
    let out = run("select name from t where age between 11 and 13;", &mut s);
    assert_eq!(out.height(), 2);
}

#[test]
fn except_distinct() {
    let mut s = make_session();
    write_multi(&mut s);
    // EXCEPT (DISTINCT) : valeurs de A absentes de B, dédupliquées → {2}.
    let out = run("select x from a except select x from b;", &mut s);
    assert_eq!(nums(&out, "x"), vec![2.0]);
}

#[test]
fn except_all_keeps_multiplicity() {
    let mut s = make_session();
    write_multi(&mut s);
    // EXCEPT ALL : max(0, nA - nB) copies.
    //   1 : 3-1 = 2 copies ; 2 : 1-0 = 1 ; 3 : 2-1 = 1 ; 4 : absent de A.
    let out = run("select x from a except all select x from b;", &mut s);
    assert_eq!(nums(&out, "x"), vec![1.0, 1.0, 2.0, 3.0]);
}

#[test]
fn except_all_missing_values() {
    // Les missing (null) participent comme une valeur ordinaire (`. = .`).
    let mut s = make_session();
    let a = df!["x" => [Some(1.0_f64), None, None, Some(2.0)]].unwrap();
    let b = df!["x" => [None, Some(2.0)]].unwrap();
    write_table(&mut s, "A", a, vec![num("x")]);
    write_table(&mut s, "B", b, vec![num("x")]);
    // EXCEPT ALL : null 2-1=1 copie ; 1 : 1-0=1 ; 2 : 1-1=0.
    let out = run("select x from a except all select x from b;", &mut s);
    let col = out.column("x").unwrap().f64().unwrap();
    assert_eq!(out.height(), 2);
    let n_null = col.iter().filter(|o| o.is_none()).count();
    let vals: Vec<f64> = col.iter().flatten().collect();
    assert_eq!(n_null, 1);
    assert_eq!(vals, vec![1.0]);
}

#[test]
fn intersect_distinct() {
    let mut s = make_session();
    write_multi(&mut s);
    // INTERSECT (DISTINCT) : valeurs communes dédupliquées → {1, 3}.
    let out = run("select x from a intersect select x from b;", &mut s);
    assert_eq!(nums(&out, "x"), vec![1.0, 3.0]);
}

#[test]
fn intersect_all_keeps_multiplicity() {
    let mut s = make_session();
    write_multi(&mut s);
    // INTERSECT ALL : min(nA, nB) copies.
    //   1 : min(3,1) = 1 ; 3 : min(2,1) = 1 ; 2 et 4 : absents d'un côté.
    let out = run("select x from a intersect all select x from b;", &mut s);
    assert_eq!(nums(&out, "x"), vec![1.0, 3.0]);
}

#[test]
fn intersect_all_both_sides_duplicated() {
    // Cas où les deux côtés ont plusieurs copies : min(2,3)=2.
    let mut s = make_session();
    let a = df!["x" => [5.0_f64, 5.0, 6.0]].unwrap();
    let b = df!["x" => [5.0_f64, 5.0, 5.0, 6.0, 6.0]].unwrap();
    write_table(&mut s, "A", a, vec![num("x")]);
    write_table(&mut s, "B", b, vec![num("x")]);
    let out = run("select x from a intersect all select x from b;", &mut s);
    // 5 : min(2,3)=2 ; 6 : min(1,2)=1.
    assert_eq!(nums(&out, "x"), vec![5.0, 5.0, 6.0]);
}

#[test]
fn intersect_all_missing_values() {
    let mut s = make_session();
    let a = df!["x" => [None, None, Some(7.0_f64)]].unwrap();
    let b = df!["x" => [None, Some(7.0_f64), Some(7.0)]].unwrap();
    write_table(&mut s, "A", a, vec![num("x")]);
    write_table(&mut s, "B", b, vec![num("x")]);
    // INTERSECT ALL : null min(2,1)=1 ; 7 min(1,2)=1.
    let out = run("select x from a intersect all select x from b;", &mut s);
    assert_eq!(out.height(), 2);
    let col = out.column("x").unwrap().f64().unwrap();
    assert_eq!(col.iter().filter(|o| o.is_none()).count(), 1);
    assert_eq!(col.iter().flatten().collect::<Vec<f64>>(), vec![7.0]);
}
