// ── J02-P4 : contrat FACTOR (replis silencieux supprimés) ────────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic (texte, code de sortie). Oracle de sévérité : CONTRIBUTING
// §5 ; syntaxe de référence : SAS/STAT 14.1 (SAS 9.4) User's Guide, The
// FACTOR Procedure, Syntax
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_factor_syntax.htm

use super::*;

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
        !out.listing.contains("The FACTOR Procedure"),
        "{ctx}: the step must be rejected before any output\n{}",
        out.listing
    );
}

/// m27 fixture data (one factor retained by MINEIGEN) plus a constant `z`.
const T: &str = "data t; input x y z; datalines;
1 2 7
2 3 7
3 3 7
4 5 7
5 4 7
;
run;
";

/// Base : une table au format TYPE=CORR/COV (`_TYPE_`/`_NAME_`, lignes MEAN,
/// STD, N, CORR) était analysée comme des observations brutes. SAS/STAT 9.4,
/// PROC FACTOR statement, DATA= : une telle table est lue comme une matrice.
/// ERROR (unité du manifeste : roadmap-avancee J09-P5).
#[test]
fn ra_j02_p4_factor_type_corr_input() {
    let out = run_sas(
        "data c; input _TYPE_ $ _NAME_ $ x y; datalines;
MEAN . 3 3.4
STD . 1.58 1.14
N . 5 5
CORR x 1 0.83
CORR y 0.83 1
;
run;
proc factor data=c; var x y; run;",
    );
    assert_error(
        &out,
        "ERROR: A TYPE=CORR/COV input data set (WORK.C has _TYPE_ and _NAME_ variables) is not \
         supported in PROC FACTOR; it can affect results and cannot be ignored (planned: \
         roadmap-avancee J09-P5).",
        "TYPE=CORR",
    );
}

/// Base : avec la matrice de corrélation, une variable de variance nulle
/// voyait ses corrélations forcées à 0 (diagonale 1) : facteur inventé,
/// aucun diagnostic. ERROR nommant la variable (J09-P5 alignera sur le
/// comportement documenté) ; l'analyse COV reste définie.
#[test]
fn ra_j02_p4_factor_zero_variance() {
    let out = run_sas(&format!("{T}proc factor data=t; var x z y; run;"));
    assert_error(
        &out,
        "ERROR: The VAR variable Z has zero variance (its correlations are undefined), which is \
         not supported in PROC FACTOR; it can affect results and cannot be ignored (planned: \
         roadmap-avancee J09-P5).",
        "zero variance",
    );
    let out = run_sas(&format!("{T}proc factor data=t cov; var x z y; run;"));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing.contains("Eigenvalues of the Covariance Matrix"),
        "{}",
        out.listing
    );
}

/// Base : ROTATE=VARIMAX/PROMAX avec un seul facteur retenu sautait la
/// rotation en silence (listing identique à ROTATE=NONE). NOTE ; le listing
/// reste celui de la solution non tournée.
#[test]
fn ra_j02_p4_factor_single_factor_rotation_note() {
    let plain = run_sas(&format!("{T}proc factor data=t; var x y; run;"));
    for rotate in ["varimax", "promax"] {
        let out = run_sas(&format!(
            "{T}proc factor data=t rotate={rotate}; var x y; run;"
        ));
        assert_eq!(out.exit_code, 0, "{rotate}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "NOTE: Only one factor is retained in PROC FACTOR; the ROTATE={} rotation, \
                 which needs at least two factors, is not performed.",
                rotate.to_uppercase()
            )),
            "{rotate}: {}",
            out.log
        );
        assert_eq!(out.listing, plain.listing, "{rotate}");
    }
    // ROTATE=NONE: no NOTE.
    assert!(!plain.log.contains("rotation"), "{}", plain.log);
}

