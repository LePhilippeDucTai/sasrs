use super::*;

// --- M11.6 : %sysfunc ---

#[test]
fn sysfunc_upcase() {
    assert_eq!(run("%sysfunc(upcase(abc))"), "ABC");
}

#[test]
fn sysfunc_substr() {
    assert_eq!(run("%sysfunc(substr(abcdef,2,3))"), "bcd");
}

#[test]
fn sysfunc_with_macro_var_arg() {
    assert_eq!(run("%let w=hello; %sysfunc(upcase(&w))"), "HELLO");
}

#[test]
fn sysfunc_numeric_function() {
    assert_eq!(run("%sysfunc(length(abcd))"), "4");
}

#[test]
fn sysfunc_unknown_function_errors_no_panic() {
    let out = run("%sysfunc(nosuchfn(1))");
    assert!(
        out.contains("not supported") || out.contains("unknown"),
        "got: {out}"
    );
}

// --- M35.1 : %sysfunc délègue à la bibliothèque COMPLÈTE (plus de whitelist) ---

#[test]
fn sysfunc_non_whitelisted_reverse() {
    // REVERSE n'était PAS dans l'ancienne liste blanche.
    assert_eq!(run("%sysfunc(reverse(abc))"), "cba");
}

#[test]
fn sysfunc_non_whitelisted_repeat() {
    // REPEAT(x, 2) → 2 copies (implémentation s.repeat(n) de la lib).
    assert_eq!(run("%sysfunc(repeat(x,2))"), "xx");
}

#[test]
fn sysfunc_non_whitelisted_propcase() {
    assert_eq!(run("%sysfunc(propcase(hello world))"), "Hello World");
}

#[test]
fn sysfunc_non_whitelisted_math_sqrt() {
    assert_eq!(run("%sysfunc(sqrt(16))"), "4");
}

// --- M35.1 : argument de format optionnel ---

#[test]
fn sysfunc_format_dollar() {
    // sum(1000, 234.5) = 1234.5 reformaté en dollar10.2 ; les blancs de tête
    // sont retirés pour l'insertion macro.
    assert_eq!(run("%sysfunc(sum(1000,234.5), dollar10.2)"), "$1,234.50");
}

#[test]
fn sysfunc_format_round_width() {
    // round(3.7) = 4, formaté en 8. → "4" (blancs de tête retirés).
    assert_eq!(run("%sysfunc(round(3.7), 8.)"), "4");
}

#[test]
fn sysfunc_format_date9() {
    assert_eq!(run("%sysfunc(mdy(1,1,2020), date9.)"), "01JAN2020");
}

#[test]
fn sysfunc_no_format_unchanged() {
    // Sans format : comportement identique à avant (texte brut).
    assert_eq!(run("%sysfunc(mdy(1,1,2020))"), "21915");
}

// --- M11.6 : automatic macro variables (deterministic frozen) ---

#[test]
fn auto_var_sysdate9() {
    assert_eq!(expand("&sysdate9"), "01JAN1960");
}

#[test]
fn auto_var_sysver() {
    assert_eq!(expand("&sysver"), "9.4");
}

#[test]
fn auto_var_systime_and_sysday() {
    assert_eq!(expand("&systime &sysday"), "00:00 Friday");
}

// --- M11.6 : %str / %nrstr quoting ---

#[test]
fn str_masks_semicolon_and_comma() {
    // Le `;` et le `,` internes sont littéraux (non terminateurs).
    assert_eq!(expand("%str(a;b,c)"), "a;b,c");
}

#[test]
fn str_semicolon_does_not_terminate() {
    // Sans %str, `;` terminerait ; avec %str il reste dans la valeur.
    assert_eq!(expand("%let v=%str(a;b); &v"), "a;b");
}

#[test]
fn str_resolves_ampersand() {
    // %str masque la ponctuation mais &x reste résolu.
    assert_eq!(expand("%let x=Z; %str(&x;y)"), "Z;y");
}

#[test]
fn nrstr_leaves_triggers_unresolved() {
    // %nrstr masque & et % : %macro et &x ne sont pas résolus.
    assert_eq!(expand("%nrstr(%macro &x)"), "%macro &x");
}

// --- M12.2 : fonctions chaîne macro simples ---

#[test]
fn macro_fn_upcase_lowcase() {
    assert_eq!(expand("%upcase(abc)"), "ABC");
    assert_eq!(expand("%lowcase(ABC)"), "abc");
}

#[test]
fn macro_fn_substr() {
    assert_eq!(expand("%substr(abcdef,2,3)"), "bcd");
    // Sans longueur : jusqu'à la fin.
    assert_eq!(expand("%substr(abcdef,4)"), "def");
}

