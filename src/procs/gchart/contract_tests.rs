// ── J02-P6 : contrat PROC GCHART (replis silencieux supprimés) ────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic (texte, code de sortie). Oracle de sévérité : CONTRIBUTING §5
// (option d'affichage non rendue → WARNING, code 1 ; demande qui change les
// objets produits → ERROR, étape rejetée, code 2). Syntaxe de référence :
// SAS/GRAPH 9.4 Reference, The GCHART Procedure.
//
// Les programmes tournent à l'identique dans les deux builds ; sous
// `--features graphics`, les images vont dans un répertoire temporaire.

/// Exécute un programme complet (mode déterministe, images éventuelles dans
/// un répertoire temporaire) : log, listing, code.
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

/// WORK.CATS shared by the tests.
const CATS: &str = "data cats; input category $ count; datalines;
A 10
B 25
C 15
;
run;
";

fn ignored(stmt: &str, option: &str) -> String {
    format!(
        "WARNING: The {option} option of the {stmt} statement is ignored in PROC GCHART; \
         display customization is not supported."
    )
}

fn assert_has(out: &crate::RunOutcome, needle: &str) {
    assert!(out.log.contains(needle), "missing «{needle}»\n{}", out.log);
}

/// Base : TYPE=PERCENT|CFREQ|CPERCENT retombaient en silence sur FREQ et
/// SUBGROUP=, GROUP=, MIDPOINTS= (comme toute autre option du diagramme)
/// étaient avalés. Désormais un WARNING par option, le diagramme FREQ est
/// dessiné ; SUMVAR= et TYPE=FREQ|SUM|MEAN restent honorés, sans diagnostic.
#[test]
fn ra_j02_p6_gchart_options_warn() {
    let out = run_sas(&format!(
        "{CATS}proc gchart data=cats;
           vbar category / type=percent subgroup=category group=category midpoints='A' 'B'
                           discrete;
           vbar category / type=cfreq;
           pie category / type=cpercent;
           vbar category / sumvar=count type=sum;
           pie category / sumvar=count type=mean;
         run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    let expected = [
        ignored("VBAR", "TYPE=PERCENT"),
        ignored("VBAR", "SUBGROUP="),
        ignored("VBAR", "GROUP="),
        ignored("VBAR", "MIDPOINTS="),
        ignored("VBAR", "DISCRETE"),
        ignored("VBAR", "TYPE=CFREQ"),
        ignored("PIE", "TYPE=CPERCENT"),
    ];
    for needle in &expected {
        assert_has(&out, needle);
    }
    assert_eq!(
        out.log.matches("WARNING:").count(),
        expected.len(),
        "{}",
        out.log
    );
    assert_has(&out, "NOTE: PROCEDURE GCHART used");
}

/// Base : HBAR était dessiné en barres verticales et VBAR3D/HBAR3D/PIE3D à
/// plat, sans diagnostic. Désormais WARNING (HBAR horizontal : J13-P5).
#[test]
fn ra_j02_p6_gchart_hbar_vertical_warn() {
    let hbar = "WARNING: The HBAR statement is drawn as a vertical bar chart in PROC GCHART; \
                horizontal bars are not supported (planned: roadmap-avancee J13-P5).";
    let out = run_sas(&format!("{CATS}proc gchart data=cats; hbar category; run;"));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert_has(&out, hbar);

    let out = run_sas(&format!(
        "{CATS}proc gchart data=cats; vbar3d category; hbar3d category; pie3d category; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for (stmt, flat) in [("VBAR3D", "VBAR"), ("HBAR3D", "HBAR"), ("PIE3D", "PIE")] {
        assert_has(
            &out,
            &format!(
                "WARNING: The {stmt} statement is drawn as a two-dimensional {flat} chart in \
                 PROC GCHART; display customization is not supported."
            ),
        );
    }
    assert_has(
        &out,
        "WARNING: The HBAR3D statement is drawn as a vertical bar chart in PROC GCHART; \
         horizontal bars are not supported (planned: roadmap-avancee J13-P5).",
    );
    assert_eq!(out.log.matches("WARNING:").count(), 4, "{}", out.log);
}

/// Base : BY était rejeté par le message partagé sans nommer l'unité qui le
/// lèvera ; DONUT, STAR, BLOCK et les instructions globales AXIS, LEGEND,
/// PATTERN, GOPTIONS, NOTE recevaient « 180-322 … not valid » ; les options
/// du statement PROC étaient sautées jeton par jeton ; le build par défaut
/// n'ouvrait jamais DATA=. Désormais : BY → ERROR nommant J13-P4 ; DONUT/
/// STAR/BLOCK → message du catalogue ; décorations et ANNOTATE= → WARNING ;
/// GOUT=/IMAGEMAP= → ERROR ; option inconnue → « Unexpected option » ; table
/// et variables validées dans les deux builds.
#[test]
fn ra_j02_p6_gchart_statements_proc_options_and_data() {
    let out = run_sas(&format!(
        "{CATS}proc gchart data=cats; by category; vbar category; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: The BY statement is not supported in PROC GCHART; it can affect results and \
         cannot be ignored (planned: roadmap-avancee J13-P4).",
    );
    for stmt in ["donut category", "star category", "block category"] {
        let out = run_sas(&format!("{CATS}proc gchart data=cats; {stmt}; run;"));
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert_has(
            &out,
            &format!(
                "ERROR: The {kw} statement is not supported in PROC GCHART; it can affect \
                 results and cannot be ignored."
            ),
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    for stmt in [
        "axis1 label=none",
        "legend1 frame",
        "pattern1 v=solid",
        "goptions reset=all",
        "note 'x'",
    ] {
        let out = run_sas(&format!(
            "{CATS}proc gchart data=cats; {stmt}; vbar category; run;"
        ));
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        assert_eq!(out.exit_code, 1, "{stmt}: {}", out.log);
        assert_has(
            &out,
            &format!(
                "WARNING: The {kw} statement is ignored in PROC GCHART; display customization \
                 is not supported."
            ),
        );
    }

    let out = run_sas(&format!(
        "{CATS}proc gchart data=cats anno=work.cats; vbar category; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert_has(
        &out,
        "WARNING: The ANNO= option is ignored in PROC GCHART; display customization is not \
         supported.",
    );
    let out = run_sas(&format!(
        "{CATS}proc gchart data=cats imagemap=map; vbar category; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: The IMAGEMAP= option is not supported in PROC GCHART; it can affect results and \
         cannot be ignored.",
    );
    let out = run_sas(&format!(
        "{CATS}proc gchart data=cats foo; vbar category; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: Unexpected option 'FOO' on PROC GCHART statement.",
    );

    for ods in ["ods graphics on;", "ods graphics off;"] {
        let out = run_sas(&format!(
            "{CATS}{ods} proc gchart data=cats; vbar nope / sumvar=nope2; run;"
        ));
        assert_eq!(out.exit_code, 2, "{ods}: {}", out.log);
        assert_has(&out, "ERROR: Variable NOPE not found.");
        assert_has(&out, "ERROR: Variable NOPE2 not found.");
        assert!(!out.log.contains("image deferred"), "{}", out.log);

        let out = run_sas(&format!("{ods} proc gchart data=work.nope; vbar a; run;"));
        assert_eq!(out.exit_code, 2, "{ods}: {}", out.log);
        assert!(out.log.contains("ERROR: file error"), "{}", out.log);
        assert!(!out.log.contains("image deferred"), "{}", out.log);
    }
}
