use super::*;

// ── parse tests ───────────────────────────────────────────────────────────

#[test]
fn parse_minimal_empty() {
    let ast = parse_format_src("proc format; run;").unwrap();
    assert!(ast.values.is_empty());
}

#[test]
fn parse_single_value_numeric() {
    let ast = parse_format_src(
        "proc format; value sexfmt 1='Male' 2='Female'; run;",
    )
    .unwrap();
    assert_eq!(ast.values.len(), 1);
    let (name, uf) = &ast.values[0];
    assert_eq!(name, "sexfmt");
    assert!(!uf.is_char);
    assert_eq!(uf.ranges.len(), 2);
    assert_eq!(uf.ranges[0].label, "Male");
    assert_eq!(uf.ranges[1].label, "Female");
}

#[test]
fn parse_char_format_with_dollar() {
    let ast = parse_format_src(
        "proc format; value $cityfmt 'PAR'='Paris' 'NYC'='New York'; run;",
    )
    .unwrap();
    assert_eq!(ast.values.len(), 1);
    let (name, uf) = &ast.values[0];
    assert_eq!(name, "$cityfmt");
    assert!(uf.is_char);
    assert_eq!(uf.ranges.len(), 2);
    assert_eq!(uf.ranges[0].label, "Paris");
    assert_eq!(uf.ranges[1].label, "New York");
}

#[test]
fn parse_inclusive_range() {
    let ast = parse_format_src(
        "proc format; value agefmt 0-17='Child' 18-64='Adult' 65-high='Senior'; run;",
    )
    .unwrap();
    let (_, uf) = &ast.values[0];
    assert_eq!(uf.ranges.len(), 3);
    // 0-17: from=Num(0), to=Num(17), both inclusive
    assert!(matches!(uf.ranges[0].from, Bound::Num(n) if n == 0.0));
    assert!(matches!(uf.ranges[0].to, Bound::Num(n) if n == 17.0));
    assert!(!uf.ranges[0].from_exclusive);
    assert!(!uf.ranges[0].to_exclusive);
    // 65-high
    assert!(matches!(uf.ranges[2].from, Bound::Num(n) if n == 65.0));
    assert!(matches!(uf.ranges[2].to, Bound::High));
}

#[test]
fn parse_low_exclusive_upper() {
    // low-<5='Below5'
    let ast = parse_format_src(
        "proc format; value f low-<5='Below5' 5-high='AtLeast5'; run;",
    )
    .unwrap();
    let (_, uf) = &ast.values[0];
    assert!(matches!(uf.ranges[0].from, Bound::Low));
    assert!(matches!(uf.ranges[0].to, Bound::Num(n) if n == 5.0));
    assert!(!uf.ranges[0].from_exclusive);
    assert!(uf.ranges[0].to_exclusive);
}

#[test]
fn parse_exclusive_lower_to_high() {
    // 5<-high='Above5'
    let ast = parse_format_src(
        "proc format; value f low-5='AtMost5' 5<-high='Above5'; run;",
    )
    .unwrap();
    let (_, uf) = &ast.values[0];
    // Second range: 5<-high
    assert!(matches!(uf.ranges[1].from, Bound::Num(n) if n == 5.0));
    assert!(uf.ranges[1].from_exclusive);
    assert!(!uf.ranges[1].to_exclusive);
    assert!(matches!(uf.ranges[1].to, Bound::High));
}

#[test]
fn parse_both_exclusive() {
    // 1<-<10='Middle'
    let ast = parse_format_src(
        "proc format; value f 1<-<10='Middle'; run;",
    )
    .unwrap();
    let (_, uf) = &ast.values[0];
    assert!(uf.ranges[0].from_exclusive);
    assert!(uf.ranges[0].to_exclusive);
}

#[test]
fn parse_comma_list() {
    // 1,2,3='Group'  → 3 ranges with same label
    let ast = parse_format_src(
        "proc format; value f 1,2,3='Group'; run;",
    )
    .unwrap();
    let (_, uf) = &ast.values[0];
    assert_eq!(uf.ranges.len(), 3);
    for r in &uf.ranges {
        assert_eq!(r.label, "Group");
    }
}

#[test]
fn parse_other() {
    let ast = parse_format_src(
        "proc format; value f 1='One' other='Unknown'; run;",
    )
    .unwrap();
    let (_, uf) = &ast.values[0];
    assert_eq!(uf.other, Some("Unknown".to_string()));
    assert_eq!(uf.ranges.len(), 1);
}

#[test]
fn parse_multiple_value_stmts() {
    let ast = parse_format_src(
        "proc format; value a 1='x'; value b 2='y'; run;",
    )
    .unwrap();
    assert_eq!(ast.values.len(), 2);
    assert_eq!(ast.values[0].0, "a");
    assert_eq!(ast.values[1].0, "b");
}

#[test]
fn parse_invalue_numeric_basic() {
    // INVALUE without $ → numeric result.
    let ast = parse_format_src(
        "proc format; invalue grade 'A'=4 'B'=3 'C'=2 'D'=1 'F'=0; run;",
    )
    .unwrap();
    assert_eq!(ast.invalues.len(), 1);
    let (name, ui) = &ast.invalues[0];
    assert_eq!(name, "grade");
    assert!(!ui.is_char_result);
    assert_eq!(ui.ranges.len(), 5);
    // First range: 'A'=4
    assert!(matches!(ui.ranges[0].from, Bound::Char(ref s) if s == "A"));
    assert!(matches!(ui.ranges[0].result, InformatValue::Num(n) if n == 4.0));
    // Last range: 'F'=0
    assert!(matches!(ui.ranges[4].result, InformatValue::Num(n) if n == 0.0));
}

