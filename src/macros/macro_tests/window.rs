use super::*;

#[test]
fn window_display_noted_and_consumed() {
    let out = run("a %window w color=red; b %display w; c");
    assert!(out.contains("%WINDOW") && out.contains("%DISPLAY"), "got: {out}");
    assert!(out.contains('a') && out.contains('b') && out.contains('c'), "got: {out}");
}

#[test]
fn syscall_noted_and_consumed() {
    let out = run("p %syscall scan(s,n,r); q");
    assert!(out.contains("%SYSCALL") && out.contains("not supported"), "got: {out}");
    assert!(out.contains('p') && out.contains('q'), "got: {out}");
}

#[test]
fn misc_unsupported_keywords_consumed() {
    for (src, tag) in [
        ("%sysmacdelete m;", "%SYSMACDELETE"),
        ("%sysmstoreclear;", "%SYSMSTORECLEAR"),
        ("%syslput x=1;", "%SYSLPUT"),
        ("%sysrput x=1;", "%SYSRPUT"),
    ] {
        let out = run(src);
        assert!(out.contains(tag) && out.contains("not supported"), "src {src} got: {out}");
    }
}

#[test]
fn unknown_macro_keyword_left_verbatim() {
    // Un `%foo` inconnu (non défini, non mot-clé) reste verbatim — pas de
    // panic ni de consommation parasite.
    let out = run("%notakeyword bar");
    assert_eq!(out, "%notakeyword bar");
}
