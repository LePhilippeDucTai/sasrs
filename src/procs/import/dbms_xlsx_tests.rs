//! Tests `dbms_xlsx_*` — PROC IMPORT/EXPORT `DBMS=XLSX|EXCEL` (J08-P1).
//!
//! Les classeurs de test sont générés par le writer XLSX du dépôt
//! (`output::xlsx`) — leur VALIDITÉ de fichier est attestée par calamine
//! (parseur indépendant, l'oracle externe de cette unité) : si calamine relit
//! les valeurs, dates et missings attendus, l'aller-retour est prouvé.
//!
//! Ancres d'époque indépendantes du code sous test : sérial Excel 25569 =
//! 1970-01-01 = jour SAS 3653 (documenté Excel/SAS 9.4).

use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::output::xlsx::{XlsxCell, XlsxSheet, xlsx_build_typed};
use crate::session::Session;
use crate::value::VarType;
use polars::prelude::df;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

type Grid = Vec<Vec<Option<XlsxCell>>>;

fn write_workbook(path: &std::path::Path, sheets: &[(&str, &[String], &Grid)]) {
    let sheets: Vec<XlsxSheet> = sheets
        .iter()
        .map(|(name, headers, rows)| XlsxSheet {
            name: (*name).to_string(),
            headers: headers.to_vec(),
            rows: rows.to_vec(),
        })
        .collect();
    std::fs::write(path, xlsx_build_typed(&sheets)).unwrap();
}

fn session_in(dir: &std::path::Path) -> Session {
    let work_dir = dir.join("work");
    std::fs::create_dir_all(&work_dir).unwrap();
    Session::new(Some(work_dir), PathBuf::from("."), true).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn xlsx_ast(
    path: &std::path::Path,
    out: &str,
    sheet: Option<&str>,
    range: Option<&str>,
    getnames: bool,
    guessingrows: Option<usize>,
) -> ImportAst {
    ImportAst {
        datafile: path.to_string_lossy().into_owned(),
        out: DatasetRef {
            libref: Some("WORK".into()),
            name: out.into(),
        },
        dbms: ImportDbms::Xlsx,
        replace: true,
        getnames,
        delimiter: None,
        guessingrows,
        sheet: sheet.map(str::to_string),
        range: range.map(str::to_string),
    }
}

fn read_back(session: &mut Session, name: &str) -> SasDataset {
    let provider = session.libs.get("WORK").unwrap();
    let (ds, _) = provider.read(name).unwrap();
    ds
}

// ---------------------------------------------------------------------------
// IMPORT : types SAS stricts, missings, dates
// ---------------------------------------------------------------------------

#[test]
fn dbms_xlsx_import_basic_types() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("basic.xlsx");
    write_workbook(
        &path,
        &[(
            "Data",
            &["name".into(), "age".into(), "score".into()],
            &vec![
                vec![
                    Some(XlsxCell::Str("Alice".into())),
                    Some(XlsxCell::Num(30.0)),
                    Some(XlsxCell::Num(95.5)),
                ],
                vec![
                    Some(XlsxCell::Str("Bob".into())),
                    Some(XlsxCell::Num(25.0)),
                    Some(XlsxCell::Num(88.0)),
                ],
            ],
        )],
    );

    let mut session = session_in(dir.path());
    execute(&xlsx_ast(&path, "T", None, None, true, None), &mut session).unwrap();

    let ds = read_back(&mut session, "T");
    assert_eq!(ds.n_obs(), 2);
    assert_eq!(ds.n_vars(), 3);
    // Types SAS stricts : name caractère, age/score numériques.
    assert_eq!(ds.vars[0].ty, VarType::Char);
    assert_eq!(ds.vars[1].ty, VarType::Num);
    assert_eq!(ds.vars[2].ty, VarType::Num);

    let name = ds.df.column("name").unwrap().as_materialized_series();
    let name = name.str().unwrap();
    assert_eq!(name.get(0), Some("Alice"));
    assert_eq!(name.get(1), Some("Bob"));
    let age = ds.df.column("age").unwrap().as_materialized_series();
    let age = age.f64().unwrap();
    assert_eq!(age.get(0), Some(30.0));
    assert_eq!(age.get(1), Some(25.0));

    let log = session.log.into_string();
    assert!(
        log.contains("The data set WORK.T has 2 observations and 3 variables."),
        "log: {log}"
    );
}

