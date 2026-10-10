// ── J02-P7 : contrat PROC TABULATE (replis silencieux supprimés) ─────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, code de sortie). Oracle de sévérité :
// CONTRIBUTING §5 ; syntaxe et sémantique de référence : Base SAS 9.4
// Procedures Guide, Seventh Edition, The TABULATE Procedure —
// TABLE statement https://documentation.sas.com/doc/en/proc/9.4/p1g617vn5t3p39n0z9o601rw74jz.htm
// CLASS statement https://documentation.sas.com/doc/en/proc/9.4/n12teq2mctxmbjn1hu0ykvqwgpvp.htm

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

/// `g` is blank on the 4th observation, `h` missing on the 2nd.
const MISS: &str = "data m; input g $ h x; if g = 'z' then g = ' '; datalines;
a 1 10
a . 20
b 2 30
z 1 40
;
run;
";

/// Contract ERROR suffix shared by the rejected constructions.
const CANNOT: &str =
    "is not supported in PROC TABULATE; it can affect results and cannot be ignored";

/// Asserts a rejected step: exit code 2, `needle` in the log, no listing at
/// all (neither the procedure heading nor the title).
fn assert_rejected(out: &crate::RunOutcome, needle: &str, ctx: &str) {
    assert_eq!(out.exit_code, 2, "{ctx}: {}", out.log);
    assert!(
        out.log.contains(needle),
        "{ctx}: missing «{needle}»\n{}",
        out.log
    );
    assert!(
        !out.listing.contains("The TABULATE Procedure"),
        "{ctx}: the step must be rejected before any output\n{}",
        out.listing
    );
}

