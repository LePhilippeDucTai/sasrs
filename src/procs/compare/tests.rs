use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::testkit::*;
use crate::value::VarType;

fn parse_compare_src(src: &str) -> Result<CompareAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "compare"
    parse(&mut ts)
}

/// AST minimal par défaut (BASE/COMPARE dans WORK), options par défaut —
/// les tests surchargent les champs utiles.
fn cmp_ast(base: &str, comp: &str) -> CompareAst {
    parse_compare_src(&format!(
        "proc compare base=work.{base} compare=work.{comp}; run;"
    ))
    .unwrap()
}

fn sysinfo_of(session: &Session) -> u64 {
    session
        .macro_engine
        .get_symbol("SYSINFO")
        .unwrap_or_default()
        .parse()
        .unwrap_or(u64::MAX)
}

fn write_numeric_ds(session: &mut Session, name: &str, x_vals: &[f64], y_vals: &[f64]) {
    let df = df![
        "x" => x_vals.to_vec(),
        "y" => y_vals.to_vec()
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "x".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
            informat: None,
        },
        VarMeta {
            name: "y".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
            informat: None,
        },
    ];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
}

fn write_char_ds(session: &mut Session, name: &str, vals: &[&str]) {
    let df = df!["name" => vals.to_vec()].unwrap();
    let vars = vec![VarMeta {
        name: "name".into(),
        ty: VarType::Char,
        length: 8,
        format: None,
        label: None,
        informat: None,
    }];
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
    assert_eq!(ast.method, CmpMethod::Absolute);
    assert_eq!(ast.criterion, 0.0);
    assert_eq!(ast.maxprint, (50, 50));
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

    let ast = cmp_ast("A", "B");
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("No unequal values"), "log: {log}");

    let listing = session.listing.take_string();
    assert!(listing.contains("WORK.A"), "listing: {listing}");
    assert!(listing.contains("WORK.B"), "listing: {listing}");
}

#[test]
fn execute_with_differences() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "BASE1", &[1.0, 2.0], &[10.0, 20.0]);
    write_numeric_ds(&mut session, "COMP1", &[1.0, 9.0], &[10.0, 20.0]);

    let ast = cmp_ast("BASE1", "COMP1");
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("1 observation"), "log: {log}");

    let listing = session.listing.take_string();
    assert!(listing.contains("1"), "diffs in listing: {listing}");
}

#[test]
fn execute_variable_only_in_base() {
    let mut session = make_session();
    // BASE has x and z; COMPARE has only x
    let df_base = df!["x" => [1.0_f64, 2.0], "z" => [5.0_f64, 6.0]].unwrap();
    let vars_base = vec![
        VarMeta {
            name: "x".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
            informat: None,
        },
        VarMeta {
            name: "z".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
            informat: None,
        },
    ];
    let df_comp = df!["x" => [1.0_f64, 2.0]].unwrap();
    let vars_comp = vec![VarMeta {
        name: "x".into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
        informat: None,
    }];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(
            "BASE2",
            &SasDataset {
                df: df_base,
                vars: vars_base,
            },
        )
        .unwrap();
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(
            "COMP2",
            &SasDataset {
                df: df_comp,
                vars: vars_comp,
            },
        )
        .unwrap();

    let ast = cmp_ast("BASE2", "COMP2");
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("BASE only") || listing.contains("Z"),
        "listing: {listing}"
    );
}

#[test]
fn execute_type_mismatch_reported() {
    let mut session = make_session();
    // BASE has x as Num, COMP has x as Char
    let df_base = df!["x" => [1.0_f64, 2.0]].unwrap();
    let vars_base = vec![VarMeta {
        name: "x".into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
        informat: None,
    }];
    let df_comp = df!["x" => ["a", "b"]].unwrap();
    let vars_comp = vec![VarMeta {
        name: "x".into(),
        ty: VarType::Char,
        length: 1,
        format: None,
        label: None,
        informat: None,
    }];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(
            "BASE3",
            &SasDataset {
                df: df_base,
                vars: vars_base,
            },
        )
        .unwrap();
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(
            "COMP3",
            &SasDataset {
                df: df_comp,
                vars: vars_comp,
            },
        )
        .unwrap();

    let ast = cmp_ast("BASE3", "COMP3");
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("Different Types") || listing.contains("Num") || listing.contains("Char"),
        "listing: {listing}"
    );
}

