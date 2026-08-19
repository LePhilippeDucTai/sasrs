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
        lib: "WORK".to_string(),
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
                ..Default::default()
            },
        )],
        invalues: vec![],
        pictures: vec![],
        cntlout: None,
        cntlin: None,
        fmtlib: false,
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
    let source = SourceFile::new("proc format; value sexfmt 1='Male' 2='Female' other='?'; run;");
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
    assert_eq!(session.format_catalog.format(&Value::Num(99.0), &spec), "?");
}

// ── M39.1 — LIBRARY=<libref> sidecar persistence ────────────────────────────

#[test]
fn execute_lib_work_writes_no_sidecar() {
    // Default (no LIBRARY=) or explicit LIBRARY=WORK: purely in-memory, byte
    // identical to the pre-M39.1 behaviour — NO file anywhere, including
    // WORK's own (temp) directory.
    let mut session = make_session();
    let work_dir = session
        .libs
        .get("WORK")
        .unwrap()
        .catalog_dir()
        .unwrap()
        .to_path_buf();
    let ast = parse_format_src("proc format; value sexfmt 1='Male'; run;").unwrap();
    assert_eq!(ast.lib, "WORK");
    execute(&ast, &mut session).unwrap();
    assert!(
        !work_dir
            .join(crate::formats::FormatCatalog::SIDECAR_FILE)
            .exists(),
        "WORK must never persist a format catalog sidecar"
    );
    assert!(session.libref_format_catalogs.is_empty());
}

#[test]
fn execute_lib_permanent_writes_sidecar_and_resolves_immediately() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let tmp = tempfile::tempdir().unwrap();
    let mut session = make_session();
    session
        .libs
        .assign("PERM", tmp.path().to_path_buf())
        .unwrap();

    let ast =
        parse_format_src("proc format lib=perm; value gradef 1='Pass' 2='Fail'; run;").unwrap();
    execute(&ast, &mut session).unwrap();

    // Sidecar written at the libref's root.
    let sidecar = tmp.path().join(crate::formats::FormatCatalog::SIDECAR_FILE);
    assert!(
        sidecar.is_file(),
        "PROC FORMAT LIB=perm should persist a sidecar"
    );

    // Immediately resolvable in THIS session, like any user format.
    let spec = FormatSpec::parse("GRADEF.").unwrap();
    assert_eq!(
        session
            .format_catalog
            .format(&Value::Num(1.0), &spec)
            .trim(),
        "Pass"
    );

    // The libref's own catalog (for further accumulation / re-save) also holds it.
    assert!(session.libref_format_catalogs.contains_key("PERM"));
}

#[test]
fn execute_lib_permanent_no_substatements_writes_no_sidecar() {
    let tmp = tempfile::tempdir().unwrap();
    let mut session = make_session();
    session
        .libs
        .assign("PERM", tmp.path().to_path_buf())
        .unwrap();

    let ast = parse_format_src("proc format lib=perm; run;").unwrap();
    execute(&ast, &mut session).unwrap();

    let sidecar = tmp.path().join(crate::formats::FormatCatalog::SIDECAR_FILE);
    assert!(
        !sidecar.exists(),
        "an empty PROC FORMAT LIB= step must not write a sidecar"
    );
}

#[test]
fn execute_lib_unassigned_libref_is_error() {
    let mut session = make_session();
    let ast = parse_format_src("proc format lib=nosuchlib; value gradef 1='Pass'; run;").unwrap();
    let err = execute(&ast, &mut session).unwrap_err();
    assert!(
        err.to_string().to_uppercase().contains("NOSUCHLIB"),
        "got: {err}"
    );
}

#[test]
fn execute_invalue_numeric_registered_in_catalog() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src("proc format; invalue grade 'A'=4 'B'=3 'C'=2 'D'=1 'F'=0; run;");

    let spec = FormatSpec::parse("GRADE.").unwrap();
    assert_eq!(session.format_catalog.informat("A", &spec), Value::Num(4.0));
    assert_eq!(session.format_catalog.informat("B", &spec), Value::Num(3.0));
    assert_eq!(session.format_catalog.informat("F", &spec), Value::Num(0.0));

    // NOTE logged for informat.
    let log = session.log.into_string();
    assert!(
        log.contains("Informat GRADE has been output."),
        "log: {log}"
    );
}

#[test]
fn execute_invalue_char_dollar_registered() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session =
        run_format_src("proc format; invalue $size 'S'='Small' 'M'='Medium' 'L'='Large'; run;");

    let spec = FormatSpec::parse("$SIZE.").unwrap();
    assert_eq!(
        session.format_catalog.informat("S", &spec),
        Value::Char("Small".to_string())
    );
    assert_eq!(
        session.format_catalog.informat("M", &spec),
        Value::Char("Medium".to_string())
    );
    assert_eq!(
        session.format_catalog.informat("L", &spec),
        Value::Char("Large".to_string())
    );
}

