// ── J02-P1 : contrat GENMOD (replis silencieux supprimés) ────────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, code de sortie). Oracle de sévérité :
// CONTRIBUTING §5 ; sémantique SAS citée par test.

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

/// Poisson oracle data of the m26 fixture (y = 1..6, x = 0/1).
const POIS: &str = "data pois; input y x @@; datalines;
1 0 2 0 3 0 4 1 5 1 6 1
;
run;
";

/// 2×2 counts of the m26 fixture (y=1/0, x=1/0, FREQ count).
const COUNTS: &str = "data counts; input y x count; datalines;
1 1 20
1 0 10
0 1 5
0 0 25
;
run;
";

/// Ligne « <label> <valeur> » de la table Model Information.
fn model_info(listing: &str, label: &str) -> String {
    listing
        .lines()
        .find(|l| l.trim_start().starts_with(label))
        .unwrap_or_else(|| panic!("no '{label}' line: {listing}"))
        .trim_start()[label.len()..]
        .trim()
        .to_string()
}

/// Base : sans DIST=, un modèle de Poisson (lien log) était ajusté.
/// SAS/STAT 9.4, The GENMOD Procedure, MODEL statement, DIST= : la loi
/// normale est le défaut (lien canonique IDENTITY) — correction directe.
#[test]
fn ra_j02_p1_genmod_default_dist_normal() {
    let out = run_sas(&format!("{POIS}proc genmod data=pois; model y = x; run;"));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(model_info(&out.listing, "Distribution"), "Normal");
    assert_eq!(model_info(&out.listing, "Link Function"), "Identity");
    // OLS: intercept = mean(1,2,3) = 2, slope = 5 - 2 = 3.
    let est = out
        .listing
        .lines()
        .find(|l| l.trim_start().starts_with("x "))
        .expect("x row");
    assert_eq!(est.split_whitespace().nth(2), Some("3.0000"), "{est}");
}

/// Base : toutes les options PROC GENMOD autres que DATA= étaient sautées.
/// SAS/STAT 9.4, The GENMOD Procedure, PROC GENMOD statement : DESCENDING
/// (alias DESC) inverse l'ordre des niveaux de réponse — honoré ; toute autre
/// option est une ERROR « Unexpected option ».
#[test]
fn ra_j02_p1_genmod_proc_options() {
    for header in ["descending", "desc"] {
        let out = run_sas(&format!(
            "{COUNTS}proc genmod data=counts {header}; model y = x / dist=binomial;
             freq count; run;"
        ));
        assert_eq!(out.exit_code, 0, "{header}: {}", out.log);
        assert!(
            out.listing
                .contains("PROC GENMOD is modeling the probability that y=1."),
            "{header}: {}",
            out.listing
        );
    }
    for (opt, bad) in [
        ("noprint", "NOPRINT"),
        ("order=freq", "ORDER"),
        ("namelen=20", "NAMELEN"),
    ] {
        let out = run_sas(&format!(
            "{POIS}proc genmod data=pois {opt}; model y = x / dist=poisson; run;"
        ));
        assert_eq!(out.exit_code, 2, "{opt}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "Unexpected option '{bad}' on PROC GENMOD statement."
            )),
            "{opt}: {}",
            out.log
        );
    }
}

/// Base : `y(desc)` et `EVENT=FIRST|LAST` étaient ignorés par
/// `parse_response_options` (seuls `descending` et `event='v'` étaient lus).
/// SAS/STAT 9.4, The GENMOD Procedure, MODEL statement, « Response Variable
/// Options » ; ORDER=/REF= → ERROR.
#[test]
fn ra_j02_p1_genmod_response_options() {
    for (opts, modeled) in [
        ("(desc)", "y=1."),
        ("(event=last)", "y=1."),
        ("(event=first)", "y=0."),
        ("(event='1')", "y=1."),
    ] {
        let out = run_sas(&format!(
            "{COUNTS}proc genmod data=counts; model y{opts} = x / dist=binomial; freq count; run;"
        ));
        assert_eq!(out.exit_code, 0, "{opts}: {}", out.log);
        assert!(
            out.listing.contains(&format!(
                "PROC GENMOD is modeling the probability that {modeled}"
            )),
            "{opts}: {}",
            out.listing
        );
    }
    for (opts, name) in [("(order=freq)", "ORDER"), ("(ref='1')", "REF")] {
        let out = run_sas(&format!(
            "{COUNTS}proc genmod data=counts; model y{opts} = x / dist=binomial; freq count; run;"
        ));
        assert_eq!(out.exit_code, 2, "{opts}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "The {name}= response variable option is not supported in PROC GENMOD"
            )),
            "{opts}: {}",
            out.log
        );
    }
}

/// Base : les options CLASS (REF=, PARAM=, ORDER=, DESC, MISSING, `/ …`)
/// étaient sautées (leurs mots devenaient des variables CLASS) ; le codage
/// restait REF=LAST. ERROR jusqu'à roadmap-avancee J07-P2.
#[test]
fn ra_j02_p1_genmod_class_options_error() {
    for stmt in [
        "class x(ref='0');",
        "class x(param=effect);",
        "class x / order=data;",
        "class x / missing;",
        "class x(desc);",
    ] {
        let out = run_sas(&format!(
            "{POIS}proc genmod data=pois; {stmt} model y = x / dist=poisson; run;"
        ));
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert!(
            out.log.contains(
                "CLASS statement options (REF=, PARAM=, ORDER=, DESCENDING, MISSING, ...) \
                 are not supported in PROC GENMOD; the coding of the CLASS effects would \
                 silently differ from the request (planned: roadmap-avancee J07-P2)."
            ),
            "{stmt}: {}",
            out.log
        );
    }
}