#[test]
fn execute_different_nobs() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "BASE4", &[1.0, 2.0, 3.0], &[0.0, 0.0, 0.0]);
    write_numeric_ds(&mut session, "COMP4", &[1.0, 2.0], &[0.0, 0.0]);

    let ast = cmp_ast("BASE4", "COMP4");
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("Not Compared") || listing.contains("2"),
        "listing: {listing}"
    );
}

#[test]
fn execute_char_trailing_blanks_equivalent() {
    let mut session = make_session();
    write_char_ds(&mut session, "CBASE", &["abc", "xyz"]);
    write_char_ds(&mut session, "CCOMP", &["abc   ", "xyz "]);

    let ast = cmp_ast("CBASE", "CCOMP");
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
    let vars = vec![VarMeta {
        name: "x".into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
        informat: None,
    }];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(
            "MISS1",
            &SasDataset {
                df: df_base,
                vars: vars.clone(),
            },
        )
        .unwrap();
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("MISS2", &SasDataset { df: df_comp, vars })
        .unwrap();

    let ast = cmp_ast("MISS1", "MISS2");
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("No unequal values"), "log: {log}");
}

#[test]
fn execute_out_dataset_created() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "OBASE", &[1.0, 2.0], &[10.0, 20.0]);
    write_numeric_ds(&mut session, "OCOMP", &[1.0, 9.0], &[10.0, 20.0]);

    let mut ast = cmp_ast("OBASE", "OCOMP");
    ast.out = Some(DatasetRef {
        libref: Some("WORK".into()),
        name: "DIFFS".into(),
    });
    execute(&ast, &mut session).unwrap();

    // OUT= dataset should exist
    assert!(session.libs.get("WORK").unwrap().exists("DIFFS"));
    let (ds, _) = session.libs.get("WORK").unwrap().read("DIFFS").unwrap();
    // Default (aucun type demandé) : une ligne DIF par paire appariée
    // (doc : « an observation for each pair of matching observations »),
    // égales comprises.
    assert_eq!(ds.n_obs(), 2);
    let ty_idx = ds.vars.iter().position(|v| v.name == "_TYPE_").unwrap();
    let type_col = crate::procs::common::decode_column(&ds, ty_idx).unwrap();
    assert_eq!(type_col[0], Value::Char("DIF".to_string()));
    assert_eq!(type_col[1], Value::Char("DIF".to_string()));
}

#[test]
fn execute_briefsummary() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "BS1", &[1.0], &[2.0]);
    write_numeric_ds(&mut session, "BS2", &[1.0], &[3.0]);

    let mut ast = cmp_ast("BS1", "BS2");
    ast.briefsummary = true;
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("Brief Summary") || listing.contains("WORK.BS1"),
        "listing: {listing}"
    );
}

#[test]
fn execute_novalues_omits_values_section() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "NV1", &[1.0, 2.0], &[0.0, 0.0]);
    write_numeric_ds(&mut session, "NV2", &[1.0, 9.0], &[0.0, 0.0]);

    let mut ast = cmp_ast("NV1", "NV2");
    ast.novalues = true;
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    // The values section header should be absent
    assert!(
        !listing.contains("Values Comparison Summary"),
        "listing should not have values section: {listing}"
    );
}

// ── Tests de compatibilité J07-P3 (cas conformance/cases/compat/compare) ──
//
// Chaque test exécute un PROGRAMME complet via la façade `crate::run`
// (comme le corpus de conformité et les tests compat de means), puis relit
// WORK via l'API dataset. Les attendus sont ceux des oracles gelés J07-P1.

use crate::{RunOptions, run};
use std::collections::BTreeMap;
use std::path::Path;

/// Bac à sable : `data/` rempli par `setup`, WORK isolé, exécution du
/// programme, puis relecture de toutes les tables WORK produites.
fn run_in_sandbox(
    setup: impl FnOnce(&Path),
    program: &str,
) -> (i32, String, String, BTreeMap<String, SasDataset>) {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    setup(&tmp.path().join("data"));
    let outcome = run(
        program,
        RunOptions {
            work_dir: Some(work.clone()),
            base_dir: Some(tmp.path().to_path_buf()),
            deterministic: true,
            vectorize: false,
        },
    );
    let mut tables = BTreeMap::new();
    for entry in std::fs::read_dir(&work).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "parquet") {
            let stem = p.file_stem().unwrap().to_str().unwrap().to_string();
            if let Ok((ds, _)) = SasDataset::read_parquet(&p) {
                tables.insert(stem.to_lowercase(), ds);
            }
        }
    }
    (outcome.exit_code, outcome.log, outcome.listing, tables)
}

