//! M38.3 — ODS OUTPUT généralisé : capture de la table « OneWayFreqs ».
//!
//! `freq::execute` ACCUMULE les tranches (`Session::append_ods_output`) ; la
//! matérialisation en dataset se fait en fin de proc (`flush_ods_output`,
//! appelé par `procs::execute_proc`) — les tests appellent donc le flush
//! explicitement après `execute`, comme le ferait le dispatch.

use super::*;
use crate::value::MissingKind;

fn ods_target(session: &mut Session, table: &str, name: &str) {
    session.set_ods_output(&[(
        table.to_string(),
        DatasetRef {
            libref: None,
            name: name.to_string(),
        },
    )]);
}

fn class_sex_session() -> Session {
    let mut session = make_session();
    let df = df!["sex" => ["M", "F", "F", "M", "M"]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("sex", 8)],
    };
    write_dataset(&mut session, "T", ds);
    session
}

fn t_ref() -> DatasetRef {
    DatasetRef {
        libref: Some("WORK".into()),
        name: "T".into(),
    }
}

/// Oracle M38.3 : `ods output OneWayFreqs=f;` autour d'un FREQ une voie →
/// dataset F avec les colonnes SAS réelles Table, F_<var>, <var>, Frequency,
/// Percent, CumFrequency, CumPercent (= colonnes du listing + Table/F_).
#[test]
fn one_way_freqs_captured_with_sas_columns() {
    let mut session = class_sex_session();
    ods_target(&mut session, "OneWayFreqs", "f");

    execute(
        &fast(t_ref(), vec![tr(&["sex"], false, None)]),
        &mut session,
    )
    .unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("F").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Table",
            "F_sex",
            "sex",
            "Frequency",
            "Percent",
            "CumFrequency",
            "CumPercent"
        ]
    );
    assert_eq!(out.n_obs(), 2, "une observation par catégorie");

    // Table = « Table sex » sur chaque ligne ; catégories en ordre sas_cmp.
    assert_eq!(
        read_col(&session, "F", "Table"),
        vec![
            Value::Char("Table sex".into()),
            Value::Char("Table sex".into())
        ]
    );
    assert_eq!(
        read_col(&session, "F", "F_sex"),
        vec![Value::Char("F".into()), Value::Char("M".into())]
    );
    assert_eq!(
        read_col(&session, "F", "Frequency"),
        vec![Value::Num(2.0), Value::Num(3.0)]
    );
    assert_eq!(
        read_col(&session, "F", "CumFrequency"),
        vec![Value::Num(2.0), Value::Num(5.0)]
    );
    // Percent en PLEINE précision (pas la chaîne « 40.00 » du listing).
    assert_eq!(
        read_col(&session, "F", "Percent"),
        vec![Value::Num(40.0), Value::Num(60.0)]
    );
    assert_eq!(
        read_col(&session, "F", "CumPercent"),
        vec![Value::Num(40.0), Value::Num(100.0)]
    );

    // Labels SAS des colonnes cumulées.
    let cum = out.vars.iter().find(|v| v.name == "CumFrequency").unwrap();
    assert_eq!(cum.label.as_deref(), Some("Cumulative Frequency"));
    let cump = out.vars.iter().find(|v| v.name == "CumPercent").unwrap();
    assert_eq!(cump.label.as_deref(), Some("Cumulative Percent"));

    // _LAST_ pointe sur le dataset capturé, NOTE émise par le flush.
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.F"));
}

/// Deux requêtes TABLES une voie → UN dataset, tranches empilées avec union
/// diagonale des colonnes (F_x/x puis F_sex/sex remplies de missings hors de
/// leur tranche), comme le OneWayFreqs de SAS pour `tables x sex;`.
#[test]
fn one_way_freqs_two_tables_stack_diagonally() {
    let mut session = make_session();
    let df = df![
        "x" => [Some(1.0_f64), Some(1.0), Some(2.0)],
        "sex" => ["M", "F", "F"],
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), char_meta("sex", 8)],
    };
    write_dataset(&mut session, "T", ds);
    ods_target(&mut session, "OneWayFreqs", "f");

    execute(
        &fast(
            t_ref(),
            vec![tr(&["x"], false, None), tr(&["sex"], false, None)],
        ),
        &mut session,
    )
    .unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("F").unwrap();
    // Colonnes en ordre de première apparition : la tranche `x` d'abord, puis
    // les colonnes propres à `sex` ajoutées à droite.
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Table",
            "F_x",
            "x",
            "Frequency",
            "Percent",
            "CumFrequency",
            "CumPercent",
            "F_sex",
            "sex"
        ]
    );
    assert_eq!(out.n_obs(), 4, "2 catégories de x + 2 de sex");
    // `x` est missing sur les lignes de la tranche `sex`, et réciproquement.
    assert_eq!(
        read_col(&session, "F", "x"),
        vec![
            Value::Num(1.0),
            Value::Num(2.0),
            Value::Missing(MissingKind::Dot),
            Value::Missing(MissingKind::Dot)
        ]
    );
    assert_eq!(
        read_col(&session, "F", "Table"),
        vec![
            Value::Char("Table x".into()),
            Value::Char("Table x".into()),
            Value::Char("Table sex".into()),
            Value::Char("Table sex".into())
        ]
    );
}

/// Sans `ODS OUTPUT`, aucune capture : ni dataset, ni tampon en attente —
/// l'invariant byte-identique du chemin par défaut.
#[test]
fn one_way_freqs_not_requested_writes_nothing() {
    let mut session = class_sex_session();

    execute(
        &fast(t_ref(), vec![tr(&["sex"], false, None)]),
        &mut session,
    )
    .unwrap();
    session.flush_ods_output().unwrap();

    assert!(session.libs.get("WORK").unwrap().read("F").is_err());
    assert!(session.ods_output_pending.is_empty());
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.T"));
}

/// Les options d'affichage suppriment les mêmes colonnes que dans le listing
/// (oracle M38.3 : « colonnes du listing FREQ »).
#[test]
fn one_way_freqs_display_options_drop_columns() {
    let mut session = class_sex_session();
    ods_target(&mut session, "OneWayFreqs", "f");

    let mut req = tr(&["sex"], false, None);
    req.nocum = true;
    execute(&fast(t_ref(), vec![req]), &mut session).unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("F").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["Table", "F_sex", "sex", "Frequency", "Percent"]);
}
