// ── J02-P8 : contrat PROC GLM (replis silencieux supprimés) ──────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic (texte, code de sortie). Oracle de sévérité : CONTRIBUTING
// §5 ; syntaxe de référence : SAS/STAT 13.1 User's Guide (SAS 9.4), The GLM
// Procedure —
// Syntax https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax.htm
// CONTRAST statement https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax06.htm
// ESTIMATE statement https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax07.htm
// GLM est hors du périmètre fonctionnel de roadmap-avancee : aucune ERROR
// ne nomme d'unité.

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

/// g: a (1 2 2) / b (4 7 9) ; h: x (1 4 2) / y (2 7 9).
const DATA: &str = "data t; input g $ h $ y; datalines;
a x 1
a y 2
b x 4
b y 7
a x 2
b y 9
;
run;
";

/// Contract ERROR suffix shared by the rejected requests.
const CANNOT: &str = "is not supported in PROC GLM; it can affect results and cannot be ignored";

/// `proc glm data=t; <body> run;` after the DATA step.
fn glm(body: &str) -> crate::RunOutcome {
    run_sas(&format!("{DATA}proc glm data=t; {body} run;"))
}

/// Asserts a step rejected before execution: exit code 2, `needle` in the
/// log, no GLM output at all.
fn assert_rejected(out: &crate::RunOutcome, needle: &str, ctx: &str) {
    assert_eq!(out.exit_code, 2, "{ctx}: {}", out.log);
    assert!(
        out.log.contains(needle),
        "{ctx}: missing «{needle}»\n{}",
        out.log
    );
    assert!(
        !out.listing.contains("The GLM Procedure"),
        "{ctx}: the step must be rejected before any output\n{}",
        out.listing
    );
}

/// Base : les options après `/` d'ESTIMATE et de CONTRAST étaient sautées
/// jusqu'au `;` — `estimate 'a-b' g 1 -1 / divisor=2;` imprimait -5 (au lieu
/// de -2.5), code 0. ESTIMATE : DIVISOR= « specifies a value by which to
/// divide all coefficients », SINGULAR= « tunes the estimability checking » ;
/// CONTRAST : E= (terme d'erreur), ETYPE=, SINGULAR= → ERROR ; E (affichage
/// du vecteur L) → WARNING d'affichage ; option inconnue → ERROR.
#[test]
fn ra_j02_p8_glm_estimate_contrast_options() {
    for (stmt, opt) in [
        ("estimate 'a-b' g 1 -1", "divisor=2"),
        ("estimate 'a-b' g 1 -1", "singular=1e-6"),
        ("contrast 'a-b' g 1 -1", "e=g"),
        ("contrast 'a-b' g 1 -1", "etype=3"),
        ("contrast 'a-b' g 1 -1", "singular=1e-6"),
    ] {
        let out = glm(&format!("class g; model y = g; {stmt} / {opt};"));
        let name = opt.split('=').next().unwrap().to_uppercase();
        let kind = stmt.split_whitespace().next().unwrap().to_uppercase();
        assert_rejected(
            &out,
            &format!("ERROR: The {name}= option of the {kind} statement {CANNOT}."),
            opt,
        );
    }

    for (stmt, bad) in [
        (
            "estimate 'a-b' g 1 -1 / bogus",
            "Unknown or unsupported ESTIMATE option 'BOGUS'",
        ),
        (
            "estimate 'a-b' g 1 -1 / e=g",
            "Unknown or unsupported ESTIMATE option 'E'",
        ),
        (
            "contrast 'a-b' g 1 -1 / divisor=2",
            "Unknown or unsupported CONTRAST option 'DIVISOR'",
        ),
    ] {
        let out = glm(&format!("class g; model y = g; {stmt};"));
        assert_rejected(&out, &format!("ERROR: {bad} in PROC GLM."), stmt);
    }

    let out = glm("class g; model y = g; estimate 'a-b' g 1 -1 / e; contrast 'a-b' g 1 -1 / e;");
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for kind in ["ESTIMATE", "CONTRAST"] {
        let msg = format!(
            "WARNING: The E option of the {kind} statement is ignored in PROC GLM; display \
             customization is not supported."
        );
        assert!(out.log.contains(&msg), "missing «{msg}»\n{}", out.log);
    }
    // Both tables are produced: ȳa − ȳb = 5/3 − 20/3 = −5.
    assert!(out.listing.contains("Contrasts"), "{}", out.listing);
    assert!(out.listing.contains("-5.000000"), "{}", out.listing);
}