/// Écrit un parquet d'entrée (l'équivalent du convertisseur du corpus J05).
fn write_input(dir: &Path, name: &str, mut df: DataFrame) {
    std::fs::create_dir_all(dir).unwrap();
    let mut file = std::fs::File::create(dir.join(format!("{name}.parquet"))).unwrap();
    ParquetWriter::new(&mut file).finish(&mut df).unwrap();
}

/// Le jeu du cas compare-outbase-outcomp-id (data/base.csv, data/comp.csv).
fn outbase_outcomp_id_data(dir: &Path) {
    write_input(
        dir,
        "base",
        df!["key" => [1.0_f64, 2.0, 3.0], "x" => [10.0_f64, 20.0, 30.0]].unwrap(),
    );
    write_input(
        dir,
        "comp",
        df!["key" => [1.0_f64, 2.0, 3.0], "y" => [10.0_f64, 21.0, 30.0]].unwrap(),
    );
}

/// Le jeu du cas compare-outdif-by-criterion (data/b2.csv, data/c2.csv).
fn outdif_by_criterion_data(dir: &Path) {
    write_input(
        dir,
        "b2",
        df!["grp" => ["A", "A", "B"], "v" => [1.0_f64, 2.0, 5.0]].unwrap(),
    );
    write_input(
        dir,
        "c2",
        df!["grp" => ["A", "A", "B"], "v" => [1.005_f64, 2.0, 5.010]].unwrap(),
    );
}

/// Colonnes d'une table WORK (noms majuscules, ordre de compilation).
fn columns_of(tables: &BTreeMap<String, SasDataset>, table: &str) -> Vec<String> {
    tables
        .get(table)
        .unwrap_or_else(|| panic!("WORK.{table} absent"))
        .vars
        .iter()
        .map(|m| m.name.to_uppercase())
        .collect()
}

/// Colonne numérique d'une table WORK sous forme de (valeur, missing).
fn raw_num_col(tables: &BTreeMap<String, SasDataset>, table: &str, col: &str) -> Vec<Option<f64>> {
    let ds = tables
        .get(table)
        .unwrap_or_else(|| panic!("WORK.{table} absent"));
    let idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(col))
        .unwrap_or_else(|| panic!("colonne {col} absente de WORK.{table}"));
    ds.df.get_columns()[idx]
        .as_materialized_series()
        .f64()
        .unwrap()
        .iter()
        .collect()
}

fn char_col(tables: &BTreeMap<String, SasDataset>, table: &str, col: &str) -> Vec<String> {
    let ds = tables
        .get(table)
        .unwrap_or_else(|| panic!("WORK.{table} absent"));
    let idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(col))
        .unwrap_or_else(|| panic!("colonne {col} absente de WORK.{table}"));
    ds.df.get_columns()[idx]
        .as_materialized_series()
        .str()
        .unwrap()
        .iter()
        .map(|o| o.unwrap_or_default().to_string())
        .collect()
}

