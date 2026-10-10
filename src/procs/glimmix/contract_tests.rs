// ── J02-P3 : contrat GLIMMIX (replis silencieux supprimés) ───────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, code de sortie). Oracle de sévérité :
// CONTRIBUTING §5 ; syntaxe de référence : SAS/STAT 9.4 User's Guide, The
// GLIMMIX Procedure, Syntax
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_glimmix_syntax.htm

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

/// Poisson data of the m28 fixture (no subject).
const POIS: &str = "data pois; input y x; datalines;
1 0
2 0
3 0
4 1
5 1
6 1
;
run;
";

/// Balanced random-intercept data of the m28 fixture, plus a second CLASS
/// variable `g`.
const BAL: &str = "data bal; input subj $ g $ y; datalines;
A a 1
A b 3
B a 5
B b 7
;
run;
";

/// Counts with a random intercept per subject (PQL / LAPLACE fits).
const CNT: &str = "data cnt; input subj $ x y w; datalines;
A 0 2 1
A 1 4 2
A 2 5 1
B 0 1 2
B 1 1 1
B 2 3 2
C 0 4 1
C 1 6 2
C 2 9 1
D 0 0 2
D 1 2 1
D 2 2 2
;
run;
";

/// Repeated measures (three per subject, in time order).
const REP: &str = "data rep; input subj $ time y; datalines;
A 1 1
A 2 3
A 3 2
B 1 3
B 2 1
B 3 4
C 1 5
C 2 7
C 3 6
D 1 7
D 2 5
D 3 8
;
run;
";

/// Unbalanced random-intercept data (no closed form).
const UNB: &str = "data unb; input subj $ y; datalines;
A 1
A 3
B 3
B 1
B 4
C 5
C 7
C 6
;
run;
";

/// Binary response with a random intercept per subject.
const BIN: &str = "data bin; input subj $ x y; datalines;
A 0 0
A 1 1
A 2 1
B 0 0
B 1 0
B 2 1
C 0 1
C 1 0
C 2 1
D 0 0
D 1 1
D 2 0
E 0 0
E 1 0
E 2 1
;
run;
";

/// Counts with a two-level CLASS effect `g` and a covariate `x`.
const TWO: &str = "data two; input g $ x y; datalines;
a 0 2
a 1 4
a 2 5
a 0 1
a 1 1
a 2 3
b 0 4
b 1 6
b 2 9
b 0 0
b 1 2
b 2 2
;
run;
";

/// Contract ERROR suffix shared by every rejected construction.
const CANNOT: &str =
    "is not supported in PROC GLIMMIX; it can affect results and cannot be ignored";

/// Asserts a rejected step: exit code 2, `needle` in the log, no listing.
fn assert_error(out: &crate::RunOutcome, needle: &str, ctx: &str) {
    assert_eq!(out.exit_code, 2, "{ctx}: {}", out.log);
    assert!(
        out.log.contains(needle),
        "{ctx}: missing «{needle}»\n{}",
        out.log
    );
    assert!(
        !out.listing.contains("The GLIMMIX Procedure"),
        "{ctx}: the step must be rejected before execution\n{}",
        out.listing
    );
}

/// Asserts a display-only WARNING: exit code 1, the warning in the log and a
/// listing identical to the one of `plain` (estimates unchanged).
fn assert_display_warning(out: &crate::RunOutcome, plain: &crate::RunOutcome, warning: &str) {
    assert_eq!(out.exit_code, 1, "{warning}: {}", out.log);
    assert!(
        out.log.contains(&format!("WARNING: {warning}")),
        "missing «{warning}»\n{}",
        out.log
    );
    assert_eq!(out.listing, plain.listing, "{warning}");
}

/// Line of the listing that starts (after indentation) with `head`.
fn row<'a>(listing: &'a str, head: &str) -> Option<&'a str> {
    listing.lines().find(|l| l.trim_start().starts_with(head))
}

/// Text of the listing after the first occurrence of the table `title`.
fn after<'a>(listing: &'a str, title: &str) -> &'a str {
    listing
        .split(title)
        .nth(1)
        .unwrap_or_else(|| panic!("missing table «{title}»\n{listing}"))
}

/// Base : plusieurs RANDOM — seule la dernière instruction était ajustée.
#[test]
fn ra_j02_p3_multiple_random_statements() {
    let out = run_sas(&format!(
        "{BAL}proc glimmix data=bal; class subj g; model y = ;
         random intercept / subject=subj; random intercept / subject=g; run;"
    ));
    assert_error(
        &out,
        &format!("More than one RANDOM statement {CANNOT} (planned: roadmap-avancee J08-P4)."),
        "two RANDOM",
    );
}

