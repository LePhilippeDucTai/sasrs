//! J08-P3 — ODS OUTPUT des objets FREQ : `CrossTabFreqs`, `ChiSq`,
//! `FishersExact`.

use super::*;

fn ods_targets(session: &mut Session, mappings: &[(&str, &str)]) {
    session.set_ods_output(
        mappings
            .iter()
            .map(|(t, n)| {
                (
                    t.to_string(),
                    DatasetRef {
                        libref: None,
                        name: n.to_string(),
                    },
                )
            })
            .collect::<Vec<_>>()
            .as_slice(),
    );
}

/// Dataset r (char) × c (num) 2×2 :
/// r = a,a,b,b ; c = 1,2,1,1 → cellules (a,1)=1 (a,2)=1 (b,1)=2 (b,2)=0.
fn rc_session() -> Session {
    let mut session = make_session();
    let ds = SasDataset {
        df: df![
            "r" => ["a", "a", "b", "b"],
            "c" => [1.0_f64, 2.0, 1.0, 1.0],
        ]
        .unwrap(),
        vars: vec![char_meta("r", 1), num_meta("c")],
    };
    write_dataset(&mut session, "T", ds);
    session
}

/// Une valeur d'axe de marge : manquante, ou chaîne vide (décodage Polars
/// d'un null caractère).
fn is_missing_or_blank(v: &Value) -> bool {
    match v {
        Value::Missing(_) => true,
        Value::Char(s) => s.is_empty(),
        _ => false,
    }
}

/// Dataset sex (char) une voie : M,F,F,M,M.
fn sex_session() -> Session {
    let mut session = make_session();
    let ds = SasDataset {
        df: df!["sex" => ["M", "F", "F", "M", "M"]].unwrap(),
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

/// `ods output CrossTabFreqs=x;` → colonnes SAS réelles Table, <row>, <col>,
/// Frequency, Percent, RowPercent, ColPercent ; une ligne par cellule PLUS
/// les marges (variable d'axe manquante) et le total général.
#[test]
fn ods_output_object_freq_crosstabfreqs() {
    let mut session = rc_session();
    ods_targets(&mut session, &[("CrossTabFreqs", "x")]);

    execute(
        &fast(t_ref(), vec![tr(&["r", "c"], false, None)]),
        &mut session,
    )
    .unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("X").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Table",
            "r",
            "c",
            "Frequency",
            "Percent",
            "RowPercent",
            "ColPercent"
        ]
    );
    // 4 cellules + 2 marges de ligne + 2 marges de colonne + 1 total = 9.
    assert_eq!(out.n_obs(), 9, "cellules + marges + total général");

    let tables = read_col(&session, "X", "Table");
    assert_eq!(tables[0], Value::Char("Table of r by c".into()));

    // Cellule (b,1) : fréquence 2, row pct 100, col pct 66.666….
    let r_col = read_col(&session, "X", "r");
    let c_col = read_col(&session, "X", "c");
    let freq = read_col(&session, "X", "Frequency");
    let rowp = read_col(&session, "X", "RowPercent");
    let idx = r_col
        .iter()
        .zip(c_col.iter())
        .position(|(rv, cv)| *rv == Value::Char("b".into()) && *cv == Value::Num(1.0))
        .expect("cellule (b,1)");
    assert_eq!(freq[idx], Value::Num(2.0));
    assert_eq!(rowp[idx], Value::Num(100.0));

    // Dernière ligne : total général, les deux axes manquants, Percent = 100.
    let last = out.n_obs() - 1;
    assert!(
        is_missing_or_blank(&r_col[last]),
        "marge r manquante, reçue {:?}",
        r_col[last]
    );
    assert!(
        is_missing_or_blank(&c_col[last]),
        "marge c manquante, reçue {:?}",
        c_col[last]
    );
    assert_eq!(freq[last], Value::Num(4.0));
    assert_eq!(read_col(&session, "X", "Percent")[last], Value::Num(100.0));
    // RowPercent manquant sur la marge.
    assert!(
        matches!(rowp[last], Value::Missing(_)),
        "RowPercent manquant sur la marge"
    );
}