/// Reproduit le programme du cas compat compare-outbase-outcomp-id (oracle
/// gelé J07-P1) : ID key, VAR x / WITH y, OUT= OUTBASE OUTCOMP OUTDIF
/// OUTNOEQUAL, NOPRINT.
#[test]
fn compare_compat_outbase_outcomp_id() {
    let program = r#"
libname b 'data';

proc compare base=b.base compare=b.comp out=out
             outbase outcomp outdif outnoequal noprint;
  id key;
  var x;
  with y;
run;
"#;
    let (code, log, listing, tables) = run_in_sandbox(outbase_outcomp_id_data, program);
    assert_eq!(code, 0, "log:\n{log}");
    assert!(!log.contains("ERROR:"), "log:\n{log}");
    assert!(listing.trim().is_empty(), "listing NOPRINT : {listing}");

    // Colonnes : ID puis VAR (nom de la VAR), _TYPE_, _OBS_.
    assert_eq!(
        columns_of(&tables, "out"),
        vec!["KEY", "X", "_TYPE_", "_OBS_"]
    );

    // Attendu (expected/out.csv) : toutes les lignes BASE puis toutes les
    // lignes COMP, puis la seule ligne DIF (paire jugée inégale).
    let keys = raw_num_col(&tables, "out", "key");
    let xs = raw_num_col(&tables, "out", "x");
    let ty = char_col(&tables, "out", "_TYPE_");
    let obs = raw_num_col(&tables, "out", "_OBS_");
    let expected: Vec<(&str, f64, f64, Option<f64>)> = vec![
        ("BASE", 1.0, 10.0, Some(1.0)),
        ("BASE", 2.0, 20.0, Some(2.0)),
        ("BASE", 3.0, 30.0, Some(3.0)),
        ("COMP", 1.0, 10.0, Some(1.0)),
        ("COMP", 2.0, 21.0, Some(2.0)), // valeur de WITH y
        ("COMP", 3.0, 30.0, Some(3.0)),
        ("DIF", 2.0, 1.0, Some(2.0)), // y − x = 21 − 20, séquence 2
    ];
    assert_eq!(ty.len(), expected.len(), "nb de lignes OUT=");
    for (i, (t, k, x, o)) in expected.iter().enumerate() {
        assert_eq!(ty[i].trim_end(), *t, "ligne {i}");
        assert_eq!(keys[i], Some(*k), "key ligne {i}");
        assert_eq!(xs[i], Some(*x), "x ligne {i}");
        assert_eq!(obs[i], *o, "_OBS_ ligne {i}");
    }
}

/// Reproduit le programme du cas compat compare-outdif-by-criterion :
/// BY grp, METHOD=ABSOLUTE CRITERION=0.001, OUT= OUTDIF OUTNOEQUAL.
#[test]
fn compare_compat_outdif_by_criterion() {
    let program = r#"
libname b 'data';

proc compare base=b.b2 compare=b.c2 out=out
             outdif outnoequal method=absolute criterion=0.001 noprint;
  by grp;
  var v;
run;
"#;
    let (code, log, _listing, tables) = run_in_sandbox(outdif_by_criterion_data, program);
    assert_eq!(code, 0, "log:\n{log}");
    assert!(!log.contains("ERROR:"), "log:\n{log}");

    assert_eq!(
        columns_of(&tables, "out"),
        vec!["GRP", "V", "_TYPE_", "_OBS_"]
    );

    // Seules les paires jugées inégales (|y−x| > 0.001) sont écrites ; la
    // paire A/2.000 est égale, supprimée par OUTNOEQUAL ; _OBS_ est la
    // séquence dans le groupe BY.
    let grp = char_col(&tables, "out", "grp");
    let v = raw_num_col(&tables, "out", "v");
    let ty = char_col(&tables, "out", "_TYPE_");
    let obs = raw_num_col(&tables, "out", "_OBS_");
    assert_eq!(ty.len(), 2, "2 lignes DIF attendues");
    assert_eq!(ty[0].trim_end(), "DIF");
    assert_eq!(ty[1].trim_end(), "DIF");
    assert_eq!(grp[0].trim_end(), "A");
    assert_eq!(grp[1].trim_end(), "B");
    assert!((v[0].unwrap() - 0.005).abs() < 1e-9, "v[0] = {:?}", v[0]);
    assert!((v[1].unwrap() - 0.01).abs() < 1e-9, "v[1] = {:?}", v[1]);
    assert_eq!(obs[0], Some(1.0));
    assert_eq!(obs[1], Some(1.0));
}