/// Base : DIST=NORMAL avec LINK≠IDENTITY et un effet aléatoire sous RSPL —
/// le solveur à composantes de variance (fit_vc) ajustait le modèle à lien
/// identité, LINK= ignoré. ERROR ; sans RANDOM (mode GLM, IRLS avec le lien)
/// et avec LINK=IDENTITY, le modèle reste ajusté.
#[test]
fn ra_j02_p3_normal_link_with_random_under_rspl() {
    for (link, shown) in [("log", "LOG"), ("logit", "LOGIT")] {
        let out = run_sas(&format!(
            "{CNT}proc glimmix data=cnt; class subj; model y = x / dist=normal link={link};
             random intercept / subject=subj; run;"
        ));
        assert_error(
            &out,
            &format!(
                "DIST=NORMAL with LINK={shown} and a G-side RANDOM effect under METHOD=RSPL \
                 {CANNOT} (planned: roadmap-avancee J08-P2)."
            ),
            link,
        );
    }
    let out = run_sas(&format!(
        "{CNT}proc glimmix data=cnt; model y = x / dist=normal link=log solution; run;"
    ));
    assert_eq!(out.exit_code, 0, "GLM mode: {}", out.log);
    let out = run_sas(&format!(
        "{BAL}proc glimmix data=bal; class subj; model y = / dist=normal link=identity;
         random intercept / subject=subj; run;"
    ));
    assert_eq!(out.exit_code, 0, "identity link: {}", out.log);
}

/// Base : METHOD=LAPLACE avec une paire DIST/LINK non canonique hors loi
/// binaire (POISSON/IDENTITY, NORMAL/LOG) — log_density tombait dans sa
/// branche Bernoulli : vraisemblance fausse, sans diagnostic. ERROR ; les
/// liens binaires (branche Bernoulli exacte) restent acceptés.
#[test]
fn ra_j02_p3_laplace_noncanonical_link() {
    for (model, needle) in [
        (
            "y = x / dist=poisson link=identity",
            "METHOD=LAPLACE with DIST=POISSON and LINK=IDENTITY",
        ),
        (
            "y = x / dist=normal link=log",
            "METHOD=LAPLACE with DIST=NORMAL and LINK=LOG",
        ),
    ] {
        let out = run_sas(&format!(
            "{CNT}proc glimmix data=cnt method=laplace; class subj; model {model};
             random intercept / subject=subj; run;"
        ));
        assert_error(
            &out,
            &format!("{needle} {CANNOT} (planned: roadmap-avancee J08-P3)."),
            model,
        );
    }
    let out = run_sas(&format!(
        "{BIN}proc glimmix data=bin method=laplace; class subj;
         model y(event='1') = x / dist=binary link=probit;
         random intercept / subject=subj; run;"
    ));
    assert_eq!(out.exit_code, 0, "binary probit: {}", out.log);
    assert!(out.listing.contains("Laplace"), "{}", out.listing);
}

/// Base : WEIGHT émettait une NOTE « parse-accepted but not implemented »
/// puis l'ajustement ignorait les poids.
#[test]
fn ra_j02_p3_weight_statement() {
    let out = run_sas(&format!(
        "{CNT}proc glimmix data=cnt; class subj; model y = x / dist=poisson;
         random intercept / subject=subj; weight w; run;"
    ));
    assert_error(
        &out,
        &format!("The WEIGHT statement {CANNOT} (planned: roadmap-avancee J08-P4)."),
        "weight",
    );
    assert!(!out.log.contains("parse-accepted"), "{}", out.log);
}

/// Base : avec DIST=NORMAL, LINK=IDENTITY et un RANDOM sous RSPL, les
/// solveurs REML (composantes de variance, structure R) ne recevaient pas les
/// fréquences : FREQ était ignoré alors que « Number of Observations Used »
/// affichait leur somme. ERROR ; FREQ reste honoré en mode GLM, par la boucle
/// PQL et par LAPLACE (poids de la vraisemblance).
#[test]
fn ra_j02_p3_freq_with_normal_random_effects() {
    for random in [
        "random intercept / subject=subj",
        "random _residual_ / subject=subj type=ar(1)",
    ] {
        let out = run_sas(&format!(
            "{CNT}proc glimmix data=cnt; class subj; model y = x; freq w; {random}; run;"
        ));
        assert_error(
            &out,
            &format!(
                "A FREQ statement with DIST=NORMAL, LINK=IDENTITY and a RANDOM statement under \
                 METHOD=RSPL {CANNOT}."
            ),
            random,
        );
    }
    for program in [
        "proc glimmix data=cnt; model y = x; freq w; run;",
        "proc glimmix data=cnt; class subj; model y = x / dist=poisson; freq w;
         random intercept / subject=subj; run;",
        "proc glimmix data=cnt method=laplace; class subj; model y = x; freq w;
         random intercept / subject=subj; run;",
    ] {
        let out = run_sas(&format!("{CNT}{program}"));
        assert_eq!(out.exit_code, 0, "{program}: {}", out.log);
    }
}

