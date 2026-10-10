//! Tests de compatibilité PROC MEANS/SUMMARY — options de production (J07-P2).
//!
//! Chaque test exécute un PROGRAMME complet (parse + execute) via la façade
//! `crate::run`, comme le fait le corpus de conformité, puis relit les tables
//! WORK via l'API dataset. Les valeurs attendues sont calculables à la main
//! (jeux de données minimaux) et documentées inline ; les deux programmes
//! des cas `conformance/cases/compat/means/*` sont reproduits ici.

use crate::dataset::SasDataset;
use crate::value::Value;
use crate::{RunOptions, run};
use polars::prelude::*;
use std::collections::BTreeMap;
use std::path::Path;

/// Bac à sable : `data/` rempli par `setup`, WORK isolé, exécution du
/// programme, puis relecture de toutes les tables WORK produites.
fn run_in_sandbox(
    setup: impl FnOnce(&Path),
    program: &str,
) -> (i32, String, BTreeMap<String, SasDataset>) {
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
    (outcome.exit_code, outcome.log, tables)
}

/// Écrit un parquet d'entrée (colonne caractère → String, numérique → f64 ;
/// `None` = missing) — l'équivalent du convertisseur du corpus J05.
fn write_input(dir: &Path, name: &str, mut df: DataFrame) {
    std::fs::create_dir_all(dir).unwrap();
    let mut file = std::fs::File::create(dir.join(format!("{name}.parquet"))).unwrap();
    ParquetWriter::new(&mut file).finish(&mut df).unwrap();
}

/// Le jeu `sales` du cas compat means-class-missing-nway.
fn sales_data(dir: &Path) {
    let df = df![
        "region" => ["East", "East", "West", "", "West"],
        "amount" => [Some(10.0_f64), Some(20.0), Some(30.0), Some(40.0), None],
    ]
    .unwrap();
    write_input(dir, "sales", df);
}

/// Le jeu `weights` du cas compat means-order-freq-autoname.
fn weights_data(dir: &Path) {
    let df = df![
        "group" => ["A", "A", "B", "C"],
        "value" => [7.0_f64, 3.0, 5.0, 9.0],
        "w" => [1.0_f64, 1.0, 2.0, 1.0],
        "price" => [200.0_f64, 200.0, 100.0, 300.0],
    ]
    .unwrap();
    write_input(dir, "weights", df);
}

fn num_col(tables: &BTreeMap<String, SasDataset>, table: &str, col: &str) -> Vec<Value> {
    let ds = tables
        .get(table)
        .unwrap_or_else(|| panic!("WORK.{table} absent"));
    let idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(col))
        .unwrap_or_else(|| panic!("colonne {col} absente de WORK.{table}"));
    crate::procs::common::decode_column(ds, idx).unwrap()
}

fn num_at(tables: &BTreeMap<String, SasDataset>, table: &str, col: &str, row: usize) -> f64 {
    match &num_col(tables, table, col)[row] {
        Value::Num(f) => *f,
        v => panic!("{col}[{row}] non numérique : {v:?}"),
    }
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
    crate::procs::common::decode_column(ds, idx)
        .unwrap()
        .into_iter()
        .map(|v| match v {
            Value::Char(s) => s,
            Value::Missing(_) => String::new(),
            other => format!("{other:?}"),
        })
        .collect()
}

fn columns_of(tables: &BTreeMap<String, SasDataset>, table: &str) -> Vec<String> {
    tables
        .get(table)
        .unwrap_or_else(|| panic!("WORK.{table} absent"))
        .vars
        .iter()
        .map(|m| m.name.to_uppercase())
        .collect()
}

// ── CLASS manquant exclu par défaut / inclus avec MISSING (compat
// means-class-missing-nway) ────────────────────────────────────────────

