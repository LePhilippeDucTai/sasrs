use super::*;

#[test]
fn segmenter_splits_on_run() {
    let segs = segments("data a; x=1; run; %put &x;");
    assert_eq!(segs.len(), 2);
    assert_eq!(segs[0], "data a; x=1; run;");
    assert_eq!(segs[1], " %put &x;");
}

#[test]
fn segmenter_ignores_semicolon_in_macro_def() {
    let segs = segments("%macro m; x=1; %mend; data a; run;");
    // Un seul segment : pas de run; avant la fin de la def, puis run;.
    assert_eq!(segs.len(), 1);
}

#[test]
fn segmenter_ignores_semicolon_in_string() {
    let segs = segments("data a; t='x;y'; run; data b; run;");
    assert_eq!(segs.len(), 2);
    assert_eq!(segs[0], "data a; t='x;y'; run;");
}

#[test]
fn segmenter_trailing_open_code() {
    let segs = segments("%let x=1; %put &x;");
    // Pas de run; → un seul segment.
    assert_eq!(segs.len(), 1);
}

#[test]
fn let_then_ref() {
    assert_eq!(run("%let x = 5; y = &x;"), "y = 5;");
}

#[test]
fn dot_terminator() {
    // Un seul point consommé : &lib. -> work, puis `.a` reste.
    assert_eq!(run("%let lib = work; data &lib..a;"), "data work.a;");
}

#[test]
fn rhs_resolution() {
    assert_eq!(run("%let a = 1; %let b = &a; z = &b;"), "z = 1;");
}

#[test]
fn case_insensitive() {
    assert_eq!(run("%let Foo = 7; w = &FOO;"), "w = 7;");
}

#[test]
fn undefined_left_verbatim() {
    assert_eq!(run("x = &zzz;"), "x = &zzz;");
}

#[test]
fn undefined_macro_call_left_verbatim() {
    // %zzz non défini : laissé verbatim.
    assert_eq!(run("a %zzz b"), "a %zzz b");
    assert_eq!(run("a %zzz(1) b"), "a %zzz(1) b");
}

#[test]
fn bare_ampersand_untouched() {
    assert_eq!(run("if a & b;"), "if a & b;");
}

#[test]
fn newline_after_let_preserved() {
    assert_eq!(run("%let x = 5;\ny = &x;"), "\ny = 5;");
}

// --- M11.2 : %macro / invocation / %local / %global ---

#[test]
fn macro_positional_and_default() {
    assert_eq!(run("%macro p(a,b=2); x=&a+&b; %mend; %p(1)"), "x=1+2;");
}

#[test]
fn macro_positional_default_mix() {
    // a fourni positionnellement, b prend son défaut.
    assert_eq!(run("%macro p(a,b=2,c=3); &a-&b-&c; %mend; %p(7)"), "7-2-3;");
}

#[test]
fn macro_keyword_override() {
    assert_eq!(run("%macro p(a,b=2); x=&a+&b; %mend; %p(1,b=9)"), "x=1+9;");
}

#[test]
fn macro_too_few_args_uses_empty_for_positional() {
    // a non fourni -> chaîne vide ; b défaut. (Corps trimé aux bords.)
    assert_eq!(run("%macro p(a,b=2); [&a][&b] %mend; %p()"), "[][2]");
}

#[test]
fn macro_too_many_positional_args_ignored() {
    // 2e positionnel excédentaire ignoré (un seul paramètre).
    assert_eq!(run("%macro p(a); val=&a; %mend; %p(1,2,3)"), "val=1;");
}

#[test]
fn macro_definition_emits_nothing() {
    // La seule définition ne produit aucune sortie (hormis nl préservé).
    assert_eq!(run("%macro q(x); y=&x; %mend;"), "");
    assert_eq!(run("%macro q(x); y=&x; %mend;\n"), "\n");
}

#[test]
fn macro_nested_calls_another_macro() {
    let src = "%macro inner(z); [&z] %mend; \
               %macro outer(a); pre %inner(&a) post %mend; \
               %outer(7)";
    assert_eq!(run(src), "pre [7] post");
}

#[test]
fn macro_call_without_parens() {
    // Appel sans parenthèses : le `;` qui suit termine l'appel (consommé),
    // corps `hello` trimé puis collé à `world`.
    assert_eq!(run("%macro hi; hello %mend; %hi;world"), "helloworld");
}

#[test]
fn macro_mend_with_name() {
    assert_eq!(run("%macro p(a); &a %mend p; %p(ok)"), "ok");
}

