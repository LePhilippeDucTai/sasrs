use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::testkit::*;
use crate::value::VarType;
use polars::prelude::df;
use std::io::Write;
use std::path::PathBuf;

fn parse_export_src(src: &str) -> Result<ExportAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "export"
    parse(&mut ts)
}

fn write_test_dataset(session: &mut Session, name: &str) {
    let df = df![
        "name" => ["Alice", "Bob", "Carol"],
        "age"  => [30.0_f64, 25.0, 35.0],
        "score" => [95.5_f64, 88.0, 72.3]
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "name".into(),
            ty: VarType::Char,
            length: 5,
            format: None,
            label: None,
        },
        VarMeta {
            name: "age".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "score".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
    ];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", name.to_uppercase()));
}

// --- Tests du parser ---

#[test]
fn parse_export_csv_minimal() {
    let ast =
        parse_export_src("proc export data=work.t outfile='/tmp/out.csv' dbms=csv; run;").unwrap();
    assert_eq!(ast.outfile, "/tmp/out.csv");
    assert_eq!(ast.dbms, ExportDbms::Csv);
    assert!(ast.data.is_some());
}

#[test]
fn parse_export_tab() {
    let ast = parse_export_src("proc export data=work.t outfile='out.tsv' dbms=TAB replace; run;")
        .unwrap();
    assert_eq!(ast.dbms, ExportDbms::Tab);
    assert!(ast.replace);
}

#[test]
fn parse_export_dlm_with_delimiter() {
    let ast =
        parse_export_src("proc export data=work.t outfile='out.txt' dbms=dlm; delimiter='|'; run;")
            .unwrap();
    assert_eq!(ast.dbms, ExportDbms::Dlm);
    assert_eq!(ast.delimiter, Some(b'|'));
}

#[test]
fn parse_export_missing_outfile_errors() {
    let result = parse_export_src("proc export data=work.t dbms=csv; run;");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("OUTFILE="), "msg: {msg}");
}

#[test]
fn parse_export_xlsx_deferred_error() {
    let result = parse_export_src("proc export data=work.t outfile='out.xlsx' dbms=xlsx; run;");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("not yet implemented"), "msg: {msg}");
    assert!(msg.contains("XLSX"), "msg: {msg}");
}

#[test]
fn parse_export_excel_deferred_error() {
    let result = parse_export_src("proc export data=work.t outfile='out.xlsx' dbms=excel; run;");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("not yet implemented"), "msg: {msg}");
}

// --- Tests d'exécution ---

#[test]
fn execute_export_csv_basic() {
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("out.csv");

    let work_dir = dir.path().join("work");
    std::fs::create_dir(&work_dir).unwrap();
    let mut session = Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

    write_test_dataset(&mut session, "MYDS");

    let ast = ExportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "MYDS".into(),
        }),
        outfile: out_path.to_string_lossy().into_owned(),
        dbms: ExportDbms::Csv,
        replace: false,
        delimiter: None,
    };
    execute(&ast, &mut session).unwrap();

    // Vérifier la NOTE
    let log = session.log.into_string();
    assert!(
        log.contains("3 records were written to the file"),
        "log: {log}"
    );

    // Vérifier le fichier CSV
    let content = std::fs::read_to_string(&out_path).unwrap();
    // Doit contenir les en-têtes
    assert!(content.contains("name"), "content: {content}");
    assert!(content.contains("age"), "content: {content}");
    // Doit contenir les données
    assert!(content.contains("Alice"), "content: {content}");
    assert!(content.contains("Bob"), "content: {content}");
}

#[test]
fn execute_export_tab_separated() {
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("out.tsv");

    let work_dir = dir.path().join("work");
    std::fs::create_dir(&work_dir).unwrap();
    let mut session = Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

    write_test_dataset(&mut session, "T");

    let ast = ExportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        outfile: out_path.to_string_lossy().into_owned(),
        dbms: ExportDbms::Tab,
        replace: false,
        delimiter: None,
    };
    execute(&ast, &mut session).unwrap();

    let content = std::fs::read_to_string(&out_path).unwrap();
    // En-tête doit contenir des tabulations
    let first_line = content.lines().next().unwrap();
    assert!(first_line.contains('\t'), "first line: {first_line}");
    assert!(content.contains("Alice"), "content: {content}");
}