#[test]
fn means_compat_class_missing_excluded_by_default() {
    let (code, log, tables) = run_in_sandbox(
        sales_data,
        "libname ind 'data';\n\
         proc means data=ind.sales noprint;\n\
         class region; var amount;\n\
         output out=excl mean=m n=n;\n\
         run;\n",
    );
    // La ligne à région manquante (amount=40) est exclue de l'analyse :
    // _TYPE_=0 sur les 4 observations restantes (montants 10,20,30,missing)
    // → _FREQ_=4, n=3, m=(10+20+30)/3=20. Pas de niveau « région manquante ».
    assert_eq!(code, 0, "log: {log}");
    let ty = num_col(&tables, "excl", "_TYPE_");
    let freq = num_col(&tables, "excl", "_FREQ_");
    let m = num_col(&tables, "excl", "m");
    let n = num_col(&tables, "excl", "n");
    let region = char_col(&tables, "excl", "region");
    assert_eq!(ty.len(), 3, "3 lignes attendues (TYPE 0 + 2 niveaux)");
    assert_eq!(
        (num_at(&tables, "excl", "_TYPE_", 0), region[0].as_str()),
        (0.0, "")
    );
    assert_eq!(num_at(&tables, "excl", "_FREQ_", 0), 4.0);
    assert_eq!(num_at(&tables, "excl", "m", 0), 20.0);
    assert_eq!(num_at(&tables, "excl", "n", 0), 3.0);
    assert_eq!(
        (ty[1].clone(), freq[1].clone(), region[1].as_str()),
        (Value::Num(1.0), Value::Num(2.0), "East")
    );
    assert_eq!(
        (m[1].clone(), n[1].clone()),
        (Value::Num(15.0), Value::Num(2.0))
    );
    assert_eq!(
        (ty[2].clone(), freq[2].clone(), region[2].as_str()),
        (Value::Num(1.0), Value::Num(2.0), "West")
    );
    assert_eq!(
        (m[2].clone(), n[2].clone()),
        (Value::Num(30.0), Value::Num(1.0))
    );
}

