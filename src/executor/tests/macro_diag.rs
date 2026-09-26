use super::run_det;

// Independent oracles: SAS Macro Language Reference, %LENGTH / %ABORT,
// https://support.sas.com/documentation/cdl/en/mcrolref/61885/HTML/default/a000543620.htm
// https://support.sas.com/kb/23/addl/fusion23211_1_abort.html
// Diagnostic spelling: SAS Usage Notes 31012 and 43764.
#[test]
fn macro_diag_errors_are_counted_and_ordered() {
    let result = run_det("%put BEFORE; %let bad=%eval(1/0); %put AFTER;");
    assert_eq!(result.exit_code, 2, "{}", result.log);
    assert_eq!(result.log.matches("ERROR:").count(), 1);
    assert!(result.log.find("BEFORE").unwrap() < result.log.find("ERROR:").unwrap());
    assert!(result.log.find("ERROR:").unwrap() < result.log.find("AFTER").unwrap());
    assert!(!result.log.contains("/* ERROR"));
}

#[test]
fn macro_diag_abort_stops_program_and_returns_code() {
    for (option, code) in [
        ("return 8", 8),
        // J02-P10's independent run_failure_tests::
        // exit_code_requested_zero_preserves_counted_failure fixes this policy:
        // an explicit zero cannot hide the counted %ABORT error.
        ("return 0", 1),
        ("return", 4),
        ("abend 12", 12),
        ("abend", 5),
        ("cancel", 2),
        ("", 2),
    ] {
        let result = run_det(&format!(
            "%macro stop; %abort {option}; %mend; %stop\ndata _null_; put 'NEVER'; run;\n%put LATER;"
        ));
        assert_eq!(result.exit_code, code, "{option}: {}", result.log);
        assert!(!result.log.contains("NEVER"), "{}", result.log);
        assert!(!result.log.contains("LATER"), "{}", result.log);
    }
}

#[test]
fn macro_diag_unresolved_warnings() {
    let result = run_det("%put &missing; %put %unknown;");
    assert_eq!(result.exit_code, 1, "{}", result.log);
    assert_eq!(result.log.matches("WARNING:").count(), 2);
    assert!(
        result
            .log
            .contains("WARNING: Apparent symbolic reference MISSING not resolved.")
    );
    assert!(
        result
            .log
            .contains("WARNING: Apparent invocation of macro UNKNOWN not resolved.")
    );
}

#[test]
fn macro_diag_length_null() {
    let result = run_det("%let empty=; %put [%length()][%length(&empty)][%length(abc)];");
    assert_eq!(result.exit_code, 0, "{}", result.log);
    assert!(result.log.contains("[0][0][3]"));
}

#[test]
fn macro_diag_abort_from_call_execute_stops_outer_program() {
    let result = run_det("data _null_; call execute('%abort return 8;'); run;\n%put NEVER;");
    assert_eq!(result.exit_code, 8, "{}", result.log);
    assert!(!result.log.contains("NEVER"), "{}", result.log);
}

#[test]
fn macro_diag_abort_clears_queued_code() {
    let result = run_det("%call execute(%nrstr(%put NEVER;)); %abort return 8;\n%put LATER;");
    assert_eq!(result.exit_code, 8, "{}", result.log);
    assert!(!result.log.contains("NEVER"), "{}", result.log);
    assert!(!result.log.contains("LATER"), "{}", result.log);
}

#[test]
fn macro_diag_notes_do_not_raise_exit_code() {
    let result = run_det("%sysexec ignored; %window w; %syscall unsupported(x);");
    assert_eq!(result.exit_code, 0, "{}", result.log);
    assert_eq!(result.log.matches("NOTE:").count(), 3);
}
