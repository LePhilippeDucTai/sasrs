// ── J02-P6 : contrat PROC PLOT (replis silencieux supprimés) ──────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic (texte, code de sortie). Oracle de sévérité : CONTRIBUTING §5
// (personnalisation d'affichage non rendue → WARNING, code 1 ; demande qui
// change les objets produits → ERROR, étape rejetée, code 2). Syntaxe de
// référence : SAS 9.4 Base Procedures Guide, The PLOT Procedure.

/// Exécute un programme complet (mode déterministe) : log, listing, code.
fn run_sas(src: &str) -> crate::RunOutcome {
    let tmp = tempfile::tempdir().unwrap();
    crate::run(
        src,
        crate::RunOptions {
            deterministic: true,
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        },
    )
}

/// WORK.XY shared by the tests.
const XY: &str = "data xy; input x y g $; datalines;
1 10 a
2 20 b
3 15 a
;
run;
";

fn assert_has(out: &crate::RunOutcome, needle: &str) {
    assert!(out.log.contains(needle), "missing «{needle}»\n{}", out.log);
}

/// Base : seules les options `NAME=` après le `/` donnaient un WARNING (J02-P4
/// du plan de consolidation) ; BOX, OVERLAY et les autres options nues
/// étaient sautées sans diagnostic, et le symbole de `y*x='*'` était ignoré
/// (le listing trace les lettres A, B… des effectifs). Désormais un WARNING
/// pour chacun ; le tracé reste produit dans le listing.
#[test]
fn ra_j02_p6_plot_bare_options_and_symbol_warn() {
    let out = run_sas(&format!(
        "{XY}proc plot data=xy;
           plot y*x='*' / box overlay href=2 hzero;
         run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    let expected = [
        "WARNING: The plotting symbol '*' is ignored in PROC PLOT; display customization is \
         not supported.",
        "WARNING: The BOX option is ignored in PROC PLOT; display customization is not \
         supported.",
        "WARNING: The OVERLAY option is ignored in PROC PLOT; display customization is not \
         supported.",
        "WARNING: The HREF= option is ignored in PROC PLOT; display customization is not \
         supported.",
        "WARNING: The HZERO option is ignored in PROC PLOT; display customization is not \
         supported.",
    ];
    for needle in expected {
        assert_has(&out, needle);
    }
    assert_eq!(
        out.log.matches("WARNING:").count(),
        expected.len(),
        "{}",
        out.log
    );
    // The plot is still drawn, with the overlap letters (not the symbol).
    assert!(
        out.listing
            .contains("Plot of Y*X.  Legend: A = 1 obs, B = 2 obs, etc."),
        "{}",
        out.listing
    );
}

/// Base : BY recevait le message partagé sans nommer l'unité qui le lèvera ;
/// les options du statement PROC PLOT étaient sautées jeton par jeton,
/// inconnues comprises. Désormais : BY → ERROR nommant J13-P4 (même texte que
/// SGPLOT, GPLOT, GCHART), l'étape suivante s'exécute ; options d'affichage
/// (NOLEGEND, HPERCENT=…) → WARNING ; option inconnue → « Unexpected option ».
#[test]
fn ra_j02_p6_plot_by_and_proc_options() {
    let out = run_sas(&format!(
        "{XY}proc plot data=xy; by g; plot y*x; run;
         data _null_; put 'NEXT_STEP'; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: The BY statement is not supported in PROC PLOT; it can affect results and \
         cannot be ignored (planned: roadmap-avancee J13-P4).",
    );
    assert_has(&out, "NEXT_STEP");

    let out = run_sas(&format!(
        "{XY}proc plot data=xy nolegend hpercent=50 formchar(2)='|'; plot y*x; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for option in ["NOLEGEND", "HPERCENT=", "FORMCHAR"] {
        assert_has(
            &out,
            &format!(
                "WARNING: The {option} option is ignored in PROC PLOT; display customization \
                 is not supported."
            ),
        );
    }
    assert!(out.listing.contains("Plot of Y*X"), "{}", out.listing);

    let out = run_sas(&format!("{XY}proc plot data=xy foo; plot y*x; run;"));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: Unexpected option 'FOO' on PROC PLOT statement.",
    );
}
