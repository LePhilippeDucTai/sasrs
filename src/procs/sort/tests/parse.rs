use super::*;

// --- parse tests ---

#[test]
fn parse_minimal_by() {
    let ast = parse_sort("proc sort data=a; by x; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert!(ast.out.is_none());
    assert_eq!(ast.by, vec![("x".to_string(), false)]);
    assert!(!ast.nodupkey);
    assert!(!ast.noduprecs);
}

#[test]
fn parse_out_and_nodupkey() {
    let ast = parse_sort("proc sort data=lib.a out=work.b nodupkey; by x; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().libref.as_deref(), Some("lib"));
    assert_eq!(ast.out.as_ref().unwrap().name, "b");
    assert!(ast.nodupkey);
}

#[test]
fn parse_noduprecs_alias() {
    let a = parse_sort("proc sort data=a noduprecs; by x; run;").unwrap();
    assert!(a.noduprecs);
    let b = parse_sort("proc sort data=a nodup; by x; run;").unwrap();
    assert!(b.noduprecs);
}

#[test]
fn parse_descending_multiple() {
    let ast =
        parse_sort("proc sort data=a; by descending x y descending z; run;").unwrap();
    assert_eq!(
        ast.by,
        vec![
            ("x".to_string(), true),
            ("y".to_string(), false),
            ("z".to_string(), true),
        ]
    );
}

#[test]
fn parse_no_data_uses_last() {
    let ast = parse_sort("proc sort; by x; run;").unwrap();
    assert!(ast.data.is_none());
}

#[test]
fn parse_missing_by_errors() {
    let result = parse_sort("proc sort data=a; run;");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("BY"), "msg: {msg}");
}

#[test]
fn parse_unknown_option_errors() {
    let result = parse_sort("proc sort data=a bogus; by x; run;");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("BOGUS"), "msg: {msg}");
}

// --- M33.9 new option tests ---

#[test]
fn parse_tagsort_accepted() {
    // TAGSORT is parsed without error and set in AST.
    let ast = parse_sort("proc sort data=a tagsort; by x; run;").unwrap();
    assert!(ast.tagsort);
    assert_eq!(ast.sortseq, SortSeq::Ascii);
}

#[test]
fn parse_sortseq_ascii_accepted() {
    let ast = parse_sort("proc sort data=a sortseq=ascii; by x; run;").unwrap();
    assert_eq!(ast.sortseq, SortSeq::Ascii);
    assert!(!ast.tagsort);
}

#[test]
fn parse_sortseq_linguistic_accepted() {
    let ast = parse_sort("proc sort data=a sortseq=linguistic; by x; run;").unwrap();
    assert_eq!(ast.sortseq, SortSeq::Linguistic);
}

#[test]
fn parse_sortseq_unknown_errors() {
    let result = parse_sort("proc sort data=a sortseq=ebcdic; by x; run;");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("EBCDIC") || msg.contains("Unknown"), "msg: {msg}");
}

#[test]
fn parse_key_ascending() {
    // KEY=var without /descending → ascending (same as BY var).
    let ast = parse_sort("proc sort data=a; key=age; run;").unwrap();
    assert_eq!(ast.by, vec![("age".to_string(), false)]);
}

#[test]
fn parse_key_descending() {
    // KEY=var / descending → equivalent to BY descending var.
    let ast = parse_sort("proc sort data=a; key=age / descending; run;").unwrap();
    assert_eq!(ast.by, vec![("age".to_string(), true)]);
}

#[test]
fn parse_multiple_key_statements() {
    let ast = parse_sort(
        "proc sort data=a; key=sex; key=age / descending; run;",
    )
    .unwrap();
    assert_eq!(
        ast.by,
        vec![("sex".to_string(), false), ("age".to_string(), true)]
    );
}

#[test]
fn parse_key_overrides_by() {
    // If both BY and KEY are present, KEY takes precedence.
    let ast = parse_sort(
        "proc sort data=a; by name; key=age / descending; run;",
    )
    .unwrap();
    // KEY wins: only age (descending) is in the effective key list.
    assert_eq!(ast.by, vec![("age".to_string(), true)]);
}