/// `ods output ChiSq=k;` sur un crosstab CHISQ → colonnes SAS réelles Table,
/// Statistic, DF, Value, Prob — une ligne par statistique.
#[test]
fn ods_output_object_freq_chisq_twoway() {
    let mut session = rc_session();
    ods_targets(&mut session, &[("ChiSq", "k")]);

    let mut req = tr(&["r", "c"], false, None);
    req.chisq = true;
    execute(&fast(t_ref(), vec![req]), &mut session).unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("K").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["Table", "Statistic", "DF", "Value", "Prob"]);
    assert_eq!(out.n_obs(), 2, "Pearson + Likelihood Ratio");

    let stat = read_col(&session, "K", "Statistic");
    assert_eq!(stat[0], Value::Char("Chi-Square".into()));
    assert_eq!(stat[1], Value::Char("Likelihood Ratio Chi-Square".into()));
    // Pearson χ² de la table [[1,1],[2,0]] : attendus [[1.5,0.5],[1.5,0.5]]
    // → 0.25·(0.5/1.5 + 0.5/0.5 + 0.5/1.5 + 0.5/0.5) = 2/3.
    let value = read_col(&session, "K", "Value");
    match &value[0] {
        Value::Num(v) => assert!((v - 4.0_f64 / 3.0).abs() < 1e-9, "χ² = {v}"),
        v => panic!("Value doit être numérique, reçu {v:?}"),
    }
    assert_eq!(read_col(&session, "K", "DF")[0], Value::Num(1.0));
}

/// `ods output ChiSq=k;` sur un CHISQ une voie → même structure, colonne
/// Table = « Table <var> », une seule ligne.
#[test]
fn ods_output_object_freq_chisq_oneway() {
    let mut session = sex_session();
    ods_targets(&mut session, &[("ChiSq", "k")]);

    let mut req = tr(&["sex"], false, None);
    req.chisq = true;
    execute(&fast(t_ref(), vec![req]), &mut session).unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("K").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["Table", "Statistic", "DF", "Value", "Prob"]);
    assert_eq!(out.n_obs(), 1);
    assert_eq!(
        read_col(&session, "K", "Table")[0],
        Value::Char("Table sex".into())
    );
    assert_eq!(read_col(&session, "K", "DF")[0], Value::Num(1.0));
    // χ² une voie de M/F = 3/2 : (3−2.5)²/2.5 × 2 = 0.2.
    match &read_col(&session, "K", "Value")[0] {
        Value::Num(v) => assert!((v - 0.2).abs() < 1e-9, "χ² = {v}"),
        v => panic!("Value doit être numérique, reçu {v:?}"),
    }
}

/// `ods output FishersExact=f;` sur un 2×2 FISHER → structure SAS : Table
/// puis un triple (Name_i, cValue_i, nValue_i) par statistique, UNE
/// observation.
#[test]
fn ods_output_object_freq_fishersexact() {
    let mut session = make_session();
    let ds = SasDataset {
        df: df![
            "a" => [1.0_f64, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0],
            "b" => [1.0_f64, 1.0, 1.0, 2.0, 1.0, 2.0, 2.0, 2.0],
        ]
        .unwrap(),
        vars: vec![num_meta("a"), num_meta("b")],
    };
    write_dataset(&mut session, "T", ds);
    ods_targets(&mut session, &[("FishersExact", "f")]);

    let mut req = tr(&["a", "b"], false, None);
    req.fisher = true;
    execute(&fast(t_ref(), vec![req]), &mut session).unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("F").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Table", "Name1", "cValue1", "nValue1", "Name2", "cValue2", "nValue2", "Name3",
            "cValue3", "nValue3", "Name4", "cValue4", "nValue4", "Name5", "cValue5", "nValue5",
        ]
    );
    assert_eq!(out.n_obs(), 1, "une observation, agencement horizontal");

    assert_eq!(
        read_col(&session, "F", "Name1")[0],
        Value::Char("Cell (1,1) Frequency (F)".into())
    );
    // Table 3/1 // 1/3 → F = 3.
    assert_eq!(read_col(&session, "F", "nValue1")[0], Value::Num(3.0));
    assert_eq!(
        read_col(&session, "F", "Name5")[0],
        Value::Char("Two-sided Pr <= P".into())
    );
    // p bilatéral exact de la table [[3,1],[1,3]] = 0.4857…, dans [0,1].
    match &read_col(&session, "F", "nValue5")[0] {
        Value::Num(p) => assert!(
            (p - 0.4857).abs() < 0.01 && (0.0..=1.0).contains(p),
            "p = {p}"
        ),
        v => panic!("nValue5 doit être numérique, reçu {v:?}"),
    }
}
