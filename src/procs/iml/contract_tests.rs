// ── J02-P5 : contrat PROC IML (replis silencieux supprimés) ───────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, code de sortie). Oracle de sévérité :
// CONTRIBUTING §5 ; référence SAS : SAS/IML 13.2 User's Guide (SAS 9.4),
// https://support.sas.com/documentation/cdl/en/imlug/67502/HTML/default/viewer.htm

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

/// Contract ERROR suffix shared by every rejected construction.
const CANNOT: &str = "is not supported in PROC IML; it can affect results and cannot be ignored";

/// Data set with a missing value of `x` (second observation).
const MISS: &str = "data miss; input x y; datalines;
1 10
. 20
3 .
;
run;
";

/// Asserts a step rejected at parse time: exit code 2, `needle` in the log,
/// no IML output and no « NOTE: PROCEDURE IML used ».
fn assert_rejected(out: &crate::RunOutcome, needle: &str, ctx: &str) {
    assert_eq!(out.exit_code, 2, "{ctx}: {}", out.log);
    assert!(
        out.log.contains(needle),
        "{ctx}: missing «{needle}»\n{}",
        out.log
    );
    assert!(
        !out.listing.contains("The IML Procedure"),
        "{ctx}: the step must be rejected before execution\n{}",
        out.listing
    );
    assert!(
        !out.log.contains("PROCEDURE IML used"),
        "{ctx}: the step must not run\n{}",
        out.log
    );
}

/// Asserts that no Rust token text (`Ident("x")`, `Semi`, `Eof`…) leaks into
/// the log.
fn assert_no_rust_token(log: &str, ctx: &str) {
    for rust in [
        "Ident(", "Num(", "Str(", "Semi", "Eof", "LBracket", "RBracket", "LParen", "RParen",
        "LBrace", "RBrace", "Comma",
    ] {
        assert!(!log.contains(rust), "{ctx}: Rust token «{rust}»\n{log}");
    }
}

/// Base : les options du statement PROC IML étaient sautées jeton par jeton
/// (`proc iml foo=bar;` s'exécutait sans diagnostic). SAS/IML 9.4, « PROC IML
/// Statement » : `PROC IML <SYMSIZE=n1> <WORKSIZE=n2>;` — seules options,
/// tailles mémoire en kilo-octets ; la mémoire est étendue automatiquement
/// quand elles sont épuisées (« Memory and Workspace »), elles restent donc
/// acceptées sans effet observable. Toute autre option → ERROR, et l'étape
/// suivante s'exécute (le corps IML est consommé avec l'étape rejetée).
/// https://support.sas.com/documentation/cdl/en/imlug/66845/HTML/default/imlug_imlstart_sect011.htm
/// https://support.sas.com/documentation/cdl/en/imlug/66845/HTML/default/imlug_asstop_sect001.htm
#[test]
fn ra_j02_p5_proc_options() {
    let out = run_sas(
        "proc iml symsize=100 foo=bar;
           x = {1 2};
           print x;
         quit;
         proc iml;
           y = {7};
           print y;
         quit;",
    );
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert!(
        out.log
            .contains("ERROR: Unexpected option 'FOO' on PROC IML statement."),
        "{}",
        out.log
    );
    assert!(!out.listing.contains("\nX\n"), "{}", out.listing);
    // The following PROC IML step is not swallowed by the rejected one.
    assert!(out.listing.contains("\nY\n\n     7\n"), "{}", out.listing);

    let out = run_sas(
        "proc iml symsize=abc;
           x = {1};
         quit;",
    );
    assert_rejected(
        &out,
        "ERROR: expected a number of kilobytes after SYMSIZE=",
        "symsize=abc",
    );

    let out = run_sas(
        "proc iml worksize=8000 symsize=200;
           x = {1 2};
           print x;
         quit;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("\nX\n"), "{}", out.listing);
}

