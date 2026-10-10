// ── J02-P8 : contrat PROC DATASETS (replis silencieux supprimés) ─────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, code de sortie). Oracle de sévérité :
// CONTRIBUTING §5 ; sémantique de référence : Base SAS 9.4 Procedures Guide,
// DATASETS Procedure —
// Concepts (« Execution of Statements », « RUN-Group Processing »)
// https://support.sas.com/documentation/cdl/en/proc/65145/HTML/default/p0hvz6mq2yhkq6n19gfz4pkul8rc.htm
// FORMAT statement
// https://support.sas.com/documentation/cdl/en/proc/65145/HTML/default/n09nl7f8smat49n1rz1rav5i6a8a.htm
// Syntax (liste des statements)
// https://support.sas.com/documentation/cdl/en/proc/65145/HTML/default/p1v2467vdjbp7xn1222c7t3sejz3.htm

/// Exécute un programme complet (mode déterministe) : log, listing, code.
fn run_sas(src: &str) -> crate::RunOutcome {
    crate::run(
        src,
        crate::RunOptions {
            deterministic: true,
            ..Default::default()
        },
    )
}

/// Contract ERROR suffix shared by the rejected statements.
const CANNOT: &str =
    "is not supported in PROC DATASETS; it can affect results and cannot be ignored";

fn assert_has(out: &crate::RunOutcome, needle: &str) {
    assert!(out.log.contains(needle), "missing «{needle}»\n{}", out.log);
}

/// Whitespace-split listing line whose first two tokens are `n` and `var`
/// (a row of the PROC CONTENTS variable table).
fn contents_row(listing: &str, n: &str, var: &str) -> Vec<String> {
    listing
        .lines()
        .map(|l| l.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .find(|t| t.len() >= 2 && t[0] == n && t[1] == var)
        .unwrap_or_default()
}

/// Member names of a PROC DATASETS directory listing (`# Name Member Type`).
fn directory(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|t| t.len() == 3 && t[2] == "DATA" && t[0].parse::<usize>().is_ok())
        .map(|t| t[1].to_string())
        .collect()
}

/// Base : `format` mettait fin au groupe MODIFY — WARNING « The FORMAT
/// statement is ignored in PROC DATASETS; display customization is not
/// supported. », puis ERROR « 180-322: Statement 'RENAME' is not valid … »
/// (étape rejetée, code 2) ; sans RENAME, un LABEL suivant était lui aussi
/// abandonné avec le WARNING d'affichage. Correction directe — SAS 9.4
/// DATASETS FORMAT statement : « Assigns, changes, and removes variable
/// formats in the SAS data set specified in the MODIFY statement
/// permanently » ; `format-1` « specifies a format to apply to the variable
/// or variables listed before it. If you do not specify a format, the FORMAT
/// statement removes any format associated with the variables in
/// variable-list. » Le format est stocké dans les métadonnées (sidecar) : PROC
/// CONTENTS et PROC PRINT le voient.
#[test]
fn ra_j02_p8_datasets_modify_format_keeps_the_group() {
    let out = run_sas(
        "data m; x = 1.23456; y = 2; z = 3; w = 4; format z 5.1 w 6.2; run;
         proc datasets lib=work nolist;
           modify m;
             format x 8.2 z;
             rename y=yy;
             label x='Ex';
             informat x comma12.;
         quit;
         proc contents data=m; run;
         proc print data=m; run;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(!out.log.contains("WARNING"), "{}", out.log);
    // x: format assigned, label and informat of the same group applied.
    assert_eq!(
        contents_row(&out.listing, "1", "x"),
        ["1", "x", "Num", "8", "8.2", "COMMA", "Ex"],
        "{}",
        out.listing
    );
    // RENAME after FORMAT is applied (base: 180-322).
    assert_eq!(
        contents_row(&out.listing, "2", "yy"),
        ["2", "yy", "Num", "8"],
        "{}",
        out.listing
    );
    // z listed last without a format: its stored 5.1 format is removed.
    assert_eq!(
        contents_row(&out.listing, "3", "z"),
        ["3", "z", "Num", "8"],
        "{}",
        out.listing
    );
    // w is untouched.
    assert_eq!(
        contents_row(&out.listing, "4", "w"),
        ["4", "w", "Num", "8", "6.2"],
        "{}",
        out.listing
    );
    // PROC PRINT applies the stored format: 1.23456 → 1.23.
    assert!(
        out.listing
            .lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>() == ["1", "1.23", "2", "3", "4.00"]),
        "{}",
        out.listing
    );
}

