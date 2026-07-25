use super::*;
use crate::session::Session;
use crate::source::SourceFile;
use std::io::Write;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn parse_import_src(src: &str) -> Result<ImportAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "import"
    parse(&mut ts)
}

// --- Tests du parser ---

#[test]
fn parse_import_csv_minimal() {
    let ast = parse_import_src(
        "proc import datafile='/tmp/x.csv' out=work.myds dbms=csv; run;",
    )
    .unwrap();
    assert_eq!(ast.datafile, "/tmp/x.csv");
    assert_eq!(ast.out.name.to_uppercase(), "MYDS");
    assert_eq!(ast.dbms, ImportDbms::Csv);
    assert!(ast.getnames);
    assert!(!ast.replace);
}

#[test]
fn parse_import_tab_with_replace() {
    let ast = parse_import_src(
        "proc import datafile='data.txt' out=work.t dbms=TAB replace; run;",
    )
    .unwrap();
    assert_eq!(ast.dbms, ImportDbms::Tab);
    assert!(ast.replace);
}

#[test]
fn parse_import_getnames_no() {
    let ast = parse_import_src(
        "proc import datafile='x.csv' out=work.t dbms=csv; getnames=no; run;",
    )
    .unwrap();
    assert!(!ast.getnames);
}

#[test]
fn parse_import_delimiter_in_body() {
    let ast = parse_import_src(
        "proc import datafile='x.txt' out=work.t dbms=dlm; delimiter='|'; run;",
    )
    .unwrap();
    assert_eq!(ast.dbms, ImportDbms::Dlm);
    assert_eq!(ast.delimiter, Some(b'|'));
}

#[test]
fn parse_import_guessingrows_ignored() {
    let ast = parse_import_src(
        "proc import datafile='x.csv' out=work.t dbms=csv; guessingrows=200; run;",
    )
    .unwrap();
    assert_eq!(ast.guessingrows, Some(200));
}

#[test]
fn parse_import_missing_datafile_errors() {
    let result = parse_import_src("proc import out=work.t dbms=csv; run;");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("DATAFILE="), "msg: {msg}");
}

#[test]
fn parse_import_missing_out_errors() {
    let result = parse_import_src("proc import datafile='x.csv' dbms=csv; run;");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("OUT="), "msg: {msg}");
}

#[test]
fn parse_import_xlsx_deferred_error() {
    let result = parse_import_src(
        "proc import datafile='x.xlsx' out=work.t dbms=xlsx; run;",
    );
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("not yet implemented"),
        "expected deferral message, got: {msg}"
    );
    assert!(msg.contains("XLSX"), "msg: {msg}");
}

#[test]
fn parse_import_excel_deferred_error() {
    let result = parse_import_src(
        "proc import datafile='x.xlsx' out=work.t dbms=excel; run;",
    );
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("not yet implemented"), "msg: {msg}");
}

// --- Tests d'exécution ---

fn write_csv(path: &std::path::Path, content: &str) {
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
}

#[test]
fn execute_import_csv_basic() {
    let dir = tempfile::tempdir().unwrap();
    let csv_path = dir.path().join("test.csv");
    write_csv(
        &csv_path,
        "name,age,score\nAlice,30,95.5\nBob,25,88.0\nCarol,35,72.3\n",
    );

    let mut session = make_session();
    let ast = ImportAst {
        datafile: csv_path.to_string_lossy().into_owned(),
        out: DatasetRef {
            libref: Some("WORK".into()),
            name: "mydata".into(),
        },
        dbms: ImportDbms::Csv,
        replace: false,
        getnames: true,
        delimiter: None,
        guessingrows: None,
    };
    execute(&ast, &mut session).unwrap();

    // Vérifier la NOTE dans le log
    let log = session.log.into_string();
    assert!(
        log.contains("The data set WORK.MYDATA has 3 observations and 3 variables."),
        "log: {log}"
    );

    // Vérifier _LAST_
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.MYDATA"));

    // Re-lire le dataset et vérifier les colonnes
    // On vérifie juste que _LAST_ et la NOTE sont corrects.
}