/// Base : toute option PROC autre que DATA=/METHOD= était sautée jeton par
/// jeton (NOBOUND, EMPIRICAL, PCONV=, ORDER=, NOREML…), METHOD= sans valeur
/// retombait sur RSPL. ERROR ; option inconnue : « Unexpected option ».
#[test]
fn ra_j02_p3_proc_options_affecting_results() {
    for (opt, shown) in [
        ("nobound", "NOBOUND"),
        ("empirical", "EMPIRICAL"),
        ("pconv=1e-8", "PCONV="),
        ("order=data", "ORDER="),
        ("noreml", "NOREML"),
        ("maxopt=5", "MAXOPT="),
        ("chol", "CHOL"),
        ("scoring=5", "SCORING="),
        ("outdesign=work.xz", "OUTDESIGN="),
        ("nofit", "NOFIT"),
        ("ic=q", "IC="),
    ] {
        let out = run_sas(&format!(
            "{BAL}proc glimmix data=bal {opt}; class subj; model y = ;
             random intercept / subject=subj; run;"
        ));
        assert_error(&out, &format!("The {shown} option {CANNOT}."), opt);
    }
    let out = run_sas(&format!(
        "{BAL}proc glimmix data=bal invented; class subj; model y = ;
         random intercept / subject=subj; run;"
    ));
    assert_error(
        &out,
        "Unexpected option 'INVENTED' on PROC GLIMMIX statement.",
        "unknown",
    );
    let out = run_sas(&format!(
        "{BAL}proc glimmix data=bal method=; class subj; model y = ;
         random intercept / subject=subj; run;"
    ));
    assert_error(
        &out,
        "expected a value after METHOD=",
        "METHOD= without value",
    );
}

/// Base : les options PROC d'affichage (NOCLPRINT, ASYCOV, PLOTS=,
/// ODDSRATIO, IC=NONE…) étaient sautées sans diagnostic. WARNING d'affichage
/// (code 1), estimations inchangées. Honorées sans diagnostic : INITGLM
/// (valeurs initiales issues du GLM sans effet aléatoire, ce que font tous
/// les ajustements), NOITPRINT et PLOTS=NONE (aucune Iteration History ni
/// aucun graphique n'est produit), IC=NONE hors LAPLACE (SAS/STAT 9.4, PROC
/// GLIMMIX statement, INFOCRIT= : défaut des méthodes de pseudo-vraisemblance,
/// aucun critère d'information n'y est imprimé).
#[test]
fn ra_j02_p3_proc_display_options() {
    let program = |opt: &str| {
        run_sas(&format!(
            "{CNT}proc glimmix data=cnt {opt}; class subj; model y = x / dist=poisson solution;
             random intercept / subject=subj; run;"
        ))
    };
    let plain = program("");
    assert_eq!(plain.exit_code, 0, "{}", plain.log);
    for (opt, shown) in [
        ("noclprint=3", "NOCLPRINT"),
        ("asycov", "ASYCOV"),
        ("plots(only)=(residualpanel)", "PLOTS"),
        ("plots=all", "PLOTS"),
        ("oddsratio(diff=first)", "ODDSRATIO"),
    ] {
        assert_display_warning(
            &program(opt),
            &plain,
            &format!(
                "The {shown} option is ignored in PROC GLIMMIX; display customization is not \
                 supported."
            ),
        );
    }
    for opt in [
        "initglm",
        "noitprint",
        "plots=none",
        "ic=none",
        "noitprint plots=none initglm",
    ] {
        let out = program(opt);
        assert_eq!(out.exit_code, 0, "{opt}: {}", out.log);
        assert_eq!(out.listing, plain.listing, "{opt}");
    }
    // The LAPLACE fit prints AIC/AICC/BIC: IC=NONE is not honored there.
    let laplace = |opt: &str| {
        run_sas(&format!(
            "{CNT}proc glimmix data=cnt method=laplace {opt}; class subj;
             model y = x / dist=poisson; random intercept / subject=subj; run;"
        ))
    };
    assert_display_warning(
        &laplace("ic=none"),
        &laplace(""),
        "The IC=NONE option is ignored in PROC GLIMMIX; display customization is not supported.",
    );
}