/// Base : SCALE=PEARSON|P|DEVIANCE|D était sauté ; SCALE=<n> sous POISSON ou
/// BINOMIAL ne changeait rien (SE au modèle φ=1). ERROR jusqu'à
/// roadmap-avancee J07-P2 ; SCALE=<n> reste honoré pour NORMAL/GAMMA.
#[test]
fn ra_j02_p1_genmod_scale_errors() {
    for scale in ["pearson", "p", "deviance", "d"] {
        let out = run_sas(&format!(
            "{POIS}proc genmod data=pois; model y = x / dist=poisson scale={scale}; run;"
        ));
        assert_eq!(out.exit_code, 2, "{scale}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "SCALE={} is not supported in PROC GENMOD; only a numeric SCALE= value is \
                 implemented (planned: roadmap-avancee J07-P2).",
                scale.to_uppercase()
            )),
            "{scale}: {}",
            out.log
        );
    }
    for dist in ["poisson", "binomial"] {
        let out = run_sas(&format!(
            "{COUNTS}proc genmod data=counts; model y = x / dist={dist} scale=2; freq count; run;"
        ));
        assert_eq!(out.exit_code, 2, "{dist}: {}", out.log);
        assert!(
            out.log.contains(
                "SCALE=<n> is not supported for DIST=POISSON or DIST=BINOMIAL in PROC GENMOD"
            ),
            "{dist}: {}",
            out.log
        );
    }
    // Non-regression: SCALE=<n> fixes the NORMAL scale (DF 0).
    let out = run_sas(&format!(
        "{POIS}proc genmod data=pois; model y = x / dist=normal scale=2; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    let scale_row = out
        .listing
        .lines()
        .find(|l| l.split_whitespace().next() == Some("Scale"))
        .expect("Scale row");
    let toks: Vec<&str> = scale_row.split_whitespace().collect();
    assert_eq!(&toks[1..3], ["0", "2.0000"], "{scale_row}");
}

/// Base : DIST=GAMMA tronquait en silence une réponse ≤ 0 à 1e-300 dans la
/// log-vraisemblance et la déviance. Hors du support de la loi → ERROR.
#[test]
fn ra_j02_p1_genmod_gamma_nonpositive_error() {
    for bad in ["0", "-2"] {
        let out = run_sas(&format!(
            "data g; input y x @@; datalines;
2 1 4 2 {bad} 3 10 4 20 5
;
run;
proc genmod data=g; model y = x / dist=gamma link=log; run;"
        ));
        assert_eq!(out.exit_code, 2, "{bad}: {}", out.log);
        assert!(
            out.log.contains(
                "DIST=GAMMA requires a positive response in PROC GENMOD; variable Y has 1 \
                 used observation(s) with a value <= 0."
            ),
            "{bad}: {}",
            out.log
        );
    }
}

/// Base : les niveaux CLASS étaient calculés sur toutes les lignes lues ; un
/// niveau présent seulement dans des observations écartées créait une
/// colonne nulle et une ERROR de singularité trompeuse. SAS/STAT 9.4, The
/// GENMOD Procedure, « Missing Values » : niveaux issus des observations
/// utilisées.
#[test]
fn ra_j02_p1_genmod_class_levels_from_used_obs() {
    let out = run_sas(
        "data cl; input y g $ z; datalines;
2 a 1
4 a 2
6 a 3
12 b 1
14 b 2
17 b 3
7 c .
. c 2
;
run;
proc genmod data=cl; class g; model y = g z / dist=normal; run;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    let cli = out
        .listing
        .lines()
        .find(|l| l.trim_start().starts_with("g "))
        .expect("class level row");
    assert_eq!(
        cli.split_whitespace().collect::<Vec<_>>(),
        ["g", "2", "a", "b"]
    );
}

/// Base : « Convergence criterion (GCONV=1E-8) satisfied. » alors que
/// `fit_irls` teste le changement relatif des paramètres : libellé XCONV.
#[test]
fn ra_j02_p1_genmod_convergence_label_xconv() {
    let out = run_sas(&format!(
        "{POIS}proc genmod data=pois; model y = x / dist=poisson; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing
            .contains("     Convergence criterion (XCONV=1E-8) satisfied."),
        "{}",
        out.listing
    );
    assert!(!out.listing.contains("GCONV"), "{}", out.listing);
}

/// Base : les instructions GENMOD valides non implémentées (REPEATED,
/// ASSESS, BAYES, STRATA, VARIANCE, DEVIANCE, FWDLINK…) recevaient
/// « 180-322 … not valid » ; message du catalogue du contrat désormais.
/// EFFECTPLOT (graphique seul) → WARNING d'affichage.
#[test]
fn ra_j02_p1_genmod_unsupported_statements() {
    for stmt in [
        "repeated subject=x / type=exch",
        "assess var=(x)",
        "bayes seed=1",
        "strata x",
        "variance v = _mean_",
        "deviance d = 2",
        "fwdlink l = log(_mean_)",
        "zeromodel x",
    ] {
        let out = run_sas(&format!(
            "{POIS}proc genmod data=pois; model y = x / dist=poisson; {stmt}; run;"
        ));
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "The {kw} statement is not supported in PROC GENMOD; it can affect results \
                 and cannot be ignored."
            )),
            "{stmt}: {}",
            out.log
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    let out = run_sas(&format!(
        "{POIS}proc genmod data=pois; model y = x / dist=poisson; effectplot; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert!(
        out.log.contains(
            "The EFFECTPLOT statement is ignored in PROC GENMOD; display customization is \
             not supported."
        ),
        "{}",
        out.log
    );
}