#[test]
fn dbms_xlsx_import_missing_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.xlsx");
    write_workbook(
        &path,
        &[(
            "Data",
            &["name".into(), "age".into()],
            &vec![
                vec![None, Some(XlsxCell::Num(30.0))],
                vec![Some(XlsxCell::Str("Bob".into())), None],
            ],
        )],
    );

    let mut session = session_in(dir.path());
    execute(&xlsx_ast(&path, "M", None, None, true, None), &mut session).unwrap();

    let ds = read_back(&mut session, "M");
    let name = ds.df.column("name").unwrap().as_materialized_series();
    let name = name.str().unwrap();
    assert_eq!(name.get(0), None, "missing caractère préservé");
    assert_eq!(name.get(1), Some("Bob"));
    let age = ds.df.column("age").unwrap().as_materialized_series();
    let age = age.f64().unwrap();
    assert_eq!(age.get(0), Some(30.0));
    assert_eq!(age.get(1), None, "missing numérique préservé");
}

#[test]
fn dbms_xlsx_import_date_serial_to_sas_1960() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dates.xlsx");
    // Ancre indépendante : 25569 = 1970-01-01 = jour SAS 3653.
    write_workbook(
        &path,
        &[(
            "Data",
            &["d".into()],
            &vec![
                vec![Some(XlsxCell::Date(25569.0))],
                vec![Some(XlsxCell::Date(21916.0))], // 1960-01-01 → SAS 0
            ],
        )],
    );

    let mut session = session_in(dir.path());
    execute(&xlsx_ast(&path, "D", None, None, true, None), &mut session).unwrap();

    let ds = read_back(&mut session, "D");
    assert_eq!(ds.vars[0].ty, VarType::Num);
    assert_eq!(ds.vars[0].format.as_deref(), Some("DATE9."), "format SAS");
    let d = ds.df.column("d").unwrap().as_materialized_series();
    let d = d.f64().unwrap();
    assert_eq!(d.get(0), Some(3653.0), "1970-01-01 → SAS 3653");
    assert_eq!(d.get(1), Some(0.0), "1960-01-01 → SAS 0");
}

#[test]
fn dbms_xlsx_import_datetime_serial_to_sas_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dtx.xlsx");
    // 25569.5 = 1970-01-01 12:00:00 → SAS 3653*86400 + 43200.
    write_workbook(
        &path,
        &[(
            "Data",
            &["dt".into()],
            &vec![vec![Some(XlsxCell::Date(25569.5))]],
        )],
    );

    let mut session = session_in(dir.path());
    execute(&xlsx_ast(&path, "DT", None, None, true, None), &mut session).unwrap();

    let ds = read_back(&mut session, "DT");
    assert_eq!(ds.vars[0].format.as_deref(), Some("DATETIME20."));
    let dt = ds.df.column("dt").unwrap().as_materialized_series();
    let dt = dt.f64().unwrap();
    let expected = 3653.0 * 86400.0 + 43200.0;
    assert!(
        (dt.get(0).unwrap() - expected).abs() < 1e-3,
        "{:?}",
        dt.get(0)
    );
}

#[test]
fn dbms_xlsx_import_getnames_no_var_n() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nohead.xlsx");
    write_workbook(
        &path,
        &[(
            "Data",
            &["c1".into(), "c2".into()], // en-têtes ignorés par GETNAMES=NO
            &vec![
                vec![
                    Some(XlsxCell::Str("Alice".into())),
                    Some(XlsxCell::Num(30.0)),
                ],
                vec![Some(XlsxCell::Str("Bob".into())), Some(XlsxCell::Num(25.0))],
            ],
        )],
    );

    let mut session = session_in(dir.path());
    execute(
        &xlsx_ast(&path, "NH", None, None, false, None),
        &mut session,
    )
    .unwrap();

    let ds = read_back(&mut session, "NH");
    assert_eq!(ds.n_obs(), 3, "la ligne d'en-têtes devient une observation");
    let names: Vec<&str> = ds
        .df
        .get_column_names()
        .into_iter()
        .map(|s| s.as_str())
        .collect();
    assert_eq!(names, vec!["VAR1", "VAR2"], "column names: {names:?}");
}

