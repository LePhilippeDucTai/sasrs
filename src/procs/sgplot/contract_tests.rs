// ── J02-P6 : contrat PROC SGPLOT (replis silencieux supprimés) ────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic (texte, code de sortie). Oracle de sévérité : CONTRIBUTING §5
// (option d'affichage non rendue → WARNING, code 1 ; demande qui change les
// objets produits → ERROR, étape rejetée, code 2). Syntaxe de référence :
// SAS 9.4 ODS Graphics: Procedures Guide, The SGPLOT Procedure
// (https://support.sas.com/documentation/cdl/en/grstatproc/65235/HTML/default/n0yjdd910dh59zn1toodgupaj4v9.htm).
//
// Les programmes tournent à l'identique dans les deux builds : les
// diagnostics sont émis par les couches communes (parsing, execute) ; sous
// `--features graphics`, les images vont dans un répertoire temporaire.

use crate::source::SourceFile;

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

/// Data set WORK.H shared by the tests.
const HEIGHTS: &str = "data h; input age height sex $; datalines;
10 140 M
12 150 F
14 158 M
;
run;
";

/// Display WARNING of an option of a plot or axis statement.
fn ignored(stmt: &str, option: &str) -> String {
    format!(
        "WARNING: The {option} option of the {stmt} statement is ignored in PROC SGPLOT; \
         display customization is not supported."
    )
}

fn assert_has(out: &crate::RunOutcome, needle: &str) {
    assert!(out.log.contains(needle), "missing «{needle}»\n{}", out.log);
}

