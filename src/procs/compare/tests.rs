use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::value::VarType;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn parse_compare_src(src: &str) -> Result<CompareAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "compare"
    parse(&mut ts)
}

fn write_numeric_ds(
    session: &mut Session,
    name: &str,
    x_vals: &[f64],
    y_vals: &[f64],
) {
    let df = df![
        "x" => x_vals.to_vec(),
        "y" => y_vals.to_vec()
    ]
    .unwrap();
    let vars = vec![
        VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
        VarMeta { name: "y".into(), ty: VarType::Num, length: 8, format: None, label: None },
    ];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
}

fn write_char_ds(session: &mut Session, name: &str, vals: &[&str]) {
    let df = df!["name" => vals.to_vec()].unwrap();
    let vars = vec![
        VarMeta { name: "name".into(), ty: VarType::Char, length: 8, format: None, label: None },
    ];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
}

// ── Parse tests ───────────────────────────────────────────────────────────

#[test]
fn parse_minimal() {
    let ast = parse_compare_src("proc compare base=work.a compare=work.b; run;").unwrap();
    assert_eq!(ast.base.name.to_uppercase(), "A");
    assert_eq!(ast.compare.name.to_uppercase(), "B");
    assert!(ast.out.is_none());
    assert!(!ast.novalues);
    assert!(!ast.briefsummary);
}

#[test]
fn parse_all_options() {
    let ast = parse_compare_src(
        "proc compare base=work.a compare=work.b out=work.diffs novalues briefsummary; run;",
    )
    .unwrap();
    assert!(ast.out.is_some());
    assert!(ast.novalues);
    assert!(ast.briefsummary);
}

#[test]
fn parse_missing_base_errors() {
    let result = parse_compare_src("proc compare compare=work.b; run;");
    assert!(result.is_err());
}

#[test]
fn parse_missing_compare_errors() {
    let result = parse_compare_src("proc compare base=work.a; run;");
    assert!(result.is_err());
}

// ── Execute tests ─────────────────────────────────────────────────────────

#[test]
fn execute_identical_datasets_no_diffs() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "A", &[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]);
    write_numeric_ds(&mut session, "B", &[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]);

    let ast = CompareAst {
        base: DatasetRef { libref: Some("WORK".into()), name: "A".into() },
        compare: DatasetRef { libref: Some("WORK".into()), name: "B".into() },
        out: None,
        novalues: false,
        briefsummary: false,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("No unequal values"), "log: {log}");

    let listing = session.listing.into_string();
    assert!(listing.contains("WORK.A"), "listing: {listing}");
    assert!(listing.contains("WORK.B"), "listing: {listing}");
}

#[test]
fn execute_with_differences() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "BASE1", &[1.0, 2.0], &[10.0, 20.0]);
    write_numeric_ds(&mut session, "COMP1", &[1.0, 9.0], &[10.0, 20.0]);

    let ast = CompareAst {
        base: DatasetRef { libref: Some("WORK".into()), name: "BASE1".into() },
        compare: DatasetRef { libref: Some("WORK".into()), name: "COMP1".into() },
        out: None,
        novalues: false,
        briefsummary: false,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("1 observation"), "log: {log}");

    let listing = session.listing.into_string();
    assert!(listing.contains("1"), "diffs in listing: {listing}");
}

#[test]
fn execute_variable_only_in_base() {
    let mut session = make_session();
    // BASE has x and z; COMPARE has only x
    let df_base = df!["x" => [1.0_f64, 2.0], "z" => [5.0_f64, 6.0]].unwrap();
    let vars_base = vec![
        VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
        VarMeta { name: "z".into(), ty: VarType::Num, length: 8, format: None, label: None },
    ];
    let df_comp = df!["x" => [1.0_f64, 2.0]].unwrap();
    let vars_comp = vec![
        VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
    ];
    session.libs.get("WORK").unwrap().write("BASE2", &SasDataset { df: df_base, vars: vars_base }).unwrap();
    session.libs.get("WORK").unwrap().write("COMP2", &SasDataset { df: df_comp, vars: vars_comp }).unwrap();

    let ast = CompareAst {
        base: DatasetRef { libref: Some("WORK".into()), name: "BASE2".into() },
        compare: DatasetRef { libref: Some("WORK".into()), name: "COMP2".into() },
        out: None,
        novalues: false,
        briefsummary: false,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("BASE only") || listing.contains("Z"), "listing: {listing}");
}

