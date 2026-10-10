// ── J02-P6 : contrat PROC GPLOT (replis silencieux supprimés) ─────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic (texte, code de sortie). Oracle de sévérité : CONTRIBUTING §5
// (option d'affichage non rendue → WARNING, code 1 ; demande qui change les
// objets produits → ERROR, étape rejetée, code 2). Syntaxe de référence :
// SAS/GRAPH 9.4 Reference, The GPLOT Procedure, et les instructions globales
// SYMBOL / AXIS / LEGEND.
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

/// WORK.XY: numeric group Z with the levels 2 and 10, character group G.
const XY: &str = "data xy; input x y z g $; datalines;
1 2 2 a
2 4 10 b
3 3 2 a
4 5 10 b
;
run;
";

fn ignored(stmt: &str, option: &str) -> String {
    format!(
        "WARNING: The {option} option of the {stmt} statement is ignored in PROC GPLOT; \
         display customization is not supported."
    )
}

fn assert_has(out: &crate::RunOutcome, needle: &str) {
    assert!(out.log.contains(needle), "missing «{needle}»\n{}", out.log);
}

/// Base : les options après le `/` de PLOT (OVERLAY, HAXIS=, VAXIS=,
/// LEGEND=, HREF=, VREF=…) étaient sautées jusqu'au `;` sans diagnostic.
/// Désormais un WARNING par option (aucune n'est rendue), l'étape s'exécute.
/// Une deuxième requête de tracé (`plot y*x z*x;`, base : « 180-322:
/// Statement 'Z' is not valid ») porte le message du contrat.
#[test]
fn ra_j02_p6_gplot_plot_options_warn() {
    let out = run_sas(&format!(
        "{XY}proc gplot data=xy;
           plot y*x / overlay haxis=axis1 vaxis=axis2 legend=legend1 href=2 3
                      vref=0 to 10 by 5 frame;
         run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    let axis = |name: &str| {
        format!(
            "WARNING: The {name}= option of the PLOT statement is ignored in PROC GPLOT: the \
             first AXIS statement of the step is applied to the horizontal axis and the second \
             to the vertical axis."
        )
    };
    let expected = [
        ignored("PLOT", "OVERLAY"),
        axis("HAXIS"),
        axis("VAXIS"),
        ignored("PLOT", "LEGEND="),
        ignored("PLOT", "HREF="),
        ignored("PLOT", "VREF="),
        ignored("PLOT", "FRAME"),
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
    assert_has(&out, "NOTE: PROCEDURE GPLOT used");

    let out = run_sas(&format!("{XY}proc gplot data=xy; plot y*x z*x; run;"));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: A PLOT statement with more than one plot request is not supported in PROC \
         GPLOT; it can affect results and cannot be ignored (planned: roadmap-avancee J14-P3).",
    );
    assert!(!out.log.contains("180-322"), "{}", out.log);
}

/// Base : les sous-options SYMBOL/AXIS que le moteur ne dessine pas étaient
/// abandonnées sans diagnostic — SYMBOL : HEIGHT=, WIDTH=, LINE=, une
/// interpolation autre que JOIN/NONE, le symbole VALUE= (marqueur par défaut
/// toujours dessiné), une couleur hors des cinq connues (palette) ; AXIS :
/// ORDER= réduit à ses deux premiers nombres, attributs de LABEL=, LABEL=NONE
/// (texte « none » dessiné), MAJOR=, MINOR=… Désormais un WARNING chacune ;
/// INTERPOL=JOIN, les couleurs connues et le texte de LABEL= restent muets.
#[test]
fn ra_j02_p6_gplot_symbol_axis_suboptions_warn() {
    let out = run_sas(&format!(
        "{XY}proc gplot data=xy;
           symbol1 i=join c=red h=2 w=3 l=2;
           symbol2 interpol=spline value=dot color=purple;
           axis1 order=(0 to 10 by 2) label=(a=90 'X axis') major=(n=5);
           axis2 label=none minor=none;
           plot y*x;
         run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    let expected = [
        ignored("SYMBOL1", "H="),
        ignored("SYMBOL1", "W="),
        ignored("SYMBOL1", "L="),
        ignored("SYMBOL2", "INTERPOL=SPLINE"),
        "WARNING: The VALUE=DOT option of the SYMBOL2 statement is not honored in PROC GPLOT: \
         markers use the default symbol."
            .to_string(),
        ignored("SYMBOL2", "COLOR=PURPLE"),
        "WARNING: The ORDER= option of the AXIS1 statement is only partly honored in PROC \
         GPLOT: its first two numbers (0 and 10) set the axis range; the other values and the \
         tick marks are ignored."
            .to_string(),
        "WARNING: The LABEL= attributes of the AXIS1 statement are ignored in PROC GPLOT; only \
         the first label text is drawn."
            .to_string(),
        ignored("AXIS1", "MAJOR="),
        "WARNING: The LABEL=NONE option of the AXIS2 statement is not honored in PROC GPLOT: \
         the text NONE is drawn as the axis label."
            .to_string(),
        ignored("AXIS2", "MINOR="),
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

    // What the engine draws: no diagnostic.
    let out = run_sas(&format!(
        "{XY}proc gplot data=xy;
           symbol1 i=join c=blue; symbol2 i=none c=black; axis1 label=('Time');
           plot y*x;
         run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(!out.log.contains("WARNING"), "{}", out.log);
}

/// Base : `plot y*x=z` — le moteur range les séries des niveaux de `z` par
/// leur TEXTE (clés `format!("{n}")` d'une BTreeMap) : pour un `z` numérique
/// de niveaux 2 et 10, la série de 10 est dessinée d'abord et reçoit SYMBOL1,
/// alors que SAS range les niveaux par valeur. La correction (tri par valeur)
/// vit dans le code de rendu, sous `cfg(feature = "graphics")`, hors du
/// périmètre de J02-P6 (rendu réservé à J13–J14) : la couche execute, commune
/// aux deux builds, émet donc un WARNING quand les deux ordres diffèrent
/// (rien pour des niveaux à un chiffre ni pour une variable caractère).
#[test]
fn ra_j02_p6_gplot_numeric_group_text_order() {
    let out = run_sas(&format!(
        "{XY}ods graphics on; proc gplot data=xy; plot y*x=z; run; ods graphics off;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert_has(
        &out,
        "WARNING: The levels of the numeric group variable Z are ordered as text, not by \
         value, in PROC GPLOT (10 is drawn before 2); SYMBOL definitions and colors follow \
         that order (planned: roadmap-avancee J13-P2).",
    );

    for (data, group) in [
        (XY.to_string(), "g"),
        (
            "data xy; input x y z; datalines;\n1 2 3\n2 4 1\n3 3 2\n;\nrun;\n".to_string(),
            "z",
        ),
    ] {
        let out = run_sas(&format!(
            "{data}ods graphics on; proc gplot data=xy; plot y*x={group}; run; \
             ods graphics off;"
        ));
        assert_eq!(out.exit_code, 0, "{group}: {}", out.log);
        assert!(!out.log.contains("WARNING"), "{group}: {}", out.log);
    }
}

/// Base : BY était déjà rejeté par le message partagé, sans nommer l'unité
/// qui le lèvera ; BUBBLE/BUBBLE2 et les instructions globales LEGEND,
/// PATTERN, GOPTIONS, NOTE recevaient « 180-322 … not valid ». Désormais : BY
/// → ERROR nommant J13-P4 (même texte que SGPLOT, GCHART, PLOT) ; BUBBLE →
/// message du catalogue du contrat ; décorations → WARNING d'affichage.
#[test]
fn ra_j02_p6_gplot_by_and_unsupported_statements() {
    let out = run_sas(&format!(
        "{XY}proc gplot data=xy; by g; plot y*x; run;
         data _null_; put 'NEXT_STEP'; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: The BY statement is not supported in PROC GPLOT; it can affect results and \
         cannot be ignored (planned: roadmap-avancee J13-P4).",
    );
    assert_has(&out, "NEXT_STEP");

    for stmt in ["bubble y*x=z", "bubble2 y*x=z"] {
        let out = run_sas(&format!("{XY}proc gplot data=xy; {stmt}; run;"));
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert_has(
            &out,
            &format!(
                "ERROR: The {kw} statement is not supported in PROC GPLOT; it can affect \
                 results and cannot be ignored."
            ),
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    for stmt in [
        "legend1 label=none",
        "pattern1 v=solid",
        "goptions reset=all",
        "note 'n=4'",
    ] {
        let out = run_sas(&format!("{XY}proc gplot data=xy; {stmt}; plot y*x; run;"));
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        assert_eq!(out.exit_code, 1, "{stmt}: {}", out.log);
        assert_has(
            &out,
            &format!(
                "WARNING: The {kw} statement is ignored in PROC GPLOT; display customization \
                 is not supported."
            ),
        );
    }
}

/// Base : les options du statement PROC GPLOT autres que DATA= étaient
/// sautées jeton par jeton (GOUT=, IMAGEMAP= : aucun catalogue ni table
/// créés) et le build par défaut n'ouvrait jamais DATA= (table ou variable
/// absente : code 0). Désormais : UNIFORM, ANNOTATE= → WARNING ; GOUT=,
/// IMAGEMAP= → ERROR ; option inconnue → « Unexpected option » ; table et
/// variables validées dans les deux builds (ERROR).
#[test]
fn ra_j02_p6_gplot_proc_options_and_data() {
    let out = run_sas(&format!(
        "{XY}proc gplot data=xy uniform anno=work.xy; plot y*x; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert_has(
        &out,
        "WARNING: The UNIFORM option is ignored in PROC GPLOT; display customization is not \
         supported.",
    );
    assert_has(
        &out,
        "WARNING: The ANNO= option is ignored in PROC GPLOT; display customization is not \
         supported.",
    );
    for (option, label) in [("gout=mycat", "GOUT="), ("imagemap=map", "IMAGEMAP=")] {
        let out = run_sas(&format!("{XY}proc gplot data=xy {option}; plot y*x; run;"));
        assert_eq!(out.exit_code, 2, "{option}: {}", out.log);
        assert_has(
            &out,
            &format!(
                "ERROR: The {label} option is not supported in PROC GPLOT; it can affect \
                 results and cannot be ignored."
            ),
        );
    }
    let out = run_sas(&format!("{XY}proc gplot data=xy foo; plot y*x; run;"));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: Unexpected option 'FOO' on PROC GPLOT statement.",
    );

    for ods in ["ods graphics on;", "ods graphics off;"] {
        let out = run_sas(&format!(
            "{XY}{ods} proc gplot data=xy; plot nope*x=nope2; run;"
        ));
        assert_eq!(out.exit_code, 2, "{ods}: {}", out.log);
        assert_has(&out, "ERROR: Variable NOPE not found.");
        assert_has(&out, "ERROR: Variable NOPE2 not found.");
        assert!(!out.log.contains("image deferred"), "{}", out.log);

        let out = run_sas(&format!("{ods} proc gplot data=work.nope; plot y*x; run;"));
        assert_eq!(out.exit_code, 2, "{ods}: {}", out.log);
        assert!(out.log.contains("ERROR: file error"), "{}", out.log);
        assert!(!out.log.contains("image deferred"), "{}", out.log);
    }
}