/// Base : « Convergence criterion (GCONV=1E-8) satisfied. » était imprimé
/// pour tout ajustement alors qu'aucun critère de gradient n'est testé. Le
/// statut nomme désormais le critère réellement testé : changement relatif
/// des paramètres de l'IRLS (mode GLM), de la boucle de pseudo-vraisemblance
/// (PQL), drapeau du simplexe de Nelder-Mead (LAPLACE, structure R).
#[test]
fn ra_j02_p3_convergence_criterion_named() {
    for (program, criterion) in [
        (
            format!("{POIS}proc glimmix data=pois; model y = x / dist=poisson; run;"),
            "Convergence criterion (XCONV=1E-10) satisfied.",
        ),
        (
            format!(
                "{CNT}proc glimmix data=cnt; class subj; model y = x / dist=poisson;
                 random intercept / subject=subj; run;"
            ),
            "Convergence criterion (PCONV=1E-6) satisfied.",
        ),
        (
            format!(
                "{CNT}proc glimmix data=cnt method=laplace; class subj;
                 model y = x / dist=poisson; random intercept / subject=subj; run;"
            ),
            "Convergence criterion (Nelder-Mead simplex: FTOL=1E-12, XTOL=1E-10) satisfied.",
        ),
        (
            format!(
                "{REP}proc glimmix data=rep; class subj; model y = ;
                 random _residual_ / subject=subj type=ar(1); run;"
            ),
            "Convergence criterion (Nelder-Mead simplex: FTOL=1E-12, XTOL=1E-10) satisfied.",
        ),
    ] {
        let out = run_sas(&program);
        assert_eq!(out.exit_code, 0, "{criterion}: {}", out.log);
        assert!(out.listing.contains(criterion), "{}", out.listing);
        assert!(!out.listing.contains("GCONV"), "{}", out.listing);
    }
}

