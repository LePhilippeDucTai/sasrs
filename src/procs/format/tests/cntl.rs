//! M39.2 — tests `CNTLOUT=`/`CNTLIN=`.

use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use crate::procs::common::decode_column;
use crate::value::Value;
use polars::df;

fn read_work(session: &Session, table: &str) -> SasDataset {
    session.libs.get("WORK").unwrap().read(table).unwrap().0
}

/// Decode a char column of `ds` (by name) into plain `String`s. Panics if the
/// column is absent or not character (both would be a test bug, not a
/// scenario under test).
fn col_strs(ds: &SasDataset, name: &str) -> Vec<String> {
    let i = ds
        .vars
        .iter()
        .position(|v| v.name == name)
        .unwrap_or_else(|| panic!("no column {name} — columns: {:?}", ds.vars));
    decode_column(ds, i)
        .unwrap()
        .into_iter()
        .map(|v| match v {
            Value::Char(s) => s,
            other => panic!("expected char column {name}, got {other:?}"),
        })
        .collect()
}

/// One row of a CNTLOUT dataset, decoded for easy assertions.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    fmtname: String,
    start: String,
    end: String,
    label: String,
    ty: String,
    sexcl: String,
    eexcl: String,
    hlo: String,
}

fn rows(ds: &SasDataset) -> Vec<Row> {
    let fmtname = col_strs(ds, "FMTNAME");
    let start = col_strs(ds, "START");
    let end = col_strs(ds, "END");
    let label = col_strs(ds, "LABEL");
    let ty = col_strs(ds, "TYPE");
    let sexcl = col_strs(ds, "SEXCL");
    let eexcl = col_strs(ds, "EEXCL");
    let hlo = col_strs(ds, "HLO");
    (0..ds.n_obs())
        .map(|i| Row {
            fmtname: fmtname[i].clone(),
            start: start[i].clone(),
            end: end[i].clone(),
            label: label[i].clone(),
            ty: ty[i].clone(),
            sexcl: sexcl[i].clone(),
            eexcl: eexcl[i].clone(),
            hlo: hlo[i].clone(),
        })
        .collect()
}

// ── parsing ──────────────────────────────────────────────────────────────

#[test]
fn parse_cntlout_one_level() {
    let ast = parse_format_src("proc format cntlout=fmtctl; value f 1='A'; run;").unwrap();
    let out = ast.cntlout.expect("cntlout parsed");
    assert_eq!(out.libref, None);
    assert_eq!(out.name, "fmtctl");
    assert!(ast.cntlin.is_none());
}

#[test]
fn parse_cntlin_two_level() {
    let ast = parse_format_src("proc format cntlin=work.fmtctl; run;").unwrap();
    let inp = ast.cntlin.expect("cntlin parsed");
    assert_eq!(inp.libref.as_deref(), Some("work"));
    assert_eq!(inp.name, "fmtctl");
}

#[test]
fn parse_cntlout_and_cntlin_together_with_lib() {
    let ast = parse_format_src("proc format lib=work cntlin=a cntlout=b; run;").unwrap();
    assert_eq!(ast.lib, "WORK");
    assert_eq!(ast.cntlin.unwrap().name, "a");
    assert_eq!(ast.cntlout.unwrap().name, "b");
}

// ── CNTLOUT= : catalogue → dataset ──────────────────────────────────────────

#[test]
fn cntlout_numeric_ranges_and_other() {
    let session = run_format_src(
        "proc format cntlout=fmtctl; \
         value agef low-<21='Minor' 21-high='Adult' other='Unknown'; \
         run;",
    );
    let ds = read_work(&session, "FMTCTL");
    assert_eq!(ds.n_obs(), 3, "2 ranges + 1 other");
    let rs = rows(&ds);

    assert_eq!(rs[0].fmtname, "AGEF");
    assert_eq!(rs[0].ty, "N");
    assert_eq!(rs[0].start, "", "LOW start is blank");
    assert_eq!(rs[0].end, "21");
    assert_eq!(rs[0].label, "Minor");
    assert_eq!(rs[0].hlo, "L");
    assert_eq!(rs[0].sexcl, "N");
    assert_eq!(rs[0].eexcl, "Y", "21-<... : end exclusive");

    assert_eq!(rs[1].start, "21");
    assert_eq!(rs[1].end, "", "HIGH end is blank");
    assert_eq!(rs[1].label, "Adult");
    assert_eq!(rs[1].hlo, "H");
    assert_eq!(rs[1].sexcl, "N");
    assert_eq!(rs[1].eexcl, "N");

    assert_eq!(rs[2].start, "");
    assert_eq!(rs[2].end, "");
    assert_eq!(rs[2].label, "Unknown");
    assert_eq!(rs[2].hlo, "O");
}

