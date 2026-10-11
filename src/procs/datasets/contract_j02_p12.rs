// ── J02-P12 : correctif contrat J02 (constats du paragraphe J02-P8) ───────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// la correction ou l'ERROR (texte, code de sortie). Oracle de sévérité :
// CONTRIBUTING §5 ; sémantique de référence : Base SAS 9.4 Procedures
// Guide, DATASETS Procedure —
// DELETE statement (MEMTYPE=)
// https://support.sas.com/documentation/cdl/en/proc/65145/HTML/default/n1dsrfw3zlxkfqn1wct59dubq3mp.htm
// INFORMAT statement
// https://support.sas.com/documentation/cdl/en/proc/65145/HTML/default/n18g9ro4y99vlmn1ccwbe0zbvtfk.htm

use super::contract_tests::{CANNOT, contents_row, run_sas};

/// Base : dans DELETE, SAVE, CHANGE, EXCHANGE et SELECT, les jetons autres
/// que des noms (`/ MEMTYPE=`, listes `( … )`, autres options) étaient
/// sautés jusqu'au `;`. Doc SAS 9.4 DATASETS, DELETE statement : MEMTYPE=
/// « restricts processing to one member type », « Alias: MT=, MTYPE= »,
/// « Default: DATA ». `/ MEMTYPE=DATA` est honoré (tout membre d'une
/// bibliothèque sasrs est une table DATA, le comportement est inchangé).
#[test]
fn ra_j02_p12_datasets_memtype_data_is_honored() {
    for (stmt, note) in [
        ("delete m / memtype=data;", "Deleting WORK.M"),
        ("delete m / mt=data;", "Deleting WORK.M"),
        ("delete m / mtype=DATA;", "Deleting WORK.M"),
        ("save keep / memtype=data;", "Deleting WORK.M"),
        (
            "change m=mm / memtype=data;",
            "Changing the name WORK.M to WORK.MM",
        ),
        (
            "exchange m=keep / memtype=data;",
            "Exchanging the names WORK.M",
        ),
    ] {
        let out = run_sas(&format!(
            "data m; x = 1; run;\ndata keep; x = 2; run;
             proc datasets lib=work nolist; {stmt} quit;"
        ));
        assert_eq!(out.exit_code, 0, "{stmt}: {}", out.log);
        assert!(!out.log.contains("WARNING"), "{stmt}: {}", out.log);
        assert!(!out.log.contains("ERROR"), "{stmt}: {}", out.log);
        assert!(
            out.log.contains(note),
            "{stmt}: missing «{note}»\n{}",
            out.log
        );
    }

    // SELECT (of COPY) honors MEMTYPE=DATA too: the member is copied.
    let out = run_sas(
        "data m; x = 1; run;
         proc datasets lib=work nolist;
           copy out=work in=work;
           select m / memtype=data;
         quit;
         proc print data=m; run;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.log.contains("Copying WORK.M"), "{}", out.log);
}

/// Même constat, volet ERROR : toute autre valeur de MEMTYPE= ou toute autre
/// option, une liste entre parenthèses (options par fichier : ALTER=,
/// GENNUM=…) et une plage de noms (`m1-m2`) sont rejetées AVANT toute
/// exécution du groupe — le DELETE écrit avant le statement fautif ne
/// s'exécute pas (base : `delete m / memtype=catalog;` supprimait la table
/// M, perte de données).
#[test]
fn ra_j02_p12_datasets_member_list_tokens_are_errors() {
    for (stmt, msg) in [
        (
            "delete m / memtype=catalog;",
            format!("ERROR: The MEMTYPE=CATALOG option of the DELETE statement {CANNOT}."),
        ),
        (
            "delete m / mt=view;",
            format!("ERROR: The MT=VIEW option of the DELETE statement {CANNOT}."),
        ),
        (
            "save m keep / memtype=all;",
            format!("ERROR: The MEMTYPE=ALL option of the SAVE statement {CANNOT}."),
        ),
        (
            "change m=mm / gennum=all;",
            format!("ERROR: The GENNUM= option of the CHANGE statement {CANNOT}."),
        ),
        (
            "exchange m=keep / memtype=catalog;",
            format!("ERROR: The MEMTYPE=CATALOG option of the EXCHANGE statement {CANNOT}."),
        ),
        (
            "delete m (gennum=all);",
            format!(
                "ERROR: A member list or member options in parentheses in the DELETE statement \
                 {CANNOT}."
            ),
        ),
        (
            "delete m1-m2;",
            format!(
                "ERROR: A member list (range or prefix starting at M1) in the DELETE statement \
                 {CANNOT}."
            ),
        ),
    ] {
        // The DELETE written before the faulty statement must not run: the
        // step is rejected at parse time.
        let out = run_sas(&format!(
            "data m; x = 1; run;\ndata keep; x = 2; run;\ndata m1; x = 3; run;\ndata m2; x = 4; run;
             proc datasets lib=work nolist; delete keep; {stmt} quit;
             proc print data=keep; run;"
        ));
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert!(
            out.log.contains(&msg),
            "{stmt}: missing «{msg}»\n{}",
            out.log
        );
        assert!(
            out.log
                .contains("NOTE: There were 1 observations read from the data set WORK.KEEP."),
            "{stmt}: the step is rejected before the DELETE runs\n{}",
            out.log
        );
    }

    // SELECT (of COPY) options are errors too.
    let out = run_sas(
        "data m; x = 1; run;
         proc datasets lib=work nolist;
           copy out=work in=work;
           select m / memtype=catalog;
         quit;
         proc print data=m; run;",
    );
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert!(
        out.log.contains(&format!(
            "ERROR: The MEMTYPE=CATALOG option of the SELECT statement {CANNOT}."
        )),
        "{}",
        out.log
    );
}