/// Base : sur le chemin multiway (plusieurs effets ou plusieurs variables
/// CLASS), ESTIMATE et CONTRAST d'effet principal étaient ignorés sans
/// sortie ni NOTE (code 0). ERROR avant exécution, aucune unité nommée
/// (GLM hors du périmètre fonctionnel du plan).
#[test]
fn ra_j02_p8_glm_multiway_estimate_contrast_is_error() {
    for (body, kind) in [
        (
            "class g h; model y = g h; estimate 'a-b' g 1 -1;",
            "An ESTIMATE",
        ),
        (
            "class g h; model y = g h; contrast 'a-b' g 1 -1;",
            "A CONTRAST",
        ),
        (
            "class g h; model y = g h g*h; contrast 'x-y' h 1 -1;",
            "A CONTRAST",
        ),
        // Two CLASS variables and a single effect also run the multi-way
        // engine.
        (
            "class g h; model y = g; estimate 'a-b' g 1 -1;",
            "An ESTIMATE",
        ),
    ] {
        let out = glm(body);
        assert_rejected(
            &out,
            &format!(
                "ERROR: {kind} statement in a model with several effects or CLASS variables \
                 {CANNOT}."
            ),
            body,
        );
    }

    // The multi-way model itself still runs without ESTIMATE/CONTRAST.
    let out = glm("class g h; model y = g h;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("Type III SS"), "{}", out.listing);
}

/// Base : sur le chemin une voie, un ESTIMATE/CONTRAST portant sur un autre
/// effet que celui du MODEL (INTERCEPT compris) était abandonné sans un mot
/// (code 0) ; sans nom d'effet, idem. ERROR avant exécution ; l'effet du
/// MODEL, dans n'importe quelle casse, reste calculé.
#[test]
fn ra_j02_p8_glm_oneway_estimate_on_another_effect_is_error() {
    for (stmt, msg) in [
        (
            "estimate 'x-y' h 1 -1",
            format!(
                "ERROR: An ESTIMATE statement on the effect H, which is not the MODEL effect \
                 G, {CANNOT}."
            ),
        ),
        (
            "estimate 'mean' intercept 1",
            format!(
                "ERROR: An ESTIMATE statement on the effect INTERCEPT, which is not the MODEL \
                 effect G, {CANNOT}."
            ),
        ),
        (
            "contrast 'x-y' h 1 -1",
            format!(
                "ERROR: A CONTRAST statement on the effect H, which is not the MODEL effect \
                 G, {CANNOT}."
            ),
        ),
        (
            "estimate 'a-b' 1 -1",
            "ERROR: expected an effect name in the ESTIMATE statement".to_string(),
        ),
    ] {
        let out = glm(&format!("class g; model y = g; {stmt};"));
        assert_rejected(&out, &msg, stmt);
    }

    let out = glm("class g; model y = g; estimate 'a-b' G 1 -1;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("-5.000000"), "{}", out.listing);
}

/// Base : ABSORB, CODE, MANOVA, RANDOM, REPEATED, STORE et TEST (instructions
/// SAS/STAT valides de PROC GLM) étaient signalées « 180-322: Statement
/// 'TEST' is not valid … ». Message du catalogue du contrat ; une
/// instruction inventée garde 180-322.
#[test]
fn ra_j02_p8_glm_valid_statements_are_not_supported() {
    for stmt in [
        "absorb h",
        "code file='score.sas'",
        "manova h=g",
        "random g",
        "repeated time 2",
        "store out=work.glmstore",
        "test h=g e=g",
    ] {
        let out = glm(&format!("class g; model y = g; {stmt};"));
        let name = stmt.split_whitespace().next().unwrap().to_uppercase();
        assert_rejected(
            &out,
            &format!("ERROR: The {name} statement {CANNOT}."),
            stmt,
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }

    let out = glm("class g; model y = g; invented g;");
    assert_rejected(
        &out,
        "ERROR: 180-322: Statement 'INVENTED' is not valid or it is used out of proper order \
         in PROC GLM.",
        "invented",
    );
}
