use super::*;

// ---- M15.6 — CALL EXECUTE end-to-end (post-step replay) -------------

/// CALL EXECUTE queues code that runs AFTER the current step's RUN.
#[test]
fn call_execute_runs_queued_step_after_run() {
    let out = run_det(
        "data _null_; call execute('data made; v = 7; output; run;'); run;\n\
         proc print data=made; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    // The queued DATA step created WORK.MADE.
    assert!(
        out.log
            .contains("The data set WORK.MADE has 1 observations and 1 variables."),
        "log was:\n{}",
        out.log
    );
    assert!(out.listing.contains('7'), "listing:\n{}", out.listing);
}

/// CALL EXECUTE, one per input row, builds several statements that run in
/// order after the generating step.
#[test]
fn call_execute_per_row_generates_multiple_steps() {
    let out = run_det(
        "data seed; do i = 1 to 3; output; end; run;\n\
         data _null_; set seed; \
           call execute('data g'||left(put(i,1.))||'; x=i_val; run;'); run;\n",
    );
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    // Three datasets were generated (WORK.G1, WORK.G2, WORK.G3).
    assert!(out.log.contains("WORK.G1"), "log:\n{}", out.log);
    assert!(out.log.contains("WORK.G2"), "log:\n{}", out.log);
    assert!(out.log.contains("WORK.G3"), "log:\n{}", out.log);
}

// ---- M35.3 — %LENGTH conformity ----------------------------------------

/// %LENGTH of empty/null argument returns 1 (SAS behaviour).
#[test]
fn length_empty_returns_1() {
    let out = run_det("%put %length();");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    // %put emits the value on its own line; check for a line that is exactly "1".
    assert!(
        out.log.lines().any(|l| l == "1"),
        "expected a line '1' in log:\n{}", out.log
    );
}

/// %LENGTH of a single character returns 1.
#[test]
fn length_single_char_returns_1() {
    let out = run_det("%put %length(a);");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.log.lines().any(|l| l == "1"),
        "expected a line '1' in log:\n{}", out.log
    );
}

/// %LENGTH of "abc" returns 3.
#[test]
fn length_abc_returns_3() {
    let out = run_det("%put %length(abc);");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.log.lines().any(|l| l == "3"),
        "expected a line '3' in log:\n{}", out.log
    );
}

// ---- M35.3 — automatic macro variables ---------------------------------

/// &SYSCC, &SYSERR initial values are "0".
#[test]
fn auto_vars_status_codes_zero() {
    let out = run_det("%put &syscc; %put &syserr;");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    // Both %put emit "0" — at least two occurrences of a standalone 0.
    let count = out.log.lines().filter(|l| l.trim() == "0").count();
    assert!(count >= 2, "expected at least 2 lines of '0' in log:\n{}", out.log);
}

/// &SYSPROCESSNAME resolves to "DMS Process".
#[test]
fn auto_vars_sysprocessname() {
    let out = run_det("%put &sysprocessname;");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(out.log.contains("DMS Process"), "log:\n{}", out.log);
}

/// &SYSLAST is "_NULL_" before any step.
#[test]
fn syslast_initial_null() {
    let out = run_det("%put &syslast;");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(out.log.contains("_NULL_"), "log:\n{}", out.log);
}

/// &SYSLAST is updated to WORK.A after a DATA step creates dataset A.
#[test]
fn syslast_updated_after_data_step() {
    let out = run_det("data a; x=1; run;\n%put &syslast;");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(out.log.contains("WORK.A"), "expected WORK.A in log:\n{}", out.log);
}