#[test]
fn macro_fn_scan() {
    assert_eq!(expand("%scan(a.b.c,2,.)"), "b");
    // Délimiteurs par défaut (blanc).
    assert_eq!(expand("%scan(one two three,3)"), "three");
    // Index depuis la fin.
    assert_eq!(expand("%scan(a.b.c,-1,.)"), "c");
}

#[test]
fn macro_fn_index_length() {
    assert_eq!(expand("%index(abcdef,cd)"), "3");
    assert_eq!(expand("%index(abcdef,zz)"), "0");
    assert_eq!(expand("%length(abcd)"), "4");
}

#[test]
fn macro_fn_resolves_refs_in_args() {
    // Les `&refs` des arguments sont résolus avant calcul.
    assert_eq!(expand("%let w=hello; %upcase(&w)"), "HELLO");
}

// --- M12.2 : %superq ---

#[test]
fn superq_returns_value_without_resolving() {
    // v = "a&b" avec b indéfini ; %superq(v) rend "a&b" littéral, sans
    // tenter de résoudre &b (donc pas d'expansion). On utilise %nrstr pour
    // stocker la valeur sans déclencher la résolution au moment du %let.
    assert_eq!(expand("%let v=%nrstr(a&b); %superq(v)"), "a&b");
}

#[test]
fn superq_undefined_is_empty() {
    assert_eq!(expand("[%superq(nope)]"), "[]");
}

// --- M12.2 : %bquote / %nrbquote ---

#[test]
fn bquote_masks_comma_and_semicolon() {
    // `,` et `;` restent littéraux dans la sortie finale.
    assert_eq!(expand("%bquote(a,b;c)"), "a,b;c");
}

#[test]
fn bquote_semicolon_does_not_terminate_let() {
    assert_eq!(expand("%let v=%bquote(a;b); &v"), "a;b");
}

#[test]
fn bquote_unmatched_quote_ok() {
    // Une quote non appariée dans l'argument ne fait pas planter : elle est
    // traitée comme un caractère ordinaire puis masquée (littérale en sortie).
    assert_eq!(expand("%bquote(it's a test)"), "it's a test");
}

#[test]
fn bquote_unmatched_paren_stays_verbatim() {
    // Parenthèse non appariée → l'appel `%bquote` n'est pas reconnu comme
    // équilibré ; on ne plante pas, le texte reste verbatim (pas d'erreur).
    let s = expand("%bquote(a (b)");
    assert!(s.contains("%bquote"));
}

#[test]
fn bquote_resolves_then_masks() {
    // &x est résolu, puis le `;` reste littéral.
    assert_eq!(expand("%let x=Z; %bquote(&x;y)"), "Z;y");
}

#[test]
fn nrbquote_masks_triggers_in_result() {
    // nrbquote masque les `&`/`%` résiduels : &z (indéfini) reste littéral
    // et inerte. (Après résolution &z est inchangé, puis masqué.)
    assert_eq!(expand("%nrbquote(a&z b)"), "a&z b");
}

// --- M12.2 : variantes %q* (résultat masqué) ---

#[test]
fn qsysfunc_upcase_masked() {
    assert_eq!(expand("%qsysfunc(upcase(abc))"), "ABC");
}

#[test]
fn qupcase_qlowcase() {
    assert_eq!(expand("%qupcase(abc)"), "ABC");
    assert_eq!(expand("%qlowcase(ABC)"), "abc");
}

#[test]
fn qupcase_masks_residual_ampersand() {
    // x indéfini : &x reste, est mis en MAJ (inchangé), puis masqué donc
    // inerte ; la sortie finale (unmask) montre `&X` littéral.
    assert_eq!(expand("%qupcase(a&x)"), "A&X");
}

#[test]
fn qsubstr_qscan() {
    assert_eq!(expand("%qsubstr(abcdef,2,3)"), "bcd");
    assert_eq!(expand("%qscan(a.b.c,2,.)"), "b");
}

// --- M19.1 : %unquote ---

#[test]
fn unquote_reenables_resolution_after_nrstr() {
    // %nrstr masque le `&` : sans %unquote, `&x` reste littéral. %unquote
    // ré-active la résolution → la valeur de x est splicée.
    assert_eq!(expand("%let x=hi; %unquote(%nrstr(&x))"), "hi");
}

#[test]
fn unquote_roundtrip_plain_text() {
    // Texte sans déclencheur : %unquote est l'identité.
    assert_eq!(expand("%unquote(abc)"), "abc");
}

// --- M41.1 : %quote / %nrquote (quoting exécution, échappements `%`) ---

#[test]
fn quote_resolves_then_masks() {
    // Comme %bquote : SAS résout D'ABORD &x, PUIS masque le résultat — le `;`
    // devient littéral (il ne termine rien en aval).
    assert_eq!(expand("%let x=Z; %quote(&x;y)"), "Z;y");
}