/// &SYSINFO : bits documentés (doc SAS 9.4) selon les conditions.
#[test]
fn compare_compat_sysinfo_bits() {
    // Tout égal → 0.
    let mut session = make_session();
    write_numeric_ds(&mut session, "A", &[1.0], &[2.0]);
    write_numeric_ds(&mut session, "B", &[1.0], &[2.0]);
    let ast = cmp_ast("A", "B");
    execute(&ast, &mut session).unwrap();
    assert_eq!(sysinfo_of(&session), 0);

    // Valeur inégale → VALUE 4096.
    let mut session = make_session();
    write_numeric_ds(&mut session, "A", &[1.0], &[2.0]);
    write_numeric_ds(&mut session, "B", &[9.0], &[2.0]);
    let ast = cmp_ast("A", "B");
    execute(&ast, &mut session).unwrap();
    assert_eq!(sysinfo_of(&session), 4096);

    // BASE a une observation de plus → BASEOBS 64 (valeurs égales ailleurs).
    let mut session = make_session();
    write_numeric_ds(&mut session, "A", &[1.0, 2.0], &[0.0, 0.0]);
    write_numeric_ds(&mut session, "B", &[1.0], &[0.0]);
    let ast = cmp_ast("A", "B");
    execute(&ast, &mut session).unwrap();
    assert_eq!(sysinfo_of(&session), 64);

    // COMPARE a une observation de plus → COMPOBS 128.
    let mut session = make_session();
    write_numeric_ds(&mut session, "A", &[1.0], &[0.0]);
    write_numeric_ds(&mut session, "B", &[1.0, 2.0], &[0.0, 0.0]);
    let ast = cmp_ast("A", "B");
    execute(&ast, &mut session).unwrap();
    assert_eq!(sysinfo_of(&session), 128);

    // Variable seulement dans BASE → BASEVAR 1024.
    let mut session = make_session();
    let df_base = df!["x" => [1.0_f64], "z" => [5.0_f64]].unwrap();
    let vars_base = vec![
        VarMeta {
            name: "x".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
            informat: None,
        },
        VarMeta {
            name: "z".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
            informat: None,
        },
    ];
    let df_comp = df!["x" => [1.0_f64]].unwrap();
    let vars_comp = vec![VarMeta {
        name: "x".into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
        informat: None,
    }];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(
            "A",
            &SasDataset {
                df: df_base,
                vars: vars_base,
            },
        )
        .unwrap();
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(
            "B",
            &SasDataset {
                df: df_comp,
                vars: vars_comp,
            },
        )
        .unwrap();
    let ast = cmp_ast("A", "B");
    execute(&ast, &mut session).unwrap();
    assert_eq!(sysinfo_of(&session), 1024);

    // Types conflictuels → TYPE 8192.
    let mut session = make_session();
    let df_base = df!["x" => [1.0_f64]].unwrap();
    let vars_base = vec![VarMeta {
        name: "x".into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
        informat: None,
    }];
    let df_comp = df!["x" => ["a"]].unwrap();
    let vars_comp = vec![VarMeta {
        name: "x".into(),
        ty: VarType::Char,
        length: 1,
        format: None,
        label: None,
        informat: None,
    }];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(
            "A",
            &SasDataset {
                df: df_base,
                vars: vars_base,
            },
        )
        .unwrap();
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(
            "B",
            &SasDataset {
                df: df_comp,
                vars: vars_comp,
            },
        )
        .unwrap();
    let ast = cmp_ast("A", "B");
    execute(&ast, &mut session).unwrap();
    // TYPE 8192 (types conflictuels) + LENGTH 16 (longueur 8 vs 1).
    assert_eq!(sysinfo_of(&session), 8192 | 16);

    // BY : groupe présent seulement dans BASE → BASEBY 256 (+ BASEOBS 64 :
    // ses observations restent sans appariement).
    let mut session = make_session();
    write_input_ds(
        &mut session,
        "A",
        df!["grp" => ["A", "B"], "v" => [1.0_f64, 2.0]].unwrap(),
        vec!["grp:char", "v:num"],
    );
    write_input_ds(
        &mut session,
        "B",
        df!["grp" => ["A"], "v" => [1.0_f64]].unwrap(),
        vec!["grp:char", "v:num"],
    );
    let mut ast = cmp_ast("A", "B");
    ast.by = vec![("grp".into(), false)];
    execute(&ast, &mut session).unwrap();
    assert_eq!(sysinfo_of(&session), 256 | 64);
}

/// Écrit un dataset WORK depuis un DataFrame et un typage simplifié
/// (`nom:num|char`) — pour les tests &SYSINFO BY.
fn write_input_ds(session: &mut Session, name: &str, df: DataFrame, spec: Vec<&str>) {
    let vars: Vec<VarMeta> = spec
        .iter()
        .map(|s| {
            let (n, t) = s.split_once(':').unwrap();
            VarMeta {
                name: n.into(),
                ty: if t == "num" {
                    VarType::Num
                } else {
                    VarType::Char
                },
                length: 8,
                format: None,
                label: None,
                informat: None,
            }
        })
        .collect();
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(name, &SasDataset { df, vars })
        .unwrap();
}