/// Base : les options PRINT `[…]` (COLNAME=, ROWNAME=, FORMAT=, LABEL=)
/// étaient lues puis abandonnées sans diagnostic. Elles ne changent que
/// l'affichage (SAS/IML 9.4, PRINT Statement ; abréviations C= R= F= L=) :
/// WARNING par option, listing inchangé, code 1. Contenu invalide entre
/// crochets → ERROR.
#[test]
fn ra_j02_p5_print_options_warning() {
    let plain = run_sas(
        "proc iml;
           x = {1 2, 3 4};
           print x;
         quit;",
    );
    assert_eq!(plain.exit_code, 0, "{}", plain.log);
    let out = run_sas(
        "proc iml;
           x = {1 2, 3 4};
           print x[label=\"My X\" colname={\"a\" \"b\"} rowname={\"r1\" \"r2\"} format=8.2];
         quit;",
    );
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for opt in ["LABEL", "COLNAME", "ROWNAME", "FORMAT"] {
        assert!(
            out.log.contains(&format!(
                "WARNING: The {opt}= option of the PRINT statement is ignored in PROC IML; \
                 display customization is not supported."
            )),
            "{opt}: {}",
            out.log
        );
    }
    assert_eq!(out.listing, plain.listing);

    // Abbreviated forms (C= R= F= L=) are the same options.
    let out = run_sas(
        "proc iml;
           x = {1 2, 3 4};
           print x[c={\"a\" \"b\"} r={\"r1\" \"r2\"} f=8.2 l=\"lab\"];
         quit;",
    );
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert_eq!(
        out.log.matches("of the PRINT statement is ignored").count(),
        4
    );
    assert_eq!(out.listing, plain.listing);

    for (opts, needle) in [
        (
            "[foo=1]",
            "ERROR: IML: unknown PRINT option 'FOO='; the PRINT options are COLNAME=, ROWNAME=, \
             FORMAT= and LABEL=.",
        ),
        (
            "[1, 2]",
            "ERROR: IML: expected a PRINT option (COLNAME=, ROWNAME=, FORMAT= or LABEL=), found \
             '1'.",
        ),
        (
            "[label=]",
            "ERROR: IML: the PRINT option LABEL= requires a value.",
        ),
    ] {
        let out = run_sas(&format!(
            "proc iml;
               x = {{1 2}};
               print x{opts};
             quit;"
        ));
        assert_rejected(&out, needle, opts);
    }
}

/// Base : une erreur d'exécution faisait perdre toute la sortie PRINT déjà
/// produite (le listing n'était rendu qu'en fin d'exécution réussie). SAS
/// imprime au fil de l'exécution : la sortie des instructions exécutées
/// avant l'erreur est rendue ; une instruction PRINT en erreur n'imprime
/// rien. L'étape s'arrête à l'erreur (ERROR, code 2).
#[test]
fn ra_j02_p5_output_before_runtime_error() {
    let out = run_sas(
        "proc iml;
           x = {1 2};
           print x;
           do i = 1 to 2;
             print i;
           end;
           y = inv({1 2 3});
           print y;
         quit;",
    );
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert!(
        out.log
            .contains("ERROR: IML: INV requires a square matrix (got 1x3)."),
        "{}",
        out.log
    );
    assert!(out.listing.contains("The IML Procedure"), "{}", out.listing);
    assert!(
        out.listing
            .contains("\nX\n\n  COL1  COL2\n\n     1     2\n"),
        "{}",
        out.listing
    );
    assert_eq!(out.listing.matches("\nI\n").count(), 2, "{}", out.listing);
    assert!(!out.listing.contains("\nY\n"), "{}", out.listing);

    // A PRINT statement in error prints nothing (no partial output).
    let out = run_sas(
        "proc iml;
           x = {1 2};
           print x undefined;
         quit;",
    );
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert!(
        out.log
            .contains("ERROR: IML: matrix UNDEFINED has not been set to a value."),
        "{}",
        out.log
    );
    assert!(
        !out.listing.contains("The IML Procedure"),
        "{}",
        out.listing
    );
}

/// Base : une table créée par CREATE/APPEND sans CLOSE n'était jamais
/// écrite, et `CREATE x` puis `CLOSE work.x` désignaient deux tables
/// distinctes (la table restait ouverte, puis perdue). « SAS/IML software
/// automatically closes all open data sets when a QUIT statement is
/// executed » (SAS/IML 9.4, CLOSE Statement) : les tables encore ouvertes
/// sont écrites à QUIT, dans l'ordre des CREATE ; les noms à un niveau sont
/// normalisés en WORK.
/// https://support.sas.com/documentation/cdl/en/imlug/66845/HTML/default/imlug_langref_sect070.htm
#[test]
fn ra_j02_p5_open_tables_closed_at_quit() {
    let out = run_sas(
        "proc iml;
           m = {1 2, 3 4};
           create b from m;
           append from m;
           close work.b;
           create work.c from m;
           append from m;
           close c;
           create a from m;
           append from m;
         quit;
         proc print data=a; run;
         proc print data=b; run;
         proc print data=c; run;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    let b = out
        .log
        .find("NOTE: The data set WORK.B has 2 observations and 2 variables.")
        .expect(&out.log);
    let c = out
        .log
        .find("NOTE: The data set WORK.C has 2 observations and 2 variables.")
        .expect(&out.log);
    let a = out
        .log
        .find("NOTE: The data set WORK.A has 2 observations and 2 variables.")
        .expect(&out.log);
    // B and C are closed by their CLOSE statement, A by QUIT.
    assert!(b < c && c < a, "{}", out.log);
    assert_eq!(
        out.listing.matches("Obs    COL1    COL2").count(),
        3,
        "{}",
        out.listing
    );

    // Several tables still open at QUIT are written in CREATE order.
    let out = run_sas(
        "proc iml;
           m = {5 6};
           create t2 from m;
           create t1 from m;
         quit;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    let t2 = out
        .log
        .find("NOTE: The data set WORK.T2 has 0 observations")
        .expect(&out.log);
    let t1 = out
        .log
        .find("NOTE: The data set WORK.T1 has 0 observations")
        .expect(&out.log);
    assert!(t2 < t1, "{}", out.log);

    // An execution error stops the step before QUIT: an open table is not
    // written (no partial data set), and the log says so.
    let out = run_sas(
        "proc iml;
           m = {1 2};
           create w from m;
           append from m;
           y = log({1 -1});
         quit;",
    );
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert!(
        out.log.contains(
            "WARNING: The data set WORK.W was not written because PROC IML stopped at an \
             execution error."
        ),
        "{}",
        out.log
    );
    assert!(
        !out.log.contains("NOTE: The data set WORK.W"),
        "{}",
        out.log
    );
}

/// Base : SOLVE(A, b) résolvait une matrice A non carrée au sens des
/// moindres carrés, sans diagnostic, même pour un système incohérent.
/// « The matrix A must be square and nonsingular » (SAS/IML 9.4, SOLVE
/// Function) : ERROR ; un système carré reste résolu.
/// https://support.sas.com/documentation/cdl/en/imlug/66845/HTML/default/imlug_langref_sect428.htm
#[test]
fn ra_j02_p5_solve_requires_square_matrix() {
    for (a, b, dims) in [
        // Overdetermined, inconsistent: x1 = 1, x2 = 2, x1 + x2 = 4.
        ("{1 0, 0 1, 1 1}", "{1, 2, 4}", "3x2"),
        // Underdetermined.
        ("{1 1 0}", "{2}", "1x3"),
    ] {
        let out = run_sas(&format!(
            "proc iml;
               x = solve({a}, {b});
               print x;
             quit;"
        ));
        assert_eq!(out.exit_code, 2, "{a}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "ERROR: IML: SOLVE requires a square matrix (got {dims})."
            )),
            "{a}: {}",
            out.log
        );
        assert!(!out.listing.contains("\nX\n"), "{a}: {}", out.listing);
    }
    let out = run_sas(
        "proc iml;
           x = solve({2 0, 0 3}, {6, 9});
           print x;
         quit;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing.contains("\nX\n\nROW1     3\nROW2     3\n"),
        "{}",
        out.listing
    );
}

/// Base : MEAN et STD réduisaient toute la matrice à un scalaire ; SUM, MIN
/// et MAX ignoraient leurs arguments après le premier. SAS/IML 9.4 :
/// « If x is an n×p matrix, the function returns a 1×p row vector. The
/// value of the jth element is the mean for the jth column » (MEAN) ; STD
/// « is computed for each column » et « returns a missing value for
/// columns with fewer than two nonmissing observations » ; SUM, MIN et MAX
/// acceptent jusqu'à 15 matrices et rendent une valeur unique sur tous
/// leurs éléments, valeurs manquantes exclues. Oracles publiés : exemple
/// MEAN (x = {5 1 10, 6 2 3, 6 8 5, 6 7 9, 7 2 13} → 6 4 8) et exemple STD
/// (même matrice avec un manquant en ligne 3, colonne 3 → 0.7071068
/// 3.2403703 4.1932485, « Standard Deviation of Columns »).
/// https://support.sas.com/documentation/cdl/en/imlug/66845/HTML/default/imlug_langref_sect250.htm
/// https://support.sas.com/documentation/cdl/en/imlug/66845/HTML/default/imlug_langref_sect449.htm
/// https://support.sas.com/documentation/cdl/en/imlug/66845/HTML/default/imlug_langref_sect253.htm
/// https://support.sas.com/documentation/cdl/en/imlug/67502/HTML/default/imlug_langref_sect457.htm
#[test]
fn ra_j02_p5_column_statistics_and_all_arguments() {
    let out = run_sas(
        "data stdex; input a b c; datalines;
5 1 10
6 2 3
6 8 .
6 7 9
7 2 13
;
run;
         proc iml;
           x = {5 1 10, 6 2 3, 6 8 5, 6 7 9, 7 2 13};
           m = mean(x);
           use stdex;
           read all var {a b c} into y;
           close stdex;
           sd = std(y);
           s = std({1 2, 3 4, 5 9});
           r = std({2 4 6 8 10});
           t = sum({1 2, 3 4}, {10}, {100 200});
           mn = min({4 2}, {-5}, {7});
           mx = max({4 2}, {50}, {7});
           print m sd s r t mn mx;
         quit;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    for block in [
        // Documented example of the MEAN function.
        "\nM\n\n  COL1  COL2  COL3\n\n     6     4     8\n",
        // Documented example of the STD function (4 decimals in the listing).
        "\nSD\n\n    COL1    COL2    COL3\n\n  0.7071  3.2404  4.1932\n",
        // Column standard deviations: sd(1,3,5) = 2, sd(2,4,9) = sqrt(13).
        "\nS\n\n  COL1    COL2\n\n     2  3.6056\n",
        // One observation per column: missing standard deviations.
        "\nR\n\n  COL1  COL2  COL3  COL4  COL5\n\n     .     .     .     .     .\n",
        "\nT\n\n   320\n",
        "\nMN\n\n    -5\n",
        "\nMX\n\n    50\n",
    ] {
        assert!(out.listing.contains(block), "«{block}»\n{}", out.listing);
    }

    // Missing values are excluded: MEAN/STD per column, SUM/MIN/MAX overall.
    let out = run_sas(&format!(
        "{MISS}proc iml;
           use miss;
           read all var {{x y}} into d;
           close miss;
           m = mean(d);
           t = sum(d);
           mn = min(d);
           mx = max(d);
           print m t mn mx;
         quit;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    for block in [
        "\nM\n\n  COL1  COL2\n\n     2    15\n",
        "\nT\n\n    34\n",
        "\nMN\n\n     1\n",
        "\nMX\n\n    20\n",
    ] {
        assert!(out.listing.contains(block), "«{block}»\n{}", out.listing);
    }
}

/// Base : `{1 2} / {0 1}` rendait `inf` en silence (0/0 : NaN). SAS/IML 9.4,
/// Division Operator : « If a divisor is zero, the operation displays a
/// warning and assigns a missing value for the corresponding element in the
/// result » (texte du WARNING SAS/IML : « Division by zero, result set to
/// missing value. ») ; un opérande manquant donne un quotient manquant.
/// https://support.sas.com/documentation/cdl/en/imlug/66845/HTML/default/imlug_langref_sect031.htm
#[test]
fn ra_j02_p5_division_by_zero_warning() {
    let out = run_sas(
        "proc iml;
           a = {1 2} / {0 1};
           b = {0 4} / 0;
           c = 6 / {3 2};
           print a b c;
         quit;",
    );
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert_eq!(
        out.log
            .matches(
                "WARNING: Division by zero, result set to missing value.\n         operation : /"
            )
            .count(),
        2,
        "{}",
        out.log
    );
    for block in [
        "\nA\n\n  COL1  COL2\n\n     .     2\n",
        "\nB\n\n  COL1  COL2\n\n     .     .\n",
        "\nC\n\n  COL1  COL2\n\n     2     3\n",
    ] {
        assert!(out.listing.contains(block), "«{block}»\n{}", out.listing);
    }
    // A missing operand gives a missing quotient, without the warning.
    let out = run_sas(&format!(
        "{MISS}proc iml;
           use miss;
           read all var {{x}} into x;
           close miss;
           q = x / 2;
           print q;
         quit;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing
            .contains("\nQ\n\nROW1   0.5\nROW2     .\nROW3   1.5\n"),
        "{}",
        out.listing
    );
}

/// Base : `log(0)` rendait -inf, `log(-1)` et `sqrt(-1)` NaN, en silence.
/// SAS/IML signale l'ERROR d'exécution « Invalid argument to function »
/// avec l'opération en cause (SAS/IML blog, « How to interpret SAS/IML
/// error messages » : `operation : LOG`) ; une valeur manquante en entrée
/// reste manquante.
/// https://blogs.sas.com/content/iml/2010/11/29/how-to-interpret-sasiml-error-messages
#[test]
fn ra_j02_p5_log_sqrt_invalid_argument() {
    for (expr, fname) in [
        ("log(0)", "LOG"),
        ("log({1 -1})", "LOG"),
        ("sqrt(-1)", "SQRT"),
        ("sqrt({4, -0.5})", "SQRT"),
    ] {
        let out = run_sas(&format!(
            "proc iml;
               y = {expr};
               print y;
             quit;"
        ));
        assert_eq!(out.exit_code, 2, "{expr}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "ERROR: (execution) Invalid argument to function.\n       operation : {fname}"
            )),
            "{expr}: {}",
            out.log
        );
        assert!(!out.listing.contains("\nY\n"), "{expr}: {}", out.listing);
    }
    let out = run_sas(
        "proc iml;
           y = log({1 2.718281828459045}) + sqrt({0 4});
           print y;
         quit;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing
            .contains("\nY\n\n  COL1  COL2\n\n     0     3\n"),
        "{}",
        out.listing
    );
    // Missing in, missing out.
    let out = run_sas(&format!(
        "{MISS}proc iml;
           use miss;
           read all var {{x}} into x;
           close miss;
           y = log(x);
           z = sqrt(x);
           print y z;
         quit;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing.contains("\nY\n\nROW1       0\nROW2       .\n"),
        "{}",
        out.listing
    );
    assert!(
        out.listing.contains("\nZ\n\nROW1       1\nROW2       .\n"),
        "{}",
        out.listing
    );
}

/// Base : « ERROR: ERROR: The argument to the EIGVAL function must be a
/// symmetric matrix. » — le message portait déjà le préfixe que le log
/// ajoute. Un seul préfixe (EIGVAL, EIGVEC, CALL EIGEN).
#[test]
fn ra_j02_p5_no_doubled_error_prefix() {
    for (stmt, fname) in [
        ("e = eigval({1 2, 3 4});", "EIGVAL"),
        ("e = eigvec({1 2, 3 4});", "EIGVEC"),
        ("call eigen(v, e, {1 2, 3 4});", "EIGEN"),
    ] {
        let out = run_sas(&format!(
            "proc iml;
               {stmt}
             quit;"
        ));
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "ERROR: The argument to the {fname} function must be a symmetric matrix."
            )),
            "{stmt}: {}",
            out.log
        );
        assert!(!out.log.contains("ERROR: ERROR:"), "{stmt}: {}", out.log);
    }
}

/// Base : une instruction inconnue était lue comme une affectation mal
/// formée : « IML: expected '=' in an assignment, found Ident("x") » (texte
/// de jeton Rust), BY compris. Instruction inconnue → ERROR 180-322 du
/// repli partagé ; BY (et les autres instructions du repli partagé : WHERE,
/// WEIGHT…) → message partagé « not supported » ; FORMAT → WARNING
/// d'affichage partagé. Une affectation à une variable de même nom reste
/// une affectation.
#[test]
fn ra_j02_p5_unknown_statement_180_322() {
    let out = run_sas(
        "proc iml;
           invented x;
         quit;",
    );
    assert_rejected(
        &out,
        "ERROR: 180-322: Statement 'INVENTED' is not valid or it is used out of proper order in \
         PROC IML.",
        "invented",
    );
    assert_no_rust_token(&out.log, "invented");

    for stmt in [
        "by g",
        "where x > 1",
        "weight w",
        "freq f",
        "class c",
        "id i",
    ] {
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        let out = run_sas(&format!(
            "proc iml;
               {stmt};
             quit;"
        ));
        assert_rejected(
            &out,
            &format!(
                "ERROR: The {kw} statement is not supported in PROC IML; it can affect results \
                 and cannot be ignored."
            ),
            stmt,
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }

    let out = run_sas(
        "proc iml;
           x = {1};
           format x 8.2;
           print x;
         quit;",
    );
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert!(
        out.log.contains(
            "WARNING: The FORMAT statement is ignored in PROC IML; display customization is not \
             supported."
        ),
        "{}",
        out.log
    );

    let out = run_sas(
        "proc iml;
           by = {1 2};
           where = by + 1;
           print where;
         quit;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing
            .contains("\nWHERE\n\n  COL1  COL2\n\n     2     3\n"),
        "{}",
        out.listing
    );
}

/// Base : les instructions SAS/IML valides mais non implémentées étaient
/// lues comme des affectations mal formées (START, RUN, RESET…) ou
/// n'échouaient qu'à l'exécution (STORE, LOAD, SHOW, FREE, REMOVE, EDIT,
/// READ NEXT, clause WHERE de READ). Message du catalogue « not supported …
/// cannot be ignored » au parsing, avec l'unité qui les livrera ; MATTRIB
/// et SHOW (affichage seul) et TITLE/FOOTNOTE → WARNING ; les autres
/// instructions globales (OPTIONS, LIBNAME, FILENAME, ODS) → ERROR.
/// Référence : SAS/IML 13.2 User's Guide, « Control Statements », « Data
/// Set and File Functions ».
/// https://support.sas.com/documentation/cdl/en/imlug/67502/HTML/default/imlug_langref_sect017.htm
/// https://support.sas.com/documentation/cdl/en/imlug/67502/HTML/default/imlug_langref_sect018.htm
#[test]
fn ra_j02_p5_unsupported_statements() {
    for (stmt, needle) in [
        (
            "start mymod",
            format!("The START statement {CANNOT} (planned: roadmap-avancee J10-P2)."),
        ),
        (
            "finish",
            format!("The FINISH statement {CANNOT} (planned: roadmap-avancee J10-P2)."),
        ),
        (
            "run mymod",
            format!("The RUN statement {CANNOT} (planned: roadmap-avancee J10-P2)."),
        ),
        ("store x", format!("The STORE statement {CANNOT}.")),
        ("load x", format!("The LOAD statement {CANNOT}.")),
        ("free x", format!("The FREE statement {CANNOT}.")),
        ("remove x", format!("The REMOVE statement {CANNOT}.")),
        ("reset print", format!("The RESET statement {CANNOT}.")),
        ("edit work.t", format!("The EDIT statement {CANNOT}.")),
        ("sort work.t by x", format!("The SORT statement {CANNOT}.")),
        ("options ls=80", format!("The OPTIONS statement {CANNOT}.")),
        ("libname l '.'", format!("The LIBNAME statement {CANNOT}.")),
        (
            "x = {1 2}; x[1, 2] = 5",
            format!(
                "Assignment to a subscripted matrix (X[...] = ...) {CANNOT} (planned: \
                 roadmap-avancee J10-P3)."
            ),
        ),
        (
            "use t; read next var {x} into m",
            format!("READ NEXT {CANNOT} (planned: roadmap-avancee J10-P3)."),
        ),
        (
            "use t; read all var {x} where(x > 1) into m",
            format!(
                "The WHERE clause of the READ statement {CANNOT} (planned: roadmap-avancee \
                 J10-P3)."
            ),
        ),
        (
            "use t; read all var _num_ into m",
            format!("READ ALL VAR _NUM_ {CANNOT} (planned: roadmap-avancee J10-P3)."),
        ),
        ("do data; end", format!("The DO DATA statement {CANNOT}.")),
    ] {
        let out = run_sas(&format!(
            "data t; x = 1; run;
             proc iml;
               {stmt};
             quit;"
        ));
        assert_rejected(&out, &format!("ERROR: {needle}"), stmt);
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
        assert_no_rust_token(&out.log, stmt);
    }

    // The shared catalogue message, verbatim, for statements without a plan.
    let shared = crate::procs::common::unsupported_statement("IML", "store").to_string();
    let out = run_sas("proc iml; store x; quit;");
    assert!(out.log.contains(&format!("ERROR: {shared}")), "{}", out.log);

    for (stmt, kw) in [
        ("show names", "SHOW"),
        ("mattrib x label='X'", "MATTRIB"),
        ("title 'In IML'", "TITLE"),
        ("footnote2 'Source'", "FOOTNOTE2"),
    ] {
        let out = run_sas(&format!(
            "proc iml;
               x = {{1}};
               {stmt};
               print x;
             quit;"
        ));
        assert_eq!(out.exit_code, 1, "{stmt}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "WARNING: The {kw} statement is ignored in PROC IML; display customization is \
                 not supported."
            )),
            "{stmt}: {}",
            out.log
        );
        assert!(out.listing.contains("\nX\n"), "{stmt}: {}", out.listing);
    }
}

/// Base : une valeur manquante s'imprimait « NaN » (lecture d'une table par
/// READ) ; « a numeric missing value is specified as a single period »
/// (SAS/IML 9.4, Missing Values) : elle s'imprime `.`. CREATE écrit une
/// valeur manquante comme manquant SAS (null), pas comme NaN.
/// https://support.sas.com/documentation/cdl/en/imlug/67502/HTML/default/imlug_languagechap_sect024.htm
#[test]
fn ra_j02_p5_missing_values_print_dot() {
    let out = run_sas(&format!(
        "{MISS}proc iml;
           use miss;
           read all var {{x y}} into d;
           close miss;
           print d;
           create back from d;
           append from d;
           close back;
         quit;
         proc means data=back n nmiss; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing.contains(
            "\nD\n\n      COL1  COL2\n\nROW1     1    10\nROW2     .    20\nROW3     3     .\n"
        ),
        "{}",
        out.listing
    );
    assert!(!out.listing.contains("NaN"), "{}", out.listing);
    let means = out
        .listing
        .split("The MEANS Procedure")
        .nth(1)
        .expect(&out.listing);
    let row = |var: &str| {
        means
            .lines()
            .find(|l| l.trim_start().starts_with(var))
            .unwrap_or_else(|| panic!("{var}\n{means}"))
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
    };
    assert_eq!(row("COL1"), vec!["2", "1"]);
    assert_eq!(row("COL2"), vec!["2", "1"]);
}

/// Base : les fonctions ignoraient leurs arguments surnuméraires (ABS(x, y)
/// rendait ABS(x) ; MEAN(x, méthode) la moyenne arithmétique ; SHAPE sa
/// valeur de remplissage). Nombre d'arguments faux → ERROR au parsing ;
/// formes SAS valides non implémentées (méthode de MEAN, pad-value de
/// SHAPE) → « not supported ».
#[test]
fn ra_j02_p5_function_arity() {
    for (expr, needle) in [
        (
            "abs({1}, {2})",
            "ERROR: IML: the ABS function takes 1 argument; 2 were specified.".to_string(),
        ),
        (
            "solve({1})",
            "ERROR: IML: the SOLVE function takes 2 arguments; 1 was specified.".to_string(),
        ),
        (
            "sum({1},{2},{3},{4},{5},{6},{7},{8},{9},{10},{11},{12},{13},{14},{15},{16})",
            "ERROR: IML: the SUM function takes from 1 to 15 arguments; 16 were specified."
                .to_string(),
        ),
        (
            "mean({1 2}, m)",
            format!(
                "ERROR: The method argument of the MEAN function (trimmed or Winsorized mean) \
                 {CANNOT}."
            ),
        ),
        (
            "mean({1 2}, \"trimmed\", 0.2)",
            format!("ERROR: A character literal (\"trimmed\") in an expression {CANNOT}."),
        ),
        (
            "shape({1 2}, 2, 2, 0)",
            format!("ERROR: The pad-value argument of the SHAPE function {CANNOT}."),
        ),
    ] {
        let out = run_sas(&format!(
            "proc iml;
               m = {{1}};
               y = {expr};
             quit;"
        ));
        assert_rejected(&out, &needle, expr);
    }
}

/// Base : les erreurs de syntaxe IML citaient la forme Debug des jetons Rust
/// (« found Ident(\"x\") », « found Semi », « found LBracket »…). Elles
/// citent le texte source du jeton.
#[test]
fn ra_j02_p5_parse_errors_without_rust_tokens() {
    for (stmt, needle) in [
        (
            "x = {1 2} +;",
            "ERROR: IML: unexpected ';' in an expression.",
        ),
        ("x = (1 + 2;", "ERROR: IML: expected ')', found ';'."),
        (
            "print x +;",
            "ERROR: IML: unexpected '+' in the PRINT statement.",
        ),
        (
            "x = {1 2 a};",
            "ERROR: IML: matrix literals support only numeric constants, found 'A'.",
        ),
        (
            "{1 2};",
            "ERROR: 180-322: Statement is not valid or it is used out of proper order in PROC IML.",
        ),
        ("call qr(q r);", "ERROR: IML: expected ')', found 'R'."),
        ("do i = 1 to 3;", "ERROR: IML: missing END for a DO block"),
    ] {
        let out = run_sas(&format!(
            "proc iml;
               x = {{1}};
               {stmt}
             quit;"
        ));
        assert_rejected(&out, needle, stmt);
        assert_no_rust_token(&out.log, stmt);
    }
}

/// Base : une valeur manquante (issue d'une division par zéro ou d'une
/// table) entrait dans INV, SOLVE, DET, CHOL, CALL QR/SVDCD ou le produit
/// matriciel et en ressortait en NaN silencieux. « SAS/IML software does
/// not support missing values in most matrix operations and functions »,
/// « Matrix multiplication with missing values is not supported »
/// (SAS/IML 9.4, Missing Values) : ERROR d'exécution SAS/IML « Invalid
/// argument or operand; contains missing values ».
/// https://support.sas.com/documentation/cdl/en/imlug/66845/HTML/default/imlug_asstop_sect005.htm
#[test]
fn ra_j02_p5_missing_values_in_matrix_operations() {
    for (stmt, operation) in [
        ("y = inv(a);", "INV"),
        ("y = solve(a, {1, 2});", "SOLVE"),
        ("y = det(a);", "DET"),
        ("y = chol(a);", "CHOL"),
        ("call qr(q, r, a);", "QR"),
        ("call svdcd(u, d, v, a);", "SVDCD"),
        ("y = a * {1, 1};", "*"),
    ] {
        let out = run_sas(&format!(
            "proc iml;
               a = {{4 1, 1 3}} / {{1 0, 1 1}};
               {stmt}
             quit;"
        ));
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "ERROR: (execution) Invalid argument or operand; contains missing values.\n       \
                 operation : {operation}"
            )),
            "{stmt}: {}",
            out.log
        );
    }
    // Elementwise arithmetic keeps propagating missing values (documented).
    let out = run_sas(
        "proc iml;
           a = {4 1} / {1 0};
           b = a + 1;
           c = 2 * a;
           print b c;
         quit;",
    );
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert!(
        out.listing
            .contains("\nB\n\n  COL1  COL2\n\n     5     .\n"),
        "{}",
        out.listing
    );
    assert!(
        out.listing
            .contains("\nC\n\n  COL1  COL2\n\n     8     .\n"),
        "{}",
        out.listing
    );
}
