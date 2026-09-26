use super::*;

// SAS Macro Language Reference: %EVAL, %SYSEVALF, %SYSFUNC, %DO, %GOTO.
// https://support.sas.com/documentation/cdl/en/mcrolref/67912/HTML/default/titlepage.htm
// Diagnostic oracles: https://support.sas.com/kb/31/012.html
// https://support.sas.com/kb/43764.html
#[test]
fn macro_diag_errors_use_channel_without_generated_comments() {
    for (source, message) in [
        ("%eval(1/0)", "Division by zero"),
        ("%eval(abc+1)", "character operand"),
        ("%sysevalf(1/0)", "%SYSEVALF"),
        ("%sysevalf(1,invalid)", "conversion operand"),
        ("%sysfunc(unknown_function(1))", "%SYSFUNC"),
        ("%do i=1 %to 2 %by 0; %end;", "step is zero"),
        ("%do %while(1); %end;", "runaway guard"),
        ("%macro again; %again %mend; %again", "recursion limit"),
        ("%macro m; %goto missing; %mend; %m", "%GOTO label MISSING"),
        (
            "%macro m(a); NEVER %mend; %m(1,2)",
            "More positional parameters found than defined.",
        ),
        (
            "%macro m(a=); NEVER %mend; %m(b=1)",
            "The keyword parameter B was not defined with the macro.",
        ),
    ] {
        let mut engine = MacroEngine::new(true);
        let code = engine.expand_open_code(source);
        assert!(code.trim().is_empty(), "{source}: {code}");
        let entries = engine.take_pending_log();
        assert!(
            matches!(entries.as_slice(), [MacroLogEntry::Error(text)] if text.contains(message)),
            "{source}: {entries:?}"
        );
        assert!(engine.take_pending_log().is_empty());
    }
}

#[test]
fn macro_diag_include_errors_use_channel() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine_in(dir.path());
    assert!(engine.expand_open_code("%include 'absent.sas';").is_empty());
    assert!(
        matches!(engine.take_pending_log().as_slice(), [MacroLogEntry::Error(s)] if s.contains("cannot read"))
    );
    write_file(dir.path(), "cycle.sas", "%include 'cycle.sas';");
    assert!(engine.expand_open_code("%include 'cycle.sas';").is_empty());
    assert!(
        matches!(engine.take_pending_log().as_slice(), [MacroLogEntry::Error(s)] if s.contains("nesting limit"))
    );
}

#[test]
fn macro_diag_notes_are_typed() {
    let mut engine = MacroEngine::new(true);
    let code =
        engine.expand_open_code("%sysexec echo ignored; %window w; %syscall unsupported(x);");
    assert!(code.trim().is_empty());
    let entries = engine.take_pending_log();
    assert_eq!(entries.len(), 3);
    assert!(
        entries
            .iter()
            .all(|entry| matches!(entry, MacroLogEntry::Note(_)))
    );
}

#[test]
fn macro_diag_quoting_comments_and_indirection() {
    // SAS %NRSTR protects references; double quotes allow resolution, single
    // quotes and block/macro comments do not. Missing references stay verbatim.
    // https://support.sas.com/documentation/cdl/en/mcrolref/61885/HTML/default/a001061290.htm
    let mut engine = MacroEngine::new(true);
    let code = engine.expand_open_code("'&missing %missing' /* &missing %missing */ %* &missing %missing; %nrstr(&missing %missing)");
    assert!(code.contains("'&missing %missing'"));
    assert!(engine.take_pending_log().is_empty());
    assert_eq!(engine.expand_open_code("%let i=1; %let v1=ok; &&v&i"), "ok");
    assert!(engine.take_pending_log().is_empty());
    assert_eq!(
        engine.expand_open_code("\"It's &missing.\" %missing"),
        "\"It's &missing.\" %missing"
    );
    assert_eq!(
        engine.take_pending_log(),
        vec![
            MacroLogEntry::Warning("Apparent symbolic reference MISSING not resolved.".into()),
            MacroLogEntry::Warning("Apparent invocation of macro MISSING not resolved.".into()),
        ]
    );
}

#[test]
fn macro_diag_abort_in_loop_exits_without_runaway_error() {
    let mut engine = MacroEngine::new(true);
    engine.expand_open_code("%macro m; %do %while(1); %abort return 8; %end; %mend; %m");
    assert_eq!(
        engine.take_abort_request(),
        Some(AbortKind::Return(Some(8)))
    );
    assert_eq!(engine.take_pending_log().len(), 1);
    assert_eq!(engine.take_abort_request(), None);
    assert_eq!(engine.expand_open_code("%eval(2+2)"), "4");
}
