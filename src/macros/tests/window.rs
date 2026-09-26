use super::*;

#[test]
fn window_display_noted_and_consumed() {
    let (out, log) = run_logged("a %window w color=red; b %display w; c");
    assert!(
        log.contains("%WINDOW") && log.contains("%DISPLAY"),
        "got: {out}"
    );
    assert!(
        out.contains('a') && out.contains('b') && out.contains('c'),
        "got: {out}"
    );
    assert!(!out.contains("/* ERROR") && !out.contains("/* NOTE"));
}

#[test]
fn syscall_noted_and_consumed() {
    let (out, log) = run_logged("p %syscall scan(s,n,r); q");
    assert!(
        log.contains("%SYSCALL") && log.contains("not supported"),
        "got: {out}"
    );
    assert!(out.contains('p') && out.contains('q'), "got: {out}");
    assert!(!out.contains("/* ERROR") && !out.contains("/* NOTE"));
}

#[test]
fn misc_unsupported_keywords_consumed() {
    // M41.3 — `%sysmacdelete` est sorti de cette liste : il est maintenant
    // implémenté (voir `tests/syscall.rs`) et n'émet plus de NOTE.
    for (src, tag) in [
        ("%sysmstoreclear;", "%SYSMSTORECLEAR"),
        ("%syslput x=1;", "%SYSLPUT"),
        ("%sysrput x=1;", "%SYSRPUT"),
    ] {
        let (out, log) = run_logged(src);
        assert!(
            log.contains(tag) && log.contains("not supported"),
            "src {src} got: {out}"
        );
        assert!(!out.contains("/* ERROR") && !out.contains("/* NOTE"));
    }
}

#[test]
fn unknown_macro_keyword_left_verbatim() {
    // Un `%foo` inconnu (non défini, non mot-clé) reste verbatim — pas de
    // panic ni de consommation parasite.
    let out = run("%notakeyword bar");
    assert_eq!(out, "%notakeyword bar");
}
