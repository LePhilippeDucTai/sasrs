use super::*;

// ── Fonctions PRX (M40.1) ────────────────────────────────────────────────
//
// Les résultats attendus sont les valeurs SAS 9.4 documentées (positions
// 1-based, 0 = pas de match, PRXPOSN extrait le groupe du dernier match).

#[test]
fn prxparse_returns_sequential_ids() {
    let mut c = ctx();
    assert_eq!(invoke_ctx("PRXPARSE", &[chr("/a/")], &mut c), num(1.0));
    assert_eq!(invoke_ctx("PRXPARSE", &[chr("/b/")], &mut c), num(2.0));
}

#[test]
fn prxparse_same_literal_reuses_id() {
    // SAS ne compile un littéral constant qu'une fois : le cache par texte
    // rend l'usage naïf (sans `if _n_=1`) équivalent.
    let mut c = ctx();
    let id1 = invoke_ctx("PRXPARSE", &[chr("/\\d+/")], &mut c);
    let id2 = invoke_ctx("PRXPARSE", &[chr("/\\d+/")], &mut c);
    assert_eq!(id1, id2);
}

#[test]
fn prxmatch_position_is_one_based() {
    let mut c = ctx();
    let id = invoke_ctx("PRXPARSE", &[chr("/World/")], &mut c);
    assert_eq!(
        invoke_ctx("PRXMATCH", &[id.clone(), chr("Hello World")], &mut c),
        num(7.0)
    );
    // Sensible à la casse sans flag `i` → 0 (pas de match).
    assert_eq!(
        invoke_ctx("PRXMATCH", &[id, chr("hello world")], &mut c),
        num(0.0)
    );
}

#[test]
fn prxmatch_flag_i_is_case_insensitive() {
    let mut c = ctx();
    let id = invoke_ctx("PRXPARSE", &[chr("/world/i")], &mut c);
    assert_eq!(
        invoke_ctx("PRXMATCH", &[id, chr("Hello WORLD")], &mut c),
        num(7.0)
    );
}

#[test]
fn prxmatch_accepts_pattern_text_directly() {
    // Sémantique SAS : le 1er argument peut être le pattern lui-même.
    let mut c = ctx();
    assert_eq!(
        invoke_ctx("PRXMATCH", &[chr("/b+/"), chr("abbb")], &mut c),
        num(2.0)
    );
}

#[test]
fn prxmatch_supports_lookahead() {
    // Lookaround Perl (moteur fancy-regex) : `q(?=u)`.
    let mut c = ctx();
    assert_eq!(
        invoke_ctx("PRXMATCH", &[chr("/q(?=u)/"), chr("Iraq quit")], &mut c),
        num(6.0)
    );
}

#[test]
fn prxchange_all_occurrences_and_once() {
    let mut c = ctx();
    let id = invoke_ctx("PRXPARSE", &[chr("s/a/X/")], &mut c);
    assert_eq!(
        invoke_ctx("PRXCHANGE", &[id.clone(), num(-1.0), chr("banana")], &mut c),
        chr("bXnXnX")
    );
    assert_eq!(
        invoke_ctx("PRXCHANGE", &[id, num(1.0), chr("banana")], &mut c),
        chr("bXnana")
    );
}

#[test]
fn prxchange_backreference_dollar() {
    // Références `$1`/`$2` du remplacement (oracle SAS : inversion de mots).
    let mut c = ctx();
    assert_eq!(
        invoke_ctx(
            "PRXCHANGE",
            &[chr("s/(\\w+) (\\w+)/$2 $1/"), num(-1.0), chr("Hello World")],
            &mut c
        ),
        chr("World Hello")
    );
}

#[test]
fn prxchange_backslash_reference() {
    // Forme Perl `\1` équivalente à `$1`.
    let mut c = ctx();
    assert_eq!(
        invoke_ctx(
            "PRXCHANGE",
            &[chr("s/(b+)/[\\1]/"), num(-1.0), chr("abba")],
            &mut c
        ),
        chr("a[bb]a")
    );
}

#[test]
fn prxparse_invalid_pattern_missing_plus_single_error() {
    let mut c = ctx();
    let v = invoke_ctx("PRXPARSE", &[chr("/(/")], &mut c);
    assert!(matches!(v, Value::Missing(_)));
    assert_eq!(c.runtime_errors.len(), 1);
    assert!(
        c.runtime_errors[0].contains("/(/"),
        "{:?}",
        c.runtime_errors
    );
    // Même texte invalide re-parsé : toujours missing, mais UNE seule ERROR.
    let v2 = invoke_ctx("PRXPARSE", &[chr("/(/")], &mut c);
    assert!(matches!(v2, Value::Missing(_)));
    assert_eq!(c.runtime_errors.len(), 1);
}

#[test]
fn prxmatch_invalid_id_is_fatal() {
    let mut c = ctx();
    let v = invoke_ctx("PRXMATCH", &[num(99.0), chr("abc")], &mut c);
    assert!(matches!(v, Value::Missing(_)));
    let msg = c.fatal.take().expect("expected fatal error").to_string();
    assert!(msg.contains("Invalid regular expression id"), "{msg}");
}

#[test]
fn prxposn_and_prxparen_after_match() {
    let mut c = ctx();
    let id = invoke_ctx("PRXPARSE", &[chr("/(\\d+)-(\\d+)/")], &mut c);
    let src = chr("abc 12-345");
    assert_eq!(
        invoke_ctx("PRXMATCH", &[id.clone(), src.clone()], &mut c),
        num(5.0)
    );
    assert_eq!(
        invoke_ctx("PRXPOSN", &[id.clone(), num(1.0), src.clone()], &mut c),
        chr("12")
    );
    assert_eq!(
        invoke_ctx("PRXPOSN", &[id.clone(), num(2.0), src.clone()], &mut c),
        chr("345")
    );
    // Groupe 0 = match entier.
    assert_eq!(
        invoke_ctx("PRXPOSN", &[id.clone(), num(0.0), src], &mut c),
        chr("12-345")
    );
    // PRXPAREN : plus grand groupe ayant participé.
    assert_eq!(invoke_ctx("PRXPAREN", &[id], &mut c), num(2.0));
}

#[test]
fn prxposn_without_match_is_blank() {
    let mut c = ctx();
    let id = invoke_ctx("PRXPARSE", &[chr("/(z)/")], &mut c);
    assert_eq!(
        invoke_ctx("PRXMATCH", &[id.clone(), chr("abc")], &mut c),
        num(0.0)
    );
    assert_eq!(
        invoke_ctx("PRXPOSN", &[id, num(1.0), chr("abc")], &mut c),
        chr("")
    );
}

#[test]
fn prx_alternate_delimiter_and_flag_x() {
    // Délimiteur libre (`#`) + flag étendu `x` (blancs ignorés).
    let mut c = ctx();
    assert_eq!(
        invoke_ctx("PRXMATCH", &[chr("#a b c#x"), chr("xxabc")], &mut c),
        num(3.0)
    );
}
