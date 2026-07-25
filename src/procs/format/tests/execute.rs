use super::super::*;
use super::*;
use crate::source::SourceFile;

// ── execute tests ─────────────────────────────────────────────────────────

#[test]
fn execute_registers_format_in_catalog() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let mut session = make_session();
    let ast = FormatAst {
        values: vec![(
            "SEXFMT".to_string(),
            UserFormat {
                is_char: false,
                ranges: vec![
                    crate::formats::userdef::Range {
                        from: Bound::Num(1.0),
                        to: Bound::Num(1.0),
                        from_exclusive: false,
                        to_exclusive: false,
                        label: "Male".to_string(),
                    },
                    crate::formats::userdef::Range {
                        from: Bound::Num(2.0),
                        to: Bound::Num(2.0),
                        from_exclusive: false,
                        to_exclusive: false,
                        label: "Female".to_string(),
                    },
                ],
                other: Some("Unknown".to_string()),
            },
        )],
        invalues: vec![],
        pictures: vec![],
    };

    execute(&ast, &mut session).unwrap();

    // Verify it's in the catalog.
    let spec = FormatSpec::parse("SEXFMT.").unwrap();
    let result = session.format_catalog.format(&Value::Num(1.0), &spec);
    // Right-justified to w=0 (no width in spec) → label as-is.
    assert!(result.contains("Male"), "result: {result}");

    // NOTE logged.
    let log = session.log.into_string();
    assert!(log.contains("Format SEXFMT has been output."), "log: {log}");
}

#[test]
fn execute_round_trip_parse_and_execute() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let mut session = make_session();
    let source = SourceFile::new(
        "proc format; value sexfmt 1='Male' 2='Female' other='?'; run;",
    );
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // format
    let ast = parse(&mut ts).unwrap();
    execute(&ast, &mut session).unwrap();

    let spec = FormatSpec::parse("SEXFMT.").unwrap();
    assert_eq!(
        session.format_catalog.format(&Value::Num(1.0), &spec),
        "Male"
    );
    assert_eq!(
        session.format_catalog.format(&Value::Num(2.0), &spec),
        "Female"
    );
    assert_eq!(
        session.format_catalog.format(&Value::Num(99.0), &spec),
        "?"
    );
}

#[test]
fn execute_invalue_numeric_registered_in_catalog() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src(
        "proc format; invalue grade 'A'=4 'B'=3 'C'=2 'D'=1 'F'=0; run;",
    );

    let spec = FormatSpec::parse("GRADE.").unwrap();
    assert_eq!(session.format_catalog.informat("A", &spec), Value::Num(4.0));
    assert_eq!(session.format_catalog.informat("B", &spec), Value::Num(3.0));
    assert_eq!(session.format_catalog.informat("F", &spec), Value::Num(0.0));

    // NOTE logged for informat.
    let log = session.log.into_string();
    assert!(log.contains("Informat GRADE has been output."), "log: {log}");
}

#[test]
fn execute_invalue_char_dollar_registered() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src(
        "proc format; invalue $size 'S'='Small' 'M'='Medium' 'L'='Large'; run;",
    );

    let spec = FormatSpec::parse("$SIZE.").unwrap();
    assert_eq!(session.format_catalog.informat("S", &spec), Value::Char("Small".to_string()));
    assert_eq!(session.format_catalog.informat("M", &spec), Value::Char("Medium".to_string()));
    assert_eq!(session.format_catalog.informat("L", &spec), Value::Char("Large".to_string()));
}

#[test]
fn execute_invalue_unmatched_returns_missing() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src(
        "proc format; invalue grade 'A'=4 'B'=3; run;",
    );

    let spec = FormatSpec::parse("GRADE.").unwrap();
    // "X" not matched, no other → missing.
    assert_eq!(session.format_catalog.informat("X", &spec), Value::missing());
}

