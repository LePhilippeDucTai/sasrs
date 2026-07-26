use super::*;
use crate::ast::GlobalStmt;

// ── LIBNAME ──────────────────────────────────────────────────────────────

#[test]
fn libname_with_path() {
    let stmt = parse("libname mylib '/data/sas';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Libname {
            libref: "mylib".into(),
            engine: None,
            path: "/data/sas".into(),
        }
    );
}

#[test]
fn libname_relative_path() {
    let stmt = parse("libname outlib 'output/results';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Libname {
            libref: "outlib".into(),
            engine: None,
            path: "output/results".into(),
        }
    );
}

#[test]
fn libname_with_csv_engine() {
    let stmt = parse("libname csvlib csv '/data/csv';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Libname {
            libref: "csvlib".into(),
            engine: Some("CSV".into()),
            path: "/data/csv".into(),
        }
    );
}

#[test]
fn libname_with_xlsx_engine() {
    let stmt = parse("libname xl xlsx '/data/xl';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Libname {
            libref: "xl".into(),
            engine: Some("XLSX".into()),
            path: "/data/xl".into(),
        }
    );
}

#[test]
fn libname_with_parquet_engine() {
    let stmt = parse("libname pq parquet '/data/pq';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Libname {
            libref: "pq".into(),
            engine: Some("PARQUET".into()),
            path: "/data/pq".into(),
        }
    );
}

#[test]
fn libname_engine_is_uppercased() {
    let stmt = parse("libname x Csv '/tmp';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Libname {
            libref: "x".into(),
            engine: Some("CSV".into()),
            path: "/tmp".into(),
        }
    );
}

#[test]
fn libname_clear() {
    let stmt = parse("libname mylib clear;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::LibnameClear {
            libref: "mylib".into(),
        }
    );
}

#[test]
fn libname_clear_case_insensitive() {
    let stmt = parse("LIBNAME MYLIB CLEAR;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::LibnameClear {
            libref: "MYLIB".into(),
        }
    );
}

#[test]
fn libname_libref_too_long_is_error() {
    // "toolonglib" = 10 characters — must error.
    let err = parse("libname toolonglib '/path';").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("libref") || msg.contains("8"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn libname_missing_libref_is_error() {
    // `libname '/path';` — no libref identifier.
    let err = parse("libname '/path';").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("libref"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn libname_missing_path_is_error() {
    // `libname mylib 123;` — path is not a string literal.
    let err = parse("libname mylib 123;").unwrap_err();
    assert!(!err.to_string().is_empty());
}

// ── FILENAME (M35.2) ─────────────────────────────────────────────────────

#[test]
fn filename_quoted_path() {
    let stmt = parse("filename inc '/tmp/x.sas';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Filename {
            fileref: "inc".into(),
            path: Some("/tmp/x.sas".into()),
            device: None,
        }
    );
}

#[test]
fn filename_bare_path() {
    // Un identifiant nu (non-device) est traité comme chemin.
    let stmt = parse("filename inc myfile;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Filename {
            fileref: "inc".into(),
            path: Some("myfile".into()),
            device: None,
        }
    );
}

#[test]
fn filename_device_temp_ignored() {
    let stmt = parse("filename tmp TEMP;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Filename {
            fileref: "tmp".into(),
            path: None,
            device: Some("TEMP".into()),
        }
    );
}

#[test]
fn filename_options_after_path_ignored() {
    // Résidu d'options après le chemin → consommé sans erreur.
    let stmt = parse("filename inc '/tmp/x.sas' lrecl=256;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Filename {
            fileref: "inc".into(),
            path: Some("/tmp/x.sas".into()),
            device: None,
        }
    );
}

// ── TITLE ────────────────────────────────────────────────────────────────

#[test]
fn title_simple() {
    let stmt = parse("title 'My Report';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Title {
            n: 1,
            text: Some("My Report".into()),
        }
    );
}

