use super::*;

// ── INFORMAT crée la variable inconnue à sa position textuelle (J01-P3) ──
//
// Doc SAS 9.4, INFORMAT statement : comme LENGTH/FORMAT/ATTRIB, une
// variable encore inconnue référencée par INFORMAT entre au PDV à cette
// ligne (caractère si l'informat commence par `$`, numérique sinon), PAS
// seulement à sa première affectation. `compat/informat/informat-contents-declared`
// dépend de cet ordre pour PROC CONTENTS OUT=.

#[test]
fn informat_statement_creates_unknown_var_at_its_position() {
    let mut s = session();
    let prog = compile_src(
        "data typed; informat name $10.; informat amount comma12.2; \
         length note $3; name = 'Ann'; amount = 1234.5; note = 'abc'; \
         output; run;",
        &mut s,
    )
    .unwrap();
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    // name et amount (INFORMAT, lignes 1-2) précèdent note (LENGTH, ligne 3).
    assert_eq!(names, vec!["name", "amount", "note"]);
    assert_eq!(prog.pdv.vars()[0].ty, VarType::Char);
    // L'informat $10. fixe la longueur char déclarée à 10.
    assert_eq!(prog.pdv.vars()[0].length, 10);
    assert_eq!(prog.pdv.vars()[1].ty, VarType::Num);
    assert_eq!(prog.pdv.vars()[1].length, 8);
}

#[test]
fn informat_statement_numeric_default_type_and_length() {
    let mut s = session();
    let prog = compile_src(
        "data out; informat amount comma12.2; amount = 1234.5; output; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(prog.pdv.vars()[0].name, "amount");
    assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
    assert_eq!(prog.pdv.vars()[0].length, 8);
}