#[test]
fn execute_import_csv_values_correct() {
    let dir = tempfile::tempdir().unwrap();
    let csv_path = dir.path().join("vals.csv");
    write_csv(&csv_path, "x,y\n1.0,a\n2.0,b\n3.0,c\n");

    // Créer une session pointant le WORK vers un répertoire connu.
    let work_dir = dir.path().join("work");
    std::fs::create_dir(&work_dir).unwrap();
    let mut session =
        Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

    let ast = ImportAst {
        datafile: csv_path.to_string_lossy().into_owned(),
        out: DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        dbms: ImportDbms::Csv,
        replace: false,
        getnames: true,
        delimiter: None,
        guessingrows: None,
    };
    execute(&ast, &mut session).unwrap();

    // Re-lire le dataset depuis le même WORK
    let provider = session.libs.get("WORK").unwrap();
    let (ds, _) = provider.read("T").unwrap();
    assert_eq!(ds.n_obs(), 3);
    assert_eq!(ds.n_vars(), 2);

    let x_col = ds.df.column("x").unwrap();
    let x = x_col.f64().unwrap();
    assert_eq!(x.get(0), Some(1.0));
    assert_eq!(x.get(1), Some(2.0));
    assert_eq!(x.get(2), Some(3.0));

    let y_col = ds.df.column("y").unwrap();
    let y = y_col.str().unwrap();
    assert_eq!(y.get(0), Some("a"));
    assert_eq!(y.get(1), Some("b"));
    assert_eq!(y.get(2), Some("c"));
}

#[test]
fn execute_import_tab_separated() {
    let dir = tempfile::tempdir().unwrap();
    let tsv_path = dir.path().join("test.tsv");
    write_csv(&tsv_path, "a\tb\n10\t20\n30\t40\n");

    let work_dir = dir.path().join("work");
    std::fs::create_dir(&work_dir).unwrap();
    let mut session =
        Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

    let ast = ImportAst {
        datafile: tsv_path.to_string_lossy().into_owned(),
        out: DatasetRef {
            libref: Some("WORK".into()),
            name: "TAB".into(),
        },
        dbms: ImportDbms::Tab,
        replace: false,
        getnames: true,
        delimiter: None,
        guessingrows: None,
    };
    execute(&ast, &mut session).unwrap();

    let provider = session.libs.get("WORK").unwrap();
    let (ds, _) = provider.read("TAB").unwrap();
    assert_eq!(ds.n_obs(), 2);
    assert_eq!(ds.n_vars(), 2);

    let a = ds.df.column("a").unwrap().f64().unwrap();
    assert_eq!(a.get(0), Some(10.0));
    assert_eq!(a.get(1), Some(30.0));
}

#[test]
fn execute_import_dlm_pipe() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.txt");
    write_csv(&path, "x|y\n1|hello\n2|world\n");

    let work_dir = dir.path().join("work");
    std::fs::create_dir(&work_dir).unwrap();
    let mut session =
        Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

    let ast = ImportAst {
        datafile: path.to_string_lossy().into_owned(),
        out: DatasetRef {
            libref: Some("WORK".into()),
            name: "PIPE".into(),
        },
        dbms: ImportDbms::Dlm,
        replace: false,
        getnames: true,
        delimiter: Some(b'|'),
        guessingrows: None,
    };
    execute(&ast, &mut session).unwrap();

    let provider = session.libs.get("WORK").unwrap();
    let (ds, _) = provider.read("PIPE").unwrap();
    assert_eq!(ds.n_obs(), 2);
    assert_eq!(ds.n_vars(), 2);

    let y = ds.df.column("y").unwrap().str().unwrap();
    assert_eq!(y.get(0), Some("hello"));
    assert_eq!(y.get(1), Some("world"));
}

#[test]
fn execute_import_getnames_no_produces_var_n() {
    let dir = tempfile::tempdir().unwrap();
    let csv_path = dir.path().join("noheader.csv");
    write_csv(&csv_path, "Alice,30\nBob,25\n");

    let work_dir = dir.path().join("work");
    std::fs::create_dir(&work_dir).unwrap();
    let mut session =
        Session::new(Some(work_dir.clone()), PathBuf::from("."), true).unwrap();

    let ast = ImportAst {
        datafile: csv_path.to_string_lossy().into_owned(),
        out: DatasetRef {
            libref: Some("WORK".into()),
            name: "NOHEAD".into(),
        },
        dbms: ImportDbms::Csv,
        replace: false,
        getnames: false,
        delimiter: None,
        guessingrows: None,
    };
    execute(&ast, &mut session).unwrap();

    let provider = session.libs.get("WORK").unwrap();
    let (ds, _) = provider.read("NOHEAD").unwrap();
    assert_eq!(ds.n_vars(), 2);
    // Les noms doivent être VAR1, VAR2
    let names: Vec<&str> = ds.df.get_column_names().into_iter().map(|s| s.as_str()).collect();
    assert_eq!(names, vec!["VAR1", "VAR2"], "column names: {names:?}");
}