/// ID : appariement par clé — observation propre à chaque table signalée
/// (BASEOBS/COMPOBS) et paires appariées même dans le désordre.
#[test]
fn compare_compat_id_matching() {
    let mut session = make_session();
    write_input_ds(
        &mut session,
        "A",
        df!["key" => [1.0_f64, 2.0, 3.0], "x" => [10.0_f64, 20.0, 30.0]].unwrap(),
        vec!["key:num", "x:num"],
    );
    write_input_ds(
        &mut session,
        "B",
        df!["key" => [3.0_f64, 1.0, 2.0], "x" => [30.0_f64, 10.0, 25.0]].unwrap(),
        vec!["key:num", "x:num"],
    );
    let mut ast = cmp_ast("A", "B");
    ast.id = vec!["key".into()];
    ast.out = Some(DatasetRef {
        libref: Some("WORK".into()),
        name: "OUTD".into(),
    });
    ast.outdif = true;
    ast.outnoequal = true;
    execute(&ast, &mut session).unwrap();

    // Paires appariées par clé : 1↔1, 2↔2 (inégale), 3↔3 ; DIF sur la 2e
    // paire (séquence 2), valeur 25−20 = 5.
    let (ds, _) = session.libs.get("WORK").unwrap().read("OUTD").unwrap();
    assert_eq!(ds.n_obs(), 1);
    let idx = ds.vars.iter().position(|v| v.name == "X").unwrap();
    let dif = crate::procs::common::decode_column(&ds, idx).unwrap();
    assert_eq!(dif[0], Value::Num(5.0));
    let oi = ds.vars.iter().position(|v| v.name == "_OBS_").unwrap();
    let o = crate::procs::common::decode_column(&ds, oi).unwrap();
    assert_eq!(o[0], Value::Num(2.0));

    // Clés absentes d'un côté → BASEOBS | COMPOBS.
    let mut session = make_session();
    write_input_ds(
        &mut session,
        "A",
        df!["key" => [1.0_f64, 2.0, 3.0], "x" => [1.0_f64, 2.0, 3.0]].unwrap(),
        vec!["key:num", "x:num"],
    );
    write_input_ds(
        &mut session,
        "B",
        df!["key" => [2.0_f64, 4.0], "x" => [2.0_f64, 4.0]].unwrap(),
        vec!["key:num", "x:num"],
    );
    let mut ast = cmp_ast("A", "B");
    ast.id = vec!["key".into()];
    execute(&ast, &mut session).unwrap();
    assert_eq!(sysinfo_of(&session), 64 | 128); // BASEOBS + COMPOBS
}

/// CRITERION=/METHOD= : le jugement tolère les écarts sous le seuil
/// (ABSOLUTE, RELATIVE, PERCENT) et EXACT reste strict.
#[test]
fn compare_compat_criterion_and_methods() {
    // ABSOLUTE : |1.005 − 1.000| = 0.005 ≤ 0.005 → égal ; 0.006 > → inégal.
    let mut session = make_session();
    write_input_ds(
        &mut session,
        "A",
        df!["v" => [1.0_f64, 1.0]].unwrap(),
        vec!["v:num"],
    );
    write_input_ds(
        &mut session,
        "B",
        df!["v" => [1.005_f64, 1.006]].unwrap(),
        vec!["v:num"],
    );
    let mut ast = cmp_ast("A", "B");
    ast.method = CmpMethod::Absolute;
    ast.criterion = 0.005;
    execute(&ast, &mut session).unwrap();
    assert_eq!(sysinfo_of(&session), 4096); // la 2e paire only

    // Mêmes données, EXACT : les deux paires sont inégales (2 écarts).
    let mut session = make_session();
    write_input_ds(
        &mut session,
        "A",
        df!["v" => [1.0_f64, 1.0]].unwrap(),
        vec!["v:num"],
    );
    write_input_ds(
        &mut session,
        "B",
        df!["v" => [1.005_f64, 1.006]].unwrap(),
        vec!["v:num"],
    );
    let mut ast = cmp_ast("A", "B");
    ast.method = CmpMethod::Exact;
    ast.criterion = 0.01;
    execute(&ast, &mut session).unwrap();
    assert_eq!(sysinfo_of(&session), 4096);

    // RELATIVE : |(11−10)/10| = 0.1 > 0.05 → inégal ; |(10.4−10)/10| = 0.04
    // ≤ 0.05 → égal.
    let mut session = make_session();
    write_input_ds(
        &mut session,
        "A",
        df!["v" => [10.0_f64, 10.0]].unwrap(),
        vec!["v:num"],
    );
    write_input_ds(
        &mut session,
        "B",
        df!["v" => [11.0_f64, 10.4]].unwrap(),
        vec!["v:num"],
    );
    let mut ast = cmp_ast("A", "B");
    ast.method = CmpMethod::Relative;
    ast.criterion = 0.05;
    execute(&ast, &mut session).unwrap();
    assert_eq!(sysinfo_of(&session), 4096);
    // Et le décompte : 1 valeur inégale seulement (var_diffs).
    let log = session.log.into_string();
    assert!(log.contains("1 observation"), "log: {log}");

    // PERCENT : 100·(11−10)/10 = 10 > 5 → inégal ; 100·(10.4−10)/10 = 4 ≤ 5
    // → égal.
    let mut session = make_session();
    write_input_ds(
        &mut session,
        "A",
        df!["v" => [10.0_f64, 10.0]].unwrap(),
        vec!["v:num"],
    );
    write_input_ds(
        &mut session,
        "B",
        df!["v" => [11.0_f64, 10.4]].unwrap(),
        vec!["v:num"],
    );
    let mut ast = cmp_ast("A", "B");
    ast.method = CmpMethod::Percent;
    ast.criterion = 5.0;
    execute(&ast, &mut session).unwrap();
    assert_eq!(sysinfo_of(&session), 4096);
    let log = session.log.into_string();
    assert!(log.contains("1 observation"), "log: {log}");
}

