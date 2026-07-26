use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::testkit::*;

// ── Helpers ───────────────────────────────────────────────────────────────

fn parse_contents_src(src: &str) -> Result<ContentsAst> {
    let full = format!("proc contents {}; run;", src);
    let source = SourceFile::new(&full);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "contents"
    parse(&mut ts)
}

fn parse_contents_full(src: &str) -> Result<ContentsAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "contents"
    parse(&mut ts)
}

/// Write a small dataset with one Num and one Char variable.
/// `age` has a format and label; `name` has neither.
fn write_test_dataset(session: &mut Session) {
    let df = df![
        "name" => ["Alice", "Bob"],
        "age"  => [30.0_f64, 25.0]
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "name".to_string(),
            ty: VarType::Char,
            length: 5,
            format: None,
            label: None,
        },
        VarMeta {
            name: "age".to_string(),
            ty: VarType::Num,
            length: 8,
            format: Some("best12.".to_string()),
            label: Some("Age of subject".to_string()),
        },
    ];
    let ds = SasDataset { df, vars };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("CLASS", &ds)
        .unwrap();
    session.last_dataset = Some("WORK.CLASS".to_string());
}

// ── Parse tests ───────────────────────────────────────────────────────────

#[test]
fn parse_minimal() {
    let ast = parse_contents_src("").unwrap();
    assert!(ast.data.is_none());
    assert!(!ast.varnum);
    assert!(!ast.all);
}

#[test]
fn parse_data_option() {
    let ast = parse_contents_src("data=work.x").unwrap();
    assert_eq!(
        ast.data,
        Some(DatasetRef {
            libref: Some("work".into()),
            name: "x".into()
        })
    );
    assert!(!ast.varnum);
    assert!(!ast.all);
}

#[test]
fn parse_varnum_option() {
    let ast = parse_contents_src("data=work.x varnum").unwrap();
    assert!(ast.varnum);
    assert!(!ast.all);
}

#[test]
fn parse_all_option() {
    let ast = parse_contents_full("proc contents data=work._all_; run;").unwrap();
    assert!(ast.all);
    assert_eq!(ast.data.as_ref().unwrap().libref, Some("work".into()));
}

#[test]
fn parse_all_uppercase() {
    let ast = parse_contents_full("proc contents data=MYLIB._ALL_; run;").unwrap();
    assert!(ast.all);
}

#[test]
fn parse_unknown_option_errors() {
    let result = parse_contents_src("bogus");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("BOGUS") || msg.contains("bogus"), "msg: {msg}");
}

// ── Execute tests ─────────────────────────────────────────────────────────

#[test]
fn execute_basic_contents() {
    let mut session = make_session();
    write_test_dataset(&mut session);

    let ast = ContentsAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "CLASS".into(),
        }),
        varnum: false,
        all: false,
        out: None,
        short: false,
        details: false,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();

    // Header block should contain dataset name and observation count
    assert!(listing.contains("WORK.CLASS"), "listing: {listing}");
    assert!(listing.contains('2'), "obs count: {listing}");

    // Variable names must appear
    assert!(
        listing.contains("name") || listing.contains("NAME"),
        "listing: {listing}"
    );
    assert!(
        listing.contains("age") || listing.contains("AGE"),
        "listing: {listing}"
    );

    // Type column
    assert!(listing.contains("Num"), "Num type: {listing}");
    assert!(listing.contains("Char"), "Char type: {listing}");
}

#[test]
fn execute_shows_format_and_label() {
    let mut session = make_session();
    write_test_dataset(&mut session);

    let ast = ContentsAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "CLASS".into(),
        }),
        varnum: false,
        all: false,
        out: None,
        short: false,
        details: false,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();

    // Format should appear in the variable table
    assert!(
        listing.contains("best12.") || listing.contains("BEST12."),
        "format: {listing}"
    );

    // Label should appear in the variable table
    assert!(listing.contains("Age of subject"), "label: {listing}");
}

#[test]
fn execute_varnum_ordering() {
    // With varnum, variables should appear in creation order (name then age).
    // Without varnum (default), they appear alphabetically (age then name).
    let mut session = make_session();
    write_test_dataset(&mut session);

    // Default: alphabetical → age before name
    let ast_alpha = ContentsAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "CLASS".into(),
        }),
        varnum: false,
        all: false,
        out: None,
        short: false,
        details: false,
    };
    execute(&ast_alpha, &mut session).unwrap();
    let listing = session.listing.take_string();

    // Find positions of "age" and "name" in the variable table section.
    // Use rfind so the header "Data Set Name:" (which also contains "name")
    // does not confuse the position check — the last occurrence of each
    // token is in the variable table, where alphabetical order must hold.
    let lower = listing.to_lowercase();
    let pos_age = lower.rfind("age");
    let pos_name = lower.rfind("name");
    assert!(
        pos_age.is_some() && pos_name.is_some(),
        "listing: {listing}"
    );
    assert!(
        pos_age.unwrap() < pos_name.unwrap(),
        "alphabetical: age before name; listing:\n{listing}"
    );

    // With varnum: creation order → name (index 0) before age (index 1)
    let mut session2 = make_session();
    write_test_dataset(&mut session2);
    let ast_varnum = ContentsAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "CLASS".into(),
        }),
        varnum: true,
        all: false,
        out: None,
        short: false,
        details: false,
    };
    execute(&ast_varnum, &mut session2).unwrap();
    let listing2 = session2.listing.take_string();
    let lower2 = listing2.to_lowercase();
    // Skip the header "Variables: 2" which also contains text before variable table
    // Find the variable table section after the blank line following the header
    let pos_age2 = lower2.rfind("age");
    let pos_name2 = lower2.rfind("name");
    assert!(
        pos_age2.is_some() && pos_name2.is_some(),
        "listing2: {listing2}"
    );
    assert!(
        pos_name2.unwrap() < pos_age2.unwrap(),
        "varnum: name (index 0) before age (index 1); listing:\n{listing2}"
    );
}