#[test]
fn cntlout_char_format() {
    let session = run_format_src(
        "proc format cntlout=fmtctl; value $cityf 'PAR'='Paris' 'NYC'='New York'; run;",
    );
    let ds = read_work(&session, "FMTCTL");
    let rs = rows(&ds);
    assert_eq!(rs.len(), 2);
    assert_eq!(rs[0].fmtname, "CITYF", "FMTNAME excludes the leading $");
    assert_eq!(rs[0].ty, "C");
    assert_eq!(rs[0].start, "PAR");
    assert_eq!(rs[0].end, "PAR");
    assert_eq!(rs[0].label, "Paris");
    assert_eq!(rs[1].start, "NYC");
    assert_eq!(rs[1].label, "New York");
}

#[test]
fn cntlout_exclusive_bounds_both_sides() {
    let session = run_format_src("proc format cntlout=fmtctl; value f 1<-<10='mid'; run;");
    let ds = read_work(&session, "FMTCTL");
    let rs = rows(&ds);
    assert_eq!(rs[0].sexcl, "Y");
    assert_eq!(rs[0].eexcl, "Y");
    assert_eq!(rs[0].hlo, "");
}

#[test]
fn cntlout_single_value_range() {
    // `1='Male'` : from == to, no LOW/HIGH, no exclusivity.
    let session = run_format_src("proc format cntlout=fmtctl; value sexf 1='Male'; run;");
    let ds = read_work(&session, "FMTCTL");
    let rs = rows(&ds);
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].start, "1");
    assert_eq!(rs[0].end, "1");
    assert_eq!(rs[0].hlo, "");
    assert_eq!(rs[0].sexcl, "N");
    assert_eq!(rs[0].eexcl, "N");
}

#[test]
fn cntlout_multiple_formats_sorted_deterministically() {
    let session = run_format_src(
        "proc format cntlout=fmtctl; \
         value zzz 1='z'; \
         value aaa 1='a'; \
         run;",
    );
    let ds = read_work(&session, "FMTCTL");
    let rs = rows(&ds);
    // Deterministic (FMTNAME, TYPE) sort, not HashMap iteration order.
    assert_eq!(rs[0].fmtname, "AAA");
    assert_eq!(rs[1].fmtname, "ZZZ");
}

#[test]
fn cntlout_dataset_note_and_last_dataset() {
    let session = run_format_src("proc format cntlout=fmtctl; value f 1='A'; run;");
    let log = session.log.into_string();
    assert!(
        log.contains("The data set WORK.FMTCTL has 1 observations and 8 variables."),
        "log: {log}"
    );
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.FMTCTL"));
}

#[test]
fn cntlout_picture_formats_excluded_with_note() {
    let session = run_format_src(
        "proc format cntlout=fmtctl; \
         value f 1='A'; \
         picture p low-high='009'; \
         run;",
    );
    let ds = read_work(&session, "FMTCTL");
    let rs = rows(&ds);
    assert_eq!(rs.len(), 1, "the PICTURE format must not appear");
    assert_eq!(rs[0].fmtname, "F");
    let log = session.log.into_string();
    assert!(
        log.contains("1 PICTURE format(s)") && log.contains("not included"),
        "log: {log}"
    );
}

// ── CNTLIN= : dataset → catalogue ───────────────────────────────────────────

