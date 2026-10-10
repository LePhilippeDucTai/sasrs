// ── J02-P4 : contrat DISCRIM (replis silencieux supprimés) ───────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, code de sortie). Oracle de sévérité :
// CONTRIBUTING §5 ; syntaxe de référence : SAS/STAT 14.1 (SAS 9.4) User's
// Guide, The DISCRIM Procedure, PROC DISCRIM Statement
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_discrim_syntax01.htm

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

/// Two well-separated classes (data of the m27 fixture).
const LDA: &str = "data lda; input class $ x; datalines;
A 1
A 2
A 3
B 5
B 6
B 7
;
run;
";

/// Contract ERROR suffix shared by the rejected constructions.
const CANNOT: &str =
    "is not supported in PROC DISCRIM; it can affect results and cannot be ignored";

/// Asserts a rejected step: exit code 2, `needle` in the log, no listing.
fn assert_error(out: &crate::RunOutcome, needle: &str, ctx: &str) {
    assert_eq!(out.exit_code, 2, "{ctx}: {}", out.log);
    assert!(
        out.log.contains(needle),
        "{ctx}: missing «{needle}»\n{}",
        out.log
    );
    assert!(
        !out.listing.contains("The DISCRIMINANT Procedure"),
        "{ctx}: the step must be rejected before execution\n{}",
        out.listing
    );
}

/// `proc discrim data=lda <opts>; class class; var x; <stmts> run;`
fn discrim(opts: &str, stmts: &str) -> crate::RunOutcome {
    run_sas(&format!(
        "{LDA}proc discrim data=lda {opts}; class class; var x; {stmts} run;"
    ))
}

/// Base : toute option PROC inconnue était sautée jeton par jeton
/// (TESTDATA=, TESTOUT=, OUTCROSS=, OUTD=, LIST, CROSSLIST, CANONICAL,
/// NOPRINT…), les options de table de DATA= comprises. Parsing commun :
/// option inconnue → « Unexpected option » ; option SAS valide non
/// implémentée qui peut changer un résultat ou créer une table → ERROR
/// (J09-P2 quand le plan la livre) ; option d'affichage → WARNING.
#[test]
fn ra_j02_p4_discrim_proc_options() {
    let out = discrim("bogus", "");
    assert_error(
        &out,
        "ERROR: Unexpected option 'BOGUS' on PROC DISCRIM statement.",
        "unknown",
    );

    for (opt, shown, planned) in [
        (
            "testdata=lda",
            "TESTDATA=",
            " (planned: roadmap-avancee J09-P2)",
        ),
        (
            "testout=t",
            "TESTOUT=",
            " (planned: roadmap-avancee J09-P2)",
        ),
        (
            "crosslist",
            "CROSSLIST",
            " (planned: roadmap-avancee J09-P2)",
        ),
        ("outcross=c", "OUTCROSS=", ""),
        ("outd=d", "OUTD=", ""),
        ("canonical", "CANONICAL", ""),
        ("threshold=0.9", "THRESHOLD=", ""),
        ("singular=1e-6", "SINGULAR=", ""),
    ] {
        let out = discrim(opt, "");
        assert_error(
            &out,
            &format!("ERROR: The {shown} option {CANNOT}{planned}."),
            opt,
        );
    }

    let out = run_sas(&format!(
        "{LDA}proc discrim data=lda(where=(x>1)); class class; var x; run;"
    ));
    assert_error(
        &out,
        "ERROR: Data set options on DATA= are not supported in PROC DISCRIM; they can affect \
         results and cannot be ignored.",
        "DATA= options",
    );
    let out = discrim("out=o(keep=x)", "");
    assert_error(
        &out,
        "Data set options on OUT= are not supported",
        "OUT= options",
    );

    // POOL= without a value used to mean POOL=YES.
    let out = discrim("pool=", "");
    assert_error(&out, "ERROR: expected YES, NO or TEST after POOL=", "POOL=");

    // Display-only options: WARNING (exit 1), listing unchanged.
    let plain = discrim("", "");
    let out = discrim("noprint simple posterr distance", "");
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for opt in ["NOPRINT", "SIMPLE", "POSTERR"] {
        assert!(
            out.log.contains(&format!(
                "WARNING: The {opt} option is ignored in PROC DISCRIM; display customization \
                 is not supported."
            )),
            "{opt}: {}",
            out.log
        );
    }
    assert!(
        out.log.contains(
            "WARNING: The F statistics and probabilities of the DISTANCE option are not \
             displayed in PROC DISCRIM; display customization is not supported."
        ),
        "{}",
        out.log
    );
    assert_eq!(out.listing, plain.listing);

    // PCOV / WCOV request the covariance matrices this listing always
    // prints: honored without diagnostic.
    let out = discrim("pcov wcov", "");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(!out.log.contains("WARNING"), "{}", out.log);
    assert!(
        out.listing
            .contains("Pooled Within-Class Covariance Matrix"),
        "{}",
        out.listing
    );
    assert_eq!(out.listing, plain.listing);
}

