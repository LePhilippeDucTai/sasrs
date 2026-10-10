// ── J02-P7 : contrat PROC REPORT (replis silencieux supprimés) ───────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, code de sortie). Oracle de sévérité :
// CONTRIBUTING §5 ; syntaxe et sémantique de référence : Base SAS 9.4
// Procedures Guide, The REPORT Procedure (DEFINE, BREAK, RBREAK, COMPUTE
// statements ; « Usage of Variables in a Report »).

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

/// sashelp.class subset: 4 F (ages 13 13 14 12), 3 M (ages 14 14 12).
const CLS: &str = "data cls; input name $ sex $ age height; datalines;
Alfred M 14 69.0
Alice F 13 56.5
Barbara F 13 65.3
Carol F 14 62.8
Henry M 14 63.5
James M 12 57.3
Jane F 12 59.8
;
run;
";

/// Contract ERROR suffix shared by the rejected constructions.
const CANNOT: &str = "is not supported in PROC REPORT; it can affect results and cannot be ignored";

/// `proc report data=cls nowd; <body> run;` after the CLS data step.
fn report(body: &str) -> crate::RunOutcome {
    run_sas(&format!("{CLS}proc report data=cls nowd; {body} run;"))
}

/// Asserts a rejected step: exit code 2, `needle` in the log, no listing.
fn assert_rejected(out: &crate::RunOutcome, needle: &str, ctx: &str) {
    assert_eq!(out.exit_code, 2, "{ctx}: {}", out.log);
    assert!(
        out.log.contains(needle),
        "{ctx}: missing «{needle}»\n{}",
        out.log
    );
    assert!(
        out.listing.trim().is_empty(),
        "{ctx}: the step must be rejected before any output\n{}",
        out.listing
    );
}

fn assert_has(out: &crate::RunOutcome, needle: &str) {
    assert!(out.log.contains(needle), "missing «{needle}»\n{}", out.log);
}

