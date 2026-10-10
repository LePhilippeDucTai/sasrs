// ── J02-P4 : contrat CLUSTER (replis silencieux supprimés) ───────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic (texte, code de sortie). Oracle de sévérité : CONTRIBUTING
// §5 ; syntaxe de référence : SAS/STAT 14.1 (SAS 9.4) User's Guide, The
// CLUSTER Procedure, Syntax
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_cluster_syntax.htm

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

/// Asserts a rejected step: exit code 2, `needle` in the log, no listing.
fn assert_error(out: &crate::RunOutcome, needle: &str, ctx: &str) {
    assert_eq!(out.exit_code, 2, "{ctx}: {}", out.log);
    assert!(
        out.log.contains(needle),
        "{ctx}: missing «{needle}»\n{}",
        out.log
    );
    assert!(
        !out.listing.contains("The CLUSTER Procedure"),
        "{ctx}: the step must be rejected before any output\n{}",
        out.listing
    );
}

/// Labelled points of the m34 OUTTREE fixture.
const PTS: &str = "data pts; input name $ x y; datalines;
P1 1 1
P2 1 2
P3 8 8
P4 9 8
P5 9 9
;
run;
";

/// Base : une valeur manquante d'une variable VAR entrait dans toutes les
/// distances comme NaN ; l'historique fusionnait des grappes NaN sans
/// diagnostic et OUTTREE= était écrit. ERROR jusqu'à roadmap-avancee J09-P6
/// (observations exclues comme SAS).
#[test]
fn ra_j02_p4_cluster_missing_values() {
    let out = run_sas(
        "data m; input x y; datalines;
1 1
2 .
8 8
9 9
;
run;
proc cluster data=m method=ward outtree=tree; var x y; run;
proc print data=tree; run;",
    );
    assert_error(
        &out,
        "ERROR: A missing value in a VAR variable (Y, observation 2) is not supported in PROC \
         CLUSTER; it can affect results and cannot be ignored (planned: roadmap-avancee \
         J09-P6).",
        "missing",
    );
    assert!(!out.log.contains("WORK.TREE has"), "{}", out.log);
}

/// Base : `ID a b;` ne gardait que la première variable ; les instructions
/// CLUSTER valides non implémentées COPY et RMSSTD étaient signalées
/// « 180-322 … not valid ». SAS/STAT 9.4 : `ID variable;` ; COPY/RMSSTD →
/// message du catalogue « not supported … cannot be ignored » (BY, FREQ :
/// déjà le message partagé) ; une instruction inventée reste une 180-322.
#[test]
fn ra_j02_p4_cluster_unsupported_statements() {
    let out = run_sas(&format!(
        "{PTS}proc cluster data=pts method=average; var x y; id name x; run;"
    ));
    assert_error(
        &out,
        "ERROR: The ID statement of PROC CLUSTER takes a single variable; found NAME X.",
        "id",
    );

    for stmt in ["copy name", "rmsstd x", "by name", "freq x"] {
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        let out = run_sas(&format!(
            "{PTS}proc cluster data=pts method=average; var x y; {stmt}; run;"
        ));
        assert_error(
            &out,
            &format!(
                "ERROR: The {kw} statement is not supported in PROC CLUSTER; it can affect \
                 results and cannot be ignored."
            ),
            stmt,
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    let out = run_sas(&format!(
        "{PTS}proc cluster data=pts method=average; var x y; invented x; run;"
    ));
    assert_error(
        &out,
        "180-322: Statement 'INVENTED' is not valid or it is used out of proper order in PROC \
         CLUSTER.",
        "invented",
    );

    // A single ID variable keeps labelling the leaves.
    let out = run_sas(&format!(
        "{PTS}proc cluster data=pts method=average; var x y; id name; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("P1"), "{}", out.listing);
}
