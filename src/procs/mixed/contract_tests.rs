// ── J02-P2 : contrat MIXED (replis silencieux supprimés) ─────────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, code de sortie). Oracle de sévérité :
// CONTRIBUTING §5 ; syntaxe de référence : SAS/STAT 9.4 User's Guide, The
// MIXED Procedure, Syntax
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_mixed_syntax_toc.htm

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

/// Balanced random-intercept data of the m28 fixture.
const BAL: &str = "data bal; input subj $ g $ y; datalines;
A a 1
A b 3
B a 5
B b 7
;
run;
";

/// Repeated-measures data of the m34 fixture (complete and sorted).
const REP: &str = "data rep; input subj $ time y; datalines;
A 1 1
A 2 3
B 1 3
B 2 1
C 1 5
C 2 7
D 1 7
D 2 5
;
run;
";

/// Contract ERROR suffix shared by every rejected construction.
const CANNOT: &str = "is not supported in PROC MIXED; it can affect results and cannot be ignored";

/// Asserts a rejected step: exit code 2, `needle` in the log, no listing.
fn assert_error(out: &crate::RunOutcome, needle: &str, ctx: &str) {
    assert_eq!(out.exit_code, 2, "{ctx}: {}", out.log);
    assert!(
        out.log.contains(needle),
        "{ctx}: missing «{needle}»\n{}",
        out.log
    );
    assert!(
        !out.listing.contains("The Mixed Procedure"),
        "{ctx}: the step must be rejected before execution\n{}",
        out.listing
    );
}

