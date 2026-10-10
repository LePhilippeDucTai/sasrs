// ── J02-P4 : contrat PRINCOMP (replis silencieux supprimés) ──────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic (texte, code de sortie). Oracle de sévérité : CONTRIBUTING
// §5 ; syntaxe de référence : SAS/STAT 14.1 (SAS 9.4) User's Guide, The
// PRINCOMP Procedure, PROC PRINCOMP Statement
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_princomp_syntax01.htm

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
        !out.listing.contains("The PRINCOMP Procedure"),
        "{ctx}: the step must be rejected before any output\n{}",
        out.listing
    );
}

/// A TYPE=CORR data set as PROC CORR OUTP= lays it out (SAS 9.4, Appendix A
/// « Special SAS Data Sets »): `_TYPE_`/`_NAME_` plus one row per statistic.
const TYPE_CORR: &str = "data c; input _TYPE_ $ _NAME_ $ x y; datalines;
MEAN . 3 3.4
STD . 1.58 1.14
N . 5 5
CORR x 1 0.83
CORR y 0.83 1
;
run;
";

/// m27 fixture data plus a constant `z` (and `w`, whose mean is inexact in
/// binary floating point).
const ZV: &str = "data t; input x y z w; datalines;
1 2 7 0.1
2 3 7 0.1
3 3 7 0.1
4 5 7 0.1
5 4 7 0.1
;
run;
";

/// Base : une table au format TYPE=CORR/COV (`_TYPE_`/`_NAME_`, lignes MEAN,
/// STD, N, CORR) était analysée comme 5 observations brutes. SAS/STAT 9.4,
/// PROC PRINCOMP statement, DATA= : une telle table est lue comme une
/// matrice. ERROR jusqu'à roadmap-avancee J09-P5 ; une table brute portant
/// seulement `_NAME_` reste analysée.
#[test]
fn ra_j02_p4_princomp_type_corr_input() {
    let out = run_sas(&format!("{TYPE_CORR}proc princomp data=c; var x y; run;"));
    assert_error(
        &out,
        "ERROR: A TYPE=CORR/COV input data set (WORK.C has _TYPE_ and _NAME_ variables) is not \
         supported in PROC PRINCOMP; it can affect results and cannot be ignored (planned: \
         roadmap-avancee J09-P5).",
        "TYPE=CORR",
    );
    let out = run_sas(
        "data raw; input _NAME_ $ x y; datalines;
a 1 2
b 2 3
c 3 3
d 4 5
e 5 4
;
run;
proc princomp data=raw; var x y; run;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("1.8321"), "{}", out.listing);
}

/// Base : avec la matrice de corrélation, une variable de variance nulle
/// voyait ses corrélations forcées à 0 (diagonale 1) : valeur propre 1
/// inventée, aucun diagnostic. ERROR nommant la variable (J09-P5 alignera
/// sur le comportement documenté) ; l'analyse COV reste définie.
#[test]
fn ra_j02_p4_princomp_zero_variance() {
    for (vars, name) in [("x y z", "Z"), ("x w y", "W")] {
        let out = run_sas(&format!("{ZV}proc princomp data=t; var {vars}; run;"));
        assert_error(
            &out,
            &format!(
                "ERROR: The VAR variable {name} has zero variance (its correlations are \
                 undefined), which is not supported in PROC PRINCOMP; it can affect results \
                 and cannot be ignored (planned: roadmap-avancee J09-P5)."
            ),
            vars,
        );
    }
    let out = run_sas(&format!("{ZV}proc princomp data=t cov; var x y z; run;"));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing.contains("Eigenvalues of the Covariance Matrix"),
        "{}",
        out.listing
    );
}

/// Base : `jacobi` rendait sa dernière itérée sans convergence ; une
/// matrice de covariance infinie (dépassement sur des valeurs ~1e200)
/// produisait des valeurs propres NaN imprimées sans diagnostic. ERROR typée
/// (`numerical error`) avant toute sortie.
#[test]
fn ra_j02_p4_princomp_non_finite_matrix() {
    let out = run_sas(
        "data big; input x y; datalines;
1e200 1
2e200 2
3e200 4
;
run;
proc princomp data=big cov; var x y; run;",
    );
    assert_error(
        &out,
        "ERROR: numerical error: matrix has missing or infinite entries (Jacobi)",
        "overflow",
    );
    assert!(!out.listing.contains("NaN"), "{}", out.listing);
}

/// Base : l'instruction PRINCOMP valide non implémentée PARTIAL était
/// signalée « 180-322 … not valid ». Message du catalogue « not supported …
/// cannot be ignored » (BY, FREQ, ID, WEIGHT : déjà le message partagé) ;
/// une instruction inventée reste une 180-322.
#[test]
fn ra_j02_p4_princomp_unsupported_statements() {
    for stmt in ["partial w", "by w", "freq w", "id w", "weight w"] {
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        let out = run_sas(&format!("{ZV}proc princomp data=t; var x y; {stmt}; run;"));
        assert_error(
            &out,
            &format!(
                "ERROR: The {kw} statement is not supported in PROC PRINCOMP; it can affect \
                 results and cannot be ignored."
            ),
            stmt,
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    let out = run_sas(&format!(
        "{ZV}proc princomp data=t; var x y; invented x; run;"
    ));
    assert_error(
        &out,
        "180-322: Statement 'INVENTED' is not valid or it is used out of proper order in PROC \
         PRINCOMP.",
        "invented",
    );
}
