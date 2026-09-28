//! Diagnostic oracle: CONTRIBUTING.md §5 and docs/support-contract.md (J02-P3).
//! NOPRINT/default options: SAS 9.4 PROC UNIVARIATE and PROC SQL statements.
use crate::{RunOptions, RunOutcome};

fn run(src: &str) -> RunOutcome {
    crate::run(
        src,
        RunOptions {
            deterministic: true,
            ..Default::default()
        },
    )
}

#[test]
fn silent_opt_result_options_error() {
    let cases = [
        "proc univariate data=t vardef=n; var x; run;",
        "proc univariate data=t pctldef=1; var x; run;",
        "proc freq data=t; tables x / invented; run;",
        "proc freq data=t; tables x / invented=2; run;",
        "proc datasets lib=work kill; quit;",
        "proc datasets lib=work invented; quit;",
        "proc datasets lib=work; copy out=work invented; quit;",
        "proc means data=t; var x; output out=result clm(x)=c; run;",
        "proc means data=t; var x; output out=result invented(x)=c; run;",
        "proc sql outobs=1; select * from t; quit;",
        "proc sql inobs=1; select * from t; quit;",
        "proc sql; invented; quit;",
    ];
    for proc in cases {
        let out = run(&format!("data t; x=1; output; x=2; output; run; {proc}"));
        assert_eq!(out.exit_code, 2, "{proc}\n{}", out.log);
        assert!(out.log.contains("ERROR"), "{proc}\n{}", out.log);
    }
}

#[test]
fn silent_opt_univariate_noprint_keeps_output() {
    let out = run("data t; x=1; output; x=3; output; run;
        proc univariate data=t noprint vardef=df pctldef=5; var x;
        output out=result mean=m; run;
        data _null_; set result; put 'MEAN=' m; run;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.is_empty(), "{}", out.listing);
    assert!(out.log.contains("MEAN= 2"), "{}", out.log);
}

#[test]
fn silent_opt_sql_noprint_keeps_create_and_restores_printing() {
    let out = run("data t; x=7; run;
        proc sql noprint; select x from t; create table result as select x from t; quit;
        data _null_; set result; put 'KEPT=' x; run;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.is_empty(), "{}", out.listing);
    assert!(out.log.contains("KEPT= 7"), "{}", out.log);
    let out = run("data t; x=7; run;
        proc sql noprint; select x from t; quit;
        proc sql; select x as visible from t; quit;");
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing.to_lowercase().contains("visible"),
        "{}",
        out.listing
    );
}

#[test]
fn silent_opt_printto_routes_for_real() {
    // J07-P5 — PRINTTO route réellement log et listing : plus de WARNING
    // « reconnu mais ignoré ». Le segment routé part dans les fichiers, le
    // reset ramène les destinations par défaut.
    let dir = tempfile::tempdir().unwrap();
    let out = crate::run(
        "proc printto log='used.log' print='used.lst'; run;\n\
         data _null_; put 'IN_ROUTE'; run;\n\
         proc printto; run;\n\
         data _null_; put 'AFTER_RESET'; run;",
        RunOptions {
            deterministic: true,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        },
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(!out.log.contains("WARNING"), "{}", out.log);
    assert!(!out.log.contains("ERROR"), "{}", out.log);
    // Le PUT routé part dans used.log, pas dans le log par défaut ; après le
    // reset les lignes reviennent au log par défaut.
    assert!(!out.log.contains("IN_ROUTE"), "{}", out.log);
    assert!(out.log.contains("AFTER_RESET"), "{}", out.log);
    let routed_log = std::fs::read_to_string(dir.path().join("used.log")).unwrap();
    assert!(routed_log.contains("IN_ROUTE"), "{routed_log}");
    assert!(!routed_log.contains("AFTER_RESET"), "{routed_log}");
}

#[test]
fn silent_opt_plot_warns_and_stays_synchronized() {
    let out = run("data t; x=1; y=2; run;
        proc plot data=t; plot y*x / href=1 vref=2 haxis=0 to 5 by 1; run;
        data _null_; put 'AFTER_PLOT'; run;");
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert!(out.log.contains("WARNING"), "{}", out.log);
    assert!(out.log.contains("AFTER_PLOT"), "{}", out.log);
    assert!(out.listing.contains("Plot of Y*X"), "{}", out.listing);
}