#[test]
fn execute_invalue_other_fallback() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src(
        "proc format; invalue grade 'A'=4 'B'=3 other=.; run;",
    );

    let spec = FormatSpec::parse("GRADE.").unwrap();
    assert_eq!(session.format_catalog.informat("A", &spec), Value::Num(4.0));
    assert_eq!(session.format_catalog.informat("Z", &spec), Value::missing());
}

#[test]
fn execute_invalue_and_value_coexist() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src(
        "proc format; \
         value sexfmt 1='Male' 2='Female'; \
         invalue grade 'A'=4 'B'=3; \
         run;",
    );

    // VALUE format still works.
    let fspec = FormatSpec::parse("SEXFMT.").unwrap();
    assert_eq!(session.format_catalog.format(&Value::Num(1.0), &fspec), "Male");

    // INVALUE informat also works.
    let ispec = FormatSpec::parse("GRADE.").unwrap();
    assert_eq!(session.format_catalog.informat("A", &ispec), Value::Num(4.0));
}

// ── PICTURE execute tests (M18.3) ─────────────────────────────────────────

#[test]
fn execute_picture_registered_and_applies() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src(
        "proc format; picture dollarpic low-high = '000,000,009.99' (prefix='$'); run;",
    );
    let spec = FormatSpec::parse("DOLLARPIC.").unwrap();
    // No width → rendered as-is.
    assert_eq!(
        session.format_catalog.format(&Value::Num(1234.5), &spec),
        "$1,234.50"
    );
    // NOTE logged.
    let log = session.log.into_string();
    assert!(log.contains("Format DOLLARPIC has been output."), "log: {log}");
}

#[test]
fn execute_picture_mult_directive() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src(
        "proc format; picture pct low-high = '009.9%' (mult=100); run;",
    );
    let spec = FormatSpec::parse("PCT.").unwrap();
    assert_eq!(session.format_catalog.format(&Value::Num(0.125), &spec), "  1.3%");
}

#[test]
fn execute_picture_with_width_right_justifies() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src(
        "proc format; picture p low-high = '009'; run;",
    );
    let spec = FormatSpec::parse("P10.").unwrap();
    // Rendered "  5" then right-justified to width 10.
    let out = session.format_catalog.format(&Value::Num(5.0), &spec);
    assert_eq!(out.len(), 10);
    assert!(out.ends_with("5"));
}

#[test]
fn execute_picture_missing_value() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src(
        "proc format; picture p low-high = '009.99'; run;",
    );
    let spec = FormatSpec::parse("P5.").unwrap();
    // Numeric missing intercepted before picture → missing char, right-justified.
    assert_eq!(session.format_catalog.format(&Value::missing(), &spec), "    .");
}

#[test]
fn execute_picture_shadows_builtin_name() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    // Define a picture named COMMA (a builtin format name) — user picture wins.
    let session = run_format_src(
        "proc format; picture comma low-high = '009'; run;",
    );
    let spec = FormatSpec::parse("COMMA.").unwrap();
    // Builtin COMMA on 5 would give "5"; our picture '009' gives "  5".
    assert_eq!(session.format_catalog.format(&Value::Num(5.0), &spec), "  5");
}

#[test]
fn execute_picture_via_put_function() {
    // PUT(value, picture.) through the data step function path.
    let out = run_det(
        "proc format; picture dp low-high='000,009.99' (prefix='$'); run;\n\
         data _null_; x = 1234.5; y = put(x, dp.); put y=; run;",
    );
    assert_eq!(out.exit_code, 0, "log: {}", out.log);
    // PUT y= renders "y=$1,234.50" (PUT() result trimmed of leading blanks).
    assert!(out.log.contains("$1,234.50"), "log: {}", out.log);
}

#[test]
fn execute_picture_via_format_statement() {
    // FORMAT statement + PROC PRINT path.
    let out = run_det(
        "proc format; picture dp low-high='009.99'; run;\n\
         data t; x = 12.34; format x dp.; output; run;\n\
         proc print data=t; run;",
    );
    assert_eq!(out.exit_code, 0, "log: {}", out.log);
    assert!(out.listing.contains("12.34"), "listing: {}", out.listing);
}