#[test]
fn quote_semicolon_does_not_terminate_let() {
    // Le `;` masqué par %quote ne clôt pas le %let (région sautée).
    assert_eq!(expand("%let v=%quote(a;b); &v"), "a;b");
}

#[test]
fn quote_escaped_quote_is_literal() {
    // Quote NON APPARIÉE : SAS exige l'échappement `%'` — le `%` disparaît et
    // la quote devient un caractère littéral (masqué).
    assert_eq!(expand("%quote(it%'s)"), "it's");
}

#[test]
fn quote_escaped_paren_bounds_correctly() {
    // `%(` ne compte pas dans l'équilibrage : l'appel est borné sur la `)`
    // finale et la parenthèse échappée reste du texte.
    assert_eq!(expand("%quote(a%(b)"), "a(b");
}

#[test]
fn quote_escaped_percent_is_literal() {
    // `%%` → un `%` littéral, masqué : il ne redéclenche pas le processeur.
    assert_eq!(expand("%quote(50%%)"), "50%");
}

#[test]
fn quote_escaped_paren_in_let_value() {
    // Le saut de région de %let doit lui aussi ignorer la parenthèse échappée
    // (lecture via read_balanced_parens_pct).
    assert_eq!(expand("%let v=%quote(a%(b); [&v]"), "[a(b]");
}

#[test]
fn nrquote_masks_triggers_in_result() {
    // Forme NR : les `&`/`%` RÉSIDUELS du résultat (ici &z indéfini) sont
    // masqués en plus — inertes en aval, littéraux en sortie.
    assert_eq!(expand("%nrquote(a&z b)"), "a&z b");
}

#[test]
fn nrquote_resolves_defined_refs_first() {
    // Contrairement à %nrstr (compilation), %nrquote RÉSOUT d'abord : une
    // &ref définie est bien remplacée avant masquage.
    assert_eq!(expand("%let x=ok; %nrquote(&x!)"), "ok!");
}

// --- M41.1 : interactions de la famille %bquote (imbrication, %unquote,
// --- %if, argument d'appel de macro, idempotence) ---

#[test]
fn bquote_nested_is_idempotent() {
    // Imbrication %bquote(%bquote(...)) : le masquage est IDEMPOTENT — les
    // sentinelles internes traversent le masquage externe inchangées.
    assert_eq!(expand("%bquote(%bquote(a;b))"), "a;b");
}

#[test]
fn nrbquote_over_bquote_masks_residual_trigger() {
    // %bquote laisse le `&z` (indéfini) ACTIF ; le %nrbquote englobant le
    // masque à son tour — littéral en sortie.
    assert_eq!(expand("%nrbquote(%bquote(a&z))"), "a&z");
}

#[test]
fn unquote_reverses_bquote_family() {
    // Aller-retour : %nrstr masque `&x`, %nrbquote re-masque (idempotent),
    // %unquote dé-masque et RÉ-ACTIVE la résolution → valeur de x.
    assert_eq!(expand("%let x=hi; %unquote(%nrbquote(%nrstr(&x)))"), "hi");
}

#[test]
fn bquote_result_in_if_condition() {
    // Le résultat d'un %bquote participe normalement à une comparaison %if.
    assert_eq!(
        expand("%let x=3; %if %bquote(&x) = 3 %then Y; %else N;"),
        "Y;"
    );
}

#[test]
fn bquote_as_macro_argument_protects_semicolon() {
    // En argument d'appel de macro, le `;` masqué ne coupe rien : la valeur
    // transite intacte jusqu'au corps.
    let s = expand("%macro m(a); [&a] %mend; %m(%bquote(x;y))");
    assert_eq!(s.trim(), "[x;y]");
}

#[test]
fn bquote_in_let_then_reused() {
    // Valeur %bquote stockée par %let puis relue : les sentinelles survivent
    // dans la table des symboles et l'unmask final rétablit les littéraux.
    assert_eq!(expand("%let v=%bquote(p,q;r); <&v>"), "<p,q;r>");
}

// --- M41.1 : complétude du jeu de caractères masqués (liste SAS) ---

#[test]
fn mask_covers_caret_notsign_hash() {
    // SAS masque aussi `^`, `¬` (graphies du NOT) et `#` (IN), comme `~` :
    // aller-retour masque → unmask = identité, et le caractère EST masqué.
    for c in ['^', '¬', '#'] {
        let m = MacroEngine::mask_special(&c.to_string(), false);
        assert_ne!(m, c.to_string(), "`{c}` doit être masqué");
        assert_eq!(MacroEngine::unmask(&m), c.to_string());
    }
}

#[test]
fn bquote_caret_roundtrips_verbatim() {
    // `^` masqué par %bquote puis rétabli à l'unmask final (littéral).
    assert_eq!(expand("%bquote(a^b¬c#d)"), "a^b¬c#d");
}