#[test]
fn cntlin_reconstructs_value_format_and_resolves() {
    use crate::formats::FormatSpec;

    // Hand-authored minimal control dataset: only the mandatory columns
    // (FMTNAME/TYPE/START/END/LABEL) — SEXCL/EEXCL/HLO are OMITTED to check
    // the documented "tolerates absent optional columns" behaviour.
    let mut session = make_session();
    let df = df![
        "FMTNAME" => ["AGEF", "AGEF", "AGEF"],
        "TYPE" => ["N", "N", "N"],
        "START" => ["", "21", ""],
        "END" => ["21", "", ""],
        "LABEL" => ["Minor", "Adult", "Unknown"],
        "HLO" => ["L", "H", "O"],
    ]
    .unwrap();
    let vars = vec![
        crate::procs::common::char_var_meta("FMTNAME", 8),
        crate::procs::common::char_var_meta("TYPE", 1),
        crate::procs::common::char_var_meta("START", 8),
        crate::procs::common::char_var_meta("END", 8),
        crate::procs::common::char_var_meta("LABEL", 8),
        crate::procs::common::char_var_meta("HLO", 1),
    ];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("CTL", &SasDataset { df, vars })
        .unwrap();

    let ast = parse_format_src("proc format cntlin=work.ctl; run;").unwrap();
    execute(&ast, &mut session).unwrap();

    let spec = FormatSpec::parse("AGEF.").unwrap();
    assert_eq!(
        session
            .format_catalog
            .format(&Value::Num(10.0), &spec)
            .trim(),
        "Minor"
    );
    assert_eq!(
        session
            .format_catalog
            .format(&Value::Num(30.0), &spec)
            .trim(),
        "Adult"
    );
    assert_eq!(
        session
            .format_catalog
            .format(&Value::Num(21.0), &spec)
            .trim(),
        "Minor",
        "EEXCL was omitted from the control dataset -> defaults to inclusive, \
         so 21 falls in the first (LOW-to-21) range, not the second"
    );
    assert_eq!(
        session
            .format_catalog
            .format(&Value::Char("x".to_string()), &spec)
            .trim(),
        "Unknown"
    );

    let log = session.log.into_string();
    assert!(log.contains("Format AGEF has been output."), "log: {log}");
}

#[test]
fn cntlin_missing_fmtname_column_is_error() {
    let mut session = make_session();
    let df = df!["TYPE" => ["N"]].unwrap();
    let vars = vec![crate::procs::common::char_var_meta("TYPE", 1)];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("CTL", &SasDataset { df, vars })
        .unwrap();

    let ast = parse_format_src("proc format cntlin=work.ctl; run;").unwrap();
    let err = execute(&ast, &mut session).unwrap_err();
    assert!(
        err.to_string().to_uppercase().contains("FMTNAME"),
        "got: {err}"
    );
}

#[test]
fn cntlin_picture_rows_are_skipped_with_note() {
    let mut session = make_session();
    let df = df![
        "FMTNAME" => ["F", "P"],
        "TYPE" => ["N", "P"],
        "START" => ["1", ""],
        "END" => ["1", ""],
        "LABEL" => ["A", "009"],
    ]
    .unwrap();
    let vars = vec![
        crate::procs::common::char_var_meta("FMTNAME", 8),
        crate::procs::common::char_var_meta("TYPE", 1),
        crate::procs::common::char_var_meta("START", 8),
        crate::procs::common::char_var_meta("END", 8),
        crate::procs::common::char_var_meta("LABEL", 8),
    ];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("CTL", &SasDataset { df, vars })
        .unwrap();

    let ast = parse_format_src("proc format cntlin=work.ctl; run;").unwrap();
    execute(&ast, &mut session).unwrap();

    use crate::formats::FormatSpec;
    let spec = FormatSpec::parse("F.").unwrap();
    assert_eq!(session.format_catalog.format(&Value::Num(1.0), &spec), "A");
    let log = session.log.into_string();
    assert!(
        log.contains("1 PICTURE format observation(s)") && log.contains("skipped"),
        "log: {log}"
    );
}

// ── round trip (the M39.2 oracle) ───────────────────────────────────────────

