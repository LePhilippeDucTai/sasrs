use super::*;

// ── J07-P6 : VarMeta.informat — persistance de l'informat déclaré ────
//
// L'informat posé par INFORMAT / ATTRIB informat= est une MÉTADONNÉE : il
// voyage jusqu'au dataset de sortie (VarMeta.informat → sidecar). Ces
// tests prouvent la propriété : ils échouent si l'informat déclaré n'est
// plus persisté (ou si le sidecar ne le restitue plus).

/// `informat dt date9.; informat code $8.;` + affectations : les tokens
/// sont persistés tels quels dans WORK.out, lus depuis le sidecar.
#[test]
fn informat_meta_persisted_by_informat_statement() {
    let mut s = session();
    run(
        "data out; informat dt date9.; informat code $8.; \
         dt = '02JAN2020'd; code = 'XY'; output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let dt = ds.vars.iter().find(|v| v.name == "dt").unwrap();
    let code = ds.vars.iter().find(|v| v.name == "code").unwrap();
    assert_eq!(dt.informat.as_deref(), Some("date9."));
    assert_eq!(code.informat.as_deref(), Some("$8."));
    // L'informat $w. déclarée avant la première utilisation fixe la
    // longueur déclarée du caractère (comme LENGTH) : 8, pas 2 ('XY').
    assert_eq!(code.ty, VarType::Char);
    assert_eq!(code.length, 8);
    // Le numérique garde 8 (modèle SAS).
    assert_eq!(dt.ty, VarType::Num);
    assert_eq!(dt.length, 8);
}

/// Un informat NUMÉRIQUE avec décimales (`comma12.2`) : persisté, longueur
/// numérique inchangée.
#[test]
fn informat_meta_numeric_wd_token_persisted() {
    let mut s = session();
    run(
        "data out; informat amount comma12.2; amount = 1234.5; output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.vars[0].informat.as_deref(), Some("comma12.2"));
    assert_eq!(ds.vars[0].length, 8);
}

/// `attrib v informat=...;` — même canal déclaratif, même persistance.
#[test]
fn informat_meta_persisted_by_attrib() {
    let mut s = session();
    run(
        "data out; attrib d informat=date9.; d = '02JAN2020'd; output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.vars[0].informat.as_deref(), Some("date9."));
}

/// SET conserve l'informat persisté de la table lue : la métadonnée
/// survit au round-trip sidecar puis à une étape SET.
#[test]
fn informat_meta_preserved_by_set() {
    let mut s = session();
    run(
        "data src; informat d date9.; d = '02JAN2020'd; output; run;",
        &mut s,
    )
    .unwrap();
    run("data out; set src; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.vars[0].informat.as_deref(), Some("date9."));
}

/// Sans INFORMAT, VarMeta.informat est None (aucun effet de bord du champ).
#[test]
fn informat_meta_absent_without_declaration() {
    let mut s = session();
    run("data out; x = 1; output; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    assert!(ds.vars[0].informat.is_none());
}