#[test]
fn parse_invalue_char_with_dollar() {
    // INVALUE with $ → character result.
    let ast = parse_format_src(
        "proc format; invalue $size 'S'='Small' 'M'='Medium' 'L'='Large'; run;",
    )
    .unwrap();
    assert_eq!(ast.invalues.len(), 1);
    let (name, ui) = &ast.invalues[0];
    assert_eq!(name, "$size");
    assert!(ui.is_char_result);
    assert_eq!(ui.ranges.len(), 3);
    assert!(matches!(&ui.ranges[0].result, InformatValue::Char(s) if s == "Small"));
    assert!(matches!(&ui.ranges[2].result, InformatValue::Char(s) if s == "Large"));
}

#[test]
fn parse_invalue_other_and_same() {
    // `_same_` → Same variant; `other=.` (unquoted dot) → Missing.
    let ast = parse_format_src(
        "proc format; invalue $code low-'Z'=_same_ other=.; run;",
    )
    .unwrap();
    let (_, ui) = &ast.invalues[0];
    assert!(matches!(ui.ranges[0].result, InformatValue::Same));
    assert!(matches!(ui.other, Some(InformatValue::Missing(_))));
}

#[test]
fn parse_invalue_quoted_string_other() {
    // `other='?'` → Char variant (quoted string result).
    let ast = parse_format_src(
        "proc format; invalue $code 'A'='Alpha' other='?'; run;",
    )
    .unwrap();
    let (_, ui) = &ast.invalues[0];
    assert!(matches!(&ui.other, Some(InformatValue::Char(s)) if s == "?"));
}

#[test]
fn parse_invalue_range_with_exclusion() {
    let ast = parse_format_src(
        "proc format; invalue f 'A'-<'Z'=1; run;",
    )
    .unwrap();
    let (_, ui) = &ast.invalues[0];
    assert!(!ui.ranges[0].from_exclusive);
    assert!(ui.ranges[0].to_exclusive);
}

#[test]
fn parse_invalue_mixed_with_value() {
    // Can have both VALUE and INVALUE in same PROC FORMAT.
    let ast = parse_format_src(
        "proc format; value sexfmt 1='Male'; invalue grade 'A'=4; run;",
    )
    .unwrap();
    assert_eq!(ast.values.len(), 1);
    assert_eq!(ast.invalues.len(), 1);
}

// ── PICTURE parse tests (M18.3) ───────────────────────────────────────────

#[test]
fn parse_picture_string_bounds_rejected() {
    // PICTURE is numeric-only: quoted (string) bounds are a parse error.
    let ast = parse_format_src(
        "proc format; picture mmddyy '01'-'12' = '99/99/9999'; run;",
    );
    assert!(ast.is_err());
}

#[test]
fn parse_picture_numeric_range_template() {
    let ast = parse_format_src(
        "proc format; picture mmddyy low-high = '99/99/9999'; run;",
    )
    .unwrap();
    assert_eq!(ast.pictures.len(), 1);
    let (name, up) = &ast.pictures[0];
    assert_eq!(name, "mmddyy");
    assert_eq!(up.ranges.len(), 1);
    assert_eq!(up.ranges[0].template, "99/99/9999");
    assert!(matches!(up.ranges[0].from, Bound::Low));
    assert!(matches!(up.ranges[0].to, Bound::High));
}

#[test]
fn parse_picture_with_prefix_directive() {
    let ast = parse_format_src(
        "proc format; picture dollarpic low-high = '000,000,009.99' (prefix='$'); run;",
    )
    .unwrap();
    let (_, up) = &ast.pictures[0];
    assert_eq!(up.ranges[0].directives.prefix.as_deref(), Some("$"));
    assert_eq!(up.ranges[0].directives.mult, None);
    assert_eq!(up.ranges[0].directives.fill, None);
}

#[test]
fn parse_picture_with_mult_and_fill() {
    let ast = parse_format_src(
        "proc format; picture pct other = '009.9%' (mult=100 fill='*'); run;",
    )
    .unwrap();
    let (_, up) = &ast.pictures[0];
    assert!(up.ranges.is_empty());
    let (tpl, dir) = up.other.as_ref().unwrap();
    assert_eq!(tpl, "009.9%");
    assert_eq!(dir.mult, Some(100.0));
    assert_eq!(dir.fill, Some('*'));
}

#[test]
fn parse_picture_multiple_ranges() {
    let ast = parse_format_src(
        "proc format; picture p 0-9='9' 10-high='999'; run;",
    )
    .unwrap();
    let (_, up) = &ast.pictures[0];
    assert_eq!(up.ranges.len(), 2);
    assert_eq!(up.ranges[0].template, "9");
    assert_eq!(up.ranges[1].template, "999");
}

#[test]
fn parse_picture_coexists_with_value() {
    let ast = parse_format_src(
        "proc format; value sexfmt 1='Male'; picture p low-high='009'; run;",
    )
    .unwrap();
    assert_eq!(ast.values.len(), 1);
    assert_eq!(ast.pictures.len(), 1);
}