/// Base : la boucle VARIMAX s'arrêtait au plafond d'itérations et le motif
/// passait pour convergé. WARNING (VARIMAX et pré-rotation PROMAX) ; une
/// rotation convergée n'en émet pas.
#[test]
fn ra_j02_p4_factor_varimax_non_convergence_warning() {
    let loadings = vec![
        vec![0.8, 0.2],
        vec![0.7, 0.5],
        vec![0.3, 0.9],
        vec![0.6, 0.1],
    ];
    // Two factors need a second sweep to confirm convergence.
    assert!(!varimax_with_limit(&loadings, 1).converged);
    assert!(varimax(&loadings).converged);

    for rotate in ["varimax", "promax"] {
        let mut session = crate::testkit::make_session();
        let rotation = rotate_loadings(&mut session, rotate, &loadings, 2, 1).unwrap();
        assert!(rotation.is_some(), "{rotate}");
        let log = session.log.current_text();
        assert!(
            log.contains(
                "WARNING: The VARIMAX rotation did not converge after 1 iterations in PROC \
                 FACTOR; the rotated factor pattern may be inaccurate."
            ),
            "{rotate}: {log}"
        );

        let mut session = crate::testkit::make_session();
        rotate_loadings(&mut session, rotate, &loadings, 2, VARIMAX_MAX_ITER).unwrap();
        let log = session.log.current_text();
        assert!(!log.contains("WARNING"), "{rotate}: {log}");
    }

    // End to end: the converged fixture rotation carries no WARNING.
    let out = run_sas(
        "data v; input x y z; datalines;
1 2 5
2 4 4
3 3 3
4 5 2
5 1 1
6 6 6
;
run;
proc factor data=v nfactors=2 rotate=varimax; var x y z; run;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing.contains("Rotation Method: Varimax"),
        "{}",
        out.listing
    );
}

/// Base : OUT= était validé après l'impression du listing — matrice de
/// corrélation singulière (y = 2x) ou libref non assigné : listing complet
/// puis ERROR. Validés désormais avant toute sortie ; sans OUT= l'analyse
/// s'imprime normalement.
#[test]
fn ra_j02_p4_factor_out_validated_before_output() {
    let singular = "data s; input x z; y = 2 * x; datalines;
1 5
2 3
3 4
4 1
5 2
;
run;
";
    let out = run_sas(&format!(
        "{singular}proc factor data=s out=sc; var x y z; run;
         proc print data=sc; run;"
    ));
    assert_error(
        &out,
        "ERROR: The correlation matrix is singular; PROC FACTOR cannot compute the OUT= factor \
         scores.",
        "singular",
    );
    assert!(!out.log.contains("WORK.SC has"), "{}", out.log);

    let out = run_sas(&format!("{singular}proc factor data=s; var x y z; run;"));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing.contains("The FACTOR Procedure"),
        "{}",
        out.listing
    );

    let out = run_sas(&format!(
        "{T}proc factor data=t out=nolib.sc; var x y; run;"
    ));
    assert_error(&out, "ERROR: Libref NOLIB is not assigned.", "libref");
}

/// Base : les instructions FACTOR valides non implémentées PARTIAL et PRIORS
/// étaient signalées « 180-322 … not valid ». Message du catalogue « not
/// supported … cannot be ignored » (BY, FREQ, WEIGHT : déjà le message
/// partagé) ; PATHDIAGRAM (graphique seul) → WARNING d'affichage ; une
/// instruction inventée reste une 180-322.
#[test]
fn ra_j02_p4_factor_unsupported_statements() {
    for stmt in ["partial z", "priors 1 1", "by z", "freq z", "weight z"] {
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        let out = run_sas(&format!("{T}proc factor data=t; var x y; {stmt}; run;"));
        assert_error(
            &out,
            &format!(
                "ERROR: The {kw} statement is not supported in PROC FACTOR; it can affect \
                 results and cannot be ignored."
            ),
            stmt,
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }

    let plain = run_sas(&format!("{T}proc factor data=t; var x y; run;"));
    let out = run_sas(&format!(
        "{T}proc factor data=t; var x y; pathdiagram; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert!(
        out.log.contains(
            "WARNING: The PATHDIAGRAM statement is ignored in PROC FACTOR; display \
             customization is not supported."
        ),
        "{}",
        out.log
    );
    assert_eq!(out.listing, plain.listing);

    let out = run_sas(&format!("{T}proc factor data=t; var x y; invented x; run;"));
    assert_error(
        &out,
        "180-322: Statement 'INVENTED' is not valid or it is used out of proper order in PROC \
         FACTOR.",
        "invented",
    );
}
