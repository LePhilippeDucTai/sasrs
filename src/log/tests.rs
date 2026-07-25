use super::*;

#[test]
fn echo_and_messages() {
    let mut log = LogWriter::new(true);
    log.echo_source(&["data a;", "run;"]);
    log.note("The data set WORK.A has 1 observations and 1 variables.");
    log.error("Syntax error.");
    let s = log.into_string();
    assert!(s.contains("1     data a;"));
    assert!(s.contains("2     run;"));
    assert!(s.contains("NOTE: The data set WORK.A"));
    assert!(s.contains("ERROR: Syntax error."));
}

#[test]
fn deterministic_timing() {
    let mut log = LogWriter::new(true);
    log.step_used("DATA statement", &StepTimer::start());
    let s = log.into_string();
    assert!(s.contains("real time           0.00 seconds"));
    assert!(s.contains("cpu time            0.00 seconds"));
}