#[test]
fn local_does_not_leak() {
    // %local v confine le %let à la macro : v reste indéfini en open code.
    let src = "%macro m; %local v; %let v = inside; got=&v; %mend; \
               %m got2=&v;";
    // Dans la macro &v -> inside ; après, &v indéfini -> verbatim.
    assert_eq!(run(src), "got=inside; got2=&v;");
}

#[test]
fn global_let_in_macro_leaks() {
    // Sans %local, le %let crée un global visible après l'appel.
    let src = "%macro m; %let g = out; in=&g; %mend; %m after=&g;";
    assert_eq!(run(src), "in=out; after=out;");
}

#[test]
fn global_decl_creates_symbol() {
    // %global puis %let global, lu en open code.
    assert_eq!(run("%global gg; %let gg = 5; v=&gg;"), "v=5;");
}

#[test]
fn recursion_guard_does_not_panic() {
    // Auto-appel infini : la garde coupe sans paniquer.
    let out = run("%macro r; %r %mend; %r");
    assert!(out.contains("recursion limit"), "got: {out}");
}

#[test]
fn arg_with_macro_ref_resolved_in_caller() {
    // &x défini en open code, passé à la macro.
    let src = "%macro p(a); v=&a; %mend; %let x = 9; %p(&x)";
    assert_eq!(run(src), "v=9;");
}

#[test]
fn eval_precedence() {
    assert_eq!(eval("3+4*2").unwrap(), 11);
}

#[test]
fn eval_integer_division_truncates() {
    assert_eq!(eval("7/2").unwrap(), 3);
    assert_eq!(eval("-7/2").unwrap(), -3); // tronqué vers zéro
}

#[test]
fn eval_power() {
    assert_eq!(eval("2**10").unwrap(), 1024);
    // Associatif à droite : 2**3**2 = 2**9 = 512.
    assert_eq!(eval("2**3**2").unwrap(), 512);
}

#[test]
fn eval_logical_and() {
    assert_eq!(eval("1 and 0").unwrap(), 0);
    assert_eq!(eval("1 & 1").unwrap(), 1);
}

#[test]
fn eval_comparison() {
    assert_eq!(eval("5 ge 5").unwrap(), 1);
    assert_eq!(eval("5 > 6").unwrap(), 0);
    assert_eq!(eval("3 = 3").unwrap(), 1);
    assert_eq!(eval("3 ne 4").unwrap(), 1);
    assert_eq!(eval("2 <> 2").unwrap(), 0); // <> = NE
}

#[test]
fn eval_parens() {
    assert_eq!(eval("(1+2)*3").unwrap(), 9);
}

#[test]
fn eval_unary_minus() {
    assert_eq!(eval("-3 + 5").unwrap(), 2);
    assert_eq!(eval("- -4").unwrap(), 4);
}

#[test]
fn eval_not() {
    assert_eq!(eval("not 0").unwrap(), 1);
    assert_eq!(eval("^0").unwrap(), 1);
    assert_eq!(eval("not 5").unwrap(), 0);
}

#[test]
fn eval_or() {
    assert_eq!(eval("0 or 0").unwrap(), 0);
    assert_eq!(eval("0 | 1").unwrap(), 1);
}

#[test]
fn eval_non_integer_operand_errors() {
    let e = eval("abc + 1").unwrap_err();
    assert!(e.message.contains("character operand"), "got: {}", e.message);
}

#[test]
fn eval_division_by_zero_errors() {
    let e = eval("1/0").unwrap_err();
    assert!(e.message.contains("Division by zero"), "got: {}", e.message);
}

#[test]
fn eval_function_splices_in_open_code() {
    assert_eq!(run("x = %eval(3+4*2);"), "x = 11;");
    assert_eq!(run("x = %eval((1+2)*3);"), "x = 9;");
}

#[test]
fn eval_function_with_macro_var() {
    assert_eq!(run("%let n = 4; x = %eval(&n*2);"), "x = 8;");
}

// --- M11.3 : %if / %then / %else ---

#[test]
fn if_simple_then_else() {
    assert_eq!(run("%if 1 %then a; %else b;"), "a;");
    assert_eq!(run("%if 0 %then a; %else b;"), "b;");
}

#[test]
fn if_then_no_else_false_emits_nothing() {
    assert_eq!(run("%if 0 %then x;"), "");
}

#[test]
fn if_with_do_groups() {
    assert_eq!(
        run("%if 0 %then %do; a=1; %end; %else %do; a=2; %end;"),
        "a=2;"
    );
    assert_eq!(
        run("%if 1 %then %do; a=1; %end; %else %do; a=2; %end;"),
        "a=1;"
    );
}

