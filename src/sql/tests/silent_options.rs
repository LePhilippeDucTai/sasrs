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
        "proc compare base=t compare=t criterion=0.1; run;",
        "proc compare base=t compare=t method=absolute; run;",
        "proc compare base=t compare=t brief; run;",
        "proc compare base=t compare=t listall; run;",
        "proc compare base=t compare=t outbase; run;",
        "proc compare base=t compare=t outcomp; run;",
        "proc compare base=t compare=t outdif; run;",
        "proc compare base=t compare=t outnoequal; run;",
        "proc compare base=t compare=t maxprint=2; run;",
        "proc compare base=t compare=t; id x; run;",
        "proc compare base=t compare=t; var x; run;",
        "proc compare base=t compare=t; with x; run;",
        "proc compare base=t compare=t; by x; run;",
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
fn silent_opt_printto_warns_honestly() {
    let out = run("proc printto log='unused.log' print='unused.lst'; run;");
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert_eq!(
        out.log.matches("routing not supported").count(),
        2,
        "{}",
        out.log
    );
    assert!(!out.log.contains("redirected to"), "{}", out.log);
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