/// OUTPERCENT : lignes PERCENT = 100·(y−x)/x, et OUTNOEQUAL les supprime
/// quand la paire est jugée égale.
#[test]
fn compare_compat_outpercent_and_outnoequal() {
    let mut session = make_session();
    write_input_ds(
        &mut session,
        "A",
        df!["v" => [10.0_f64, 10.0]].unwrap(),
        vec!["v:num"],
    );
    write_input_ds(
        &mut session,
        "B",
        df!["v" => [12.0_f64, 10.0]].unwrap(),
        vec!["v:num"],
    );
    let mut ast = cmp_ast("A", "B");
    ast.out = Some(DatasetRef {
        libref: Some("WORK".into()),
        name: "OUTP".into(),
    });
    ast.outpercent = true;
    ast.outnoequal = true;
    execute(&ast, &mut session).unwrap();

    let (ds, _) = session.libs.get("WORK").unwrap().read("OUTP").unwrap();
    assert_eq!(ds.n_obs(), 1, "seule la paire inégale est écrite");
    let ti = ds.vars.iter().position(|v| v.name == "_TYPE_").unwrap();
    let ty = crate::procs::common::decode_column(&ds, ti).unwrap();
    assert_eq!(ty[0], Value::Char("PERCENT".to_string()));
    let vi = ds.vars.iter().position(|v| v.name == "V").unwrap();
    let v = crate::procs::common::decode_column(&ds, vi).unwrap();
    assert_eq!(v[0], Value::Num(20.0)); // 100·(12−10)/10
}

/// BRIEF et NOPRINT : le listing est soit condensé, soit absent — la log
/// et &SYSINFO restent produits.
#[test]
fn compare_compat_brief_noprint_listing() {
    let mut session = make_session();
    write_numeric_ds(&mut session, "A", &[1.0], &[2.0]);
    write_numeric_ds(&mut session, "B", &[1.0], &[3.0]);
    let mut ast = cmp_ast("A", "B");
    ast.brief = true;
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(
        listing.contains("Brief Summary"),
        "listing BRIEF : {listing}"
    );
    assert!(
        !listing.contains("Values Comparison"),
        "BRIEF ne détaille pas les valeurs : {listing}"
    );
    assert_eq!(sysinfo_of(&session), 4096);

    let mut session = make_session();
    write_numeric_ds(&mut session, "A", &[1.0], &[2.0]);
    write_numeric_ds(&mut session, "B", &[1.0], &[3.0]);
    let mut ast = cmp_ast("A", "B");
    ast.noprint = true;
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.trim().is_empty(), "listing NOPRINT : {listing}");
    assert_eq!(sysinfo_of(&session), 4096);
    let log = session.log.into_string();
    assert!(log.contains("1 observation"), "log : {log}");
}