#[test]
fn dbms_xlsx_import_mixed_column_is_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.xlsx");
    write_workbook(
        &path,
        &[(
            "Data",
            &["x".into()],
            &vec![
                vec![Some(XlsxCell::Num(42.0))],
                vec![Some(XlsxCell::Str("abc".into()))],
            ],
        )],
    );

    let mut session = session_in(dir.path());
    execute(&xlsx_ast(&path, "MX", None, None, true, None), &mut session).unwrap();

    let ds = read_back(&mut session, "MX");
    assert_eq!(ds.vars[0].ty, VarType::Char, "colonne mixte → caractère");
    let x = ds.df.column("x").unwrap().as_materialized_series();
    let x = x.str().unwrap();
    assert_eq!(x.get(0), Some("42"), "42.0 rendu sans .0");
    assert_eq!(x.get(1), Some("abc"));
}

#[test]
fn dbms_xlsx_import_guessingrows_window() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guess.xlsx");
    write_workbook(
        &path,
        &[(
            "Data",
            &["x".into()],
            &vec![
                vec![Some(XlsxCell::Num(42.0))],
                vec![Some(XlsxCell::Num(43.0))],
                vec![Some(XlsxCell::Str("late string".into()))],
            ],
        )],
    );

    // GUESSINGROWS=2 : la fenêtre ne voit que des nombres → numérique, la
    // chaîne tardive devient missing (le type est figé par la fenêtre).
    let mut session = session_in(dir.path());
    execute(
        &xlsx_ast(&path, "G2", None, None, true, Some(2)),
        &mut session,
    )
    .unwrap();
    let ds = read_back(&mut session, "G2");
    assert_eq!(
        ds.vars[0].ty,
        VarType::Num,
        "fenêtre de 2 lignes → numérique"
    );
    let x = ds.df.column("x").unwrap().as_materialized_series();
    let x = x.f64().unwrap();
    assert_eq!(x.get(0), Some(42.0));
    assert_eq!(x.get(2), None, "chaîne hors fenêtre → missing");

    // MAX (défaut) : toute la colonne scannée → caractère.
    let mut session2 = session_in(dir.path());
    execute(
        &xlsx_ast(&path, "GMAX", None, None, true, None),
        &mut session2,
    )
    .unwrap();
    let ds2 = read_back(&mut session2, "GMAX");
    assert_eq!(ds2.vars[0].ty, VarType::Char);
}

// ---------------------------------------------------------------------------
// IMPORT : SHEET= / RANGE=
// ---------------------------------------------------------------------------

#[test]
fn dbms_xlsx_import_sheet_selection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("multi.xlsx");
    write_workbook(
        &path,
        &[
            (
                "First",
                &["x".into()],
                &vec![vec![Some(XlsxCell::Num(1.0))]],
            ),
            (
                "Second",
                &["y".into()],
                &vec![vec![Some(XlsxCell::Num(2.0))]],
            ),
        ],
    );

    // Par nom exact.
    let mut session = session_in(dir.path());
    execute(
        &xlsx_ast(&path, "S1", Some("Second"), None, true, None),
        &mut session,
    )
    .unwrap();
    let ds = read_back(&mut session, "S1");
    assert_eq!(
        ds.df
            .column("y")
            .unwrap()
            .as_materialized_series()
            .f64()
            .unwrap()
            .get(0),
        Some(2.0)
    );

    // Par numéro 1-based.
    let mut session2 = session_in(dir.path());
    execute(
        &xlsx_ast(&path, "S2", Some("2"), None, true, None),
        &mut session2,
    )
    .unwrap();
    let ds2 = read_back(&mut session2, "S2");
    assert!(ds2.df.column("y").is_ok(), "SHEET=2 doit ouvrir Second");

    // Défaut : première feuille.
    let mut session3 = session_in(dir.path());
    execute(
        &xlsx_ast(&path, "S3", None, None, true, None),
        &mut session3,
    )
    .unwrap();
    let ds3 = read_back(&mut session3, "S3");
    assert!(ds3.df.column("x").is_ok(), "défaut = première feuille");

    // Feuille inconnue → ERROR explicite.
    let mut session4 = session_in(dir.path());
    let err = execute(
        &xlsx_ast(&path, "S4", Some("Nope"), None, true, None),
        &mut session4,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(err.contains("not found"), "err: {err}");
}