#[test]
fn means_compat_class_missing_option_and_nway() {
    let (code, log, tables) = run_in_sandbox(
        sales_data,
        // MISSING (option PROC) + NWAY : la région manquante forme un niveau
        // (40) et l'OUT= ne garde que le _TYPE_ maximal.
        "libname ind 'data';\n\
         proc means data=ind.sales nway noprint missing;\n\
         class region; var amount;\n\
         output out=kept mean=m n=n;\n\
         run;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    let ty = num_col(&tables, "kept", "_TYPE_");
    let region = char_col(&tables, "kept", "region");
    assert_eq!(ty, vec![Value::Num(1.0); 3], "NWAY → uniquement _TYPE_=1");
    assert_eq!(region, vec!["", "East", "West"]);
    assert_eq!(num_at(&tables, "kept", "_FREQ_", 0), 1.0);
    assert_eq!(
        (
            num_at(&tables, "kept", "m", 0),
            num_at(&tables, "kept", "n", 0)
        ),
        (40.0, 1.0)
    );
    assert_eq!(num_at(&tables, "kept", "m", 1), 15.0);
    assert_eq!(num_at(&tables, "kept", "m", 2), 30.0);
}

#[test]
fn means_compat_class_missing_option_on_class_statement() {
    let (code, log, tables) = run_in_sandbox(
        sales_data,
        // MISSING portée par le statement CLASS (`class region / missing;`).
        "libname ind 'data';\n\
         proc means data=ind.sales noprint;\n\
         class region / missing; var amount;\n\
         output out=k mean=m;\n\
         run;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    let region = char_col(&tables, "k", "region");
    assert_eq!(
        region,
        vec!["", "", "East", "West"],
        "niveau manquant présent"
    );
}

#[test]
fn means_compat_nway_keeps_only_max_type() {
    let setup = |d: &Path| {
        let df = df![
            "g" => ["a", "a", "b", "b"],
            "h" => [1.0_f64, 2.0, 1.0, 2.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0],
        ]
        .unwrap();
        write_input(d, "t", df);
    };
    let (_, _, plain) = run_in_sandbox(
        setup,
        "libname ind 'data';\nproc means data=ind.t noprint;\nclass g h; var x;\noutput out=all mean=m;\nrun;\n",
    );
    let (_, _, nway) = run_in_sandbox(
        setup,
        "libname ind 'data';\nproc means data=ind.t nway noprint;\nclass g h; var x;\noutput out=only mean=m;\nrun;\n",
    );
    // Sans NWAY : _TYPE_ 0,1,2,3 → 1+2+2+4 = 9 lignes. Avec NWAY : les 4
    // lignes de _TYPE_=3 uniquement.
    assert_eq!(num_col(&plain, "all", "_TYPE_").len(), 9);
    let ty = num_col(&nway, "only", "_TYPE_");
    assert_eq!(ty, vec![Value::Num(3.0); 4]);
}

// ── ORDER=FREQ, FREQ, ID, MAXDEC, AUTONAME (compat
// means-order-freq-autoname) ───────────────────────────────────────────

#[test]
fn means_compat_order_freq_and_weights_and_id_and_autoname() {
    let (code, log, tables) = run_in_sandbox(
        weights_data,
        // Reproduit le programme du cas compat : ORDER=FREQ (effectifs
        // décroissants), FREQ w (N et variance pondérés par Σw), ID price,
        // OUTPUT / AUTONAME. MAXDEC= ne touche que le rapport imprimé.
        "libname ind 'data';\n\
         proc means data=ind.weights nway noprint order=freq maxdec=2;\n\
         class group; var value; freq w; id price;\n\
         output out=out mean= std= / autoname;\n\
         run;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    // Colonnes : CLASS (group), _TYPE_, _FREQ_, ID (price), stats AUTONAME
    // (arbitrage J01-P2 rév. 3, décision c96c6088 : BY → CLASS → _TYPE_ →
    // _FREQ_ → ID → statistiques).
    assert_eq!(
        columns_of(&tables, "out"),
        vec![
            "GROUP",
            "_TYPE_",
            "_FREQ_",
            "PRICE",
            "VALUE_MEAN",
            "VALUE_STD"
        ]
    );
    // Ordre ORDER=FREQ : A (Σw=2), B (Σw=2, égalité → interne), C (Σw=1).
    assert_eq!(char_col(&tables, "out", "group"), vec!["A", "B", "C"]);
    // _FREQ_ = Σw ; ID price recopié par groupe.
    assert_eq!(num_at(&tables, "out", "_FREQ_", 0), 2.0);
    assert_eq!(num_at(&tables, "out", "price", 2), 300.0);
    // Moyennes exactes ; STD divisé par Σw−1 : A → sqrt(8/1)=2.8284271,
    // B (1 obs × w=2) → 0, C (1 obs × w=1) → missing.
    assert_eq!(num_at(&tables, "out", "VALUE_MEAN", 0), 5.0);
    assert!(
        (num_at(&tables, "out", "VALUE_STD", 0) - 8.0_f64.sqrt()).abs() < 1e-12,
        "std A"
    );
    assert_eq!(num_at(&tables, "out", "VALUE_STD", 1), 0.0);
    assert!(num_col(&tables, "out", "VALUE_STD")[2].is_missing());
}

#[test]
fn means_compat_order_freq_beats_internal_order() {
    let setup = |d: &Path| {
        let df = df![
            "g" => ["Y", "Y", "Y", "Y", "Y", "X", "X", "X"],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        ]
        .unwrap();
        write_input(d, "t", df);
    };
    let (_, _, ofreq) = run_in_sandbox(
        setup,
        "libname ind 'data';\nproc means data=ind.t nway noprint order=freq;\nclass g; var x;\noutput out=o mean=m;\nrun;\n",
    );
    let (_, _, oint) = run_in_sandbox(
        setup,
        "libname ind 'data';\nproc means data=ind.t nway noprint;\nclass g; var x;\noutput out=o mean=m;\nrun;\n",
    );
    // Y (5 obs) passe devant X (3 obs) sous ORDER=FREQ ; l'inverse en
    // ORDER=INTERNAL (défaut).
    assert_eq!(char_col(&ofreq, "o", "g"), vec!["Y", "X"]);
    assert_eq!(char_col(&oint, "o", "g"), vec!["X", "Y"]);
}

#[test]
fn means_compat_order_data_uses_first_appearance() {
    let (code, log, tables) = run_in_sandbox(
        |d: &Path| {
            let df = df![
                "g" => ["C", "A", "B", "A"],
                "x" => [1.0_f64, 2.0, 3.0, 4.0],
            ]
            .unwrap();
            write_input(d, "t", df);
        },
        "libname ind 'data';\nproc means data=ind.t nway noprint order=data;\nclass g; var x;\noutput out=o mean=m;\nrun;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    assert_eq!(char_col(&tables, "o", "g"), vec!["C", "A", "B"]);
}

#[test]
fn means_compat_freq_weights_n_and_nmiss() {
    let (code, log, tables) = run_in_sandbox(
        |d: &Path| {
            // w=2 sur une valeur manquante : N=3 (1+2), NMiss=2,
            // mean=(7+2·5)/3.
            let df = df![
                "g" => ["A", "A", "A"],
                "x" => [Some(7.0_f64), None, Some(5.0)],
                "w" => [1.0_f64, 2.0, 2.0],
            ]
            .unwrap();
            write_input(d, "t", df);
        },
        "libname ind 'data';\nproc means data=ind.t noprint;\nclass g; var x; freq w;\noutput out=o n=n nmiss=nm mean=m;\nrun;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    // Ligne _TYPE_=0 : N=Σw des valeurs présentes (1+2), NMiss=2.
    assert_eq!(num_at(&tables, "o", "n", 0), 3.0);
    assert_eq!(num_at(&tables, "o", "nm", 0), 2.0);
    assert!((num_at(&tables, "o", "m", 0) - 17.0 / 3.0).abs() < 1e-12);
}

#[test]
fn means_compat_id_takes_largest_level() {
    let (_, _, tables) = run_in_sandbox(
        |d: &Path| {
            // ID variable dont les valeurs diffèrent dans un groupe → la
            // plus grande.
            let df = df![
                "g" => ["A", "A", "B"],
                "x" => [1.0_f64, 2.0, 3.0],
                "price" => [200.0_f64, 300.0, 50.0],
            ]
            .unwrap();
            write_input(d, "t", df);
        },
        "libname ind 'data';\nproc means data=ind.t nway noprint;\nclass g; var x; id price;\noutput out=o mean=m;\nrun;\n",
    );
    assert_eq!(num_at(&tables, "o", "price", 0), 300.0);
    assert_eq!(num_at(&tables, "o", "price", 1), 50.0);
}

#[test]
fn means_compat_output_autoname_bare_stat_and_lists() {
    let setup = |d: &Path| {
        let df = df![
            "g" => ["a", "a", "b"],
            "x" => [1.0_f64, 3.0, 10.0],
            "y" => [2.0_f64, 4.0, 20.0],
        ]
        .unwrap();
        write_input(d, "t", df);
    };
    // `mean=` sans parenthèses ni noms + AUTONAME → une colonne par VAR.
    let (code, log, tables) = run_in_sandbox(
        setup,
        "libname ind 'data';\nproc means data=ind.t noprint;\nclass g; var x y;\noutput out=o mean= / autoname;\nrun;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    assert!(columns_of(&tables, "o").contains(&"X_MEAN".to_string()));
    assert!(columns_of(&tables, "o").contains(&"Y_MEAN".to_string()));

    // `mean(x y)=mx my` — liste de variables et autant de noms.
    let (code2, log2, tables2) = run_in_sandbox(
        setup,
        "libname ind 'data';\nproc means data=ind.t noprint;\nclass g; var x y;\noutput out=o2 mean(x y)=mx my;\nrun;\n",
    );
    assert_eq!(code2, 0, "log: {log2}");
    assert!(columns_of(&tables2, "o2").contains(&"MX".to_string()));
    assert!(columns_of(&tables2, "o2").contains(&"MY".to_string()));

    // `mean=` sans AUTONAME sur UNE variable → colonne MEAN.
    let (code3, log3, tables3) = run_in_sandbox(
        setup,
        "libname ind 'data';\nproc means data=ind.t noprint;\nclass g; var x;\noutput out=o3 mean=;\nrun;\n",
    );
    assert_eq!(code3, 0, "log: {log3}");
    assert!(columns_of(&tables3, "o3").contains(&"MEAN".to_string()));

    // `mean=` sans AUTONAME sur PLUSIEURS variables → ERROR explicite.
    let (code4, log4, _) = run_in_sandbox(
        setup,
        "libname ind 'data';\nproc means data=ind.t noprint;\nclass g; var x y;\noutput out=o4 mean=;\nrun;\n",
    );
    assert_ne!(code4, 0);
    assert!(log4.contains("AUTONAME"), "log: {log4}");
}

#[test]
fn means_compat_maxdec_prints_only() {
    let (code, log, tables) = run_in_sandbox(
        |d: &Path| {
            let df = df![
                "g" => ["a", "a", "a"],
                "x" => [1.0_f64, 1.0, 2.0],
            ]
            .unwrap();
            write_input(d, "t", df);
        },
        "libname ind 'data';\nproc means data=ind.t maxdec=2 n mean;\nclass g; var x;\noutput out=o mean=m;\nrun;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    assert!(!log.contains("ERROR"), "log: {log}");
    // MAXDEC= n'a AUCUN effet sur l'OUT= : pleine précision (4/3).
    assert!((num_at(&tables, "o", "m", 0) - 4.0 / 3.0).abs() < 1e-12);
}

// ── DESCENDTYPES / COMPLETETYPES / CHARTYPE / EXCLNPWGT / VARDEF ──────

#[test]
fn means_compat_descendtypes_orders_types_descending() {
    let (_, _, tables) = run_in_sandbox(
        |d: &Path| {
            let df = df![
                "g" => ["a", "a", "b", "b"],
                "x" => [1.0_f64, 2.0, 3.0, 4.0],
            ]
            .unwrap();
            write_input(d, "t", df);
        },
        "libname ind 'data';\nproc means data=ind.t noprint descendtypes;\nclass g; var x;\noutput out=o mean=m;\nrun;\n",
    );
    let ty = num_col(&tables, "o", "_TYPE_");
    assert_eq!(ty, vec![Value::Num(1.0), Value::Num(1.0), Value::Num(0.0)]);
}

#[test]
fn means_compat_completetypes_emits_unobserved_combinations() {
    let (code, log, tables) = run_in_sandbox(
        |d: &Path| {
            // La combinaison (b,2) n'est jamais observée.
            let df = df![
                "g" => ["a", "a", "b", "a"],
                "h" => [1.0_f64, 2.0, 1.0, 2.0],
                "x" => [1.0_f64, 2.0, 3.0, 4.0],
            ]
            .unwrap();
            write_input(d, "t", df);
        },
        "libname ind 'data';\nproc means data=ind.t nway noprint completetypes;\nclass g h; var x;\noutput out=o mean=m;\nrun;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    let freq = num_col(&tables, "o", "_FREQ_");
    let m = num_col(&tables, "o", "m");
    assert_eq!(freq.len(), 4, "4 combinaisons complètes : {freq:?}");
    assert!(
        freq.contains(&Value::Num(0.0)),
        "combinaison non observée : {freq:?}"
    );
    assert!(m.iter().any(|v| v.is_missing()));
}

#[test]
fn means_compat_chartype_character_type_mask() {
    let (code, log, tables) = run_in_sandbox(
        |d: &Path| {
            let df = df![
                "g" => ["a", "a", "b"],
                "x" => [1.0_f64, 2.0, 3.0],
            ]
            .unwrap();
            write_input(d, "t", df);
        },
        "libname ind 'data';\nproc means data=ind.t noprint chartype;\nclass g; var x;\noutput out=o mean=m;\nrun;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    assert_eq!(char_col(&tables, "o", "_TYPE_"), vec!["0", "1", "1"]);
}

#[test]
fn means_compat_exclnpwgt_accepted_and_vardef() {
    let (code, log, _) = run_in_sandbox(
        |d: &Path| {
            // EXCLNPWGT accepté ; VARDEF=WEIGHT change la variance
            // (diviseur Σw−Σw²/Σw).
            let df = df![
                "x" => [1.0_f64, 2.0, 3.0, 9.0],
                "w" => [1.0_f64, 2.0, 3.0, 0.0],
            ]
            .unwrap();
            write_input(d, "t", df);
        },
        "libname ind 'data';\nproc means data=ind.t exclnpwgt vardef=weight;\nvar x; weight w;\nrun;\n",
    );
    assert_eq!(code, 0, "log: {log}");
}

// ── Statements multiples + ODS Summary avec CLASS/BY ───────────────────

#[test]
fn means_compat_multiple_class_var_output_statements() {
    let (code, log, tables) = run_in_sandbox(
        |d: &Path| {
            let df = df![
                "g" => ["a", "a", "b"],
                "h" => [1.0_f64, 1.0, 2.0],
                "x" => [1.0_f64, 3.0, 10.0],
                "y" => [2.0_f64, 4.0, 20.0],
            ]
            .unwrap();
            write_input(d, "t", df);
        },
        "libname ind 'data';\n\
         proc means data=ind.t noprint;\n\
         class g; class h;\n\
         var x; var y;\n\
         output out=o1 mean(x)=mx;\n\
         output out=o2 n(y)=ny;\n\
         run;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    assert!(tables.contains_key("o1"));
    assert!(tables.contains_key("o2"));
    assert!(columns_of(&tables, "o1").contains(&"MX".to_string()));
    assert!(columns_of(&tables, "o2").contains(&"NY".to_string()));
    // Deux CLASS cumulées : TYPE0 (1) + TYPE1 h{1,2} (2) + TYPE2 g{a,b} (2)
    // + TYPE3 croisements observés (a,1),(b,2) (2) = 7 lignes dans o1.
    assert_eq!(num_col(&tables, "o1", "_TYPE_").len(), 7);
}

#[test]
fn means_compat_ods_summary_with_class_and_by() {
    let (code, log, tables) = run_in_sandbox(
        |d: &Path| {
            let df = df![
                "b" => ["g1", "g1", "g2"],
                "g" => ["a", "b", "a"],
                "x" => [1.0_f64, 2.0, 3.0],
            ]
            .unwrap();
            write_input(d, "t", df);
        },
        "libname ind 'data';\n\
         ods output Summary=cap;\n\
         proc means data=ind.t noprint;\n\
         by b; class g; var x;\n\
         run;\n\
         ods output close;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    let cols = columns_of(&tables, "cap");
    // La table Summary porte les colonnes BY/CLASS (J07-P2).
    assert!(cols.contains(&"B".to_string()), "cols: {cols:?}");
    assert!(cols.contains(&"G".to_string()), "cols: {cols:?}");
    assert!(cols.contains(&"VARIABLE".to_string()), "cols: {cols:?}");
    // g1 → niveaux a,b ; g2 → niveau a : 3 lignes.
    let ds = &tables["cap"];
    assert_eq!(ds.n_obs(), 3);
}

// ── J01-P2 (issue #14) : liste de stats stat=<var> et OUT= par défaut ──

/// Le jeu `sales` du cas base means-class-output (issue #14).
fn issue14_sales(dir: &Path) {
    let df = df![
        "region" => ["North", "North", "South", "South", "East", "East"],
        "amount" => [10.0_f64, 20.0, 30.0, 40.0, 50.0, 60.0],
    ]
    .unwrap();
    write_input(dir, "sales", df);
}

#[test]
fn means_compat_output_stat_list() {
    // `sum=total_amount mean=avg_amount` : une liste de specs stat=<nom>,
    // chacune appliquée à toutes les variables VAR (doc MEANS, OUTPUT).
    let (code, log, tables) = run_in_sandbox(
        issue14_sales,
        "libname ind 'data';\n\
         proc means data=ind.sales noprint;\n\
         class region; var amount;\n\
         output out=summary sum=total_amount mean=avg_amount;\n\
         run;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    assert_eq!(
        columns_of(&tables, "summary"),
        vec!["REGION", "_TYPE_", "_FREQ_", "TOTAL_AMOUNT", "AVG_AMOUNT"]
    );
    // Ligne globale _TYPE_=0 puis une ligne par niveau, tri alpha interne.
    assert_eq!(
        num_col(&tables, "summary", "_TYPE_"),
        vec![
            Value::Num(0.0),
            Value::Num(1.0),
            Value::Num(1.0),
            Value::Num(1.0)
        ]
    );
    assert_eq!(
        char_col(&tables, "summary", "region"),
        vec!["", "East", "North", "South"]
    );
    let sums = num_col(&tables, "summary", "total_amount");
    assert_eq!(
        sums,
        vec![
            Value::Num(210.0),
            Value::Num(110.0),
            Value::Num(30.0),
            Value::Num(70.0)
        ]
    );
    let means = num_col(&tables, "summary", "avg_amount");
    assert_eq!(
        means,
        vec![
            Value::Num(35.0),
            Value::Num(55.0),
            Value::Num(15.0),
            Value::Num(35.0)
        ]
    );
}

#[test]
fn means_compat_output_default_stats() {
    // `output out=summary;` sans mot-clé statistique : CLASS + _TYPE_ +
    // _FREQ_ + les statistiques par défaut (N MEAN STD MIN MAX) de chaque
    // variable d'analyse, nommées par la convention AUTONAME.
    let (code, log, tables) = run_in_sandbox(
        issue14_sales,
        "libname ind 'data';\n\
         proc means data=ind.sales noprint;\n\
         class region; var amount;\n\
         output out=summary;\n\
         run;\n",
    );
    assert_eq!(code, 0, "log: {log}");
    assert_eq!(
        columns_of(&tables, "summary"),
        vec![
            "REGION",
            "_TYPE_",
            "_FREQ_",
            "AMOUNT_N",
            "AMOUNT_MEAN",
            "AMOUNT_STD",
            "AMOUNT_MIN",
            "AMOUNT_MAX",
        ]
    );
    assert_eq!(num_at(&tables, "summary", "amount_n", 0), 6.0);
    assert_eq!(num_at(&tables, "summary", "amount_mean", 0), 35.0);
    // std global : sqrt(350) — écarts ±25,±15,±5 (n−1 = 5).
    assert!((num_at(&tables, "summary", "amount_std", 0) - 350.0_f64.sqrt()).abs() < 1e-9);
    assert_eq!(num_at(&tables, "summary", "amount_min", 0), 10.0);
    assert_eq!(num_at(&tables, "summary", "amount_max", 0), 60.0);
    // Groupe East (50, 60) : n=2, mean=55, std=sqrt(50), min=50, max=60.
    assert_eq!(num_at(&tables, "summary", "amount_n", 1), 2.0);
    assert_eq!(num_at(&tables, "summary", "amount_mean", 1), 55.0);
    assert!((num_at(&tables, "summary", "amount_std", 1) - 50.0_f64.sqrt()).abs() < 1e-9);
    assert_eq!(num_at(&tables, "summary", "amount_min", 1), 50.0);
    assert_eq!(num_at(&tables, "summary", "amount_max", 1), 60.0);
    // Quatre lignes : la globale (_TYPE_=0) + les trois niveaux (_TYPE_=1).
    assert_eq!(tables["summary"].n_obs(), 4);
}
