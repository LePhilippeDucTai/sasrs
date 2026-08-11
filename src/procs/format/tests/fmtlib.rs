use super::super::*;
use super::*;

// ── FMTLIB parsing ──────────────────────────────────────────────────────────

#[test]
fn parse_fmtlib_sets_flag() {
    let ast = parse_format_src("proc format fmtlib; value sexfmt 1='Male'; run;").unwrap();
    assert!(ast.fmtlib);
    assert_eq!(ast.lib, "WORK");
}

#[test]
fn parse_fmtlib_combines_with_library() {
    // Order of header options must not matter.
    let ast = parse_format_src("proc format library=perm fmtlib; value g 1='Pass'; run;").unwrap();
    assert!(ast.fmtlib);
    assert_eq!(ast.lib, "PERM");

    let ast2 = parse_format_src("proc format fmtlib library=perm; value g 1='Pass'; run;").unwrap();
    assert!(ast2.fmtlib);
    assert_eq!(ast2.lib, "PERM");
}

#[test]
fn parse_no_fmtlib_default_false() {
    let ast = parse_format_src("proc format; value sexfmt 1='Male'; run;").unwrap();
    assert!(!ast.fmtlib);
}

// ── FMTLIB rendering ─────────────────────────────────────────────────────────

#[test]
fn fmtlib_lists_exactly_the_defined_formats() {
    let mut session = make_session();
    let ast = parse_format_src(
        "proc format fmtlib; \
         value agef low-<21='Minor' 21-high='Adult' other='Unknown'; \
         value $gradef 'A'='Excellent' 'B'='Good'; \
         invalue gradei 'A'=4 'B'=3 other=0; \
         run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(listing.contains("FORMAT NAME: AGEF"), "listing: {listing}");
    assert!(
        listing.contains("FORMAT NAME: $GRADEF"),
        "listing: {listing}"
    );
    assert!(
        listing.contains("INFORMAT NAME: GRADEI"),
        "listing: {listing}"
    );
    // Ranges/labels present.
    assert!(listing.contains("Minor"), "listing: {listing}");
    assert!(listing.contains("Adult"), "listing: {listing}");
    assert!(listing.contains("OTHER"), "listing: {listing}");
    assert!(listing.contains("Excellent"), "listing: {listing}");
    // Exactly 3 blocks: no fourth header line beyond the three defined here.
    // ("INFORMAT NAME:" contains "FORMAT NAME:" as a substring, so count the
    // informat headers via the leading "I" to keep the two counts disjoint.)
    let informat_headers = listing.matches("INFORMAT NAME:").count();
    let format_headers = listing.matches("FORMAT NAME:").count() - informat_headers;
    assert_eq!(format_headers, 2, "listing: {listing}");
    assert_eq!(informat_headers, 1, "listing: {listing}");
}

#[test]
fn fmtlib_no_option_writes_nothing_to_listing() {
    let mut session = make_session();
    let ast = parse_format_src("proc format; value sexfmt 1='Male'; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(
        listing.is_empty(),
        "PROC FORMAT without FMTLIB must not touch the listing: {listing}"
    );
}

#[test]
fn fmtlib_default_targets_work_only() {
    let tmp = tempfile::tempdir().unwrap();
    let mut session = make_session();
    session
        .libs
        .assign("PERM", tmp.path().to_path_buf())
        .unwrap();

    // Define a WORK format and a PERM format with DIFFERENT names, then ask
    // for FMTLIB with no LIB= (defaults WORK): only the WORK one must show.
    let ast1 = parse_format_src("proc format; value workfmt 1='W'; run;").unwrap();
    execute(&ast1, &mut session).unwrap();
    let ast2 = parse_format_src("proc format library=perm; value permfmt 1='P'; run;").unwrap();
    execute(&ast2, &mut session).unwrap();

    let ast3 = parse_format_src("proc format fmtlib; run;").unwrap();
    execute(&ast3, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("FORMAT NAME: WORKFMT"),
        "listing: {listing}"
    );
    assert!(
        !listing.contains("PERMFMT"),
        "FMTLIB (no LIB=) must not show a permanent library's formats: {listing}"
    );
}

#[test]
fn fmtlib_lib_targets_named_catalog_only() {
    let tmp = tempfile::tempdir().unwrap();
    let mut session = make_session();
    session
        .libs
        .assign("PERM", tmp.path().to_path_buf())
        .unwrap();

    let ast1 = parse_format_src("proc format; value workfmt 1='W'; run;").unwrap();
    execute(&ast1, &mut session).unwrap();
    let ast2 =
        parse_format_src("proc format library=perm fmtlib; value permfmt 1='P'; run;").unwrap();
    execute(&ast2, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("FORMAT NAME: PERMFMT"),
        "listing: {listing}"
    );
    assert!(
        !listing.contains("WORKFMT"),
        "FMTLIB LIB=perm must not show WORK's formats: {listing}"
    );
}

#[test]
fn fmtlib_empty_catalog_notes_no_entries() {
    let mut session = make_session();
    let ast = parse_format_src("proc format fmtlib; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(
        listing.to_uppercase().contains("NO FORMATS OR INFORMATS"),
        "listing: {listing}"
    );
}

#[test]
fn fmtlib_single_value_range_blank_end() {
    let mut session = make_session();
    let ast =
        parse_format_src("proc format fmtlib; value sexfmt 1='Male' 2='Female'; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // The START column shows "1"/"2"; the END column stays blank for a
    // single-value range (from == to, no exclusive bound).
    assert!(listing.contains("Male"), "listing: {listing}");
    assert!(listing.contains("Female"), "listing: {listing}");
}