/// FORMAT dans MODIFY : variable absente → WARNING (comme RENAME et LABEL),
/// format invalide et listes de variables non supportées → ERROR avant toute
/// exécution ; hors d'un groupe MODIFY, FORMAT/LABEL (base : WARNING
/// d'affichage trompeur, code 1) et RENAME/INFORMAT sont « used out of
/// proper order » (SAS 9.4 : « Must appear in a MODIFY RUN group »).
#[test]
fn ra_j02_p8_datasets_format_errors_and_placement() {
    let data = "data m; x = 1; run;\n";

    let out = run_sas(&format!(
        "{data}proc datasets lib=work nolist; modify m; format nope 8.2; quit;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert_has(
        &out,
        "WARNING: Variable NOPE not found in WORK.M; format not assigned.",
    );

    for (stmt, msg) in [
        ("format x .", "ERROR: The format . is not valid."),
        (
            "format _all_ 8.2",
            &format!("ERROR: The variable list _ALL_ in the FORMAT statement {CANNOT}."),
        ),
        (
            "format x1-x3 8.2",
            &format!(
                "ERROR: A variable list (range or prefix starting at X1) in the FORMAT \
                 statement {CANNOT}."
            ),
        ),
    ] {
        // The DELETE written before the faulty statement must not run: the
        // step is rejected at parse time.
        let out = run_sas(&format!(
            "{data}proc datasets lib=work nolist; delete m; modify m; {stmt}; quit;
             proc print data=m; run;"
        ));
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert_has(&out, msg);
        assert!(
            out.log
                .contains("NOTE: There were 1 observations read from the data set WORK.M."),
            "{stmt}: WORK.M must survive the rejected step\n{}",
            out.log
        );
    }

    for kw in ["format x 8.2", "label x='X'", "rename x=y", "informat x 8."] {
        let out = run_sas(&format!("{data}proc datasets lib=work nolist; {kw}; quit;"));
        let name = kw.split_whitespace().next().unwrap().to_uppercase();
        assert_eq!(out.exit_code, 2, "{kw}: {}", out.log);
        assert_has(
            &out,
            &format!(
                "ERROR: 180-322: Statement '{name}' is not valid or it is used out of proper \
                 order in PROC DATASETS."
            ),
        );
        assert!(!out.log.contains("display customization"), "{}", out.log);
    }

    // Comments and empty statements stay inside the MODIFY group.
    let out = run_sas(&format!(
        "{data}proc datasets lib=work nolist; modify m; * a comment; ; format x 8.3; \
         rename x=y; quit; proc contents data=m; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(
        contents_row(&out.listing, "1", "y"),
        ["1", "y", "Num", "8", "8.3"],
        "{}",
        out.listing
    );
}

/// Base : tous les DELETE, puis tous les CHANGE, puis les autres statements.
/// `change a=c; delete c;` supprimait C AVANT de renommer A (WARNING « Table
/// WORK.C does not exist », C survivait) ; `modify b; … change b=d;` renommait
/// B avant le MODIFY (WARNING « Member WORK.B not found; MODIFY skipped »).
/// Correction directe — SAS 9.4 DATASETS, « Execution of Statements » :
/// « Statements execute in the order in which they are written. » (Un seul
/// groupe d'instructions : un `run;` intérieur termine l'étape dans sasrs,
/// cf. l'en-tête du module.)
#[test]
fn ra_j02_p8_datasets_statements_run_in_source_order() {
    let out = run_sas(
        "data a; x = 1; run;
         data b; x = 2; run;
         proc datasets lib=work nolist;
           change a=c;
           delete c;
           modify b;
             rename x=y;
           change b=d;
         quit;
         proc contents data=d; run;
         proc datasets lib=work; quit;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    let change = out
        .log
        .find("NOTE: Changing the name WORK.A to WORK.C (memtype=DATA).")
        .expect(&out.log);
    let delete = out
        .log
        .find("NOTE: Deleting WORK.C (memtype=DATA).")
        .expect(&out.log);
    assert!(change < delete, "CHANGE runs before DELETE\n{}", out.log);
    let rename = out
        .log
        .find("NOTE: Variable X renamed to Y in WORK.B.")
        .expect(&out.log);
    let change_b = out
        .log
        .find("NOTE: Changing the name WORK.B to WORK.D (memtype=DATA).")
        .expect(&out.log);
    assert!(rename < change_b, "MODIFY runs before CHANGE\n{}", out.log);
    assert_eq!(
        contents_row(&out.listing, "1", "y"),
        ["1", "y", "Num", "8"],
        "{}",
        out.listing
    );
    assert_eq!(directory(&out.listing), ["D"], "{}", out.listing);
}

/// Base : le répertoire (sans NOLIST) était imprimé APRÈS toutes les
/// modifications. SAS 9.4 DATASETS, « RUN-Group Processing » : « The PROC
/// DATASETS statement always executes immediately. … the PROC DATASETS
/// statement alone is a RUN group » — le répertoire est celui de la
/// bibliothèque à l'ouverture de la procédure, avant le DELETE.
#[test]
fn ra_j02_p8_datasets_directory_listed_when_the_proc_starts() {
    let out = run_sas(
        "data a; x = 1; run;
         data b; x = 2; run;
         proc datasets lib=work;
           delete a;
         quit;
         proc datasets lib=work; quit;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    let first = out.listing.find("Member Type").expect(&out.listing);
    let second = out.listing.rfind("Member Type").expect(&out.listing);
    assert!(first < second, "two directory listings\n{}", out.listing);
    assert_eq!(
        directory(&out.listing[..second]),
        ["A", "B"],
        "directory of the first step, before DELETE\n{}",
        out.listing
    );
    assert_eq!(directory(&out.listing[second..]), ["B"], "{}", out.listing);
}

/// Base : les statements SAS valides non implémentés étaient signalés
/// « 180-322: Statement 'APPEND' is not valid or it is used out of proper
/// order » (ATTRIB : WARNING d'affichage, l'attribut était perdu) ; les
/// options du statement MODIFY donnaient « Statement '?' is not valid ».
/// Message du catalogue du contrat, unité qui lèvera l'ERROR nommée ;
/// l'étape est rejetée avant toute exécution (le DELETE qui précède ne
/// s'exécute pas).
#[test]
fn ra_j02_p8_datasets_valid_statements_are_not_supported() {
    for (stmt, name, unit) in [
        ("age a b", "AGE", None),
        ("append base=a data=b", "APPEND", Some("J03-P4")),
        ("audit a", "AUDIT", None),
        ("contents data=a", "CONTENTS", Some("J03-P4")),
        ("rebuild a", "REBUILD", None),
        ("repair a", "REPAIR", Some("J11-P6")),
        ("copy out=work; exclude a", "EXCLUDE", None),
        ("modify a; attrib x format=8.2", "ATTRIB", None),
        ("modify a; index create x", "INDEX", None),
        ("modify a; ic create primary key(x)", "IC", None),
        ("modify a; xattr add var x role='key'", "XATTR", None),
    ] {
        let out = run_sas(&format!(
            "data a; x = 1; run;
             data b; x = 2; run;
             proc datasets lib=work nolist; delete b; {stmt}; quit;
             proc print data=b; run;"
        ));
        let planned = unit
            .map(|u| format!(" (planned: roadmap-avancee {u})"))
            .unwrap_or_default();
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert_has(
            &out,
            &format!("ERROR: The {name} statement {CANNOT}{planned}."),
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
        assert!(
            out.log
                .contains("NOTE: There were 1 observations read from the data set WORK.B."),
            "{stmt}: the step is rejected before the DELETE runs\n{}",
            out.log
        );
    }

    for (stmt, what) in [
        (
            "modify a (label='new')",
            "A data set option list on the MODIFY statement",
        ),
        (
            "modify a / memtype=data",
            "An option after / on the MODIFY statement",
        ),
    ] {
        let out = run_sas(&format!(
            "data a; x = 1; run; proc datasets lib=work nolist; {stmt}; quit;"
        ));
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert_has(&out, &format!("ERROR: {what} {CANNOT}."));
    }

    // An invented statement keeps the 180-322 diagnostic.
    let out = run_sas("proc datasets lib=work nolist; invented x; quit;");
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: 180-322: Statement 'INVENTED' is not valid or it is used out of proper order \
         in PROC DATASETS.",
    );
}