#[test]
fn execute_all_lists_tables() {
    let mut session = make_session();
    write_test_dataset(&mut session);

    // Write a second dataset so there are 2 tables
    let df2 = df!["x" => [1.0_f64]].unwrap();
    let vars2 = vec![VarMeta {
        name: "x".to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }];
    let ds2 = SasDataset {
        df: df2,
        vars: vars2,
    };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("SCORES", &ds2)
        .unwrap();

    let ast = ContentsAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "_ALL_".into(),
        }),
        varnum: false,
        all: true,
        out: None,
        short: false,
        details: false,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(listing.contains("CLASS"), "listing: {listing}");
    assert!(listing.contains("SCORES"), "listing: {listing}");
    assert!(listing.contains("Member Name"), "listing: {listing}");
}

#[test]
fn execute_uses_last_dataset_when_no_data() {
    let mut session = make_session();
    write_test_dataset(&mut session);

    let ast = ContentsAst {
        data: None,
        varnum: false,
        all: false,
        out: None,
        short: false,
        details: false,
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("WORK.CLASS"), "listing: {listing}");
}

#[test]
fn execute_no_last_dataset_errors() {
    let mut session = make_session();

    let ast = ContentsAst {
        data: None,
        varnum: false,
        all: false,
        out: None,
        short: false,
        details: false,
    };
    let result = execute(&ast, &mut session);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("_LAST_") || msg.contains("undefined"),
        "msg: {msg}"
    );
}

// ── M33.7 : OUT= / SHORT / DETAILS ────────────────────────────────────────

#[test]
fn parse_out_short_details() {
    let ast =
        parse_contents_full("proc contents data=work.x out=work.meta short details; run;").unwrap();
    assert!(ast.short);
    assert!(ast.details);
    assert_eq!(
        ast.out,
        Some(DatasetRef {
            libref: Some("work".into()),
            name: "meta".into()
        })
    );
}

#[test]
fn execute_out_dataset_shape_and_values() {
    let mut session = make_session();
    write_test_dataset(&mut session); // name (Char,5), age (Num,8, best12., "Age of subject")

    let ast = ContentsAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "CLASS".into(),
        }),
        varnum: false,
        all: false,
        out: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "META".into(),
        }),
        short: false,
        details: false,
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("META").unwrap();
    // 6 columns, 2 rows (one per variable in creation order: name, age).
    assert_eq!(out.n_obs(), 2, "one row per variable");
    let cols: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    assert_eq!(
        cols,
        vec!["NAME", "TYPE", "LENGTH", "VARNUM", "LABEL", "FORMAT"]
    );

    // Decode rows. Row 0 = name (Char → TYPE 2, LENGTH 5, VARNUM 1).
    let name = out.df.column("NAME").unwrap().str().unwrap();
    assert_eq!(name.get(0), Some("name"));
    assert_eq!(name.get(1), Some("age"));
    let ty = out.df.column("TYPE").unwrap().f64().unwrap();
    assert_eq!(ty.get(0), Some(2.0)); // char
    assert_eq!(ty.get(1), Some(1.0)); // num
    let len = out.df.column("LENGTH").unwrap().f64().unwrap();
    assert_eq!(len.get(0), Some(5.0));
    assert_eq!(len.get(1), Some(8.0));
    let vn = out.df.column("VARNUM").unwrap().f64().unwrap();
    assert_eq!(vn.get(0), Some(1.0));
    assert_eq!(vn.get(1), Some(2.0));
    let label = out.df.column("LABEL").unwrap().str().unwrap();
    assert_eq!(label.get(1), Some("Age of subject"));
    let fmt = out.df.column("FORMAT").unwrap().str().unwrap();
    assert_eq!(fmt.get(1), Some("best12."));

    // NOTE about the OUT= dataset.
    let log = session.log.into_string();
    assert!(
        log.contains("The data set WORK.META has 2 observations and 6 variables."),
        "log: {log}"
    );
}

#[test]
fn execute_short_lists_variable_names_only() {
    let mut session = make_session();
    write_test_dataset(&mut session);
    let ast = ContentsAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "CLASS".into(),
        }),
        varnum: false,
        all: false,
        out: None,
        short: true,
        details: false,
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // Alphabetical name list "age name" (default sort).
    assert!(listing.contains("age name"), "short var list: {listing}");
    // SHORT suppresses the per-variable detail table: the "Num"/"Char" type
    // cells of the detail table must not appear.
    assert!(
        !listing.contains("Char"),
        "no detail table under SHORT: {listing}"
    );
    assert!(
        !listing.contains("Num"),
        "no detail table under SHORT: {listing}"
    );
}

#[test]
fn execute_details_adds_header_lines() {
    let mut session = make_session();
    write_test_dataset(&mut session);
    let ast = ContentsAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "CLASS".into(),
        }),
        varnum: false,
        all: false,
        out: None,
        short: false,
        details: true,
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(
        listing.contains("# Observations:"),
        "details obs line: {listing}"
    );
    assert!(
        listing.contains("# Variables:"),
        "details var line: {listing}"
    );
}