#[test]
fn unquote_reenables_macro_call() {
    // %nrstr masque le `%` d'un appel ; %unquote le ré-active → la macro
    // s'exécute et émet son corps.
    assert_eq!(expand("%macro m; got %mend; %unquote(%nrstr(%m))"), "got");
}

// --- M19.1 : %cmpres / %qcmpres ---

#[test]
fn cmpres_compresses_internal_blanks() {
    assert_eq!(expand("%cmpres(a    b     c)"), "a b c");
}

#[test]
fn cmpres_trims_edges() {
    assert_eq!(expand("%cmpres(   hello   world   )"), "hello world");
}

#[test]
fn cmpres_resolves_refs() {
    assert_eq!(expand("%let v=  x   y  ; %cmpres(&v)"), "x y");
}

#[test]
fn qcmpres_masks_result() {
    // Le résultat de %qcmpres est masqué : un `;` interne ne termine pas le
    // %let. La valeur stockée (puis ré-émise) garde le `;` littéral.
    assert_eq!(expand("%let v=%qcmpres(a ;  b); &v"), "a ; b");
}

// --- M19.1 : %symexist ---

#[test]
fn symexist_found() {
    assert_eq!(expand("%let a=1; %symexist(a)"), "1");
}

#[test]
fn symexist_not_found() {
    assert_eq!(expand("%symexist(nope)"), "0");
}

#[test]
fn symexist_accepts_ampersand_name() {
    // %symexist(&which) : &which désigne le NOM à tester.
    assert_eq!(expand("%let a=1; %let which=a; %symexist(&which)"), "1");
}

// --- M19.1 : %sysmexist ---

#[test]
fn sysmexist_defined_macro() {
    assert_eq!(expand("%macro foo; %mend; %sysmexist(foo)"), "1");
}

#[test]
fn sysmexist_undefined_macro() {
    assert_eq!(expand("%sysmexist(bar)"), "0");
}

// --- M19.1 : %sysget (env var posée en mémoire dans le test) ---

#[test]
fn sysget_reads_env_var() {
    // SAFETY: test mono-thread sur une variable d'env dédiée à ce test ;
    // posée puis retirée localement.
    unsafe {
        std::env::set_var("SASRS_TEST_VAR_M19", "hello_env");
    }
    assert_eq!(expand("%sysget(SASRS_TEST_VAR_M19)"), "hello_env");
    unsafe {
        std::env::remove_var("SASRS_TEST_VAR_M19");
    }
}

#[test]
fn sysget_unset_is_empty() {
    // SAFETY: variable d'env dédiée, jamais posée ailleurs.
    unsafe {
        std::env::remove_var("SASRS_DEFINITELY_UNSET_M19");
    }
    assert_eq!(expand("%sysget(SASRS_DEFINITELY_UNSET_M19)"), "");
}

// --- M19.1 : %sysevalf (évaluation flottante) ---

#[test]
fn sysevalf_float_division() {
    assert_eq!(expand("%sysevalf(7/2)"), "3.5");
}

#[test]
fn sysevalf_vs_eval_integer_division() {
    // %eval tronque (entier) ; %sysevalf est réel.
    assert_eq!(expand("%eval(7/2)"), "3");
    assert_eq!(expand("%sysevalf(7/2)"), "3.5");
}

#[test]
fn sysevalf_decimal_literals() {
    assert_eq!(expand("%sysevalf(0.5 + 0.25)"), "0.75");
}

#[test]
fn sysevalf_integer_result_has_no_decimals() {
    assert_eq!(expand("%sysevalf(4/2)"), "2");
}

#[test]
fn sysevalf_conv_boolean() {
    assert_eq!(expand("%sysevalf(3.5, boolean)"), "1");
    assert_eq!(expand("%sysevalf(0, boolean)"), "0");
}

#[test]
fn sysevalf_conv_ceil_floor_integer() {
    assert_eq!(expand("%sysevalf(7/2, ceil)"), "4");
    assert_eq!(expand("%sysevalf(7/2, floor)"), "3");
    assert_eq!(expand("%sysevalf(7/2, integer)"), "3");
    assert_eq!(expand("%sysevalf(-7/2, integer)"), "-3");
    assert_eq!(expand("%sysevalf(-7/2, floor)"), "-4");
}

#[test]
fn sysevalf_resolves_refs() {
    assert_eq!(expand("%let n=5; %sysevalf(&n / 2)"), "2.5");
}

#[test]
fn sysevalf_power_is_real() {
    assert_eq!(expand("%sysevalf(2 ** 0.5)"), f64::sqrt(2.0).to_string());
}

#[test]
fn sysevalf_syntax_error_no_panic() {
    let out = expand("%sysevalf(2 + + )");
    assert!(out.contains("ERROR"), "got: {out}");
}