/// Whitespace-split listing lines whose first token is `stub` (or, for an
/// empty stub, whose tokens are all numbers).
fn table_row(listing: &str, stub: &str) -> Vec<String> {
    listing
        .lines()
        .map(|l| l.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .find(|t| {
            if stub.is_empty() {
                !t.is_empty() && t.iter().all(|x| x.parse::<f64>().is_ok())
            } else {
                t.first().is_some_and(|f| f == stub)
            }
        })
        .unwrap_or_default()
}

/// Base : plusieurs instructions TABLE → seule la dernière table était
/// produite, code 0. SAS 9.4 TABLE statement : « To create several tables use
/// multiple TABLE statements. » → ERROR jusqu'à J03-P2 ; l'étape suivante
/// s'exécute.
#[test]
fn ra_j02_p7_tabulate_multiple_table_statements() {
    let out = run_sas(&format!(
        "{CLS}proc tabulate data=cls; class sex; var height;
           table sex, height*mean;
           table sex;
         run;"
    ));
    assert_rejected(
        &out,
        &format!(
            "ERROR: More than one TABLE statement {CANNOT} (planned: roadmap-avancee J03-P2)."
        ),
        "two TABLE statements",
    );

    let out = run_sas(&format!(
        "{CLS}proc tabulate data=cls; class sex; table sex; table sex; run;
         proc tabulate data=cls; class sex; table sex; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_eq!(
        out.listing.matches("The TABULATE Procedure").count(),
        1,
        "only the next step runs\n{}",
        out.listing
    );
}

/// Base : les niveaux CLASS étaient les valeurs brutes (12, 13, 14) malgré le
/// format stocké. Correction directe — SAS 9.4 CLASS statement : GROUPINTERNAL
/// « specifies not to apply formats to the class variables when PROC TABULATE
/// groups the values » (par défaut, regroupement sur la valeur formatée) ;
/// ORDER=UNFORMATTED (défaut) « orders values by their unformatted values,
/// which yields the same order as PROC SORT ». Young (11-12) précède donc
/// Adult (13+) bien que « Adult » < « Young ».
#[test]
fn ra_j02_p7_tabulate_class_formatted_levels() {
    let out = run_sas(&format!(
        "proc format;
           value agegrp low-12='Young' 13-high='Adult';
           value $anysex 'F', 'M' = 'Any';
         run;
         {CLS}data cls2; set cls; format age agegrp. sex $anysex.; run;
         proc tabulate data=cls2; class age; var height; table age, height*(n mean); run;
         proc tabulate data=cls2; class sex; table sex, n; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    let young = table_row(&out.listing, "Young");
    let adult = table_row(&out.listing, "Adult");
    // Young: 12-year-olds 57.3 and 59.8 ; Adult: 69.0 56.5 65.3 62.8 63.5.
    assert_eq!(young, ["Young", "2", "58.55"], "{}", out.listing);
    assert_eq!(adult, ["Adult", "5", "63.42"], "{}", out.listing);
    assert!(
        out.listing.find("Young").unwrap() < out.listing.find("Adult").unwrap(),
        "levels in unformatted order\n{}",
        out.listing
    );
    assert!(
        !out.listing
            .lines()
            .any(|l| l.trim_start().starts_with("13")),
        "no raw level left\n{}",
        out.listing
    );
    // Character format: F and M collapse into one formatted level.
    assert_eq!(
        table_row(&out.listing, "Any"),
        ["Any", "7"],
        "{}",
        out.listing
    );
}

/// Base : une observation dont une variable CLASS est manquante était
/// comptée dans les cellules qui ne contraignent pas cette variable, dans ALL
/// et dans les dénominateurs de PCTN (a : N=2, ALL : N=4, PctN 50/25) ;
/// l'option PROC MISSING était refusée (« Unexpected option »). SAS 9.4 CLASS
/// statement, MISSING : « If you omit the MISSING option, then PROC TABULATE
/// excludes the observations with any missing CLASS variable values » — même
/// quand la variable n'apparaît pas dans la table (h ici) ; avec MISSING, la
/// valeur manquante est un niveau.
#[test]
fn ra_j02_p7_tabulate_missing_class_values() {
    let table = "class g h; var x; table g all, x*(n sum) pctn;";
    let out = run_sas(&format!("{MISS}proc tabulate data=m; {table} run;"));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert_eq!(table_row(&out.listing, "a"), ["a", "1", "10", "50"]);
    assert_eq!(table_row(&out.listing, "b"), ["b", "1", "30", "50"]);
    assert_eq!(
        table_row(&out.listing, "All"),
        ["All", "2", "40", "100"],
        "{}",
        out.listing
    );

    let out = run_sas(&format!("{MISS}proc tabulate data=m missing; {table} run;"));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    // Blank level of g (4th observation), then a (both observations), b.
    assert_eq!(
        table_row(&out.listing, ""),
        ["1", "40", "25"],
        "{}",
        out.listing
    );
    assert_eq!(table_row(&out.listing, "a"), ["a", "2", "30", "50"]);
    assert_eq!(table_row(&out.listing, "b"), ["b", "1", "30", "25"]);
    assert_eq!(table_row(&out.listing, "All"), ["All", "4", "100", "100"]);

    let out = run_sas(&format!(
        "{MISS}proc tabulate data=m missing; class h; table h, n; run;"
    ));
    assert_eq!(table_row(&out.listing, "."), [".", "1"], "{}", out.listing);
    assert_eq!(table_row(&out.listing, "1"), ["1", "2"]);
}

/// Base : une statistique non supportée (COLPCTN, MEDIAN, STDDEV…) ou un
/// croisement refusé n'était détecté qu'à l'exécution, après l'écriture du
/// titre et de l'en-tête « The TABULATE Procedure » (sortie orpheline).
/// Désormais : ERROR au parsing (statistique) ou avant toute sortie
/// (croisement), listing vide.
#[test]
fn ra_j02_p7_tabulate_errors_before_any_output() {
    for (table, stat) in [
        ("sex, colpctn", "COLPCTN"),
        ("sex, height*median", "MEDIAN"),
        ("sex, stddev*height", "STDDEV"),
        ("sex*rowpctn", "ROWPCTN"),
    ] {
        let out = run_sas(&format!(
            "{CLS}title 'Orphan title'; proc tabulate data=cls; class sex; var height;
               table {table}; run;"
        ));
        assert_rejected(
            &out,
            &format!("ERROR: The {stat} statistic {CANNOT}."),
            table,
        );
        assert!(!out.listing.contains("Orphan title"), "{}", out.listing);
    }

    // Two analysis variables crossed: rejected while computing the cells,
    // before the first listing line.
    let out = run_sas(&format!(
        "{CLS}title 'Orphan title'; proc tabulate data=cls; class sex; var height age;
           table sex, height*age; run;"
    ));
    assert_rejected(
        &out,
        "ERROR: PROC TABULATE: crossing two analysis variables not yet supported",
        "height*age",
    );
    assert!(out.listing.trim().is_empty(), "{}", out.listing);
}

/// Base : OUT= écrivait une observation par cellule et par statistique, sans
/// le dire. Approximation documentée (docs/support-contract.md) jusqu'à
/// J03-P2 : NOTE à chaque exécution ; la forme reste figée (2 niveaux × 2
/// statistiques = 4 observations, là où SAS en écrit 2).
#[test]
fn ra_j02_p7_tabulate_out_approximation_note() {
    let out = run_sas(&format!(
        "{CLS}proc tabulate data=cls out=work.o; class sex; var height;
           table sex, height*(mean n); run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.log.contains(
            "NOTE: PROC TABULATE approximates the SAS OUT= data set: it writes one observation \
             per table cell and statistic, where SAS writes one observation per combination of \
             class variable values with all of its statistics (planned: roadmap-avancee J03-P2)."
        ),
        "{}",
        out.log
    );
    assert!(
        out.log
            .contains("NOTE: The data set WORK.O has 4 observations and 6 variables."),
        "{}",
        out.log
    );

    let out = run_sas(&format!(
        "{CLS}proc tabulate data=cls; class sex; table sex; run;"
    ));
    assert!(!out.log.contains("approximates"), "{}", out.log);
}

/// Grammaire vérifiée sur la doc SAS 9.4 (TABLE statement) : les seuls
/// opérateurs d'une expression de dimension sont le blanc (concaténation),
/// `*`, les parenthèses (« group elements and associate an operator with each
/// concatenated element ») et les chevrons (« specify denominator
/// definitions »). `pctn(age)` est donc PCTN suivi du groupe (age) — même
/// table que `pctn age`, comme le faisait déjà sasrs. Base : `pctn<age>`
/// donnait une erreur de syntaxe générique ; désormais ERROR du contrat
/// (dénominateurs : J03-P2).
#[test]
fn ra_j02_p7_tabulate_pctn_parentheses_concatenate() {
    let run_table = |table: &str| {
        run_sas(&format!(
            "{CLS}proc tabulate data=cls; class sex age; var height; table {table}; run;"
        ))
    };
    let paren = run_table("sex, pctn(age)");
    let blank = run_table("sex, pctn age");
    assert_eq!(paren.exit_code, 0, "{}", paren.log);
    assert_eq!(paren.listing, blank.listing);
    assert!(paren.listing.contains("PctN"), "{}", paren.listing);

    for (table, kw) in [
        ("sex, pctn<age>", "PCTN"),
        ("sex, height*pctsum<sex>", "PCTSUM"),
    ] {
        assert_rejected(
            &run_table(table),
            &format!(
                "ERROR: A denominator definition ({kw}<...>) {CANNOT} (planned: roadmap-avancee \
                 J03-P2)."
            ),
            table,
        );
    }
}

/// Base : KEYLABEL, CLASSLEV et KEYWORD (instructions SAS valides, en-têtes
/// seulement) donnaient « 180-322 … not valid ». Désormais WARNING
/// d'affichage, la table est produite ; FREQ et WEIGHT gardent l'ERROR
/// « not supported » du catalogue ; une instruction inventée reste 180-322.
#[test]
fn ra_j02_p7_tabulate_valid_statements() {
    let out = run_sas(&format!(
        "{CLS}proc tabulate data=cls; class sex;
           keylabel n='Count';
           classlev sex / style=[font_weight=bold];
           keyword n / style=[background=yellow];
           table sex;
         run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for stmt in ["KEYLABEL", "CLASSLEV", "KEYWORD"] {
        let needle = format!(
            "WARNING: The {stmt} statement is ignored in PROC TABULATE; display customization is \
             not supported."
        );
        assert!(out.log.contains(&needle), "{needle}\n{}", out.log);
    }
    assert!(!out.log.contains("180-322"), "{}", out.log);
    assert!(
        out.listing.contains("The TABULATE Procedure"),
        "{}",
        out.listing
    );

    for stmt in ["freq age", "weight age"] {
        let out = run_sas(&format!(
            "{CLS}proc tabulate data=cls; class sex; {stmt}; table sex; run;"
        ));
        let kw = stmt.split(' ').next().unwrap().to_ascii_uppercase();
        assert_rejected(&out, &format!("ERROR: The {kw} statement {CANNOT}."), stmt);
    }

    let out = run_sas(&format!(
        "{CLS}proc tabulate data=cls; class sex; invented x; table sex; run;"
    ));
    assert_rejected(
        &out,
        "180-322: Statement 'INVENTED' is not valid or it is used out of proper order in PROC \
         TABULATE.",
        "invented",
    );
}