/// Base : GROUP=, RESPONSE=, STAT=, SCALE=, FILL/NOFILL, LEGENDLABEL=,
/// MARKERATTRS=, LINEATTRS= et les autres options de tracé étaient lus (ou
/// sautés) sans être rendus, sans diagnostic ; STAT= inconnu (PERCENT,
/// MEDIAN, faute de frappe) retombait en silence sur FREQ. Désormais un
/// WARNING par option (code 1), l'étape s'exécute ; les options rendues
/// (STAT=FREQ, SCALE=COUNT, BINWIDTH=, SMOOTH=, DEGREE=, TYPE=, CATEGORY=)
/// restent muettes.
#[test]
fn ra_j02_p6_sgplot_plot_options_warn() {
    let out = run_sas(&format!(
        "{HEIGHTS}proc sgplot data=h;
           scatter x=age y=height / group=sex markerattrs=(symbol=circle) legendlabel='H'
                                    transparency=0.5;
           series x=age y=height / lineattrs=(pattern=dash);
           vbar sex / response=height stat=sum fill;
           hbar sex / stat=percent nofill;
           vbar sex / stat=bogus;
           histogram height / scale=percent binwidth=5;
           loess x=age y=height / smooth=0.4;
           reg x=age y=height / degree=2;
           density height / type=kernel;
           vbox height / category=sex;
         run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    let expected = [
        ignored("SCATTER", "GROUP="),
        ignored("SCATTER", "MARKERATTRS="),
        ignored("SCATTER", "LEGENDLABEL="),
        ignored("SCATTER", "TRANSPARENCY="),
        ignored("SERIES", "LINEATTRS="),
        ignored("VBAR", "RESPONSE="),
        ignored("VBAR", "STAT=SUM"),
        ignored("VBAR", "FILL"),
        ignored("HBAR", "STAT=PERCENT"),
        ignored("HBAR", "NOFILL"),
        ignored("VBAR", "STAT=BOGUS"),
        ignored("HISTOGRAM", "SCALE=PERCENT"),
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
    // The step still runs (ODS GRAPHICS is off: non-activation NOTE).
    assert_has(&out, "NOTE: ODS GRAPHICS is not enabled.");
    assert_has(&out, "NOTE: PROCEDURE SGPLOT used");

    // Options that the engine renders: no diagnostic.
    let out = run_sas(&format!(
        "{HEIGHTS}proc sgplot data=h;
           vbar sex / stat=freq;
           histogram height / scale=count binwidth=10;
           loess x=age y=height / smooth=0.5;
           density height / type=normal;
         run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(!out.log.contains("WARNING"), "{}", out.log);
}

/// Base : XAXIS/YAXIS VALUES= ne gardait que ses deux premiers nombres comme
/// bornes de l'axe (`VALUES=(10 20 30 40)` → axe 10..20, graduations perdues,
/// signe de `-10` perdu) ; TYPE= était lu sans être rendu et les autres
/// options d'axe (GRID, MIN=…) sautées, sans diagnostic. Désormais WARNING
/// (LABEL= reste honoré, sans diagnostic).
#[test]
fn ra_j02_p6_sgplot_axis_options_warn() {
    let out = run_sas(&format!(
        "{HEIGHTS}proc sgplot data=h;
           scatter x=age y=height;
           xaxis label='Age' values=(10 20 30 40) grid;
           yaxis type=log min=100 values=(-10 to 200 by 10) tickvalueformat=$char10.;
         run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for needle in [
        "WARNING: The VALUES= option of the XAXIS statement is only partly honored in PROC \
         SGPLOT: its first two numbers (10 and 20) set the axis range; the other values and \
         the tick marks are ignored."
            .to_string(),
        ignored("XAXIS", "GRID"),
        ignored("YAXIS", "TYPE=LOG"),
        ignored("YAXIS", "MIN="),
        "WARNING: The VALUES= option of the YAXIS statement is only partly honored in PROC \
         SGPLOT: its first two numbers (-10 and 200) set the axis range; the other values and \
         the tick marks are ignored."
            .to_string(),
    ] {
        assert_has(&out, &needle);
    }
    assert_has(&out, &ignored("YAXIS", "TICKVALUEFORMAT="));
    assert_eq!(out.log.matches("WARNING:").count(), 6, "{}", out.log);
    assert!(!out.log.contains("The LABEL= option"), "{}", out.log);

    // The sign of a negative bound is kept in the AST (it was dropped).
    let src = SourceFile::new(
        "proc sgplot data=h; scatter x=age y=height; yaxis values=(-10 to 200); run;",
    );
    let mut ts = crate::parser::StatementStream::new(&src).unwrap();
    ts.next();
    ts.next();
    let ast = super::parse(&mut ts).unwrap();
    let y = ast.yaxis.unwrap();
    assert_eq!((y.values_min, y.values_max), (Some(-10.0), Some(200.0)));
}

/// Base : sous `--features graphics`, le moteur abandonnait en silence les
/// tracés qu'il ne sait pas superposer au tracé principal (REG, VBOX, HBAR,
/// mais aussi un VBAR ou un HISTOGRAM secondaire) ; le build par défaut
/// annonçait « renders only the first plot statement », faux pour les
/// superpositions LOESS/DENSITY/SERIES/SCATTER que le moteur dessine. Le
/// diagnostic vient désormais de la couche execute, identique dans les deux
/// builds : un WARNING par tracé abandonné ; un REG/VBOX/HBAR principal donne
/// la NOTE « deferred » du moteur au lieu d'une image annoncée.
#[test]
fn ra_j02_p6_sgplot_dropped_statements_warn() {
    let out = run_sas(&format!(
        "{HEIGHTS}ods graphics on;
         proc sgplot data=h;
           scatter x=age y=height;
           reg x=age y=height;
           loess x=age y=height;
           vbox height / category=sex;
           hbar sex;
           histogram height;
         run;
         ods graphics off;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for kind in ["REG", "VBOX", "HBAR"] {
        assert_has(
            &out,
            &format!(
                "WARNING: The {kind} statement is ignored in PROC SGPLOT: it is not drawn over \
                 the SCATTER plot (planned: roadmap-avancee J13-P3)."
            ),
        );
    }
    assert_has(
        &out,
        "WARNING: The HISTOGRAM statement is ignored in PROC SGPLOT: it is not drawn over the \
         SCATTER plot.",
    );
    // LOESS is drawn over the scatter plot: no diagnostic.
    assert!(!out.log.contains("The LOESS statement"), "{}", out.log);
    assert_eq!(out.log.matches("WARNING:").count(), 4, "{}", out.log);
    assert!(!out.log.contains("renders only the first"), "{}", out.log);

    // REG as the main plot: no image, in both builds (the default build used
    // to announce a deferred image).
    let out = run_sas(&format!(
        "{HEIGHTS}ods graphics on; proc sgplot data=h; reg x=age y=height; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_has(
        &out,
        "NOTE: REG plot deferred (not yet rendered in PROC SGPLOT).",
    );
    assert!(!out.log.contains("image deferred"), "{}", out.log);
}

/// Base : BY était noté (« BY-group processing deferred in PROC SGPLOT »),
/// aucune image, code 0. Désormais l'ERROR du contrat, comme GPLOT, GCHART et
/// PLOT, jusqu'aux images par groupe BY (J13-P4) ; l'étape est rejetée avant
/// exécution et l'étape suivante s'exécute.
#[test]
fn ra_j02_p6_sgplot_by_is_error() {
    let out = run_sas(&format!(
        "{HEIGHTS}ods graphics on;
         proc sgplot data=h; by sex; scatter x=age y=height; run;
         data _null_; put 'NEXT_STEP'; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: The BY statement is not supported in PROC SGPLOT; it can affect results and \
         cannot be ignored (planned: roadmap-avancee J13-P4).",
    );
    assert!(
        !out.log.contains("BY-group processing deferred"),
        "{}",
        out.log
    );
    assert!(!out.log.contains("PROCEDURE SGPLOT used"), "{}", out.log);
    assert_has(&out, "NEXT_STEP");
}

/// Base : les instructions SGPLOT valides non implémentées recevaient
/// « 180-322 … not valid ». Les instructions de tracé portent désormais le
/// message du catalogue du contrat (ERROR, HBOX nomme J13-P3) ; les
/// instructions de décoration (légendes, encarts, lignes de référence,
/// styles, axes secondaires) un WARNING d'affichage.
#[test]
fn ra_j02_p6_sgplot_unsupported_statements() {
    for stmt in [
        "hbox height",
        "band x=age upper=height lower=height",
        "bubble x=age y=height size=height",
        "needle x=age y=height",
        "step x=age y=height",
        "vline sex",
        "heatmap x=age y=height",
        "text x=age y=height text=sex",
    ] {
        let out = run_sas(&format!("{HEIGHTS}proc sgplot data=h; {stmt}; run;"));
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        let unit = if kw == "HBOX" {
            " (planned: roadmap-avancee J13-P3)"
        } else {
            ""
        };
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert_has(
            &out,
            &format!(
                "ERROR: The {kw} statement is not supported in PROC SGPLOT; it can affect \
                 results and cannot be ignored{unit}."
            ),
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    for stmt in [
        "keylegend / title='Sex'",
        "refline 150 / axis=y",
        "inset 'n=3'",
        "styleattrs datacolors=(red blue)",
        "x2axis grid",
    ] {
        let out = run_sas(&format!(
            "{HEIGHTS}proc sgplot data=h; scatter x=age y=height; {stmt}; run;"
        ));
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        assert_eq!(out.exit_code, 1, "{stmt}: {}", out.log);
        assert_has(
            &out,
            &format!(
                "WARNING: The {kw} statement is ignored in PROC SGPLOT; display customization \
                 is not supported."
            ),
        );
    }
}

/// Base : les options du statement PROC SGPLOT autres que DATA= étaient
/// sautées jeton par jeton, inconnues comprises. Désormais : option
/// d'affichage → WARNING ; TMPLOUT= (fichier de sortie) → ERROR ; option
/// inconnue → ERROR « Unexpected option ».
#[test]
fn ra_j02_p6_sgplot_proc_options() {
    let out = run_sas(&format!(
        "{HEIGHTS}proc sgplot data=h noautolegend description='Heights' pad=0.5in;
           scatter x=age y=height / markersize=10px;
         run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    // A dimension keeps its unit (`0.5in`, `10px`): one WARNING per option.
    assert_eq!(out.log.matches("WARNING:").count(), 4, "{}", out.log);
    assert_has(
        &out,
        "WARNING: The PAD= option is ignored in PROC SGPLOT; display customization is not \
         supported.",
    );
    assert_has(&out, &ignored("SCATTER", "MARKERSIZE="));
    assert_has(
        &out,
        "WARNING: The NOAUTOLEGEND option is ignored in PROC SGPLOT; display customization is \
         not supported.",
    );
    assert_has(
        &out,
        "WARNING: The DESCRIPTION= option is ignored in PROC SGPLOT; display customization is \
         not supported.",
    );
    let out = run_sas(&format!(
        "{HEIGHTS}proc sgplot data=h tmplout='t.sas'; scatter x=age y=height; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: The TMPLOUT= option is not supported in PROC SGPLOT; it can affect results and \
         cannot be ignored.",
    );
    let out = run_sas(&format!(
        "{HEIGHTS}proc sgplot data=h foo; scatter x=age y=height; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        "ERROR: Unexpected option 'FOO' on PROC SGPLOT statement.",
    );
}

/// Base : le build par défaut n'ouvrait jamais DATA= — table ou variable
/// absente : « image deferred », code 0 ; ODS GRAPHICS désactivé : NOTE de
/// non-activation, code 0. Désormais la table et chaque variable nommée sont
/// validées dans les deux builds, avant l'état ODS GRAPHICS (ERROR, code 2).
#[test]
fn ra_j02_p6_sgplot_data_validated() {
    for ods in ["ods graphics on;", "ods graphics off;"] {
        let out = run_sas(&format!(
            "{HEIGHTS}{ods} proc sgplot data=h; scatter x=age y=nope; vbar nope2; run;"
        ));
        assert_eq!(out.exit_code, 2, "{ods}: {}", out.log);
        assert_has(&out, "ERROR: Variable NOPE not found.");
        assert_has(&out, "ERROR: Variable NOPE2 not found.");
        assert!(!out.log.contains("image deferred"), "{}", out.log);
        assert!(!out.log.contains("is not enabled"), "{}", out.log);

        let out = run_sas(&format!(
            "{ods} proc sgplot data=work.nope; scatter x=age y=height; run;"
        ));
        assert_eq!(out.exit_code, 2, "{ods}: {}", out.log);
        assert_has(&out, "ERROR:");
        assert!(out.log.contains("ERROR: file error"), "{}", out.log);
        assert!(!out.log.contains("image deferred"), "{}", out.log);
    }
}