#[test]
fn if_condition_uses_macro_var() {
    assert_eq!(run("%let n = 5; %if &n ge 5 %then big; %else small;"), "big;");
    assert_eq!(run("%let n = 1; %if &n ge 5 %then big; %else small;"), "small;");
}

#[test]
fn if_condition_uses_eval_expression() {
    assert_eq!(run("%if 3+4 gt 5 %then yes; %else no;"), "yes;");
}

// --- M11.3 : %do / %end (groupe) et itératif ---

#[test]
fn do_group_plain() {
    assert_eq!(run("%do; a=1; b=2; %end;"), "a=1; b=2;");
}

#[test]
fn iterative_do_basic() {
    let src = "%macro g(n); %do i=1 %to &n; v&i=&i; %end; %mend; %g(3)";
    assert_eq!(run(src), "v1=1; v2=2; v3=3;");
}

#[test]
fn iterative_do_with_by() {
    let src = "%macro g; %do i=1 %to 5 %by 2; [&i] %end; %mend; %g";
    assert_eq!(run(src), "[1] [3] [5]");
}

#[test]
fn iterative_do_zero_iterations() {
    // start > stop avec pas positif -> aucune itération.
    let src = "%macro g; pre%do i=5 %to 1; x%end;post %mend; %g";
    assert_eq!(run(src), "prepost");
}

#[test]
fn iterative_do_negative_step() {
    let src = "%macro g; %do i=3 %to 1 %by -1; [&i] %end; %mend; %g";
    assert_eq!(run(src), "[3] [2] [1]");
}

#[test]
fn iterative_do_in_open_code() {
    assert_eq!(run("%do i=1 %to 3; n&i; %end;"), "n1; n2; n3;");
}

#[test]
fn if_do_nested_in_macro_body() {
    let src = "%macro m(n); \
               %do i=1 %to &n; \
               %if &i ge 2 %then big&i; %else small&i; \
               %end; \
               %mend; %m(3)";
    // i=1 -> small1 ; i=2 -> big2 ; i=3 -> big3.
    assert_eq!(run(src), "small1; big2; big3;");
}

#[test]
fn runaway_loop_guard_does_not_hang() {
    // step négatif avec start<stop et pas négatif s'arrête tout de suite ;
    // ici on teste le pas nul -> erreur propre, pas de hang.
    let out = run("%do i=1 %to 10 %by 0; x %end;");
    assert!(out.contains("step is zero"), "got: {out}");
}

// --- M12.1 : %do %while / %do %until ---

#[test]
fn do_while_counter_loop() {
    let out = run("%let i=1; %do %while(&i <= 3); v&i=&i; %let i=%eval(&i+1); %end;");
    assert_eq!(out, "v1=1; v2=2; v3=3;");
}

#[test]
fn do_while_zero_iterations() {
    // Condition fausse d'emblée -> aucune itération, sortie vide.
    let out = run("%let i=5; %do %while(&i < 3); v&i=&i; %end;");
    assert_eq!(out, "");
}

#[test]
fn do_while_inside_macro_body() {
    let src = "%macro m; %let i=1; %do %while(&i <= 3); v&i=&i; %let i=%eval(&i+1); %end; %mend; %m";
    let out = run(src);
    assert_eq!(out.trim(), "v1=1; v2=2; v3=3;");
}

#[test]
fn do_while_runaway_guard() {
    // Condition toujours vraie, jamais mise à jour -> garde anti-runaway.
    let out = run("%do %while(1); x %end;");
    assert!(out.contains("runaway guard"), "got: {out}");
}

#[test]
fn do_until_counter_loop() {
    let out = run("%let i=1; %do %until(&i > 3); v&i=&i; %let i=%eval(&i+1); %end;");
    assert_eq!(out, "v1=1; v2=2; v3=3;");
}

#[test]
fn do_until_runs_at_least_once() {
    // Condition déjà vraie à l'entrée : `%until` itère quand même une fois.
    let out = run("%let i=5; %do %until(&i > 3); hit; %end;");
    assert_eq!(out, "hit;");
}

// --- M11.6 : &&& / &&var&i nested indirection ---

#[test]
fn triple_ampersand_indirection() {
    // &&&y : y -> x, &x -> ab.
    assert_eq!(run("%let x=ab; %let y=x; &&&y"), "ab");
}

#[test]
fn double_ampersand_with_index() {
    // &&v&i : i -> 2, v2 -> hit.
    assert_eq!(run("%let i=2; %let v2=hit; &&v&i"), "hit");
}