#[test]
fn dbms_xlsx_import_range_subset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("range.xlsx");
    write_workbook(
        &path,
        &[(
            "Data",
            &["a".into(), "b".into(), "c".into()],
            &vec![
                vec![
                    Some(XlsxCell::Num(1.0)),
                    Some(XlsxCell::Num(10.0)),
                    Some(XlsxCell::Num(100.0)),
                ],
                vec![
                    Some(XlsxCell::Num(2.0)),
                    Some(XlsxCell::Num(20.0)),
                    Some(XlsxCell::Num(200.0)),
                ],
            ],
        )],
    );

    // B1:C2 : colonnes b et c seulement.
    let mut session = session_in(dir.path());
    execute(
        &xlsx_ast(&path, "R1", None, Some("B1:C2"), true, None),
        &mut session,
    )
    .unwrap();
    let ds = read_back(&mut session, "R1");
    let names: Vec<&str> = ds
        .df
        .get_column_names()
        .into_iter()
        .map(|s| s.as_str())
        .collect();
    assert_eq!(names, vec!["b", "c"], "columns: {names:?}");
    let b = ds
        .df
        .column("b")
        .unwrap()
        .as_materialized_series()
        .f64()
        .unwrap();
    assert_eq!(b.get(0), Some(10.0));

    // Préfixe de feuille dans RANGE= : prime sur tout.
    let mut session2 = session_in(dir.path());
    execute(
        &xlsx_ast(
            &path,
            "R2",
            Some("WrongSheet"),
            Some("Data$A1:B2"),
            true,
            None,
        ),
        &mut session2,
    )
    .unwrap();
    let ds2 = read_back(&mut session2, "R2");
    assert_eq!(ds2.n_vars(), 2);
    assert!(ds2.df.column("a").is_ok());

    // Plage nommée → ERROR explicite (pas de repli silencieux).
    let mut session3 = session_in(dir.path());
    let err = execute(
        &xlsx_ast(&path, "R3", None, Some("MyNamedRange"), true, None),
        &mut session3,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(err.contains("A1-style"), "err: {err}");
}

// ---------------------------------------------------------------------------
// EXPORT → IMPORT : aller-retour sans perte (J08-P1)
// ---------------------------------------------------------------------------