#[test]
fn execute_invalue_unmatched_returns_missing() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src("proc format; invalue grade 'A'=4 'B'=3; run;");

    let spec = FormatSpec::parse("GRADE.").unwrap();
    // "X" not matched, no other → missing.
    assert_eq!(
        session.format_catalog.informat("X", &spec),
        Value::missing()
    );
}

#[test]
fn execute_invalue_other_fallback() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src("proc format; invalue grade 'A'=4 'B'=3 other=.; run;");

    let spec = FormatSpec::parse("GRADE.").unwrap();
    assert_eq!(session.format_catalog.informat("A", &spec), Value::Num(4.0));
    assert_eq!(
        session.format_catalog.informat("Z", &spec),
        Value::missing()
    );
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
    assert_eq!(
        session.format_catalog.format(&Value::Num(1.0), &fspec),
        "Male"
    );

    // INVALUE informat also works.
    let ispec = FormatSpec::parse("GRADE.").unwrap();
    assert_eq!(
        session.format_catalog.informat("A", &ispec),
        Value::Num(4.0)
    );
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
    assert!(
        log.contains("Format DOLLARPIC has been output."),
        "log: {log}"
    );
}

#[test]
fn execute_picture_mult_directive() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src("proc format; picture pct low-high = '009.9%' (mult=100); run;");
    let spec = FormatSpec::parse("PCT.").unwrap();
    assert_eq!(
        session.format_catalog.format(&Value::Num(0.125), &spec),
        "  1.3%"
    );
}

#[test]
fn execute_picture_with_width_right_justifies() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    let session = run_format_src("proc format; picture p low-high = '009'; run;");
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

    let session = run_format_src("proc format; picture p low-high = '009.99'; run;");
    let spec = FormatSpec::parse("P5.").unwrap();
    // Numeric missing intercepted before picture → missing char, right-justified.
    assert_eq!(
        session.format_catalog.format(&Value::missing(), &spec),
        "    ."
    );
}

#[test]
fn execute_picture_shadows_builtin_name() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    // Define a picture named COMMA (a builtin format name) — user picture wins.
    let session = run_format_src("proc format; picture comma low-high = '009'; run;");
    let spec = FormatSpec::parse("COMMA.").unwrap();
    // Builtin COMMA on 5 would give "5"; our picture '009' gives "  5".
    assert_eq!(
        session.format_catalog.format(&Value::Num(5.0), &spec),
        "  5"
    );
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

// ── M43.1 — MIN=/MAX=/DEFAULT=/FUZZ= end-to-end registration ────────────────

#[test]
fn execute_value_width_options_registered_in_catalog() {
    // Full parse+execute through `proc format; value ... (min= max= default=
    // fuzz=) ...; run;` — the resulting catalog entry's UserFormat must carry
    // the same four values that were parsed.
    let session = run_format_src(
        "proc format; \
         value agef (min=3 max=10 default=6 fuzz=0.5) \
         low-<21='Minor' 21-high='Adult'; \
         run;",
    );
    let (_, uf) = session
        .format_catalog
        .user_formats()
        .find(|(k, _)| *k == "AGEF")
        .expect("AGEF registered in catalog");
    assert_eq!(uf.min, Some(3));
    assert_eq!(uf.max, Some(10));
    assert_eq!(uf.default, Some(6));
    assert_eq!(uf.fuzz, Some(0.5));
}

#[test]
fn execute_value_no_width_options_leaves_fields_none() {
    // Regression: a VALUE with none of MIN=/MAX=/DEFAULT=/FUZZ= still
    // registers with all four fields `None` (pre-M43.1 byte-identical
    // behavior), matching the "nothing new set" case in parse.rs.
    let session = run_format_src("proc format; value f 1='One'; run;");
    let (_, uf) = session
        .format_catalog
        .user_formats()
        .find(|(k, _)| *k == "F")
        .expect("F registered in catalog");
    assert_eq!(uf.min, None);
    assert_eq!(uf.max, None);
    assert_eq!(uf.default, None);
    assert_eq!(uf.fuzz, None);
}

#[test]
fn execute_value_default_width_applied_via_format_catalog() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    // DEFAULT=6, no explicit width at the point of use → the catalog must
    // apply the format-level DEFAULT= as the output width: label "X" (len 1)
    // right-justified to width 6.
    let session = run_format_src("proc format; value f (default=6) 1='X'; run;");
    let spec = FormatSpec::parse("F.").unwrap();
    assert_eq!(
        session.format_catalog.format(&Value::Num(1.0), &spec),
        "     X"
    );
}

#[test]
fn execute_value_min_clamps_computed_width_via_format_catalog() {
    use crate::formats::FormatSpec;
    use crate::value::Value;

    // No DEFAULT=, no explicit width → computed width = len("X") = 1,
    // clamped up to MIN=8.
    let session = run_format_src("proc format; value f (min=8) 1='X'; run;");
    let spec = FormatSpec::parse("F.").unwrap();
    let out = session.format_catalog.format(&Value::Num(1.0), &spec);
    assert_eq!(out.len(), 8);
    assert!(out.ends_with('X'), "out: {out:?}");
}
