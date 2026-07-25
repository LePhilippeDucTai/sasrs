use super::super::*;
use super::*;

// ───────────── parse tests ─────────────

#[test]
fn parse_minimal() {
    let ast = parse_rank("proc rank data=a; var x; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert!(ast.out.is_none());
    assert!(!ast.descending);
    assert_eq!(ast.ties, Ties::Mean);
    assert!(ast.groups.is_none());
    assert_eq!(ast.var, vec!["x"]);
    assert!(ast.ranks.is_empty());
}

#[test]
fn parse_all_options() {
    let ast = parse_rank(
        "proc rank data=lib.a out=work.b descending ties=high groups=4; var x y; ranks rx ry; run;",
    )
    .unwrap();
    assert_eq!(ast.data.as_ref().unwrap().libref.as_deref(), Some("lib"));
    assert_eq!(ast.out.as_ref().unwrap().name, "b");
    assert!(ast.descending);
    assert_eq!(ast.ties, Ties::High);
    assert_eq!(ast.groups, Some(4));
    assert_eq!(ast.var, vec!["x", "y"]);
    assert_eq!(ast.ranks, vec!["rx", "ry"]);
}

#[test]
fn parse_ties_variants() {
    assert_eq!(parse_rank("proc rank ties=mean; var x; run;").unwrap().ties, Ties::Mean);
    assert_eq!(parse_rank("proc rank ties=low; var x; run;").unwrap().ties, Ties::Low);
    assert_eq!(parse_rank("proc rank ties=high; var x; run;").unwrap().ties, Ties::High);
    assert_eq!(parse_rank("proc rank ties=dense; var x; run;").unwrap().ties, Ties::Dense);
}

#[test]
fn parse_unknown_ties_errors() {
    let r = parse_rank("proc rank ties=bogus; var x; run;");
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("BOGUS"));
}

// NB : les méthodes FRACTION/PERCENT/NORMAL/SAVAGE et le statement BY sont
// désormais implémentés (M21.5) — voir `parse_method_options`,
// `parse_by_now_supported`, `method_*` et `execute_by_*`. Les anciens tests
// « not yet implemented » ont été retirés en conséquence.

#[test]
fn parse_unknown_option_errors() {
    let r = parse_rank("proc rank data=a bogus; var x; run;");
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("BOGUS"));
}

// ───────────── method parse tests ─────────────

#[test]
fn parse_method_options() {
    assert_eq!(parse_rank("proc rank fraction; var x; run;").unwrap().method, Method::Fraction);
    assert_eq!(parse_rank("proc rank nplus1; var x; run;").unwrap().method, Method::NPlus1);
    assert_eq!(parse_rank("proc rank percent; var x; run;").unwrap().method, Method::Percent);
    assert_eq!(parse_rank("proc rank savage; var x; run;").unwrap().method, Method::Savage);
    assert_eq!(
        parse_rank("proc rank normal=blom; var x; run;").unwrap().method,
        Method::Normal(NormalScore::Blom)
    );
    assert_eq!(
        parse_rank("proc rank normal=tukey; var x; run;").unwrap().method,
        Method::Normal(NormalScore::Tukey)
    );
    assert_eq!(
        parse_rank("proc rank normal=vw; var x; run;").unwrap().method,
        Method::Normal(NormalScore::Vw)
    );
}

#[test]
fn parse_normal_requires_method() {
    assert!(parse_rank("proc rank normal=bogus; var x; run;").is_err());
    assert!(parse_rank("proc rank normal; var x; run;").is_err());
}

#[test]
fn parse_two_methods_errors() {
    let r = parse_rank("proc rank fraction percent; var x; run;");
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("Only one ranking-method"));
}

#[test]
fn parse_by_now_supported() {
    let ast = parse_rank("proc rank data=a; by g; var x; run;").unwrap();
    assert_eq!(ast.by, vec![("g".to_string(), false)]);
    let ast2 = parse_rank("proc rank data=a; by descending g h; var x; run;").unwrap();
    assert_eq!(ast2.by, vec![("g".to_string(), true), ("h".to_string(), false)]);
}

#[test]
fn rank_basic_ascending() {
    let out = rank_column(&nums(&[30.0, 10.0, 20.0]), false, Ties::Mean, None, Method::Rank);
    assert_eq!(out, nums(&[3.0, 1.0, 2.0]));
}

#[test]
fn rank_descending() {
    let out = rank_column(&nums(&[30.0, 10.0, 20.0]), true, Ties::Mean, None, Method::Rank);
    // 30 largest → rank 1.
    assert_eq!(out, nums(&[1.0, 3.0, 2.0]));
}

#[test]
fn rank_ties_all_variants() {
    let data = nums(&[10.0, 20.0, 20.0, 40.0]);
    assert_eq!(
        rank_column(&data, false, Ties::Mean, None, Method::Rank),
        nums(&[1.0, 2.5, 2.5, 4.0])
    );
    assert_eq!(
        rank_column(&data, false, Ties::Low, None, Method::Rank),
        nums(&[1.0, 2.0, 2.0, 4.0])
    );
    assert_eq!(
        rank_column(&data, false, Ties::High, None, Method::Rank),
        nums(&[1.0, 3.0, 3.0, 4.0])
    );
    assert_eq!(
        rank_column(&data, false, Ties::Dense, None, Method::Rank),
        nums(&[1.0, 2.0, 2.0, 3.0])
    );
}

#[test]
fn rank_missing_excluded() {
    let data = vec![
        Value::Num(10.0),
        Value::missing(),
        Value::Num(30.0),
        Value::Num(20.0),
    ];
    let out = rank_column(&data, false, Ties::Mean, None, Method::Rank);
    assert_eq!(out[0], Value::Num(1.0));
    assert!(out[1].is_missing());
    assert_eq!(out[2], Value::Num(3.0));
    assert_eq!(out[3], Value::Num(2.0));
}

#[test]
fn rank_groups_partition() {
    // 10 distinct values, groups=4 → group = floor(4*r/11), r=1..10.
    let data = nums(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
    let out = rank_column(&data, false, Ties::Mean, Some(4), Method::Rank);
    let expected: Vec<f64> = (1..=10).map(|r| ((4 * r) / 11).min(3) as f64).collect();
    assert_eq!(out, nums(&expected));
    // sanity: groups are within 0..3.
    for v in &out {
        if let Value::Num(g) = v {
            assert!((0.0..=3.0).contains(g));
        }
    }
}

#[test]
fn rank_groups_ties_share_group() {
    // Tied values must land in the same group (same LOW ordinal r).
    let data = nums(&[10.0, 20.0, 20.0, 40.0]);
    let out = rank_column(&data, false, Ties::Mean, Some(2), Method::Rank);
    // r for the two 20s is 2 (LOW), so both share the same group.
    assert_eq!(out[1], out[2]);
}