#[test]
fn title_uppercase() {
    let stmt = parse("TITLE 'My Report';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Title {
            n: 1,
            text: Some("My Report".into()),
        }
    );
}

#[test]
fn title_without_text_clears() {
    // `title;` — no text, clears title 1.
    let stmt = parse("title;").unwrap();
    assert_eq!(stmt, GlobalStmt::Title { n: 1, text: None });
}

#[test]
fn title_unquoted_text_is_error() {
    // SAS accepts unquoted text but our M1 parser requires a quoted literal.
    // `title foo;` must return a parse error.
    let err = parse("title foo;").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("quoted") || msg.to_lowercase().contains("string"),
        "expected an error about quoted string, got: {msg}"
    );
}

#[test]
fn title3() {
    let stmt = parse("title3 'Section Header';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Title {
            n: 3,
            text: Some("Section Header".into()),
        }
    );
}

#[test]
fn title9() {
    let stmt = parse("title9 'Footer';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Title {
            n: 9,
            text: Some("Footer".into())
        }
    );
}

#[test]
fn title5_without_text_clears() {
    let stmt = parse("title5;").unwrap();
    assert_eq!(stmt, GlobalStmt::Title { n: 5, text: None });
}

// ── FOOTNOTE ───────────────────────────────────────────────────────────

#[test]
fn footnote_simple() {
    let stmt = parse("footnote 'My Note';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Footnote {
            n: 1,
            text: Some("My Note".into())
        }
    );
}

#[test]
fn footnote_without_text_clears() {
    let stmt = parse("footnote;").unwrap();
    assert_eq!(stmt, GlobalStmt::Footnote { n: 1, text: None });
}

#[test]
fn footnote_unquoted_text_is_error() {
    let err = parse("footnote foo;").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("quoted") || msg.to_lowercase().contains("string"),
        "expected an error about quoted string, got: {msg}"
    );
}

#[test]
fn footnote3() {
    let stmt = parse("footnote3 'Third';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Footnote {
            n: 3,
            text: Some("Third".into())
        }
    );
}

#[test]
fn footnote5_without_text_clears() {
    let stmt = parse("footnote5;").unwrap();
    assert_eq!(stmt, GlobalStmt::Footnote { n: 5, text: None });
}

// ── OPTIONS ──────────────────────────────────────────────────────────────

#[test]
fn options_ls_and_nocenter() {
    let stmt = parse("options ls=80 nocenter;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Options(vec![
            ("ls".into(), Some("80".into())),
            ("nocenter".into(), None),
        ])
    );
}

#[test]
fn options_string_value() {
    // FMTSEARCH= now accepts a parenthesised list — this must parse successfully.
    let stmt = parse("options fmtsearch=(mylib work);").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Options(vec![("fmtsearch".into(), Some("mylib work".into()))])
    );

    // `(` in any other options value (not FMTSEARCH/MISSING) is still an error.
    let err = parse("options notes=(yes);").unwrap_err();
    let _ = err; // just verify no panic

    // A proper string value:
    let stmt = parse("options label='My value';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Options(vec![("label".into(), Some("My value".into()))])
    );
}

#[test]
fn options_empty_is_ok() {
    // `options;` — empty list is accepted (no-op per spec).
    let stmt = parse("options;").unwrap();
    assert_eq!(stmt, GlobalStmt::Options(vec![]));
}

#[test]
fn options_multiple_flags_and_values() {
    let stmt = parse("options center ps=60 linesize=132 nodate;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Options(vec![
            ("center".into(), None),
            ("ps".into(), Some("60".into())),
            ("linesize".into(), Some("132".into())),
            ("nodate".into(), None),
        ])
    );
}

#[test]
fn options_float_value() {
    // A float value should be formatted without trailing `.0` for integers.
    let stmt = parse("options decimals=2.5;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Options(vec![("decimals".into(), Some("2.5".into()))])
    );
}