/// Base : `converged = true` codé en dur pour NORMAL + intercept aléatoire.
/// Forme close (données équilibrées) : solution exacte, aucune itération ;
/// sinon la recherche du rapport λ = σ²u/σ²e rapporte son propre critère.
#[test]
fn ra_j02_p3_normal_vc_convergence_not_hard_coded() {
    let out = run_sas(&format!(
        "{BAL}proc glimmix data=bal; class subj; model y = ; random intercept / subject=subj; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing
            .contains("Closed-form REML solution (balanced data): no iteration required."),
        "{}",
        out.listing
    );
    let out = run_sas(&format!(
        "{UNB}proc glimmix data=unb; class subj; model y = ; random intercept / subject=subj; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing
            .contains("Convergence criterion (golden-section search: XTOL=1E-10) satisfied."),
        "{}",
        out.listing
    );
}

/// Base : σ²u négative (forme close) ou sur la frontière λ = 0 était tronquée
/// à 0 sans diagnostic. NOTE SAS « Estimated G matrix is not positive
/// definite. » (SAS Usage Note 22614 ; Kiernan, Tao & Gibbs 2012, SGF
/// 332-2012), comme PROC MIXED.
#[test]
fn ra_j02_p3_g_matrix_not_positive_definite_note() {
    // Subject means 2, 3, 4 (MSB = 2), within deviations ±2 (MSW = 8):
    // REML σ²_u = (MSB − MSW)/n = (2 − 8)/2 = −3 → bounded at 0.
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
        "{neg}proc glimmix data=n; class subj; model y = ; random intercept / subject=subj; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.log
            .contains("NOTE: Estimated G matrix is not positive definite."),
        "{}",
        out.log
    );
    let intercept = out
        .listing
        .lines()
        .find(|l| l.trim_start().starts_with("Intercept") && l.contains("subj"))
        .expect("covariance parameter row");
    assert!(intercept.trim_end().ends_with(" 0.0000"), "{intercept}");

    let out = run_sas(&format!(
        "{BAL}proc glimmix data=bal; class subj; model y = ; random intercept / subject=subj; run;"
    ));
    assert!(!out.log.contains("not positive definite"), "{}", out.log);
}

/// Base : le rapport λ = σ²u/σ²e était plafonné à 1000 en silence (σ²e
/// quasi nulle). NOTE de frontière, même texte que PROC MIXED.
#[test]
fn ra_j02_p3_variance_ratio_boundary_note() {
    let cap = "data cap; input subj $ y; datalines;
A 1
A 1
A 1
B 5
B 5
C 9
C 9
;
run;
";
    let out = run_sas(&format!(
        "{cap}proc glimmix data=cap; class subj; model y = ; random intercept / subject=subj; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.log.contains(
            "NOTE: The variance component ratio search reached its boundary (lambda=1000) in \
             PROC GLIMMIX; the estimate may be unreliable."
        ),
        "{}",
        out.log
    );
}

/// Base : `RANDOM INTERCEPT / TYPE=AR(1)|UN` était réinterprété en structure
/// R sans effet aléatoire (un autre modèle que la demande G-side). ERROR ;
/// la structure R existante est exposée sous la syntaxe SAS
/// `RANDOM _RESIDUAL_ / SUBJECT= TYPE=AR(1)|UN` (SAS/STAT 9.4, RANDOM
/// statement, « _RESIDUAL_ »).
#[test]
fn ra_j02_p3_random_intercept_ar1_un_is_error() {
    for (ty, shown) in [("ar(1)", "AR(1)"), ("un", "UN")] {
        let out = run_sas(&format!(
            "{REP}proc glimmix data=rep; class subj; model y = ;
             random intercept / subject=subj type={ty}; run;"
        ));
        assert_error(
            &out,
            &format!(
                "TYPE={shown} for a G-side RANDOM effect {CANNOT} (planned: roadmap-avancee \
                 J08-P4). An R-side structure is requested by RANDOM _RESIDUAL_ / SUBJECT= \
                 TYPE={shown}."
            ),
            ty,
        );
    }
}

/// `RANDOM _RESIDUAL_ / SUBJECT= TYPE=AR(1)` : la structure R de
/// repeated.rs, inchangée — mêmes estimations que PROC MIXED
/// `REPEATED / SUBJECT= TYPE=AR(1)` sur les mêmes données (NORMAL/IDENTITY :
/// REML exacte). Alias `_RESID_` ; TYPE=VC/CS côté R, `_RESIDUAL_` combiné à
/// d'autres effets et TYPE=UN(1) (structure paramétrée) → ERROR.
#[test]
fn ra_j02_p3_random_residual_syntax() {
    let glimmix = run_sas(&format!(
        "{REP}proc glimmix data=rep; class subj; model y = / solution;
         random _residual_ / subject=subj type=ar(1); run;"
    ));
    assert_eq!(glimmix.exit_code, 0, "{}", glimmix.log);
    let mixed = run_sas(&format!(
        "{REP}proc mixed data=rep; class subj; model y = / solution;
         repeated / subject=subj type=ar(1); run;"
    ));
    assert_eq!(mixed.exit_code, 0, "{}", mixed.log);
    let cov_g = after(&glimmix.listing, "Covariance Parameter Estimates");
    let cov_m = after(&mixed.listing, "Covariance Parameter Estimates");
    for head in ["AR(1)", "Residual"] {
        let g = row(cov_g, head).expect("GLIMMIX covariance parameter");
        let m = row(cov_m, head).expect("MIXED covariance parameter");
        assert_eq!(
            g.split_whitespace().last(),
            m.split_whitespace().last(),
            "{head}"
        );
    }
    let alias = run_sas(&format!(
        "{REP}proc glimmix data=rep; class subj; model y = / solution;
         random _resid_ / subject=subj type=ar(1); run;"
    ));
    assert_eq!(alias.listing, glimmix.listing);

    for (stmt, needle) in [
        (
            "random _residual_ / subject=subj type=vc",
            format!(
                "RANDOM _RESIDUAL_ with TYPE=VC (only TYPE=AR(1) and TYPE=UN are implemented \
                 on the R side) {CANNOT}."
            ),
        ),
        (
            "random _residual_ / subject=subj",
            format!(
                "RANDOM _RESIDUAL_ with TYPE=VC (only TYPE=AR(1) and TYPE=UN are implemented \
                 on the R side) {CANNOT}."
            ),
        ),
        (
            "random _residual_ intercept / subject=subj type=ar(1)",
            format!("RANDOM _RESIDUAL_ combined with other random effects {CANNOT}."),
        ),
        (
            "random _residual_ / subject=subj type=un(1)",
            format!("TYPE=UN(1) (parameterized covariance structure) {CANNOT}."),
        ),
    ] {
        let out = run_sas(&format!(
            "{REP}proc glimmix data=rep; class subj; model y = ; {stmt}; run;"
        ));
        assert_error(&out, &needle, stmt);
    }
}

/// Base : la table Type III listait chaque paramètre (Intercept compris,
/// colonne de codage CLASS « g a ») avec F = t². Une ligne par effet, sans
/// ligne Intercept (défaut SAS) ; pour un effet à un paramètre, F = t² (test
/// de Wald à 1 ddl, égal au carré du t de la table des solutions).
#[test]
fn ra_j02_p3_type3_one_row_per_effect() {
    let out = run_sas(&format!(
        "{TWO}proc glimmix data=two; class g; model y = x g / dist=poisson solution; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    let type3 = after(&out.listing, "Type III Tests of Fixed Effects");
    let type3 = type3.split("Solutions for Fixed Effects").next().unwrap();
    let solutions = after(&out.listing, "Solutions for Fixed Effects");
    assert!(!type3.contains("Intercept"), "{type3}");
    assert!(!type3.contains("g a"), "{type3}");
    // F = t² on 1 DF: same p-value as the single parameter of the effect.
    for (effect, parameter) in [("x ", "x "), ("g ", "g a ")] {
        let test = row(type3, effect).unwrap_or_else(|| panic!("{effect}: {type3}"));
        assert_eq!(test.split_whitespace().nth(1), Some("1"), "{test}");
        let solution = row(solutions, parameter).expect("solution row");
        assert_eq!(
            test.split_whitespace().last(),
            solution.split_whitespace().last(),
            "{test} / {solution}"
        );
    }
}

/// Base : un effet CLASS à trois niveaux donnait deux lignes de type III
/// (« g a », « g b », F = t² de chaque colonne de codage) au lieu du test à
/// 2 ddl de l'effet. Table retirée + NOTE jusqu'à roadmap-avancee J08-P2 ;
/// les solutions restent imprimées.
#[test]
fn ra_j02_p3_type3_multi_parameter_effect_note() {
    let out = run_sas(&format!(
        "{CNT}proc glimmix data=cnt; class subj; model y = x subj / dist=poisson solution; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.log.contains(
            "NOTE: Type III tests of effects with more than one parameter (subj) are not \
             implemented in PROC GLIMMIX; the Type III Tests of Fixed Effects table is not \
             displayed (planned: roadmap-avancee J08-P2)."
        ),
        "{}",
        out.log
    );
    assert!(
        !out.listing.contains("Type III Tests of Fixed Effects"),
        "{}",
        out.listing
    );
    assert!(row(&out.listing, "subj A").is_some(), "{}", out.listing);
}

/// Base : OFFSET=, OBSWEIGHT=, DDF=, NOCENTER… et toute autre option MODEL
/// étaient sautées ; DIST= sans valeur laissait NORMAL. ERROR ; options
/// d'affichage (CL, COVB, CHISQ…) → WARNING ; HTYPE=3 (défaut) accepté.
#[test]
fn ra_j02_p3_model_options() {
    for (opt, shown) in [
        ("offset=x", "OFFSET="),
        ("obsweight=w", "OBSWEIGHT="),
        ("ddf=5", "DDF="),
        ("nocenter", "NOCENTER"),
        ("lweight=none", "LWEIGHT="),
        ("zeta=1e-8", "ZETA="),
        ("htype=1", "HTYPE=1"),
        ("bogus", "BOGUS"),
    ] {
        let out = run_sas(&format!(
            "{CNT}proc glimmix data=cnt; class subj; model y = x / dist=poisson {opt};
             random intercept / subject=subj; run;"
        ));
        assert_error(
            &out,
            &format!("The {shown} option of the MODEL statement {CANNOT}."),
            opt,
        );
    }
    let out = run_sas(&format!(
        "{CNT}proc glimmix data=cnt; model y = x / dist= ; run;"
    ));
    assert_error(&out, "expected a distribution name after DIST=", "DIST=");

    let program = |opt: &str| {
        run_sas(&format!(
            "{POIS}proc glimmix data=pois; model y = x / dist=poisson solution {opt}; run;"
        ))
    };
    let plain = program("");
    for (opt, shown) in [
        ("cl", "CL"),
        ("covb(details)", "COVB"),
        ("chisq", "CHISQ"),
        ("alpha=0.1", "ALPHA"),
        ("stdcoef", "STDCOEF"),
        ("intercept", "INTERCEPT"),
    ] {
        assert_display_warning(
            &program(opt),
            &plain,
            &format!(
                "The {shown} option of the MODEL statement is ignored in PROC GLIMMIX; display \
                 customization is not supported."
            ),
        );
    }
    let out = program("htype=3");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(out.listing, plain.listing);
}

/// Base : NOINT avec un effet CLASS — le codage de référence retirait
/// encore le dernier niveau, une colonne d'effet fixe manquait.
#[test]
fn ra_j02_p3_noint_with_class_effect() {
    let out = run_sas(&format!(
        "{BAL}proc glimmix data=bal; class subj g; model y = g / noint solution;
         random intercept / subject=subj; run;"
    ));
    assert_error(
        &out,
        &format!("NOINT with a CLASS fixed effect {CANNOT} (planned: roadmap-avancee J08-P2)."),
        "noint class",
    );
    // A continuous effect under NOINT remains accepted.
    let out = run_sas(&format!(
        "{POIS}proc glimmix data=pois; model y = x / dist=poisson noint solution; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
}

/// Base : sans RANDOM, METHOD=LAPLACE (comme RSPL) était étiqueté
/// « Residual PL ». SAS/STAT 9.4, The GLIMMIX Procedure, « GLM Mode or GLMM
/// Mode » et « Default Estimation Techniques » : en mode GLM, METHOD= n'a pas
/// d'effet ; maximum de vraisemblance hors loi normale, maximum de
/// vraisemblance restreint pour la loi normale — correction directe.
#[test]
fn ra_j02_p3_glm_mode_estimation_technique() {
    let program = |method: &str, dist: &str| {
        run_sas(&format!(
            "{POIS}proc glimmix data=pois {method}; model y = x / dist={dist} solution; run;"
        ))
    };
    let laplace = program("method=laplace", "poisson");
    assert_eq!(laplace.exit_code, 0, "{}", laplace.log);
    let technique = row(&laplace.listing, "Estimation Technique").expect("technique row");
    assert!(
        technique.trim_end().ends_with("Maximum Likelihood"),
        "{technique}"
    );
    assert!(
        !laplace.listing.contains("Residual PL"),
        "{}",
        laplace.listing
    );
    assert!(
        !laplace.listing.contains("Likelihood Approximation"),
        "{}",
        laplace.listing
    );
    // METHOD= has no effect in GLM mode.
    assert_eq!(program("method=rspl", "poisson").listing, laplace.listing);

    let normal = program("", "normal");
    let technique = row(&normal.listing, "Estimation Technique").expect("technique row");
    assert!(
        technique
            .trim_end()
            .ends_with("Restricted Maximum Likelihood"),
        "{technique}"
    );
}

/// Base : l'« Iteration History » était synthétique (deux lignes répétant
/// l'objectif final, « Change » 0.00000000 et nombres d'évaluations
/// inventés). Retirée jusqu'à une historique réelle ; aucune valeur inventée.
#[test]
fn ra_j02_p3_no_synthetic_iteration_history() {
    for program in [
        format!("{POIS}proc glimmix data=pois; model y = x / dist=poisson; run;"),
        format!(
            "{CNT}proc glimmix data=cnt; class subj; model y = x / dist=poisson;
             random intercept / subject=subj; run;"
        ),
        format!(
            "{BAL}proc glimmix data=bal method=laplace; class subj; model y = ;
             random intercept / subject=subj; run;"
        ),
    ] {
        let out = run_sas(&program);
        assert_eq!(out.exit_code, 0, "{}", out.log);
        assert!(
            !out.listing.contains("Iteration History"),
            "{}",
            out.listing
        );
        assert!(!out.listing.contains("0.00000000"), "{}", out.listing);
    }
}

/// Base : options RANDOM autres que SUBJECT=/TYPE= sautées (GROUP=,
/// RESIDUAL, NOFULLZ… et l'affichage SOLUTION, G, V…) ; `SUBJECT=id(grp)` /
/// `a*b` ne gardaient que le premier identifiant. ERROR ; affichage →
/// WARNING, estimations inchangées.
#[test]
fn ra_j02_p3_random_options() {
    for (stmt, needle) in [
        (
            "random intercept / subject=subj group=g",
            format!("The GROUP= option of the RANDOM statement {CANNOT}."),
        ),
        (
            "random intercept / subject=subj residual",
            format!("The RESIDUAL option of the RANDOM statement {CANNOT}."),
        ),
        (
            "random intercept / subject=subj nofullz",
            format!("The NOFULLZ option of the RANDOM statement {CANNOT}."),
        ),
        (
            "random intercept / subject=subj(g)",
            format!(
                "A nested or crossed SUBJECT= effect (id(group), a*b) {CANNOT} (planned: \
                 roadmap-avancee J08-P4)."
            ),
        ),
        (
            "random intercept / sub=subj*g",
            format!(
                "A nested or crossed SUBJECT= effect (id(group), a*b) {CANNOT} (planned: \
                 roadmap-avancee J08-P4)."
            ),
        ),
    ] {
        let out = run_sas(&format!(
            "{BAL}proc glimmix data=bal; class subj g; model y = ; {stmt}; run;"
        ));
        assert_error(&out, &needle, stmt);
    }

    let plain = run_sas(&format!(
        "{BAL}proc glimmix data=bal; class subj; model y = ; random intercept / subject=subj; run;"
    ));
    let out = run_sas(&format!(
        "{BAL}proc glimmix data=bal; class subj; model y = ;
         random intercept / subject=subj solution g gcorr v cl; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for opt in ["SOLUTION", "G", "GCORR", "V", "CL"] {
        assert!(
            out.log.contains(&format!(
                "WARNING: The {opt} option of the RANDOM statement is ignored in PROC GLIMMIX; \
                 display customization is not supported."
            )),
            "{opt}: {}",
            out.log
        );
    }
    assert_eq!(out.listing, plain.listing);
}

/// Base : les options de CLASS (`subj(ref='A')`, `/ order=data`) étaient
/// sautées et leurs mots devenaient des variables CLASS ; une liste
/// `x1-x3` perdait `x2`. ERROR.
#[test]
fn ra_j02_p3_class_options() {
    for (class, needle) in [
        (
            "class subj(ref='A')",
            "CLASS statement options (REF=, ORDER=, DESCENDING, ...) are not supported in \
             PROC GLIMMIX; the coding of the CLASS effects would silently differ from the \
             request.",
        ),
        (
            "class subj / order=data",
            "CLASS statement options (REF=, ORDER=, DESCENDING, ...) are not supported in \
             PROC GLIMMIX",
        ),
        (
            "class subj g1-g3",
            "expected a variable name in the CLASS statement",
        ),
    ] {
        let out = run_sas(&format!(
            "{BAL}proc glimmix data=bal; {class}; model y = ;
             random intercept / subject=subj; run;"
        ));
        assert_error(&out, needle, class);
    }
}

/// Base : les instructions GLIMMIX valides non implémentées (CODE, COVTEST,
/// EFFECT, LSMESTIMATE, NLOPTIONS, PARMS, SLICE, STORE) étaient signalées
/// « 180-322 … not valid ». Message du catalogue « not supported … cannot be
/// ignored » ; une instruction inventée reste une 180-322.
#[test]
fn ra_j02_p3_unsupported_statements() {
    for stmt in [
        "code file='x.sas'",
        "covtest 'zero' 0",
        "effect sp = spline(x)",
        "lsmestimate subj 'a' 1 -1",
        "nloptions tech=nrridg",
        "parms (1) (2)",
        "slice subj",
        "store work.s",
    ] {
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        let out = run_sas(&format!(
            "{CNT}proc glimmix data=cnt; class subj; model y = x / dist=poisson;
             random intercept / subject=subj; {stmt}; run;"
        ));
        assert_error(
            &out,
            &format!(
                "The {kw} statement is not supported in PROC GLIMMIX; it can affect results and \
                 cannot be ignored."
            ),
            stmt,
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    let out = run_sas(&format!(
        "{BAL}proc glimmix data=bal; class subj; model y = ; invented x; run;"
    ));
    assert_error(
        &out,
        "180-322: Statement 'INVENTED' is not valid or it is used out of proper order in PROC GLIMMIX.",
        "invented",
    );
}

/// Base : code mort — NOTE « parse-accepted but not implemented » pour
/// ESTIMATE/CONTRAST/LSMEANS (inatteignables, rejetées au parsing) et WEIGHT.
/// Ces instructions sont des ERROR ; aucune NOTE trompeuse n'est émise.
#[test]
fn ra_j02_p3_no_misleading_notes() {
    for stmt in ["estimate 'x' x 1", "contrast 'x' x 1", "lsmeans subj"] {
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        let out = run_sas(&format!(
            "{CNT}proc glimmix data=cnt; class subj; model y = x / dist=poisson;
             random intercept / subject=subj; {stmt}; run;"
        ));
        assert_error(&out, &format!("The {kw} statement {CANNOT}."), stmt);
        assert!(!out.log.contains("parse-accepted"), "{}", out.log);
    }
    let out = run_sas(&format!(
        "{CNT}proc glimmix data=cnt; class subj; model y = x / dist=poisson solution ddfm=contain;
         random intercept / subject=subj; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(!out.log.contains("parse-accepted"), "{}", out.log);
    assert!(!out.log.contains("not implemented"), "{}", out.log);
}