/// Whitespace-split data lines of the listing of one report: the title (« The
/// SAS System ») and the column headings are skipped.
fn data_lines(listing: &str) -> Vec<Vec<String>> {
    listing
        .lines()
        .map(|l| l.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .filter(|t| !t.is_empty())
        .skip(2)
        .collect()
}

/// Base : `where upcase(sex)='F'` évaluait l'appel de fonction à manquant et
/// vidait le rapport (« 0 observations read », code 0) ; une variable absente
/// de la table valait manquant. Désormais : appel de fonction → ERROR
/// jusqu'à J03-P3 ; variable absente → ERROR SAS « Variable … is not on
/// file » ; les comparaisons simples restent honorées.
#[test]
fn ra_j02_p7_report_where_function_call() {
    let out = report("column name sex age; where upcase(sex)='F';");
    assert_rejected(
        &out,
        &format!(
            "ERROR: A function call (UPCASE) in the WHERE statement {CANNOT} (planned: \
             roadmap-avancee J03-P3)."
        ),
        "upcase",
    );

    let out = report("column name sex age; where nope > 1;");
    assert_rejected(
        &out,
        "ERROR: Variable nope is not on file WORK.CLS.",
        "unknown variable",
    );

    let out = report("column name sex age; where sex = 'F';");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_has(
        &out,
        "NOTE: There were 4 observations read from the data set WORK.CLS.",
    );
}

/// Base : un appel de fonction dans un COMPUTE valait manquant ; le nom
/// composé `age.sum` (référence SAS d'une variable d'analyse) s'imprimait en
/// morceaux dans LINE ; `_BREAK_` valait manquant. Désormais ERROR (fonctions
/// : J03-P3 ; `_BREAK_` : J11-P2 ; nom composé : non planifié).
#[test]
fn ra_j02_p7_report_compute_unsupported_expressions() {
    let out = report(
        "column name age dbl; define age / display; define dbl / computed;
         compute dbl; dbl = round(age / 3, 0.1); endcomp;",
    );
    assert_rejected(
        &out,
        &format!(
            "ERROR: A function call (ROUND) in a COMPUTE block {CANNOT} (planned: \
             roadmap-avancee J03-P3)."
        ),
        "round",
    );

    let out = report(
        "column sex age; define sex / group;
         compute after; line 'Oldest: ' max(age, 1); endcomp;",
    );
    assert_rejected(&out, "A function call (MAX) in a COMPUTE block", "line max");

    let out = report(
        "column sex age; define sex / group; rbreak after / summarize;
         compute after; line 'Total ' age.sum best8.; endcomp;",
    );
    assert_rejected(
        &out,
        &format!("ERROR: The compound name AGE.SUM {CANNOT}."),
        "age.sum",
    );

    let out = report(
        "column name age flag; define age / display; define flag / computed;
         compute flag; flag = _break_ = ' '; endcomp;",
    );
    assert_rejected(
        &out,
        &format!(
            "ERROR: The _BREAK_ automatic variable {CANNOT} (planned: roadmap-avancee J11-P2)."
        ),
        "_break_",
    );
}

/// Base : le contexte d'un COMPUTE nommait les colonnes par leur LIBELLÉ :
/// `dbl = age * 2` avec `define age / … 'Years'` lisait un manquant et
/// l'affectation à `dbl` (libellé 'Twice') était perdue ; `years` (le
/// libellé) se résolvait ; une affectation à une variable temporaire écrasait
/// la colonne cible ; une cible absente du rapport ne faisait rien.
/// Correction directe — SAS 9.4 COMPUTE statement, « Four Ways to Reference
/// Report Items in a Compute Block » : nom de l'élément, alias, nom composé
/// ou `_Cn_` ; jamais le libellé.
#[test]
fn ra_j02_p7_report_compute_references_by_name() {
    let defines = "column name age dbl;
         define age / display 'Years'; define dbl / computed 'Twice';";
    let out = report(&format!("{defines} compute dbl; dbl = age * 2; endcomp;"));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(data_lines(&out.listing)[0], ["Alfred", "14", "28"]);

    let out = report(&format!("{defines} compute dbl; dbl = _c2_ * 3; endcomp;"));
    assert_eq!(data_lines(&out.listing)[0], ["Alfred", "14", "42"]);

    // A label is no name: an unassigned variable of the block (SAS NOTE).
    let out = report(&format!("{defines} compute dbl; dbl = years * 2; endcomp;"));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_has(&out, "NOTE: Variable years is uninitialized.");
    assert_eq!(data_lines(&out.listing)[0], ["Alfred", "14", "."]);

    let out = report(&format!("{defines} compute dbl; tmp = age; endcomp;"));
    assert_rejected(
        &out,
        &format!(
            "ERROR: An assignment to TMP, which is not a report item (a temporary variable of \
             the COMPUTE block), {CANNOT}."
        ),
        "temporary",
    );

    let out = report(&format!("{defines} compute twice; dbl = 1; endcomp;"));
    assert_rejected(
        &out,
        "ERROR: The COMPUTE block target TWICE is not a report item of the COLUMN statement in \
         PROC REPORT.",
        "target by label",
    );
}

/// Base : COMPUTE BEFORE était ignoré ; COMPUTE AFTER <var> s'imprimait une
/// seule fois en fin de rapport ; une LINE dans le COMPUTE d'une colonne était
/// ignorée ; une affectation dans COMPUTE AFTER réécrivait chaque ligne ;
/// `/ CHARACTER` était sauté (colonne restée numérique, manquante dans OUT=).
/// Désormais ERROR (J11-P2 ; CHARACTER non planifié) ; le COMPUTE AFTER de
/// niveau rapport avec LINE reste honoré.
#[test]
fn ra_j02_p7_report_compute_locations() {
    let group = "column sex age; define sex / group;";
    for (block, needle) in [
        (
            "compute before; line 'Start'; endcomp;",
            format!("ERROR: COMPUTE BEFORE {CANNOT} (planned: roadmap-avancee J11-P2)."),
        ),
        (
            "compute before sex; line 'Start'; endcomp;",
            format!("ERROR: COMPUTE BEFORE SEX {CANNOT} (planned: roadmap-avancee J11-P2)."),
        ),
        (
            "compute after sex; line 'End of sex'; endcomp;",
            format!("ERROR: COMPUTE AFTER SEX {CANNOT} (planned: roadmap-avancee J11-P2)."),
        ),
        (
            "compute age; line 'Age'; endcomp;",
            format!(
                "ERROR: A LINE statement in the COMPUTE block of report item AGE {CANNOT} \
                 (planned: roadmap-avancee J11-P2)."
            ),
        ),
        (
            "compute after; age = 0; endcomp;",
            format!(
                "ERROR: An assignment in a COMPUTE AFTER block {CANNOT} (planned: \
                 roadmap-avancee J11-P2)."
            ),
        ),
        (
            "compute age / character length=10; age = 1; endcomp;",
            format!("ERROR: The CHARACTER option of the COMPUTE statement {CANNOT}."),
        ),
    ] {
        assert_rejected(&report(&format!("{group} {block}")), &needle, block);
    }

    let out = report(&format!(
        "{group} compute after; line 'End of report'; endcomp;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("End of report"), "{}", out.listing);
}

/// Base : BREAK BEFORE (et RBREAK BEFORE) était rendu comme AFTER ; un BREAK
/// sur une variable ni GROUP ni ORDER était abandonné ; BREAK/RBREAK étaient
/// ignorés dans un rapport détaillé ; une deuxième RBREAK remplaçait la
/// première. Désormais ERROR (J11-P2) ; BREAK sur une variable non
/// GROUP/ORDER : ERROR de SAS (SAS 9.4 BREAK statement : « break-variable is
/// a group or order variable »).
#[test]
fn ra_j02_p7_report_break_rules() {
    let group = "column sex age height; define sex / group; define age / group;";
    for (body, needle) in [
        (
            format!("{group} break before sex / summarize;"),
            format!("ERROR: BREAK BEFORE {CANNOT} (planned: roadmap-avancee J11-P2)."),
        ),
        (
            format!("{group} rbreak before / summarize;"),
            format!("ERROR: RBREAK BEFORE {CANNOT} (planned: roadmap-avancee J11-P2)."),
        ),
        (
            "column sex name age; define sex / group; define name / display; \
             break after name / summarize;"
                .to_string(),
            "ERROR: You can only BREAK on GROUPing and ORDERing variables.".to_string(),
        ),
        (
            "column name age; break after name / summarize;".to_string(),
            "ERROR: You can only BREAK on GROUPing and ORDERing variables.".to_string(),
        ),
        (
            "column name age; rbreak after / summarize;".to_string(),
            format!(
                "ERROR: An RBREAK statement in a detail report (no GROUP or ORDER variable) \
                 {CANNOT} (planned: roadmap-avancee J11-P2)."
            ),
        ),
        (
            format!("{group} rbreak after / summarize; rbreak after / summarize;"),
            format!(
                "ERROR: More than one RBREAK statement {CANNOT} (planned: roadmap-avancee \
                 J11-P2)."
            ),
        ),
    ] {
        assert_rejected(&report(&body), &needle, &body);
    }

    // BREAK AFTER a GROUP variable stays honored: one sub-total line per sex.
    let out = report(&format!("{group} break after sex / summarize;"));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    let lines = data_lines(&out.listing);
    // F: 12 (59.8), 13 (121.8), 14 (62.8), sub-total; M: 12, 14, sub-total.
    assert_eq!(lines[3], ["F", "244.4"], "{}", out.listing);
    assert_eq!(lines[6], ["M", "189.8"], "{}", out.listing);
}

/// Base : OL, DOL, UL, DUL, SKIP, PAGE et SUPPRESS étaient acceptés sans un
/// mot (et tout autre jeton sauté). Désormais un WARNING d'affichage par
/// option (J11-P2), le sous-total reste imprimé ; option inconnue → ERROR.
#[test]
fn ra_j02_p7_report_break_display_options() {
    let out = report(
        "column sex age; define sex / group;
         break after sex / summarize ol dol ul dul skip page suppress color=red;
         rbreak after / summarize dol;",
    );
    assert_eq!(out.exit_code, 1, "{}", out.log);
    let expected = [
        ("BREAK", "OL", true),
        ("BREAK", "DOL", true),
        ("BREAK", "UL", true),
        ("BREAK", "DUL", true),
        ("BREAK", "SKIP", true),
        ("BREAK", "PAGE", true),
        ("BREAK", "SUPPRESS", true),
        ("BREAK", "COLOR=", false),
        ("RBREAK", "DOL", true),
    ];
    for (stmt, opt, planned) in expected {
        let suffix = if planned {
            " (planned: roadmap-avancee J11-P2)"
        } else {
            ""
        };
        assert_has(
            &out,
            &format!(
                "WARNING: The {opt} option of the {stmt} statement is ignored in PROC REPORT; \
                 display customization is not supported{suffix}."
            ),
        );
    }
    assert_eq!(
        out.log.matches("WARNING:").count(),
        expected.len(),
        "{}",
        out.log
    );
    // Grand total 92 = 52 (F) + 40 (M).
    assert!(
        data_lines(&out.listing).iter().any(|l| *l == ["92"]),
        "{}",
        out.listing
    );

    let out = report("column sex age; define sex / group; break after sex / bogus;");
    assert_rejected(
        &out,
        "ERROR: Unknown or unsupported BREAK option 'BOGUS' in PROC REPORT.",
        "bogus",
    );
}

/// Base : un rapport ACROSS ignorait BREAK, RBREAK, COMPUTE, FORMAT=, WIDTH=,
/// les colonnes DISPLAY/ORDER/COMPUTED et toute ANALYSIS au-delà de la
/// première ; OUT= était abandonné avec une NOTE. Désormais ERROR (J11-P3) ;
/// SPACING= → WARNING ; le croisement GROUP × ACROSS × ANALYSIS reste rendu.
#[test]
fn ra_j02_p7_report_across_unsupported() {
    let across = "column sex age height; define sex / group; define age / across;";
    let out = report(&format!("{across} define height / analysis sum;"));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("12 SUM"), "{}", out.listing);

    let msg = |what: &str| {
        format!("ERROR: {what} in an ACROSS report {CANNOT} (planned: roadmap-avancee J11-P3).")
    };
    for (extra, what) in [
        ("break after sex / summarize;", "A BREAK statement"),
        ("rbreak after / summarize;", "An RBREAK statement"),
        ("compute after; line 'x'; endcomp;", "A COMPUTE block"),
        (
            "define height / analysis sum format=8.2;",
            "The FORMAT= option of the DEFINE statement (HEIGHT)",
        ),
        (
            "define height / analysis sum width=12;",
            "The WIDTH= option of the DEFINE statement (HEIGHT)",
        ),
    ] {
        assert_rejected(&report(&format!("{across} {extra}")), &msg(what), extra);
    }

    let out = run_sas(&format!(
        "{CLS}proc report data=cls nowd out=work.r; {across} run;"
    ));
    assert_rejected(&out, &msg("The OUT= option"), "out=");

    let out = report("column sex name age height; define sex / group; define age / across;");
    assert_rejected(&out, &msg("A DISPLAY variable (NAME)"), "display column");

    let out = report("column sex age height; define sex / order; define age / across;");
    assert_rejected(&out, &msg("An ORDER variable (SEX)"), "order column");

    let out = run_sas(&format!(
        "proc format; value agegrp low-12='Young' 13-high='Adult'; run;
         {CLS}data cls2; set cls; format age agegrp.; run;
         proc report data=cls2 nowd; {across} run;"
    ));
    assert_rejected(&out, &msg("The format AGEGRP. of AGE"), "stored format");

    let out = report(&format!("{across} define height / analysis sum spacing=4;"));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert_has(
        &out,
        "WARNING: The SPACING= option of the DEFINE statement for HEIGHT is ignored in an \
         ACROSS report of PROC REPORT; display customization is not supported (planned: \
         roadmap-avancee J11-P3).",
    );
}

/// Base : une variable ORDER consolidait les observations comme GROUP
/// (F 52, M 40). Correction directe — SAS 9.4 REPORT, « Usage of Variables in
/// a Report » : « A report that contains one or more order variables has a
/// row for every observation in the input data set » ; une variable GROUP
/// accompagnée d'une variable ORDER ou DISPLAY ne consolide pas non plus
/// (NOTE SAS « Groups are not created because the usage of … »).
#[test]
fn ra_j02_p7_report_order_does_not_consolidate() {
    let out = report("column sex age; define sex / order; define age / analysis sum;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(
        data_lines(&out.listing),
        [
            ["F", "13"],
            ["F", "13"],
            ["F", "14"],
            ["F", "12"],
            ["M", "14"],
            ["M", "14"],
            ["M", "12"]
        ],
        "{}",
        out.listing
    );
    assert!(!out.log.contains("Groups are not created"), "{}", out.log);

    let out = report(
        "column sex age height; define sex / order; define age / group;
         define height / analysis sum; break after sex / summarize;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_has(
        &out,
        "NOTE: Groups are not created because the usage of sex is ORDER. To avoid this note, \
         change all GROUP variables to ORDER variables.",
    );
    let lines = data_lines(&out.listing);
    assert_eq!(
        lines.len(),
        9,
        "7 detail rows + 2 sub-totals\n{}",
        out.listing
    );
    assert_eq!(lines[4], ["F", "244.4"], "{}", out.listing);

    let out = report("column sex name age; define sex / group;");
    assert_has(
        &out,
        "NOTE: Groups are not created because the usage of name is DISPLAY. To avoid this \
         note, change all GROUP variables to ORDER variables.",
    );
    assert_eq!(data_lines(&out.listing).len(), 7, "{}", out.listing);
}

/// Base : HEADLINE et HEADSKIP étaient acceptés sans effet ni diagnostic ;
/// NOWINDOWS (orthographe SAS, alias NOWD) était une option inconnue.
#[test]
fn ra_j02_p7_report_headline_headskip_nowindows() {
    let out = run_sas(&format!(
        "{CLS}proc report data=cls nowd headline headskip; column name age; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for opt in ["HEADLINE", "HEADSKIP"] {
        assert_has(
            &out,
            &format!(
                "WARNING: The {opt} option is ignored in PROC REPORT; display customization is \
                 not supported (planned: roadmap-avancee J11-P3)."
            ),
        );
    }

    let out = run_sas(&format!(
        "{CLS}proc report data=cls nowindows; column name age; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("Alfred"), "{}", out.listing);
}

/// Base : le format stocké d'une variable était ignoré (affichage brut,
/// regroupement sur la valeur brute). Correction directe — SAS 9.4 DEFINE
/// statement, FORMAT= : « PROC REPORT honors the first of these formats that
/// it finds: the format assigned with FORMAT= in the DEFINE statement, […]
/// the format associated with the variable in the data set » ; GROUP
/// consolide sur les valeurs formatées.
#[test]
fn ra_j02_p7_report_stored_formats() {
    let setup = "proc format; value agegrp low-12='Young' 13-high='Adult'; run;";
    let run_fmt = |body: &str| {
        run_sas(&format!(
            "{setup}{CLS}data cls2; set cls; format age agegrp.; run;
             proc report data=cls2 nowd; {body} run;"
        ))
    };
    let out = run_fmt("column name age;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(data_lines(&out.listing)[0], ["Alfred", "Adult"]);
    assert_eq!(data_lines(&out.listing)[5], ["James", "Young"]);

    // GROUP on the formatted values: Young (12) before Adult (13+).
    let out = run_fmt("column age height; define age / group; define height / analysis sum;");
    assert_eq!(
        data_lines(&out.listing),
        [["Young", "117.1"], ["Adult", "317.1"]],
        "{}",
        out.listing
    );

    // DEFINE FORMAT= first.
    let out =
        run_fmt("column age height; define age / group format=3.; define height / analysis sum;");
    assert_eq!(data_lines(&out.listing).len(), 3, "{}", out.listing);
    assert_eq!(
        data_lines(&out.listing)[0],
        ["12", "117.1"],
        "{}",
        out.listing
    );
}

/// Les instructions SAS valides de PROC REPORT sont toutes implémentées
/// (BREAK, COLUMN, COMPUTE, DEFINE, RBREAK) sauf BY, FREQ et WEIGHT, qui
/// portent le message « not supported » du catalogue ; une instruction
/// inventée reste 180-322 ; FORMAT est un WARNING d'affichage.
#[test]
fn ra_j02_p7_report_valid_statements() {
    for stmt in ["by sex", "freq age", "weight age"] {
        let kw = stmt.split(' ').next().unwrap().to_ascii_uppercase();
        assert_rejected(
            &report(&format!("column name age; {stmt};")),
            &format!("ERROR: The {kw} statement {CANNOT}."),
            stmt,
        );
    }
    assert_rejected(
        &report("column name age; invented x;"),
        "180-322: Statement 'INVENTED' is not valid or it is used out of proper order in PROC \
         REPORT.",
        "invented",
    );
    let out = report("column name age; format age 8.2;");
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert_has(
        &out,
        "WARNING: The FORMAT statement is ignored in PROC REPORT; display customization is not \
         supported.",
    );
}