#[test]
fn cntlout_cntlin_round_trip_identical() {
    let mut session = make_session();
    let ast1 = parse_format_src(
        "proc format cntlout=x; \
         value agef low-<21='Minor' 21-high='Adult' other='Unknown'; \
         run;",
    )
    .unwrap();
    execute(&ast1, &mut session).unwrap();

    let ast2 = parse_format_src("proc format cntlin=x cntlout=y; run;").unwrap();
    execute(&ast2, &mut session).unwrap();

    let dx = read_work(&session, "X");
    let dy = read_work(&session, "Y");
    assert_eq!(
        rows(&dx),
        rows(&dy),
        "CNTLOUT -> CNTLIN -> CNTLOUT must reproduce the identical control dataset"
    );

    // And the reconstructed format resolves exactly like the original.
    use crate::formats::FormatSpec;
    let spec = FormatSpec::parse("AGEF.").unwrap();
    assert_eq!(
        session
            .format_catalog
            .format(&Value::Num(10.0), &spec)
            .trim(),
        "Minor"
    );
    assert_eq!(
        session
            .format_catalog
            .format(&Value::Num(99.0), &spec)
            .trim(),
        "Adult"
    );
}

// ── INVALUE (I/J) round trip ────────────────────────────────────────────────

#[test]
fn cntlout_invalue_numeric_and_char() {
    let session = run_format_src(
        "proc format cntlout=fmtctl; \
         invalue gradei 'A'=4 'B'=3 other=0; \
         invalue $sizej 'S'='Small' 'L'='Large'; \
         run;",
    );
    let ds = read_work(&session, "FMTCTL");
    let rs = rows(&ds);
    // gradei: 2 ranges + other; sizej: 2 ranges. Sorted (GRADEI, I) < (SIZEJ, J).
    assert_eq!(rs.len(), 5);
    assert!(rs[..3].iter().all(|r| r.fmtname == "GRADEI" && r.ty == "I"));
    assert!(rs[3..].iter().all(|r| r.fmtname == "SIZEJ" && r.ty == "J"));
    assert_eq!(rs[2].hlo, "O");
    assert_eq!(rs[2].label, "0");
}

#[test]
fn cntlin_invalue_round_trip() {
    use crate::formats::FormatSpec;

    let mut session = make_session();
    let ast1 = parse_format_src("proc format cntlout=x; invalue gradei 'A'=4 'B'=3 other=0; run;")
        .unwrap();
    execute(&ast1, &mut session).unwrap();

    let ast2 = parse_format_src("proc format cntlin=x cntlout=y; run;").unwrap();
    execute(&ast2, &mut session).unwrap();

    assert_eq!(
        rows(&read_work(&session, "X")),
        rows(&read_work(&session, "Y"))
    );

    let spec = FormatSpec::parse("GRADEI.").unwrap();
    assert_eq!(session.format_catalog.informat("A", &spec), Value::Num(4.0));
    assert_eq!(session.format_catalog.informat("Z", &spec), Value::Num(0.0));
}

// ── LIB= interaction ─────────────────────────────────────────────────────

#[test]
fn cntlin_with_lib_persists_reconstructed_formats_to_sidecar() {
    let tmp = tempfile::tempdir().unwrap();
    let mut session = make_session();
    session
        .libs
        .assign("PERM", tmp.path().to_path_buf())
        .unwrap();

    // Build the control dataset in WORK first (plain CNTLOUT of a WORK format).
    let ast1 =
        parse_format_src("proc format cntlout=work.ctl; value gradef 1='Pass' 2='Fail'; run;")
            .unwrap();
    execute(&ast1, &mut session).unwrap();

    // Reload it into the PERM library's catalog via CNTLIN=, LIB=perm.
    let ast2 = parse_format_src("proc format lib=perm cntlin=work.ctl; run;").unwrap();
    execute(&ast2, &mut session).unwrap();

    let sidecar = tmp.path().join(crate::formats::FormatCatalog::SIDECAR_FILE);
    assert!(
        sidecar.is_file(),
        "CNTLIN= reconstructed formats under LIB=perm should persist a sidecar"
    );
    let reloaded = crate::formats::FormatCatalog::load_sidecar(tmp.path()).unwrap();
    assert!(
        !reloaded.is_empty(),
        "sidecar should contain the CNTLIN-reconstructed GRADEF format"
    );
}