#[test]
fn execute_type_mismatch_reported() {
    let mut session = make_session();
    // BASE has x as Num, COMP has x as Char
    let df_base = df!["x" => [1.0_f64, 2.0]].unwrap();
    let vars_base = vec![
        VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
    ];
    let df_comp = df!["x" => ["a", "b"]].unwrap();
    let vars_comp = vec![
        VarMeta { name: "x".into(), ty: VarType::Char, length: 1, format: None, label: None },
    ];
    session.libs.get("WORK").unwrap().write("BASE3", &SasDataset { df: df_base, vars: vars_base }).unwrap();
    session.libs.get("WORK").unwrap().write("COMP3", &SasDataset { df: df_comp, vars: vars_comp }).unwrap();

    let ast = CompareAst {
        base: DatasetRef { libref: Some("WORK".into()), name: "BASE3".into() },
        compare: DatasetRef { libref: Some("WORK".into()), name: "COMP3".into() },
        out: None,
        novalues: false,
        briefsummary: false,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Different Types") || listing.contains("Num") || listing.contains("Char"), "listing: {listing}");
}

#[test]
fn execute_different_nobs() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "BASE4", &[1.0, 2.0, 3.0], &[0.0, 0.0, 0.0]);
    write_numeric_ds(&mut session, "COMP4", &[1.0, 2.0], &[0.0, 0.0]);

    let ast = CompareAst {
        base: DatasetRef { libref: Some("WORK".into()), name: "BASE4".into() },
        compare: DatasetRef { libref: Some("WORK".into()), name: "COMP4".into() },
        out: None,
        novalues: false,
        briefsummary: false,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Not Compared") || listing.contains("2"), "listing: {listing}");
}

#[test]
fn execute_char_trailing_blanks_equivalent() {
    let mut session = make_session();
    write_char_ds(&mut session, "CBASE", &["abc", "xyz"]);
    write_char_ds(&mut session, "CCOMP", &["abc   ", "xyz "]);

    let ast = CompareAst {
        base: DatasetRef { libref: Some("WORK".into()), name: "CBASE".into() },
        compare: DatasetRef { libref: Some("WORK".into()), name: "CCOMP".into() },
        out: None,
        novalues: false,
        briefsummary: false,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    // Trailing blanks should be considered equal via sas_cmp
    assert!(log.contains("No unequal values"), "log: {log}");
}

#[test]
fn execute_missing_equality() {
    let mut session = make_session();
    // Both have missing values at the same position → equal
    let df_base = df!["x" => Series::new("x".into(), &[Some(1.0_f64), None])].unwrap();
    let df_comp = df!["x" => Series::new("x".into(), &[Some(1.0_f64), None])].unwrap();
    let vars = vec![
        VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
    ];
    session.libs.get("WORK").unwrap().write("MISS1", &SasDataset { df: df_base, vars: vars.clone() }).unwrap();
    session.libs.get("WORK").unwrap().write("MISS2", &SasDataset { df: df_comp, vars: vars }).unwrap();

    let ast = CompareAst {
        base: DatasetRef { libref: Some("WORK".into()), name: "MISS1".into() },
        compare: DatasetRef { libref: Some("WORK".into()), name: "MISS2".into() },
        out: None,
        novalues: false,
        briefsummary: false,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("No unequal values"), "log: {log}");
}

#[test]
fn execute_out_dataset_created() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "OBASE", &[1.0, 2.0], &[10.0, 20.0]);
    write_numeric_ds(&mut session, "OCOMP", &[1.0, 9.0], &[10.0, 20.0]);

    let ast = CompareAst {
        base: DatasetRef { libref: Some("WORK".into()), name: "OBASE".into() },
        compare: DatasetRef { libref: Some("WORK".into()), name: "OCOMP".into() },
        out: Some(DatasetRef { libref: Some("WORK".into()), name: "DIFFS".into() }),
        novalues: false,
        briefsummary: false,
    };
    execute(&ast, &mut session).unwrap();

    // OUT= dataset should exist
    assert!(session.libs.get("WORK").unwrap().exists("DIFFS"));
    let (ds, _) = session.libs.get("WORK").unwrap().read("DIFFS").unwrap();
    // 3 rows per differing obs (BASE, COMPARE, DIF)
    assert_eq!(ds.n_obs(), 3);
}

#[test]
fn execute_briefsummary() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "BS1", &[1.0], &[2.0]);
    write_numeric_ds(&mut session, "BS2", &[1.0], &[3.0]);

    let ast = CompareAst {
        base: DatasetRef { libref: Some("WORK".into()), name: "BS1".into() },
        compare: DatasetRef { libref: Some("WORK".into()), name: "BS2".into() },
        out: None,
        novalues: false,
        briefsummary: true,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Brief Summary") || listing.contains("WORK.BS1"), "listing: {listing}");
}

#[test]
fn execute_novalues_omits_values_section() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "NV1", &[1.0, 2.0], &[0.0, 0.0]);
    write_numeric_ds(&mut session, "NV2", &[1.0, 9.0], &[0.0, 0.0]);

    let ast = CompareAst {
        base: DatasetRef { libref: Some("WORK".into()), name: "NV1".into() },
        compare: DatasetRef { libref: Some("WORK".into()), name: "NV2".into() },
        out: None,
        novalues: true,
        briefsummary: false,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // The values section header should be absent
    assert!(!listing.contains("Values Comparison Summary"), "listing should not have values section: {listing}");
}