#[test]
fn execute_import_sets_last_dataset() {
    let dir = tempfile::tempdir().unwrap();
    let csv_path = dir.path().join("a.csv");
    write_csv(&csv_path, "x\n1\n2\n");

    let work_dir = dir.path().join("work");
    std::fs::create_dir(&work_dir).unwrap();
    let mut session =
        Session::new(Some(work_dir), PathBuf::from("."), true).unwrap();

    let ast = ImportAst {
        datafile: csv_path.to_string_lossy().into_owned(),
        out: DatasetRef {
            libref: None,
            name: "LAST".into(),
        },
        dbms: ImportDbms::Csv,
        replace: false,
        getnames: true,
        delimiter: None,
        guessingrows: None,
    };
    execute(&ast, &mut session).unwrap();
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.LAST"));
}

#[test]
fn execute_import_nonexistent_file_errors() {
    let mut session = make_session();
    let ast = ImportAst {
        datafile: "/nonexistent/path/missing.csv".into(),
        out: DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        dbms: ImportDbms::Csv,
        replace: false,
        getnames: true,
        delimiter: None,
        guessingrows: None,
    };
    let result = execute(&ast, &mut session);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("PROC IMPORT"), "msg: {msg}");
}

#[test]
fn resolve_separator_csv() {
    let ast = ImportAst {
        datafile: String::new(),
        out: DatasetRef { libref: None, name: "t".into() },
        dbms: ImportDbms::Csv,
        replace: false,
        getnames: true,
        delimiter: None,
        guessingrows: None,
    };
    assert_eq!(resolve_separator(&ast).unwrap(), b',');
}

#[test]
fn resolve_separator_tab() {
    let ast = ImportAst {
        datafile: String::new(),
        out: DatasetRef { libref: None, name: "t".into() },
        dbms: ImportDbms::Tab,
        replace: false,
        getnames: true,
        delimiter: None,
        guessingrows: None,
    };
    assert_eq!(resolve_separator(&ast).unwrap(), b'\t');
}

#[test]
fn resolve_separator_dlm_default_space() {
    let ast = ImportAst {
        datafile: String::new(),
        out: DatasetRef { libref: None, name: "t".into() },
        dbms: ImportDbms::Dlm,
        replace: false,
        getnames: true,
        delimiter: None,
        guessingrows: None,
    };
    assert_eq!(resolve_separator(&ast).unwrap(), b' ');
}

#[test]
fn resolve_separator_dlm_with_delimiter() {
    let ast = ImportAst {
        datafile: String::new(),
        out: DatasetRef { libref: None, name: "t".into() },
        dbms: ImportDbms::Dlm,
        replace: false,
        getnames: true,
        delimiter: Some(b';'),
        guessingrows: None,
    };
    assert_eq!(resolve_separator(&ast).unwrap(), b';');
}

#[test]
fn parse_delimiter_char_single() {
    let span = crate::token::Span::default();
    assert_eq!(parse_delimiter_char(",", span).unwrap(), Some(b','));
    assert_eq!(parse_delimiter_char("|", span).unwrap(), Some(b'|'));
    assert_eq!(parse_delimiter_char(";", span).unwrap(), Some(b';'));
}

#[test]
fn parse_delimiter_char_mnemonic_tab() {
    let span = crate::token::Span::default();
    assert_eq!(parse_delimiter_char("TAB", span).unwrap(), Some(b'\t'));
    assert_eq!(parse_delimiter_char("tab", span).unwrap(), Some(b'\t'));
}

#[test]
fn parse_delimiter_char_mnemonic_space() {
    let span = crate::token::Span::default();
    assert_eq!(parse_delimiter_char("SPACE", span).unwrap(), Some(b' '));
}