#[test]
fn dbms_xlsx_roundtrip_export_import_lossless() {
    let dir = tempfile::tempdir().unwrap();
    let xlsx_path = dir.path().join("rt.xlsx");

    let mut session = session_in(dir.path());

    // Dataset source : caractère + missings + numériques + date + datetime.
    let df = df![
        "name" => [Some("Alice"), None, Some("Carol")],
        "score" => [Some(95.5_f64), None, Some(72.3)],
        "d" => [Some(23450.0_f64), None, Some(23376.0)], // DATE9.
        "dt" => [Some(3653.0_f64 * 86400.0 + 43200.0), None, Some(23376.0 * 86400.0)],
    ]
    .unwrap();
    let var = |name: &str, ty: VarType, format: Option<&str>| VarMeta {
        name: name.into(),
        ty,
        length: if ty == VarType::Char { 5 } else { 8 },
        format: format.map(str::to_string),
        label: None,
        informat: None,
    };
    let ds = SasDataset {
        df,
        vars: vec![
            var("name", VarType::Char, None),
            var("score", VarType::Num, None),
            var("d", VarType::Num, Some("DATE9.")),
            var("dt", VarType::Num, Some("DATETIME20.")),
        ],
    };
    session.libs.get("WORK").unwrap().write("SRC", &ds).unwrap();

    // EXPORT DBMS=XLSX.
    let export_ast = crate::procs::export::ExportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "SRC".into(),
        }),
        outfile: xlsx_path.to_string_lossy().into_owned(),
        dbms: crate::procs::export::ExportDbms::Xlsx,
        replace: true,
        delimiter: None,
        sheet: Some("Exported".into()),
    };
    crate::procs::export::execute(&export_ast, &mut session).unwrap();

    // IMPORT DBMS=XLSX (feuille nommée).
    execute(
        &xlsx_ast(&xlsx_path, "BACK", Some("Exported"), None, true, None),
        &mut session,
    )
    .unwrap();

    let back = read_back(&mut session, "BACK");
    let log = session.log.into_string();
    assert!(
        log.contains("3 records were written to the file"),
        "log: {log}"
    );
    assert_eq!(back.n_obs(), 3);
    assert_eq!(back.n_vars(), 4);

    // Types.
    assert_eq!(back.vars[0].ty, VarType::Char);
    for v in &back.vars[1..] {
        assert_eq!(v.ty, VarType::Num);
    }

    // Valeurs caractère + missings.
    let name = back.df.column("name").unwrap().as_materialized_series();
    let name = name.str().unwrap();
    assert_eq!(name.get(0), Some("Alice"));
    assert_eq!(name.get(1), None, "missing caractère traversé");
    assert_eq!(name.get(2), Some("Carol"));

    // Valeurs numériques + missings.
    let score = back.df.column("score").unwrap().as_materialized_series();
    let score = score.f64().unwrap();
    assert_eq!(score.get(0), Some(95.5));
    assert_eq!(score.get(1), None, "missing numérique traversé");
    assert_eq!(score.get(2), Some(72.3));

    // Dates : valeurs ET formats.
    assert_eq!(back.vars[2].format.as_deref(), Some("DATE9."));
    let d = back.df.column("d").unwrap().as_materialized_series();
    let d = d.f64().unwrap();
    assert_eq!(d.get(0), Some(23450.0), "date SAS identique");
    assert_eq!(d.get(1), None, "missing date traversé");
    assert_eq!(d.get(2), Some(23376.0));

    assert_eq!(back.vars[3].format.as_deref(), Some("DATETIME20."));
    let dt = back.df.column("dt").unwrap().as_materialized_series();
    let dt = dt.f64().unwrap();
    let expected = 3653.0 * 86400.0 + 43200.0;
    assert!(
        (dt.get(0).unwrap() - expected).abs() < 1e-3,
        "{:?}",
        dt.get(0)
    );
    assert_eq!(dt.get(1), None);
    assert_eq!(dt.get(2), Some(23376.0 * 86400.0));
}

#[test]
fn dbms_xlsx_export_default_sheet_is_dataset_name() {
    let dir = tempfile::tempdir().unwrap();
    let xlsx_path = dir.path().join("sheet.xlsx");

    let mut session = session_in(dir.path());
    let df = df!["x" => [1.0_f64, 2.0]].unwrap();
    let vars = vec![VarMeta {
        name: "x".into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
        informat: None,
    }];
    let ds = SasDataset { df, vars };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("MYTAB", &ds)
        .unwrap();

    let export_ast = crate::procs::export::ExportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "MYTAB".into(),
        }),
        outfile: xlsx_path.to_string_lossy().into_owned(),
        dbms: crate::procs::export::ExportDbms::Xlsx,
        replace: true,
        delimiter: None,
        sheet: None,
    };
    crate::procs::export::execute(&export_ast, &mut session).unwrap();

    // La feuille par défaut porte le nom du dataset.
    execute(
        &xlsx_ast(&xlsx_path, "T", Some("MYTAB"), None, true, None),
        &mut session,
    )
    .unwrap();
    let back = read_back(&mut session, "T");
    let x = back
        .df
        .column("x")
        .unwrap()
        .as_materialized_series()
        .f64()
        .unwrap();
    assert_eq!(x.get(0), Some(1.0));
    assert_eq!(x.get(1), Some(2.0));
}

#[test]
fn dbms_xlsx_import_open_error_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("absent.xlsx");
    let mut session = session_in(dir.path());
    let err = execute(&xlsx_ast(&path, "E", None, None, true, None), &mut session)
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("cannot open"), "err: {err}");
}
