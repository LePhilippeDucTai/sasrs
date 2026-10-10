// ── J02-P4 : contrat FASTCLUS (replis silencieux supprimés) ──────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic (texte, code de sortie). Oracle de sévérité : CONTRIBUTING
// §5 ; syntaxe de référence : SAS/STAT 14.1 (SAS 9.4) User's Guide, The
// FASTCLUS Procedure, PROC FASTCLUS Statement
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_fastclus_syntax01.htm

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
        !out.listing.contains("The FASTCLUS Procedure"),
        "{ctx}: the step must be rejected before any output\n{}",
        out.listing
    );
}

/// Points of the m27 fixture (two groups).
const PTS: &str = "data pts; input x; datalines;
1
2
3
7
8
9
;
run;
";

/// Text of the approximation NOTE logged by every PROC FASTCLUS run.
const APPROX_NOTE: &str = "NOTE: PROC FASTCLUS approximates the SAS algorithm: seeds are \
     selected farthest-first, MAXITER= defaults to 10 and MAXCLUSTERS= is required (SAS: \
     RADIUS=/REPLACE= seed rules, MAXITER=1, MAXCLUSTERS=100); cluster assignments can differ \
     from SAS (planned: roadmap-avancee J09-P7).";

/// Base : une valeur manquante d'une variable VAR entrait dans les graines,
/// les distances et les centroïdes comme NaN, sans diagnostic. ERROR jusqu'à
/// roadmap-avancee J09-P7 (règles SAS des manquants).
#[test]
fn ra_j02_p4_fastclus_missing_values() {
    let out = run_sas(
        "data m; input x; datalines;
1
2
.
7
8
9
;
run;
proc fastclus data=m maxclusters=2 out=cl; var x; run;
proc print data=cl; run;",
    );
    assert_error(
        &out,
        "ERROR: A missing value in a VAR variable (X, observation 3) is not supported in PROC \
         FASTCLUS; it can affect results and cannot be ignored (planned: roadmap-avancee \
         J09-P7).",
        "missing",
    );
    assert!(!out.log.contains("WORK.CL has"), "{}", out.log);
}

/// Base : `SEED=123` était accepté (NOTE « is accepted ») puis ignoré, les
/// graines restant farthest-first. SAS/STAT 9.4, PROC FASTCLUS statement :
/// « SEED=SAS-data-set specifies an input data set from which initial
/// cluster seeds are to be selected ». Un nombre → ERROR ; une table →
/// ERROR « not supported » (J09-P7).
#[test]
fn ra_j02_p4_fastclus_seed_number() {
    let out = run_sas(&format!(
        "{PTS}proc fastclus data=pts maxclusters=2 seed=123; var x; run;"
    ));
    assert_error(
        &out,
        "ERROR: SEED= names a SAS data set of initial cluster seeds in PROC FASTCLUS, not a \
         number.",
        "number",
    );
    assert!(!out.log.contains("is accepted"), "{}", out.log);

    let out = run_sas(&format!(
        "{PTS}proc fastclus data=pts maxclusters=2 seed=work.pts; var x; run;"
    ));
    assert_error(
        &out,
        "ERROR: The SEED= option (data set of initial cluster seeds) is not supported in PROC \
         FASTCLUS; it can affect results and cannot be ignored (planned: roadmap-avancee \
         J09-P7).",
        "data set",
    );
}

/// Base : le semis farthest-first, MAXITER=10 par défaut et MAXCLUSTERS=
/// requis divergeaient de SAS sans aucun diagnostic. SAS/STAT 9.4, PROC
/// FASTCLUS statement : « If you omit the MAXCLUSTERS= option, a value of
/// 100 is assumed » ; MAXITER= vaut 1 par défaut (sans LEAST=) ; graines
/// par les règles RADIUS=/REPLACE=. NOTE à chaque exécution (approximation
/// documentée dans docs/support-contract.md), résultats inchangés.
#[test]
fn ra_j02_p4_fastclus_approximation_note() {
    let out = run_sas(&format!(
        "{PTS}proc fastclus data=pts maxclusters=2 maxiter=20; var x; run;
         proc fastclus data=pts maxclusters=2; var x; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(out.log.matches(APPROX_NOTE).count(), 2, "{}", out.log);
    // m27 oracle unchanged: two clusters of 3, centroids 6 apart.
    assert!(out.listing.contains("Maxiter=20"), "{}", out.listing);
    assert!(out.listing.contains("Maxiter=10"), "{}", out.listing);
    assert!(
        out.listing.contains("      1            3     0.8165"),
        "{}",
        out.listing
    );
}

/// Base : les instructions FASTCLUS valides non implémentées sont déjà
/// rejetées par le message partagé (ID, BY, FREQ, WEIGHT) ; ce test fige
/// ce contrat — aucune 180-322 — et l'ERROR 180-322 d'une instruction
/// inventée.
#[test]
fn ra_j02_p4_fastclus_unsupported_statements() {
    for stmt in ["id x", "by x", "freq x", "weight x"] {
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        let out = run_sas(&format!(
            "{PTS}proc fastclus data=pts maxclusters=2; var x; {stmt}; run;"
        ));
        assert_error(
            &out,
            &format!(
                "ERROR: The {kw} statement is not supported in PROC FASTCLUS; it can affect \
                 results and cannot be ignored."
            ),
            stmt,
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    let out = run_sas(&format!(
        "{PTS}proc fastclus data=pts maxclusters=2; var x; invented x; run;"
    ));
    assert_error(
        &out,
        "180-322: Statement 'INVENTED' is not valid or it is used out of proper order in PROC \
         FASTCLUS.",
        "invented",
    );
}