/// Base : `PRIORS 'A'=.3 'B'=.7;` (et toute forme autre que EQUAL /
/// PROPORTIONAL) était remplacé par EQUAL en silence. SAS/STAT 9.4, PRIORS
/// statement : `PRIORS EQUAL | PROPORTIONAL | PROP | probabilities`.
#[test]
fn ra_j02_p4_discrim_explicit_priors() {
    for stmt in ["priors 'A'=.3 'B'=.7;", "priors A=0.3 B=0.7;"] {
        let out = discrim("", stmt);
        assert_error(
            &out,
            &format!(
                "ERROR: PRIORS with explicit probabilities {CANNOT} (planned: roadmap-avancee J09-P2)."
            ),
            stmt,
        );
    }
    let out = discrim("", "priors bogus;");
    assert_error(
        &out,
        "ERROR: The PRIORS statement of PROC DISCRIM expects EQUAL, PROPORTIONAL or \
         level=probability pairs.",
        "bogus",
    );

    // EQUAL and PROPORTIONAL remain honored (unequal class sizes 4/2).
    let unequal = "data u; input class $ x; datalines;
A 1
A 2
A 3
A 4
B 6
B 7
;
run;
";
    for (stmt, row) in [
        ("priors equal;", "0.5000    0.5000"),
        ("priors proportional;", "0.6667    0.3333"),
        ("priors prop;", "0.6667    0.3333"),
    ] {
        let out = run_sas(&format!(
            "{unequal}proc discrim data=u; class class; var x; {stmt} run;"
        ));
        assert_eq!(out.exit_code, 0, "{stmt}: {}", out.log);
        let priors = out
            .listing
            .lines()
            .find(|l| l.trim_start().starts_with("Priors"))
            .expect("Priors row");
        assert!(priors.contains(row), "{stmt}: {priors}");
    }
}

/// Base : CROSSVALIDATE émettait une NOTE « parse-accepted but not
/// implemented » puis l'analyse tournait sans validation croisée ; OUTSTAT=
/// idem, sans table créée ; NOCLASSIFY et SHORT : NOTE puis ignorés.
/// CROSSVALIDATE / OUTSTAT= → ERROR (J09-P2) ; NOCLASSIFY / SHORT
/// (affichage) → WARNING.
#[test]
fn ra_j02_p4_discrim_crossvalidate_outstat() {
    for (opt, shown) in [
        ("crossvalidate", "CROSSVALIDATE"),
        ("outstat=st", "OUTSTAT="),
    ] {
        let out = run_sas(&format!(
            "{LDA}proc discrim data=lda {opt}; class class; var x; run;
             proc print data=st; run;"
        ));
        assert_error(
            &out,
            &format!("ERROR: The {shown} option {CANNOT} (planned: roadmap-avancee J09-P2)."),
            opt,
        );
        assert!(!out.log.contains("parse-accepted"), "{opt}: {}", out.log);
    }

    let plain = discrim("", "");
    let out = discrim("noclassify short", "");
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for opt in ["NOCLASSIFY", "SHORT"] {
        assert!(
            out.log.contains(&format!(
                "WARNING: The {opt} option is ignored in PROC DISCRIM; display customization \
                 is not supported."
            )),
            "{opt}: {}",
            out.log
        );
    }
    assert!(!out.log.contains("parse-accepted"), "{}", out.log);
    assert_eq!(out.listing, plain.listing);
}

