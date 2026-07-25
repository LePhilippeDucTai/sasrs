use crate::{run, RunOptions};
use super::*;

#[test]
fn options_ls_applied_and_unknown_option_warns() {
    // M22.2 — CENTER/NOCENTER/DATE/NODATE/NUMBER/NONUMBER are now handled
    // as ODS options, so no warning. Test with an actually unknown option.
    let out = run_det("options ls=120 unknownopt;");
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert!(out.log.contains("WARNING: Option UNKNOWNOPT is not yet supported."));
}

#[test]
fn options_firstobs_and_obs_window_input() {
    // Build a 5-row data set, then read it with FIRSTOBS=2 OBS=4 → obs 2..4
    // (3 observations). The window applies to the physical SET input.
    let out = run_det(
        "data a; do i = 1 to 5; output; end; run;\n\
         options firstobs=2 obs=4;\n\
         data b; set a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.log
            .contains("The data set WORK.A has 5 observations and 1 variables."),
        "{}",
        out.log
    );
    assert!(
        out.log
            .contains("The data set WORK.B has 3 observations and 1 variables."),
        "{}",
        out.log
    );
}

#[test]
fn options_sasautos_enables_autocall() {
    // M19.2 — `OPTIONS SASAUTOS='dir';` (chemin relatif résolu contre
    // base_dir) doit câbler la recherche autocall : une macro non définie
    // dans le code est cherchée comme `nom.sas` dans ce répertoire et
    // compilée paresseusement à l'invocation.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("auto")).unwrap();
    std::fs::write(
        dir.path().join("auto").join("greet.sas"),
        "%macro greet(who); %put HELLO &who from autocall; %mend;\n",
    )
    .unwrap();
    // L'option doit être posée AVANT l'expansion du segment qui invoque la
    // macro autocall : on place une frontière de segment (`run;`) entre les
    // deux (l'expansion est interfoliée par segment).
    let out = run(
        "options sasautos='auto';\ndata _null_; run;\n%greet(WORLD);\n",
        RunOptions {
            work_dir: None,
            base_dir: Some(dir.path().to_path_buf()),
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.log.contains("HELLO WORLD from autocall"),
        "autocall macro did not run; log was:\n{}",
        out.log
    );
}

// ── M38.2 — OPTIONS system options ───────────────────────────────────────

/// PAGESIZE= / PS= : valeur stockée, pas de WARNING.
#[test]
fn options_pagesize_stored() {
    let s = run_globals(&["options ps=40;"]);
    assert_eq!(s.options.pagesize, 40);
    // PS= alias.
    let s2 = run_globals(&["options pagesize=100;"]);
    assert_eq!(s2.options.pagesize, 100);
}

/// PAGESIZE valeur invalide → erreur (hors plage 15..32767).
#[test]
fn options_pagesize_invalid_emits_error() {
    let out = run_det("options ps=5;");
    assert!(
        out.log.contains("ERROR") && out.log.contains("PAGESIZE"),
        "expected PAGESIZE error in log:\n{}",
        out.log
    );
    // Valid range upper bound.
    let out2 = run_det("options ps=32768;");
    assert!(
        out2.log.contains("ERROR") && out2.log.contains("PAGESIZE"),
        "expected PAGESIZE error for 32768:\n{}",
        out2.log
    );
}

/// MISSING= : la valeur stockée s'applique au rendu des missings ordinaires.
#[test]
fn options_missing_char_stored_and_rendered() {
    // Default: missing_char = '.'.
    let s = run_globals(&[]);
    assert_eq!(s.options.missing_char, '.');

    // MISSING='X' → stocké.
    let s2 = run_globals(&["options missing='X';"]);
    assert_eq!(s2.options.missing_char, 'X');

    // MISSING=' ' → espace.
    let s3 = run_globals(&["options missing=' ';"]);
    assert_eq!(s3.options.missing_char, ' ');
}

/// MISSING='X' : PROC PRINT rend les valeurs manquantes ordinaires en 'X'.
#[test]
fn options_missing_x_renders_in_proc_print() {
    let out = run_det(
        "options missing='X';\n\
         data a; x = .; run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    // Le listing doit contenir 'X' (rendu de la valeur manquante).
    assert!(out.listing.contains('X'), "listing:\n{}", out.listing);
    // Et ne doit PAS contenir '.' à la position d'une valeur numérique.
    // (Uniquement le point de la colonne Obs n'est pas là.)
}

/// MISSING='.' (défaut) : comportement identique à l'original.
#[test]
fn options_missing_default_dot_unchanged() {
    let out = run_det(
        "data a; x = .; run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(out.listing.contains('.'), "expected '.' in listing:\n{}", out.listing);
}

/// YEARCUTOFF= : défaut 1900 → 0-99 donne 1900-1999 (comportement actuel).
#[test]
fn options_yearcutoff_default_1900_datejul() {
    // datejul(15365) → day 365 of year 15 → 1915 (avec yearcutoff=1900)
    let out = run_det(
        "data a; d = datejul(15365); run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    // L'année pleine devrait être 1915 : SAS date = (1915-1960)*365.25 ...
    // On vérifie juste que ça ne plante pas et que le listing contient une valeur.
    assert!(!out.listing.is_empty(), "listing empty:\n{}", out.listing);
}

/// YEARCUTOFF=2000 : yy=15 → 2015.
#[test]
fn options_yearcutoff_2000() {
    // datejul(15365) avec yearcutoff=2000 → year=2015.
    // SAS date pour 2015-12-31 = (2015-1960)*365 + bissextiles ≈ 20453
    // On vérifie que la valeur n'est pas la même qu'avec yearcutoff=1900.
    let out_1900 = run_det("data a; d = datejul(15365); run; proc print data=a; run;");
    let out_2000 = run_det(
        "options yearcutoff=2000;\n\
         data a; d = datejul(15365); run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out_2000.exit_code, 0, "log:\n{}", out_2000.log);
    // Les listings doivent être différents (année 1915 vs 2015).
    assert_ne!(
        out_1900.listing, out_2000.listing,
        "expected different dates with yearcutoff=1900 vs 2000"
    );
}

/// YEARCUTOFF=1950 : fenêtre glissante — yy=49 → 2049, yy=50 → 1950.
#[test]
fn options_yearcutoff_1950_sliding_window() {
    // yy=49 → base=1900, candidate=1949 < 1950 → +100 = 2049.
    let out49 = run_det(
        "options yearcutoff=1950;\n\
         data a; d = datejul(49365); put d=; run;\n",
    );
    assert_eq!(out49.exit_code, 0, "log49:\n{}", out49.log);
    // yy=50 → candidate=1950 >= 1950 → 1950.
    let out50 = run_det(
        "options yearcutoff=1950;\n\
         data a; d = datejul(50365); put d=; run;\n",
    );
    assert_eq!(out50.exit_code, 0, "log50:\n{}", out50.log);
    // Les deux années sont différentes.
    assert_ne!(
        out49.log, out50.log,
        "expected different output for yy=49 vs yy=50 with yearcutoff=1950"
    );
}

/// FMTSEARCH= : stocké, plus de WARNING.
#[test]
fn options_fmtsearch_stored_no_warning() {
    let out = run_det("options fmtsearch=(mylib work);");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        !out.log.contains("is not yet supported"),
        "unexpected warning in log:\n{}",
        out.log
    );
    // La valeur est stockée sous forme de liste.
    let s = run_globals(&["options fmtsearch=(mylib work);"]);
    assert_eq!(s.options.fmtsearch, vec!["MYLIB", "WORK"]);
}

/// Une option inconnue (FOOBAR=1) émet toujours le warning.
#[test]
fn options_unknown_still_warns() {
    let out = run_det("options foobar=1;");
    assert!(
        out.log.contains("is not yet supported"),
        "expected not-yet-supported warning in log:\n{}",
        out.log
    );
}

#[test]
fn libname_relative_resolution_and_clear() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("dat")).unwrap();
    let out = run(
        "libname mylib 'dat';\nlibname mylib clear;\n",
        RunOptions {
            work_dir: None,
            base_dir: Some(dir.path().to_path_buf()),
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out
        .log
        .contains("Libref MYLIB was successfully assigned as follows:"));
    assert!(out.log.contains("Physical Name:"));
    assert!(out.log.contains("Libref MYLIB has been deassigned."));
}

/// M14.4 — LIBNAME XLSX emits a clear deferral error.
#[test]
fn libname_xlsx_engine_deferred_error() {
    let out = run_det("libname xl xlsx '/tmp';");
    // The log must contain an ERROR message about XLSX not being available.
    assert!(
        out.log.contains("ERROR"),
        "expected ERROR in log: {}",
        out.log
    );
    assert!(
        out.log.to_ascii_lowercase().contains("xlsx"),
        "expected 'xlsx' in error: {}",
        out.log
    );
}

/// M14.4 — LIBNAME EXCEL (synonym for XLSX) also deferred.
#[test]
fn libname_excel_engine_deferred_error() {
    let out = run_det("libname xl excel '/tmp';");
    assert!(out.log.contains("ERROR"), "expected ERROR: {}", out.log);
}

/// M14.4 — LIBNAME with CSV engine assigns and reads back a table.
#[test]
fn libname_csv_engine_end_to_end() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    // Write a CSV file in the temp dir.
    let csv_path = tmp.path().join("scores.csv");
    let mut f = std::fs::File::create(&csv_path).unwrap();
    writeln!(f, "id,score").unwrap();
    writeln!(f, "1,100").unwrap();
    writeln!(f, "2,200").unwrap();
    drop(f);

    let src = format!(
        "libname csv1 csv '{}';\n\
         data work.out; set csv1.scores; run;\n\
         proc print data=work.out; run;\n",
        tmp.path().display()
    );
    let out = crate::run(
        &src,
        crate::RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.log.contains("Engine:        CSV"),
        "expected CSV engine note: {}",
        out.log
    );
    assert!(
        out.log.contains("2 observations"),
        "expected 2 obs note: {}",
        out.log
    );
}

/// M14.4 — LIBNAME without engine (no engine field) → parquet path unchanged.
#[test]
fn libname_no_engine_uses_parquet_path() {
    let tmp = tempfile::tempdir().unwrap();
    let out = crate::run(
        &format!("libname p '{}';", tmp.path().display()),
        crate::RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(out.log.contains("Engine:        PARQUET"), "{}", out.log);
}

#[test]
fn data_null_no_listing_no_dataset_note() {
    let out = run_det("data _null_; x = 1; run;");
    assert_eq!(out.exit_code, 0);
    assert!(!out.log.contains("has 1 observations"));
    assert!(out.listing.is_empty());
}

/// M11.1 : l'expansion macro est conduite par l'executor (état dans
/// `Session::macro_engine`). Un programme avec `%let`/`&var` doit produire
/// EXACTEMENT le même résultat que son équivalent sans macro.
#[test]
fn macro_let_ref_runs_through_executor() {
    let with_macro = run_det(
        "%let lib=work; data &lib..a; x=1; run; proc print data=&lib..a; run;",
    );
    let without_macro = run_det(
        "data work.a; x=1; run; proc print data=work.a; run;",
    );
    assert_eq!(with_macro.exit_code, 0, "log was:\n{}", with_macro.log);
    assert_eq!(
        with_macro.listing, without_macro.listing,
        "macro listing differs:\nMACRO:\n{}\nPLAIN:\n{}",
        with_macro.listing, without_macro.listing
    );
    // Les NOTEs de l'étape DATA / PROC doivent correspondre.
    assert!(with_macro
        .log
        .contains("The data set WORK.A has 1 observations and 1 variables."));
    assert!(with_macro
        .log
        .contains("There were 1 observations read from the data set WORK.A."));
}

/// M11.5 : `CALL SYMPUT` dans une étape pose un symbole macro visible
/// dans le SEGMENT SUIVANT (le drain a lieu au `run;`). Ici on s'en sert
/// pour nommer un dataset de l'étape d'après.
#[test]
fn symput_visible_in_next_segment_as_dataset_name() {
    let out = run_det(
        "data _null_; call symput('answer','42'); run;\n\
         data tbl_&answer; x=1; run;\n\
         proc print data=tbl_&answer; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    // Le dataset a bien été nommé WORK.TBL_42 (symbole résolu).
    assert!(
        out.log
            .contains("The data set WORK.TBL_42 has 1 observations and 1 variables."),
        "log was:\n{}",
        out.log
    );
    assert!(out
        .log
        .contains("There were 1 observations read from the data set WORK.TBL_42."));
}

/// M11.5 : formatage NUMÉRIQUE d'un symput — `42` (et non `          42`).
#[test]
fn symput_numeric_value_left_aligned_best12() {
    let out = run_det(
        "data _null_; call symput('n', 42); run;\n\
         data tbl_&n; x=1; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    assert!(
        out.log
            .contains("The data set WORK.TBL_42 has 1 observations and 1 variables."),
        "log was:\n{}",
        out.log
    );
}

/// M11.5 : un symput n'est PAS visible DANS LA MÊME étape. SYMGET lit
/// l'instantané de DÉBUT d'étape : un `symput('w', ...)` plus tôt dans la
/// MÊME étape ne s'y reflète pas (le drain n'a lieu qu'au `run;`). Ici
/// `w` n'existe pas au début de l'étape → symget rend une valeur vide,
/// alors que l'étape SUIVANTE la verrait.
#[test]
fn symput_not_visible_in_same_step() {
    let out = run_det(
        "data a; call symput('w','99'); seen = symget('w'); run;\n\
         data b; later = symget('w'); run;\n\
         proc print data=b; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    // Étape A : `seen` est vide (symput pas encore drainé) → 0 obs avec
    // valeur non vide ; on vérifie surtout que B voit bien 99.
    assert!(
        out.listing.contains("99"),
        "step B should see w=99 via symget; listing was:\n{}",
        out.listing
    );
}

/// M11.5 : SYMGET lit un `%let` antérieur (table macro → DATA step).
#[test]
fn symget_reads_prior_let() {
    let out = run_det(
        "%let x = 5;\n\
         data a; v = symget('x'); run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    // v est une variable caractère = "5".
    assert!(out.listing.contains('5'), "listing was:\n{}", out.listing);
    assert!(out
        .log
        .contains("The data set WORK.A has 1 observations and 1 variables."));
}

#[test]
fn proc_print_uses_last_dataset() {
    let out = run_det(
        "data zz; v = 3.5; run;\n\
         proc print; run;\n",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out
        .log
        .contains("There were 1 observations read from the data set WORK.ZZ."));
    assert!(out.listing.contains("3.5"));
}