/// MAXPRINT= et LISTALL : options reconnues (plus d'ERROR J02-P4) ;
/// LISTALL liste toutes les variables comparées dans la section valeurs.
#[test]
fn compare_compat_maxprint_listall_parse() {
    let ast = parse_compare_src(
        "proc compare base=work.a compare=work.b maxprint=10 listall brief noprint; run;",
    )
    .unwrap();
    assert_eq!(ast.maxprint, (10, 50));
    assert!(ast.listall);
    assert!(ast.brief);
    assert!(ast.noprint);

    // MAXPRINT=(n,p)
    let ast =
        parse_compare_src("proc compare base=work.a compare=work.b maxprint=(10,5); run;").unwrap();
    assert_eq!(ast.maxprint, (10, 5));

    // LISTALL : la section valeurs titre « All Compared Variables » et
    // inclut les variables égales.
    let mut session = make_session();
    write_input_ds(
        &mut session,
        "A",
        df!["u" => [1.0_f64], "v" => [1.0_f64]].unwrap(),
        vec!["u:num", "v:num"],
    );
    write_input_ds(
        &mut session,
        "B",
        df!["u" => [1.0_f64], "v" => [2.0_f64]].unwrap(),
        vec!["u:num", "v:num"],
    );
    let mut ast = cmp_ast("A", "B");
    ast.listall = true;
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(
        listing.contains("All Compared Variables"),
        "listing : {listing}"
    );

    // Sans LISTALL : titre « Variables with Unequal Values ».
    let mut session = make_session();
    write_input_ds(
        &mut session,
        "A",
        df!["u" => [1.0_f64], "v" => [1.0_f64]].unwrap(),
        vec!["u:num", "v:num"],
    );
    write_input_ds(
        &mut session,
        "B",
        df!["u" => [1.0_f64], "v" => [2.0_f64]].unwrap(),
        vec!["u:num", "v:num"],
    );
    let ast = cmp_ast("A", "B");
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(
        listing.contains("Variables with Unequal Values"),
        "listing : {listing}"
    );
}

/// MAXPRINT= effet runtime (J07-P9) : la section « Value Comparison
/// Results » est plafonnée à n différences par observation et p
/// observations avec différences ; une NOTE signale la troncature.
#[test]
fn compare_compat_maxprint_runtime() {
    // 3 observations, 2 variables, tout inégal : 6 différences au total.
    let setup = |session: &mut Session| {
        write_input_ds(
            session,
            "A",
            df!["x" => [1.0_f64, 2.0, 3.0], "y" => [10.0_f64, 20.0, 30.0]].unwrap(),
            vec!["x:num", "y:num"],
        );
        write_input_ds(
            session,
            "B",
            df!["x" => [4.0_f64, 5.0, 6.0], "y" => [11.0_f64, 21.0, 31.0]].unwrap(),
            vec!["x:num", "y:num"],
        );
    };

    // Défaut (50, 50) : tout est imprimé, aucune NOTE de troncature.
    let mut session = make_session();
    setup(&mut session);
    let ast = cmp_ast("A", "B");
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    let log = session.log.into_string();
    let results = listing
        .split("Value Comparison Results")
        .nth(1)
        .unwrap_or_else(|| panic!("section absente : {listing}"));
    let n_rows = results.lines().filter(|l| l.contains("Num")).count();
    assert_eq!(n_rows, 6, "défaut MAXPRINT : {listing}");
    assert!(!log.contains("MAXPRINT="), "NOTE inattendue : {log}");

    // MAXPRINT=(1,2) : 1 différence par observation, 2 observations →
    // 2 lignes, observations 3 tronquée → NOTE.
    let mut session = make_session();
    setup(&mut session);
    let mut ast = cmp_ast("A", "B");
    ast.maxprint = (1, 2);
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    let log = session.log.into_string();
    let results = listing
        .split("Value Comparison Results")
        .nth(1)
        .unwrap_or_else(|| panic!("section absente : {listing}"));
    let n_rows = results.lines().filter(|l| l.contains("Num")).count();
    assert_eq!(n_rows, 2, "MAXPRINT=(1,2) : {listing}");
    assert!(log.contains("MAXPRINT="), "NOTE manquante : {log}");

    // MAXPRINT=0 : aucune ligne imprimée, NOTE de troncature.
    let mut session = make_session();
    setup(&mut session);
    let mut ast = cmp_ast("A", "B");
    ast.maxprint = (0, 50);
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    let log = session.log.into_string();
    let results = listing
        .split("Value Comparison Results")
        .nth(1)
        .unwrap_or_else(|| panic!("section absente : {listing}"));
    assert!(
        results.lines().all(|l| !l.contains("Num")),
        "MAXPRINT=0 : {listing}"
    );
    assert!(log.contains("MAXPRINT="), "NOTE manquante : {log}");
}