/// Base : INFORMAT d'un groupe MODIFY lisait une variable par informat —
/// `informat a b date9.;` associait l'informat « b » à A puis échouait
/// (« The informat . is not valid. », étape rejetée). Doc SAS 9.4 DATASETS
/// INFORMAT statement : l'informat « specifies an informat for the
/// variables immediately preceding it in the statement. If you do not
/// specify an informat, the INFORMAT statement removes any existing
/// informats for the variables in variable-list » (exemple de la doc :
/// `informat a b 2. x1-x3 4.1 c;`). Désormais toutes les variables listées
/// reçoivent l'informat qui suit, comme le statement FORMAT ; une liste
/// finale sans informat retire l'informat ; informat invalide et listes
/// non supportées → ERROR avant toute exécution.
#[test]
fn ra_j02_p12_datasets_modify_informat_groups() {
    let out = run_sas(
        "data m; input a b c; informat c comma.; datalines;\n1 2 3\n;\nrun;
         proc datasets lib=work nolist;
           modify m;
             informat a b date9. c;
         quit;
         proc contents data=m; run;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(!out.log.contains("ERROR"), "{}", out.log);
    // A and B share the informat that follows the list; C, listed last
    // without an informat, has its stored informat removed.
    assert_eq!(
        contents_row(&out.listing, "1", "a"),
        ["1", "a", "Num", "8", "DATE"],
        "{}",
        out.listing
    );
    assert_eq!(
        contents_row(&out.listing, "2", "b"),
        ["2", "b", "Num", "8", "DATE"],
        "{}",
        out.listing
    );
    assert_eq!(
        contents_row(&out.listing, "3", "c"),
        ["3", "c", "Num", "8"],
        "{}",
        out.listing
    );

    // Several (variables, informat) groups in one statement, as in the doc
    // example `informat a b 2. x1-x3 4.1 c;` (ranges are not supported).
    let out = run_sas(
        "data m2; input x y; datalines;\n1 2\n;\nrun;
         proc datasets lib=work nolist;
           modify m2;
             informat x comma8. y date7.;
         quit;
         proc contents data=m2; run;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(
        contents_row(&out.listing, "2", "y"),
        ["2", "y", "Num", "8", "DATE"],
        "{}",
        out.listing
    );

    // Base : l'informat invalide était une ERROR d'exécution (après le
    // répertoire) ; les listes étaient sautées en silence. Rejet au parsing
    // désormais, avant toute sortie et avant le DELETE qui précède.
    for (stmt, msg) in [
        (
            "informat a .;",
            "ERROR: The informat . is not valid.".to_string(),
        ),
        (
            "informat _all_ date9.;",
            format!("ERROR: The variable list _ALL_ in the INFORMAT statement {CANNOT}."),
        ),
        (
            "informat a1-a3 date9.;",
            format!(
                "ERROR: A variable list (range or prefix starting at A1) in the INFORMAT \
                 statement {CANNOT}."
            ),
        ),
        (
            "informat 8.2;",
            "ERROR: expected a variable name before the informat in the INFORMAT statement"
                .to_string(),
        ),
    ] {
        let out = run_sas(&format!(
            "data m3; input a1 a2 a3; datalines;\n1 2 3\n;\nrun;
             proc datasets lib=work; delete m3; modify m3; {stmt} quit;
             proc print data=m3; run;"
        ));
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert!(
            out.log.contains(msg.as_str()),
            "{stmt}: missing «{msg}»\n{}",
            out.log
        );
        assert!(
            !out.listing.contains("Member Type"),
            "{stmt}: the step is rejected before any listing\n{}",
            out.listing
        );
        assert!(
            out.log
                .contains("NOTE: There were 1 observations read from the data set WORK.M3."),
            "{stmt}: the step is rejected before the DELETE runs\n{}",
            out.log
        );
    }
}