#[test]
fn execute_export_dlm_pipe() {
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("out.txt");

    let work_dir = dir.path().join("work");
    std::fs::create_dir(&work_dir).unwrap();
    let mut session = Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

    write_test_dataset(&mut session, "T");

    let ast = ExportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        outfile: out_path.to_string_lossy().into_owned(),
        dbms: ExportDbms::Dlm,
        replace: false,
        delimiter: Some(b'|'),
    };
    execute(&ast, &mut session).unwrap();

    let content = std::fs::read_to_string(&out_path).unwrap();
    let first_line = content.lines().next().unwrap();
    assert!(first_line.contains('|'), "first line: {first_line}");
}

#[test]
fn execute_export_roundtrip_with_import() {
    // Test de round-trip : IMPORT → EXPORT → IMPORT et vérification
    let dir = tempfile::tempdir().unwrap();
    let csv_orig = dir.path().join("orig.csv");
    let csv_exported = dir.path().join("exported.csv");

    // Écrire un CSV source
    {
        let mut f = std::fs::File::create(&csv_orig).unwrap();
        writeln!(f, "x,y").unwrap();
        writeln!(f, "1.0,a").unwrap();
        writeln!(f, "2.0,b").unwrap();
        writeln!(f, "3.0,c").unwrap();
    }

    let work_dir = dir.path().join("work");
    std::fs::create_dir(&work_dir).unwrap();
    let mut session = Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

    // IMPORT
    let import_ast = crate::procs::import::ImportAst {
        datafile: csv_orig.to_string_lossy().into_owned(),
        out: DatasetRef {
            libref: Some("WORK".into()),
            name: "SRC".into(),
        },
        dbms: crate::procs::import::ImportDbms::Csv,
        replace: false,
        getnames: true,
        delimiter: None,
        guessingrows: None,
    };
    crate::procs::import::execute(&import_ast, &mut session).unwrap();

    // EXPORT
    let export_ast = ExportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "SRC".into(),
        }),
        outfile: csv_exported.to_string_lossy().into_owned(),
        dbms: ExportDbms::Csv,
        replace: false,
        delimiter: None,
    };
    execute(&export_ast, &mut session).unwrap();

    // Vérifier que le fichier exporté existe et contient les bonnes données
    let content = std::fs::read_to_string(&csv_exported).unwrap();
    assert!(content.contains("x"), "content: {content}");
    assert!(content.contains("y"), "content: {content}");
    // Les valeurs doivent être présentes (Polars peut utiliser 1.0 ou 1)
    assert!(content.contains('1'), "content: {content}");
    assert!(content.contains('a'), "content: {content}");

    // NOTE de log
    let log = session.log.into_string();
    assert!(
        log.contains("3 records were written to the file"),
        "log: {log}"
    );
}

#[test]
fn execute_export_last_dataset() {
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("out.csv");

    let work_dir = dir.path().join("work");
    std::fs::create_dir(&work_dir).unwrap();
    let mut session = Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

    write_test_dataset(&mut session, "LAST");

    // data= absent → utilise _LAST_
    let ast = ExportAst {
        data: None,
        outfile: out_path.to_string_lossy().into_owned(),
        dbms: ExportDbms::Csv,
        replace: false,
        delimiter: None,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("3 records were written"), "log: {log}");
}

#[test]
fn execute_export_invalid_path_errors() {
    let mut session = make_session();
    // Pas de dataset WORK
    let ast = ExportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "NONEXISTENT".into(),
        }),
        outfile: "/tmp/out.csv".into(),
        dbms: ExportDbms::Csv,
        replace: false,
        delimiter: None,
    };
    let result = execute(&ast, &mut session);
    assert!(result.is_err());
}

#[test]
fn resolve_separator_csv() {
    let ast = ExportAst {
        data: None,
        outfile: String::new(),
        dbms: ExportDbms::Csv,
        replace: false,
        delimiter: None,
    };
    assert_eq!(resolve_separator(&ast), b',');
}

#[test]
fn resolve_separator_tab() {
    let ast = ExportAst {
        data: None,
        outfile: String::new(),
        dbms: ExportDbms::Tab,
        replace: false,
        delimiter: None,
    };
    assert_eq!(resolve_separator(&ast), b'\t');
}

#[test]
fn resolve_separator_dlm_default_space() {
    let ast = ExportAst {
        data: None,
        outfile: String::new(),
        dbms: ExportDbms::Dlm,
        replace: false,
        delimiter: None,
    };
    assert_eq!(resolve_separator(&ast), b' ');
}

#[test]
fn resolve_separator_dlm_with_semicolon() {
    let ast = ExportAst {
        data: None,
        outfile: String::new(),
        dbms: ExportDbms::Dlm,
        replace: false,
        delimiter: Some(b';'),
    };
    assert_eq!(resolve_separator(&ast), b';');
}