/// Base : `CLASS a b;` ne gardait que la première variable (et `ID a b;`
/// aussi) ; la boucle VAR sautait tout jeton non-nom (`x1-x3` devenait
/// `x1 x3`). SAS/STAT 9.4 : `CLASS variable;`, `ID variable;`.
#[test]
fn ra_j02_p4_discrim_single_class_variable() {
    let out = run_sas(&format!(
        "{LDA}proc discrim data=lda; class class x; var x; run;"
    ));
    assert_error(
        &out,
        "ERROR: The CLASS statement of PROC DISCRIM takes a single variable; found CLASS X.",
        "class",
    );
    let out = discrim("", "id class x;");
    assert_error(
        &out,
        "ERROR: The ID statement of PROC DISCRIM takes a single variable; found CLASS X.",
        "id",
    );
    let out = run_sas(&format!(
        "{LDA}data l3; set lda; x1 = x; x2 = x * x; x3 = -x; run;
         proc discrim data=l3; class class; var x1-x3; run;"
    ));
    assert_error(&out, "ERROR: expected a ';'", "var range");
}

/// Base : la table « Classification Results for Training Data » (une ligne
/// par observation) était toujours imprimée. SAS/STAT 14.1, PROC DISCRIM
/// statement, LIST : « displays the resubstitution classification results
/// for each observation » — la table n'apparaît qu'avec LIST ; le résumé
/// « Error Count Estimates » reste imprimé. Correction directe.
#[test]
fn ra_j02_p4_discrim_list_classification_table() {
    let out = discrim("", "");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        !out.listing
            .contains("Classification Results for Training Data"),
        "{}",
        out.listing
    );
    assert!(
        out.listing
            .contains("Error Count Estimates for Training Data"),
        "{}",
        out.listing
    );

    let out = discrim("list", "");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing
            .contains("Classification Results for Training Data"),
        "{}",
        out.listing
    );
    // One row per observation, posteriors of the m27 oracle (x=3: 0.9820).
    assert!(out.listing.contains("Obs    From CLASS"), "{}", out.listing);
    assert!(
        out.listing
            .contains("3    A             A                        0.9820    0.0180"),
        "{}",
        out.listing
    );
    assert!(
        out.listing
            .contains("Error Count Estimates for Training Data"),
        "{}",
        out.listing
    );
}

/// Base : les instructions DISCRIM valides non implémentées TESTCLASS,
/// TESTFREQ et TESTID étaient signalées « 180-322 … not valid ». Message du
/// catalogue « not supported … cannot be ignored » (BY, FREQ, WEIGHT : déjà
/// le message partagé) ; une instruction inventée reste une 180-322.
#[test]
fn ra_j02_p4_discrim_unsupported_statements() {
    for stmt in [
        "testclass class",
        "testfreq x",
        "testid x",
        "by class",
        "freq x",
        "weight x",
    ] {
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        let out = discrim("", &format!("{stmt};"));
        assert_error(&out, &format!("ERROR: The {kw} statement {CANNOT}."), stmt);
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    let out = discrim("", "invented x;");
    assert_error(
        &out,
        "180-322: Statement 'INVENTED' is not valid or it is used out of proper order in PROC \
         DISCRIM.",
        "invented",
    );
}