/// Base : `TYPE=UN(1)` (UN à bande), `CS(2)`, `AR(2)` : la parenthèse était
/// avalée et la structure complète ajustée. ERROR ; `AR(1)` inchangé.
#[test]
fn ra_j02_p2_type_parenthesized_argument() {
    for (stmt, shown) in [
        ("repeated time / subject=subj type=un(1)", "TYPE=UN(1)"),
        ("repeated time / subject=subj type=ar(2)", "TYPE=AR(2)"),
        ("random intercept / subject=subj type=cs(2)", "TYPE=CS(2)"),
        ("random intercept / subject=subj type=vc(1)", "TYPE=VC(1)"),
    ] {
        let out = run_sas(&format!(
            "{REP}proc mixed data=rep; class subj time; model y = ; {stmt}; run;"
        ));
        assert_error(
            &out,
            &format!("{shown} (parameterized covariance structure) {CANNOT}."),
            stmt,
        );
    }
    let out = run_sas(&format!(
        "{REP}proc mixed data=rep; class subj time; model y = ;
         repeated time / subject=subj type=ar(1); run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("Autoregressive"), "{}", out.listing);
}

/// Base : plusieurs RANDOM — la dernière instruction gagnait en silence.
#[test]
fn ra_j02_p2_multiple_random_statements() {
    let out = run_sas(&format!(
        "{BAL}proc mixed data=bal; class subj g; model y = ;
         random intercept / subject=subj; random intercept / subject=g; run;"
    ));
    assert_error(
        &out,
        &format!("More than one RANDOM statement {CANNOT} (planned: roadmap-avancee J06-P2)."),
        "two RANDOM",
    );
}

/// Base : RANDOM + REPEATED — seule la structure REPEATED était ajustée,
/// l'effet aléatoire abandonné.
#[test]
fn ra_j02_p2_random_with_repeated() {
    let out = run_sas(&format!(
        "{REP}proc mixed data=rep; class subj time; model y = ;
         random intercept / subject=subj; repeated time / subject=subj type=un; run;"
    ));
    assert_error(
        &out,
        &format!(
            "Combining a RANDOM and a REPEATED statement {CANNOT} \
             (planned: roadmap-avancee J06-P2)."
        ),
        "RANDOM + REPEATED",
    );
}

/// Base : `SUBJECT=id(grp)` et `SUBJECT=a*b` ne gardaient que le premier
/// identifiant (sujets fusionnés entre groupes).
#[test]
fn ra_j02_p2_subject_nested_or_crossed() {
    for stmt in [
        "random intercept / subject=subj(g)",
        "random intercept / subject=subj*g",
        "repeated / subject=subj(g) type=un",
        "repeated / sub=subj*g type=un",
    ] {
        let out = run_sas(&format!(
            "{BAL}proc mixed data=bal; class subj g; model y = ; {stmt}; run;"
        ));
        assert_error(
            &out,
            &format!(
                "A nested or crossed SUBJECT= effect (id(group), a*b) {CANNOT} \
                 (planned: roadmap-avancee J06-P2)."
            ),
            stmt,
        );
    }
}

/// Base : toutes les options RANDOM/REPEATED autres que SUBJECT=/TYPE=
/// étaient sautées. GROUP=, LOCAL (et toute autre option qui peut changer le
/// modèle) → ERROR ; G, GCORR, V, R, RCORR, SOLUTION (affichage) → WARNING.
#[test]
fn ra_j02_p2_random_repeated_options() {
    for (stmt, needle) in [
        (
            "random intercept / subject=subj group=g",
            "The GROUP= option of the RANDOM statement",
        ),
        (
            "repeated time / subject=subj type=un group=g",
            "The GROUP= option of the REPEATED statement",
        ),
        (
            "repeated time / subject=subj type=un local",
            "The LOCAL option of the REPEATED statement",
        ),
        (
            "random intercept / subject=subj ratio",
            "The RATIO option of the RANDOM statement",
        ),
    ] {
        let out = run_sas(&format!(
            "{REP}data rep; set rep; g = (subj in ('A' 'B')); run;
             proc mixed data=rep; class subj time g; model y = ; {stmt}; run;"
        ));
        assert_error(&out, &format!("{needle} {CANNOT}."), stmt);
    }

    // Display-only options: WARNING (exit 1), estimates unchanged.
    let plain = run_sas(&format!(
        "{BAL}proc mixed data=bal; class subj; model y = ; random intercept / subject=subj; run;"
    ));
    let out = run_sas(&format!(
        "{BAL}proc mixed data=bal; class subj; model y = ;
         random intercept / subject=subj g gcorr v solution; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for opt in ["G", "GCORR", "V", "SOLUTION"] {
        assert!(
            out.log.contains(&format!(
                "WARNING: The {opt} option of the RANDOM statement is ignored in PROC MIXED; \
                 display customization is not supported."
            )),
            "{opt}: {}",
            out.log
        );
    }
    assert_eq!(out.listing, plain.listing);

    let plain = run_sas(&format!(
        "{REP}proc mixed data=rep; class subj time; model y = ;
         repeated time / subject=subj type=un; run;"
    ));
    let out = run_sas(&format!(
        "{REP}proc mixed data=rep; class subj time; model y = ;
         repeated time / subject=subj type=un r rcorr; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for opt in ["R", "RCORR"] {
        assert!(
            out.log.contains(&format!(
                "WARNING: The {opt} option of the REPEATED statement is ignored in PROC MIXED; \
                 display customization is not supported."
            )),
            "{opt}: {}",
            out.log
        );
    }
    assert_eq!(out.listing, plain.listing);
}

/// Base : l'effet répété de REPEATED était sauté ; R était indexée par ordre
/// d'apparition même quand les niveaux de l'effet le contredisaient. ERROR
/// pour un ordre non strictement croissant ou des niveaux manquants sous
/// AR(1)/UN ; le cas complet et trié est inchangé.
#[test]
fn ra_j02_p2_repeated_effect_order() {
    // Complete and sorted: identical to the effect-less statement.
    for ty in ["un", "ar(1)"] {
        let with = run_sas(&format!(
            "{REP}proc mixed data=rep; class subj time; model y = / solution;
             repeated time / subject=subj type={ty}; run;"
        ));
        let without = run_sas(&format!(
            "{REP}proc mixed data=rep; class subj time; model y = / solution;
             repeated / subject=subj type={ty}; run;"
        ));
        assert_eq!(with.exit_code, 0, "{ty}: {}", with.log);
        assert_eq!(with.listing, without.listing, "{ty}");
    }

    // Subject B listed as time 2 then 1.
    let unsorted = "data u; input subj $ time y; datalines;
A 1 1
A 2 3
B 2 1
B 1 3
C 1 5
C 2 7
;
run;
";
    for ty in ["un", "ar(1)"] {
        let out = run_sas(&format!(
            "{unsorted}proc mixed data=u; class subj time; model y = ;
             repeated time / subject=subj type={ty}; run;"
        ));
        assert_error(
            &out,
            "The levels of the REPEATED effect TIME are not strictly increasing within \
             subject B; PROC MIXED indexes R by order of appearance, which would not match \
             the effect levels (planned: roadmap-avancee J06-P2).",
            ty,
        );
    }

    // Subject B lacks time 2 (gap between 1 and 3).
    let gap = "data g; input subj $ time y; datalines;
A 1 1
A 2 3
A 3 2
B 1 3
B 3 1
C 1 5
C 2 7
C 3 6
;
run;
";
    for (ty, shown) in [("un", "UN"), ("ar(1)", "AR(1)")] {
        let out = run_sas(&format!(
            "{gap}proc mixed data=g; class subj time; model y = ;
             repeated time / subject=subj type={ty}; run;"
        ));
        assert_error(
            &out,
            &format!(
                "Subject B lacks levels of the REPEATED effect TIME; with TYPE={shown} PROC \
                 MIXED indexes R by order of appearance, which would not match the effect \
                 levels (planned: roadmap-avancee J06-P2)."
            ),
            ty,
        );
    }
}

/// Base : CONVG=, CONVH=, MAXITER=, ORDER=, EMPIRICAL, SCORING=, NOPROFILE et
/// toute autre option PROC étaient sautées. ERROR ; CL (limites des
/// paramètres de covariance, affichage) → WARNING.
#[test]
fn ra_j02_p2_proc_options() {
    for (opt, shown) in [
        ("convg=1e-6", "CONVG="),
        ("convh=1e-6", "CONVH="),
        ("maxiter=5", "MAXITER="),
        ("order=data", "ORDER="),
        ("empirical", "EMPIRICAL"),
        ("scoring=3", "SCORING="),
        ("noprofile", "NOPROFILE"),
    ] {
        let out = run_sas(&format!(
            "{BAL}proc mixed data=bal {opt}; class subj; model y = ;
             random intercept / subject=subj; run;"
        ));
        assert_error(&out, &format!("The {shown} option {CANNOT}."), opt);
    }
    let out = run_sas(&format!(
        "{BAL}proc mixed data=bal invented; class subj; model y = ;
         random intercept / subject=subj; run;"
    ));
    assert_error(
        &out,
        "Unexpected option 'INVENTED' on PROC MIXED statement.",
        "unknown",
    );

    let plain = run_sas(&format!(
        "{BAL}proc mixed data=bal; class subj; model y = ; random intercept / subject=subj; run;"
    ));
    let out = run_sas(&format!(
        "{BAL}proc mixed data=bal cl; class subj; model y = ;
         random intercept / subject=subj; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert!(
        out.log.contains(
            "WARNING: The CL option is ignored in PROC MIXED; display customization is not \
             supported."
        ),
        "{}",
        out.log
    );
    assert_eq!(out.listing, plain.listing);
}

/// Base : COVTEST et ASYCOV émettaient une NOTE « parse-accepted but not
/// implemented » puis étaient ignorés. ERROR jusqu'à roadmap-avancee J06-P3.
#[test]
fn ra_j02_p2_covtest_asycov() {
    for opt in ["covtest", "asycov"] {
        let out = run_sas(&format!(
            "{BAL}proc mixed data=bal {opt}; class subj; model y = ;
             random intercept / subject=subj; run;"
        ));
        assert_error(
            &out,
            &format!(
                "The {} option {CANNOT} (planned: roadmap-avancee J06-P3).",
                opt.to_uppercase()
            ),
            opt,
        );
        assert!(!out.log.contains("parse-accepted"), "{}", out.log);
    }
}

/// Base : OUTP=, OUTPM=, NOFIT et les autres options MODEL étaient sautées
/// (aucun dataset de prédictions créé, NOFIT ignoré sur le chemin général).
#[test]
fn ra_j02_p2_model_options() {
    for (opt, shown) in [
        ("outp=work.p", "OUTP="),
        ("outpm=work.pm", "OUTPM="),
        ("nofit", "NOFIT"),
        ("chisq", "CHISQ"),
        ("htype=1", "HTYPE="),
    ] {
        let out = run_sas(&format!(
            "{BAL}proc mixed data=bal; class subj; model y = / {opt};
             random intercept / subject=subj; run;
             proc print data=work.p; run;"
        ));
        assert_error(
            &out,
            &format!("The {shown} option of the MODEL statement {CANNOT}."),
            opt,
        );
    }
}

/// Base : le chemin général imprimait une « Iteration History » synthétique
/// (2 lignes, critère 0.00000000 inventé). Retirée jusqu'à la vraie
/// historique de roadmap-avancee J06-P2 ; le statut de convergence reste.
#[test]
fn ra_j02_p2_no_synthetic_iteration_history() {
    let out = run_sas(&format!(
        "{REP}proc mixed data=rep method=ml; class subj time; model y = / solution;
         repeated time / subject=subj type=un; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        !out.listing.contains("Iteration History"),
        "{}",
        out.listing
    );
    assert!(!out.listing.contains("0.00000000"), "{}", out.listing);
    assert!(
        out.listing.contains("Convergence criteria met."),
        "{}",
        out.listing
    );
    // Estimates unchanged (m34 oracle: UN(1,1)=5, UN(2,1)=3, intercept 4).
    assert!(out.listing.contains("5.0000"), "{}", out.listing);
    assert!(out.listing.contains("3.0000"), "{}", out.listing);
}

/// Base : sous METHOD=ML, l'Iteration History du chemin legacy titrait
/// « -2 Res Log Like ». SAS/STAT 9.4, The MIXED Procedure, « Iteration
/// History » : la colonne est « -2 Log Like » pour ML, « -2 Res Log Like »
/// pour REML — correction directe.
#[test]
fn ra_j02_p2_legacy_ml_objective_header() {
    let program = |method: &str| {
        run_sas(&format!(
            "{BAL}proc mixed data=bal method={method}; class subj; model y = ;
             random intercept / subject=subj; run;"
        ))
    };
    let ml = program("ml");
    assert_eq!(ml.exit_code, 0, "{}", ml.log);
    let header = ml
        .listing
        .lines()
        .find(|l| l.contains("Iteration") && l.contains("Criterion"))
        .expect("iteration header");
    assert!(header.contains("-2 Log Like"), "{header}");
    assert!(!ml.listing.contains("-2 Res Log Like"), "{}", ml.listing);

    let reml = program("reml");
    assert!(reml.listing.contains("-2 Res Log Like"), "{}", reml.listing);
}

/// Base : NOINT avec un effet CLASS — le codage de référence retirait encore
/// le dernier niveau, une colonne d'effet fixe manquait.
#[test]
fn ra_j02_p2_noint_with_class_effect() {
    let out = run_sas(&format!(
        "{BAL}proc mixed data=bal; class subj g; model y = g / noint solution;
         random intercept / subject=subj; run;"
    ));
    assert_error(
        &out,
        &format!("NOINT with a CLASS fixed effect {CANNOT} (planned: roadmap-avancee J06-P2)."),
        "noint class",
    );
    // A continuous effect under NOINT remains accepted.
    let out = run_sas(&format!(
        "{REP}proc mixed data=rep; class subj; model y = time / noint solution;
         random intercept / subject=subj; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
}

/// Base : NOBOUND émettait une NOTE « not implemented » fausse sur le chemin
/// legacy (où la forme close l'honore) et était ignoré en silence sur le
/// chemin général et sous données déséquilibrées (λ ≥ 0). Legacy équilibré :
/// sans NOTE, variance négative conservée ; ailleurs : ERROR (J06-P2).
#[test]
fn ra_j02_p2_nobound() {
    // Subject means 2, 3, 4 (MSB = 2), within deviations ±2 (MSW = 8):
    // REML σ²_u = (MSB − MSW)/n = (2 − 8)/2 = −3, σ²_e = 8.
    let neg = "data n; input subj $ y; datalines;
A 0
A 4
B 1
B 5
C 2
C 6
;
run;
";
    let out = run_sas(&format!(
        "{neg}proc mixed data=n nobound; class subj; model y = ;
         random intercept / subject=subj; run;"
    ));
    assert!(out.exit_code <= 1, "{}", out.log);
    assert!(!out.log.contains("NOBOUND"), "{}", out.log);
    let row = out
        .listing
        .lines()
        .find(|l| l.trim_start().starts_with("Intercept") && l.contains("subj"))
        .expect("covariance parameter row");
    assert!(row.trim_end().ends_with("-3.0000"), "{row}");
    // Without NOBOUND the variance is bounded at 0.
    let out = run_sas(&format!(
        "{neg}proc mixed data=n; class subj; model y = ; random intercept / subject=subj; run;"
    ));
    let row = out
        .listing
        .lines()
        .find(|l| l.trim_start().starts_with("Intercept") && l.contains("subj"))
        .expect("covariance parameter row");
    assert!(row.trim_end().ends_with(" 0.0000"), "{row}");

    let out = run_sas(&format!(
        "{REP}proc mixed data=rep nobound; class subj time; model y = ;
         repeated time / subject=subj type=un; run;"
    ));
    assert_error(
        &out,
        &format!(
            "NOBOUND outside the single random-intercept, intercept-only model {CANNOT} \
             (planned: roadmap-avancee J06-P2)."
        ),
        "general path",
    );

    let unbalanced = "data ub; input subj $ y; datalines;
A 1
A 3
A 2
B 3
B 1
;
run;
";
    let out = run_sas(&format!(
        "{unbalanced}proc mixed data=ub nobound; class subj; model y = ;
         random intercept / subject=subj; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert!(
        out.log.contains(
            "NOBOUND with unbalanced data is not supported in PROC MIXED; it can affect \
             results and cannot be ignored (planned: roadmap-avancee J06-P2)."
        ),
        "{}",
        out.log
    );
}

/// Base : les instructions MIXED valides non implémentées (CODE, LSMESTIMATE,
/// PARMS, PRIOR, SLICE, STORE) étaient signalées « 180-322 … not valid ».
/// Message du catalogue « not supported … cannot be ignored » ; une
/// instruction inventée reste une 180-322.
#[test]
fn ra_j02_p2_unsupported_statements() {
    for stmt in [
        "code file='x.sas'",
        "lsmestimate g 'a' 1 -1",
        "parms (1) (2)",
        "prior",
        "slice g",
        "store work.s",
    ] {
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        let out = run_sas(&format!(
            "{BAL}proc mixed data=bal; class subj g; model y = ;
             random intercept / subject=subj; {stmt}; run;"
        ));
        assert_error(
            &out,
            &format!(
                "The {kw} statement is not supported in PROC MIXED; it can affect results \
                 and cannot be ignored."
            ),
            stmt,
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    let out = run_sas(&format!(
        "{BAL}proc mixed data=bal; class subj; model y = ; invented x; run;"
    ));
    assert_error(
        &out,
        "180-322: Statement 'INVENTED' is not valid or it is used out of proper order in PROC MIXED.",
        "invented",
    );
}

/// Base : code mort — NOTE « parse-accepted but not implemented » pour
/// LSMEANS/ESTIMATE/CONTRAST/DDFM/REPEATED (inatteignables) ; DDFM=CONTAIN
/// reste honoré sans NOTE.
#[test]
fn ra_j02_p2_no_misleading_notes() {
    let out = run_sas(&format!(
        "{BAL}proc mixed data=bal; class subj; model y = / solution ddfm=contain;
         random intercept / subject=subj; run;
         proc mixed data=bal; class subj g; model y = g / ddfm=contain;
         random intercept / subject=subj; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(!out.log.contains("parse-accepted"), "{}", out.log);
    assert!(!out.log.contains("not implemented"), "{}", out.log);
}
