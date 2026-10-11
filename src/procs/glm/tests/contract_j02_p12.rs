// ── J02-P12 : correctif contrat J02 (constats du paragraphe J02-P8) ───────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, code de sortie). Oracle de
// sévérité : CONTRIBUTING §5 ; sémantique de référence : SAS/STAT 13.1
// User's Guide (SAS 9.4), The GLM Procedure —
// PROC GLM statement https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax01.htm
// MODEL statement https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax16.htm
// LSMEANS statement https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax10.htm
// ESTIMATE statement https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax07.htm
// « Specification of ESTIMATE Expressions » (A Check for Estimability)
// https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_details25.htm
// « The Four Types of Estimable Functions », Estimability
// https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_introglmest_sect002.htm
// GLM est hors du périmètre fonctionnel de roadmap-avancee : aucune ERROR
// ne nomme d'unité.

use super::contract::{CANNOT, DATA, assert_rejected, glm, run_sas};

/// Base : les options du statement PROC GLM autres que DATA= étaient sautées
/// jeton par jeton. SAS/STAT 9.4 (GLM), PROC GLM statement : ORDER=
/// (« specifies the order in which to sort classification variables … This
/// ordering determines which parameters in the model correspond to each
/// level in the data »), OUTSTAT= (« names an output data set »), ALPHA=
/// (« specifies the level of significance for confidence intervals ») et
/// MANOVA (« requests the multivariate mode of eliminating observations
/// with missing values ») peuvent changer un résultat ou créer une table →
/// ERROR ; NOPRINT (« suppresses the normal display of results »), PLOTS=
/// (ODS Graphics) et NAMELEN= (longueur des noms d'effets) ne changent que
/// l'affichage → WARNING (non appliquées) ; MULTIPASS (espace disque
/// contre temps de lecture) n'a aucun effet observable → acceptée sans
/// diagnostic ; option inconnue → ERROR « Unexpected option ».
#[test]
fn ra_j02_p12_glm_proc_options() {
    for (opt, name) in [
        ("order=data", "ORDER="),
        ("order=formatted", "ORDER="),
        ("outstat=work.o", "OUTSTAT="),
        ("alpha=0.01", "ALPHA="),
        ("manova", "MANOVA"),
    ] {
        let out = run_sas(&format!(
            "{DATA}proc glm data=t {opt}; class g; model y = g; run;"
        ));
        assert_rejected(
            &out,
            &format!("ERROR: The {name} option of the PROC GLM statement {CANNOT}."),
            opt,
        );
    }

    // Display-only options: WARNING, the model still runs (code 1).
    for (opt, name) in [
        ("noprint", "NOPRINT option of the PROC GLM statement"),
        ("plots=none", "PLOTS= option of the PROC GLM statement"),
        (
            "plots=(diagnostics residuals)",
            "PLOTS= option of the PROC GLM statement",
        ),
        ("namelen=32", "NAMELEN= option of the PROC GLM statement"),
    ] {
        let out = run_sas(&format!(
            "{DATA}proc glm data=t {opt}; class g; model y = g; run;"
        ));
        assert_eq!(out.exit_code, 1, "{opt}: {}", out.log);
        let msg = format!(
            "WARNING: The {name} is ignored in PROC GLM; display customization is not supported."
        );
        assert!(
            out.log.contains(&msg),
            "{opt}: missing «{msg}»\n{}",
            out.log
        );
        assert!(
            out.listing.contains("The GLM Procedure"),
            "{opt}: the step runs\n{}",
            out.listing
        );
    }

    // MULTIPASS has no observable effect: accepted without a diagnostic.
    let out = run_sas(&format!(
        "{DATA}proc glm data=t multipass; class g; model y = g; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(!out.log.contains("WARNING"), "{}", out.log);

    // Unknown option.
    let out = run_sas(&format!(
        "{DATA}proc glm data=t bogus=1; class g; model y = g; run;"
    ));
    assert_rejected(
        &out,
        "ERROR: Unexpected option 'BOGUS' on PROC GLM statement.",
        "bogus",
    );
}

/// Base : les options de MODEL autres que SOLUTION étaient sautées jeton par
/// jeton — `model y = g / noint;` ajustait le modèle AVEC constante, `/ p`
/// n'imprimait rien. SAS/STAT 9.4 (GLM), MODEL statement : NOINT (« omits
/// the intercept parameter from the model »), SS2/SS4, E2/E4, ALPHA=, CLI,
/// CLM, CLPARM, P, INVERSE, XPX, EFFECTSIZE, INTERCEPT, ALIASING,
/// TOLERANCE, SINGULAR= et ZETA= changent le modèle ajusté, ajoutent des
/// résultats calculés ou règlent les calculs → ERROR ; NOPRINT et NOUNI
/// suppriment des résultats affichés, E, E1 et E3 affichent des fonctions
/// estimables dont les sommes de carrés sont celles déjà imprimées →
/// WARNING (non appliquées) ; SS1 et SS3 demandent des tables que PROC GLM
/// « also displays by default » → acceptées sans diagnostic.
#[test]
fn ra_j02_p12_glm_model_options() {
    for (opt, name) in [
        ("noint", "NOINT"),
        ("ss2", "SS2"),
        ("ss4", "SS4"),
        ("e2", "E2"),
        ("e4", "E4"),
        ("alpha=0.10", "ALPHA="),
        ("cli", "CLI"),
        ("clm", "CLM"),
        ("clparm", "CLPARM"),
        ("p", "P"),
        ("inverse", "INVERSE"),
        ("xpx", "XPX"),
        ("effectsize", "EFFECTSIZE"),
        ("intercept", "INTERCEPT"),
        ("aliasing", "ALIASING"),
        ("tolerance", "TOLERANCE"),
        ("singular=1e-6", "SINGULAR="),
        ("zeta=1e-9", "ZETA="),
    ] {
        let out = glm(&format!("class g; model y = g / {opt};"));
        assert_rejected(
            &out,
            &format!("ERROR: The {name} option of the MODEL statement {CANNOT}."),
            opt,
        );
    }

    // Display-only options: WARNING, the model still runs (code 1).
    for name in ["NOPRINT", "NOUNI", "E", "E1", "E3"] {
        let out = glm(&format!("class g; model y = g / {};", name.to_lowercase()));
        assert_eq!(out.exit_code, 1, "{name}: {}", out.log);
        let msg = format!(
            "WARNING: The {name} option of the MODEL statement is ignored in PROC GLM; display \
             customization is not supported."
        );
        assert!(
            out.log.contains(&msg),
            "{name}: missing «{msg}»\n{}",
            out.log
        );
        assert!(
            out.listing.contains("The GLM Procedure"),
            "{name}: the step runs\n{}",
            out.listing
        );
    }

    // Already produced by default (doc: SS1/SS3 « are also displayed by
    // default ») or honored: no diagnostic.
    for opt in ["solution", "ss1", "ss3", "solution ss3"] {
        let out = glm(&format!("class g; model y = g / {opt};"));
        assert_eq!(out.exit_code, 0, "{opt}: {}", out.log);
        assert!(!out.log.contains("WARNING"), "{opt}: {}", out.log);
    }

    // Unknown option.
    let out = glm("class g; model y = g / bogus;");
    assert_rejected(
        &out,
        "ERROR: Unexpected option 'BOGUS' on the PROC GLM MODEL statement.",
        "bogus",
    );
}

/// Base : les options de LSMEANS étaient sautées jusqu'au `;`. SAS/STAT 9.4
/// (GLM), LSMEANS statement : STDERR « produces the standard error of the
/// LS-mean and the probability level » (déjà imprimés ; abréviation SE,
/// comme dans `lsmeans sex / se;`) → acceptée sans diagnostic ; E
/// (« displays the coefficients of the linear functions »), NOPRINT
/// (« suppresses the normal display of results from the LSMEANS
/// statement ») et PLOT= (ODS Graphics) ne changent que l'affichage →
/// WARNING (non appliquées) ; PDIFF, ADJUST=, OUT=, CL, ALPHA=, E=, TDIFF,
/// SLICE=… ajoutent des tests, des p-values ajustées, des limites de
/// confiance ou une table de sortie → ERROR ; effet d'interaction → ERROR
/// (il change les LS-means calculées).
#[test]
fn ra_j02_p12_glm_lsmeans_options() {
    for (opt, name) in [
        ("pdiff", "PDIFF"),
        ("diff", "DIFF"),
        ("adjust=tukey", "ADJUST="),
        ("out=work.l", "OUT="),
        ("cl", "CL"),
        ("alpha=0.01", "ALPHA="),
        ("e=g", "E="),
        ("etype=3", "ETYPE="),
        ("tdiff", "TDIFF"),
        ("slice=g", "SLICE="),
        ("om", "OM"),
    ] {
        let out = glm(&format!("class g; model y = g; lsmeans g / {opt};"));
        assert_rejected(
            &out,
            &format!("ERROR: The {name} option of the LSMEANS statement {CANNOT}."),
            opt,
        );
    }

    // Display-only options: WARNING, the LS-means are still printed (code 1).
    for (opt, name) in [
        ("e", "E option of the LSMEANS statement"),
        ("noprint", "NOPRINT option of the LSMEANS statement"),
        ("plot=meanplot", "PLOT= option of the LSMEANS statement"),
    ] {
        let out = glm(&format!("class g; model y = g; lsmeans g / {opt};"));
        assert_eq!(out.exit_code, 1, "{opt}: {}", out.log);
        let msg = format!(
            "WARNING: The {name} is ignored in PROC GLM; display customization is not supported."
        );
        assert!(
            out.log.contains(&msg),
            "{opt}: missing «{msg}»\n{}",
            out.log
        );
        assert!(
            out.listing.contains("Least Squares Means"),
            "{opt}: the LS-means are printed\n{}",
            out.listing
        );
    }

    // STDERR (and its abbreviations) is already produced: no diagnostic.
    for opt in ["stderr", "se", "std"] {
        let out = glm(&format!("class g; model y = g; lsmeans g / {opt};"));
        assert_eq!(out.exit_code, 0, "{opt}: {}", out.log);
        assert!(!out.log.contains("WARNING"), "{opt}: {}", out.log);
        assert!(
            out.listing.contains("Least Squares Means"),
            "{opt}: {}\n{}",
            out.listing,
            out.log
        );
    }

    // Unknown option, and an interaction effect (base: `lsmeans g*h;` was
    // read as two main-effect LSMEANS G and H).
    let out = glm("class g; model y = g; lsmeans g / bogus;");
    assert_rejected(
        &out,
        "ERROR: Unexpected option 'BOGUS' on the PROC GLM LSMEANS statement.",
        "bogus",
    );
    let out = glm("class g h; model y = g h; lsmeans g*h;");
    assert_rejected(
        &out,
        &format!("ERROR: An interaction effect in the LSMEANS statement {CANNOT}."),
        "g*h",
    );
}

/// Base : seule la dernière instruction LSMEANS (et la dernière MEANS)
/// était gardée — `lsmeans g; lsmeans g;` n'imprimait qu'une table, sans
/// diagnostic. Doc SAS/STAT 9.4 (GLM), LSMEANS statement : « Multiple
/// effects can be specified in one LSMEANS statement, or multiple LSMEANS
/// statements can be used » ; MEANS statement : « You can use any number of
/// MEANS statements ». Toutes les instructions sont désormais produites,
/// dans l'ordre du source (chemin une voie et multiway).
/// Case-insensitive substring count in the listing.
fn count_ci(listing: &str, needle: &str) -> usize {
    let hay = listing.to_lowercase();
    let needle = needle.to_lowercase();
    hay.matches(&needle).count()
}

#[test]
fn ra_j02_p12_glm_multiple_lsmeans_and_means_statements() {
    let out = glm("class g; model y = g; lsmeans g; lsmeans g;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(
        count_ci(&out.listing, "Least Squares Means"),
        2,
        "two LSMEANS statements → two tables\n{}",
        out.listing
    );

    let out = glm("class g; model y = g; means g; means g;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(
        count_ci(&out.listing, "Level of G"),
        2,
        "two MEANS statements → two tables\n{}",
        out.listing
    );

    // One statement listing several effects keeps printing one table per
    // effect, and the multiway path produces every statement too.
    let out = glm("class g h; model y = g h; lsmeans g; lsmeans h;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(
        count_ci(&out.listing, "Least Squares Means"),
        2,
        "{}",
        out.listing
    );
}

/// Base : un ESTIMATE une voie sans coefficient INTERCEPT dont les
/// coefficients ne somment pas à zéro était imprimé Σ cᵢ ȳᵢ
/// (`estimate 'A' g 1 0;` imprimait ȳ₁ = 1.666667, code 0). SAS/STAT 9.4
/// (GLM) : ESTIMATE statement — « The linear function is checked for
/// estimability » ; « A Check for Estimability » — L est « declared
/// nonestimable » quand ABS(L − LH) dépasse la tolérance SINGULAR=
/// (défaut 1e-4) ; ch. 15 « Estimability » — « Lβ is estimable if and only
/// if there is a K such that L = KX » : pour le dessin une voie avec
/// intercept, les lignes de X engendrent {(1, e₁) … (1, e_k)} et
/// L = (0, c₁…c_k) est estimable ssi Σ cᵢ = 0. La doc publiée ne fixe pas
/// le rendu SAS d'un estimate non estimable (aucune des pages citées
/// n'imprime « Non-est ») : ERROR provisoire plutôt qu'un nombre inventé ;
/// un ESTIMATE dont la somme est nulle reste calculé (−5), et un CONTRAST
/// à somme non nulle est inchangé (hors du périmètre de l'unité, constat
/// transmis au plan).
#[test]
fn ra_j02_p12_glm_nonestimable_estimate_is_error() {
    for stmt in [
        "estimate 'A' g 1 0;",
        "estimate 'A' g 1 1;",
        "estimate 'A' g 2;",
    ] {
        let out = glm(&format!("class g; model y = g; {stmt}"));
        assert_rejected(
            &out,
            &format!(
                "ERROR: An ESTIMATE statement whose coefficients do not sum to zero (not \
                 estimable without INTERCEPT) {CANNOT}."
            ),
            stmt,
        );
    }

    // Estimable functions (Σ cᵢ = 0, decimals included) are still computed:
    // ȳa − ȳb = 5/3 − 20/3 = −5, and half of it.
    let out = glm("class g; model y = g; estimate 'a-b' g 1 -1;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("-5.000000"), "{}", out.listing);
    let out = glm("class g; model y = g; estimate 'half diff' g 0.5 -0.5;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("-2.500000"), "{}", out.listing);

    // A CONTRAST with a non-zero sum is unchanged (out of the unit scope).
    let out = glm("class g; model y = g; contrast 'A' g 1 0;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("Contrasts"), "{}", out.listing);
}
