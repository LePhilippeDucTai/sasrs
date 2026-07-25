use super::super::*;
use super::*;
use crate::ast::{BinaryOp, Expr, UnaryOp};
use crate::value::Value;

// ── Littéraux ────────────────────────────────────────────────────────

#[test]
fn literal_num() {
    let (v, _) = ev_bare(&num(42.0));
    assert_eq!(v, Value::Num(42.0));
}

#[test]
fn literal_str() {
    let (v, _) = ev_bare(&str_("hi"));
    assert_eq!(v, Value::Char("hi".into()));
}

#[test]
fn literal_missing() {
    let (v, _) = ev_bare(&miss());
    assert_eq!(v, Value::missing());
}

// ── Comparaisons : sas_cmp PARTOUT ───────────────────────────────────

#[test]
fn dot_eq_dot_is_true() {
    // `. = .` → 1.0
    let (v, _) = ev_bare(&bin(BinaryOp::Eq, miss(), miss()));
    assert_eq!(v, Value::Num(1.0));
}

#[test]
fn dot_lt_zero_is_true() {
    // `. < 0` → 1.0 (missing trie sous tous les nombres)
    let (v, _) = ev_bare(&bin(BinaryOp::Lt, miss(), num(0.0)));
    assert_eq!(v, Value::Num(1.0));
}

#[test]
fn comparisons_full_matrix() {
    let cases = [
        (BinaryOp::Eq, 1.0, 1.0, 1.0),
        (BinaryOp::Eq, 1.0, 2.0, 0.0),
        (BinaryOp::Ne, 1.0, 2.0, 1.0),
        (BinaryOp::Lt, 1.0, 2.0, 1.0),
        (BinaryOp::Le, 2.0, 2.0, 1.0),
        (BinaryOp::Gt, 3.0, 2.0, 1.0),
        (BinaryOp::Ge, 2.0, 2.0, 1.0),
        (BinaryOp::Ge, 1.0, 2.0, 0.0),
    ];
    for (op, a, b, expected) in cases {
        let (v, _) = ev_bare(&bin(op, num(a), num(b)));
        assert_eq!(v, Value::Num(expected), "{op:?} {a} {b}");
    }
}

#[test]
fn char_comparison_ignores_trailing_blanks() {
    let (v, _) = ev_bare(&bin(BinaryOp::Eq, str_("abc"), str_("abc   ")));
    assert_eq!(v, Value::Num(1.0));
}

// ── Char en contexte numérique ──────────────────────────────────────

#[test]
fn char_numeric_string_in_arith_converts_with_note() {
    // '12 ' en contexte num → 12 + note_char_to_num.
    let (v, ctx) = ev_bare(&bin(BinaryOp::Add, str_("12 "), num(0.0)));
    assert_eq!(v, Value::Num(12.0));
    assert!(ctx.note_char_to_num);
    assert_eq!(ctx.invalid_data, 0);
    assert!(!ctx.error_flag);
}

#[test]
fn char_invalid_in_arith_is_missing_with_invalid_and_error() {
    // 'abc' → missing + invalid_data + error_flag (+ note tentée).
    let (v, ctx) = ev_bare(&bin(BinaryOp::Add, str_("abc"), num(1.0)));
    assert_eq!(v, Value::missing());
    assert!(ctx.note_char_to_num);
    assert_eq!(ctx.invalid_data, 1);
    assert!(ctx.error_flag);
    // Le missing arithmétique généré par l'opération est aussi compté.
    assert_eq!(ctx.missing_generated, 1);
}

#[test]
fn char_empty_in_arith_is_missing_generated() {
    // chaîne vide en contexte num → missing + missing_generated (note tentée).
    let (v, ctx) = ev_bare(&bin(BinaryOp::Mul, str_("   "), num(2.0)));
    assert_eq!(v, Value::missing());
    assert!(ctx.note_char_to_num);
    // coerce_num incrémente missing_generated pour la chaîne vide, puis
    // l'opération arithmétique l'incrémente une seconde fois.
    assert_eq!(ctx.missing_generated, 2);
    assert_eq!(ctx.invalid_data, 0);
    assert!(!ctx.error_flag);
}

// ── Logique ──────────────────────────────────────────────────────────

#[test]
fn not_missing_is_true() {
    // `not .` → 1.0 (missing est falsy)
    let (v, _) = ev_bare(&unary(UnaryOp::Not, miss()));
    assert_eq!(v, Value::Num(1.0));
}

#[test]
fn not_zero_is_true() {
    let (v, _) = ev_bare(&unary(UnaryOp::Not, num(0.0)));
    assert_eq!(v, Value::Num(1.0));
}

#[test]
fn not_nonzero_is_false() {
    let (v, _) = ev_bare(&unary(UnaryOp::Not, num(5.0)));
    assert_eq!(v, Value::Num(0.0));
}

#[test]
fn and_or_truth_table() {
    let (v, _) = ev_bare(&bin(BinaryOp::And, num(1.0), num(1.0)));
    assert_eq!(v, Value::Num(1.0));
    let (v, _) = ev_bare(&bin(BinaryOp::And, num(1.0), num(0.0)));
    assert_eq!(v, Value::Num(0.0));
    let (v, _) = ev_bare(&bin(BinaryOp::Or, num(0.0), num(0.0)));
    assert_eq!(v, Value::Num(0.0));
    let (v, _) = ev_bare(&bin(BinaryOp::Or, num(0.0), num(3.0)));
    assert_eq!(v, Value::Num(1.0));
    // missing est falsy.
    let (v, _) = ev_bare(&bin(BinaryOp::And, num(1.0), miss()));
    assert_eq!(v, Value::Num(0.0));
}

// ── Arithmétique nominale ────────────────────────────────────────────

#[test]
fn arithmetic_nominal() {
    let (v, _) = ev_bare(&bin(BinaryOp::Add, num(2.0), num(3.0)));
    assert_eq!(v, Value::Num(5.0));
    let (v, _) = ev_bare(&bin(BinaryOp::Sub, num(2.0), num(3.0)));
    assert_eq!(v, Value::Num(-1.0));
    let (v, _) = ev_bare(&bin(BinaryOp::Mul, num(2.0), num(3.0)));
    assert_eq!(v, Value::Num(6.0));
    let (v, _) = ev_bare(&bin(BinaryOp::Div, num(6.0), num(3.0)));
    assert_eq!(v, Value::Num(2.0));
}

#[test]
fn power_nominal() {
    // 2 ** 3 = 8
    let (v, _) = ev_bare(&bin(BinaryOp::Power, num(2.0), num(3.0)));
    assert_eq!(v, Value::Num(8.0));
}

#[test]
fn power_zero_zero_is_one() {
    let (v, _) = ev_bare(&bin(BinaryOp::Power, num(0.0), num(0.0)));
    assert_eq!(v, Value::Num(1.0));
}

#[test]
fn power_negative_base_fractional_exponent_is_missing() {
    // (-2) ** 0.5 → missing + missing_generated, PAS d'error_flag.
    let (v, ctx) = ev_bare(&bin(BinaryOp::Power, num(-2.0), num(0.5)));
    assert_eq!(v, Value::missing());
    assert_eq!(ctx.missing_generated, 1);
    assert!(!ctx.error_flag);
}

#[test]
fn unary_minus_on_power_ast() {
    // AST déjà -(2 ** 2) : -(4) = -4 (le parser produit ce nœud).
    let e = unary(UnaryOp::Minus, bin(BinaryOp::Power, num(2.0), num(2.0)));
    let (v, _) = ev_bare(&e);
    assert_eq!(v, Value::Num(-4.0));
}

#[test]
fn unary_plus_nominal() {
    let (v, _) = ev_bare(&unary(UnaryOp::Plus, num(5.0)));
    assert_eq!(v, Value::Num(5.0));
}

#[test]
fn unary_minus_on_missing_generates_missing() {
    let (v, ctx) = ev_bare(&unary(UnaryOp::Minus, miss()));
    assert_eq!(v, Value::missing());
    assert_eq!(ctx.missing_generated, 1);
}

#[test]
fn unary_plus_on_missing_generates_missing() {
    let (v, ctx) = ev_bare(&unary(UnaryOp::Plus, miss()));
    assert_eq!(v, Value::missing());
    assert_eq!(ctx.missing_generated, 1);
}

// ── Arithmétique avec missing ───────────────────────────────────────

#[test]
fn missing_plus_one_is_missing_and_counts() {
    // `. + 1` → missing + missing_generated, pas d'erreur.
    let (v, ctx) = ev_bare(&bin(BinaryOp::Add, miss(), num(1.0)));
    assert_eq!(v, Value::missing());
    assert_eq!(ctx.missing_generated, 1);
    assert!(!ctx.error_flag);
}

#[test]
fn one_minus_missing_is_missing_and_counts() {
    let (v, ctx) = ev_bare(&bin(BinaryOp::Sub, num(1.0), miss()));
    assert_eq!(v, Value::missing());
    assert_eq!(ctx.missing_generated, 1);
}

// ── Division par zéro ───────────────────────────────────────────────

#[test]
fn division_by_zero_is_missing_with_counter_and_error() {
    // `1 / 0` → missing + division_by_zero + error_flag.
    let (v, ctx) = ev_bare(&bin(BinaryOp::Div, num(1.0), num(0.0)));
    assert_eq!(v, Value::missing());
    assert_eq!(ctx.division_by_zero, 1);
    assert!(ctx.error_flag);
    // Une division par zéro n'est pas comptée comme missing arithmétique.
    assert_eq!(ctx.missing_generated, 0);
}

// ── Concaténation || ────────────────────────────────────────────────

#[test]
fn concat_char_char() {
    let (v, ctx) = ev_bare(&bin(BinaryOp::Concat, str_("ab"), str_("cd")));
    assert_eq!(v, Value::Char("abcd".into()));
    assert!(!ctx.note_num_to_char);
}

#[test]
fn concat_num_num_best12_right_justified() {
    // 2 || 3 : chaque opérande BEST12 justifié droite 12 →
    // "           2" + "           3" concaténés.
    let (v, ctx) = ev_bare(&bin(BinaryOp::Concat, num(2.0), num(3.0)));
    let expected = format!("{:>12}{:>12}", "2", "3");
    assert_eq!(v, Value::Char(expected.clone()));
    assert_eq!(expected, "           2           3");
    assert!(ctx.note_num_to_char);
}

#[test]
fn concat_mixed_num_char() {
    // num justifié droite 12, char tel quel.
    let (v, ctx) = ev_bare(&bin(BinaryOp::Concat, num(5.0), str_("x")));
    assert_eq!(v, Value::Char(format!("{:>12}x", "5")));
    assert!(ctx.note_num_to_char);
}

// ── Variables / automatiques ────────────────────────────────────────

#[test]
fn var_lookup_num() {
    let pdv = pdv_with(vec![(num_var("Age"), Value::Num(14.0))]);
    let (v, _) = ev(&var("age"), &pdv);
    assert_eq!(v, Value::Num(14.0));
}

#[test]
fn var_lookup_char() {
    let pdv = pdv_with(vec![(char_var("Name", 10), Value::Char("Alice".into()))]);
    let (v, _) = ev(&var("NAME"), &pdv);
    assert_eq!(v, Value::Char("Alice".into()));
}

#[test]
fn var_in_arithmetic() {
    let pdv = pdv_with(vec![(num_var("x"), Value::Num(10.0))]);
    let (v, _) = ev(&bin(BinaryOp::Add, var("x"), num(5.0)), &pdv);
    assert_eq!(v, Value::Num(15.0));
}

#[test]
fn automatic_n_variable() {
    let mut pdv = Pdv::new();
    pdv.n_ = 7;
    let (v, _) = ev(&var("_N_"), &pdv);
    assert_eq!(v, Value::Num(7.0));
    let (v, _) = ev(&var("_n_"), &pdv);
    assert_eq!(v, Value::Num(7.0));
}

#[test]
fn automatic_error_variable() {
    let mut pdv = Pdv::new();
    let (v, _) = ev(&var("_ERROR_"), &pdv);
    assert_eq!(v, Value::Num(0.0));
    pdv.error_ = true;
    let (v, _) = ev(&var("_error_"), &pdv);
    assert_eq!(v, Value::Num(1.0));
}

#[test]
fn unknown_variable_is_fatal() {
    let pdv = Pdv::new();
    let (v, ctx) = ev(&var("nosuch"), &pdv);
    assert_eq!(v, Value::missing());
    assert!(ctx.fatal.is_some());
    assert!(ctx
        .fatal
        .unwrap()
        .to_string()
        .contains("program data vector"));
}

// ── IN ───────────────────────────────────────────────────────────────

#[test]
fn in_match() {
    let e = Expr::In {
        expr: Box::new(num(2.0)),
        list: vec![num(1.0), num(2.0), num(3.0)],
    };
    let (v, _) = ev_bare(&e);
    assert_eq!(v, Value::Num(1.0));
}

#[test]
fn in_no_match() {
    let e = Expr::In {
        expr: Box::new(num(9.0)),
        list: vec![num(1.0), num(2.0)],
    };
    let (v, _) = ev_bare(&e);
    assert_eq!(v, Value::Num(0.0));
}

#[test]
fn in_missing_matches_missing() {
    // `. in (.)` → 1.0 (sas_cmp : `. = .` vrai)
    let e = Expr::In {
        expr: Box::new(miss()),
        list: vec![num(1.0), miss()],
    };
    let (v, _) = ev_bare(&e);
    assert_eq!(v, Value::Num(1.0));
}

#[test]
fn in_char() {
    let e = Expr::In {
        expr: Box::new(str_("b")),
        list: vec![str_("a"), str_("b")],
    };
    let (v, _) = ev_bare(&e);
    assert_eq!(v, Value::Num(1.0));
}

// ── Call ─────────────────────────────────────────────────────────────

#[test]
fn call_known_function() {
    let e = Expr::Call {
        name: "sum".to_string(),
        args: vec![num(1.0), num(2.0), num(3.0)],
    };
    let (v, ctx) = ev_bare(&e);
    assert_eq!(v, Value::Num(6.0));
    assert!(ctx.fatal.is_none());
}

#[test]
fn call_function_propagates_args_from_pdv() {
    let pdv = pdv_with(vec![
        (num_var("a"), Value::Num(10.0)),
        (num_var("b"), Value::Num(20.0)),
    ]);
    let e = Expr::Call {
        name: "MEAN".to_string(),
        args: vec![var("a"), var("b")],
    };
    let (v, _) = ev(&e, &pdv);
    assert_eq!(v, Value::Num(15.0));
}

#[test]
fn call_unknown_function_is_fatal() {
    let e = Expr::Call {
        name: "NOSUCHFN".to_string(),
        args: vec![],
    };
    let (v, ctx) = ev_bare(&e);
    assert_eq!(v, Value::missing());
    assert!(ctx.fatal.is_some());
    // Le message typé est SANS préfixe « ERROR: » (ajouté par log.error
    // à l'affichage) — on vérifie le contenu, plus le préfixe.
    let msg = ctx.fatal.unwrap().to_string();
    assert!(msg.contains("unknown"));
    assert!(msg.contains("NOSUCHFN"));
}

#[test]
fn fatal_short_circuits_outer_expression() {
    // Une fonction inconnue dans un sous-arbre stoppe l'évaluation.
    let inner = Expr::Call {
        name: "BOGUS".to_string(),
        args: vec![],
    };
    let e = bin(BinaryOp::Add, inner, num(1.0));
    let (v, ctx) = ev_bare(&e);
    assert_eq!(v, Value::missing());
    assert!(ctx.fatal.is_some());
}

// ── Composition / précédence (telle qu'encodée par l'AST) ───────────

#[test]
fn nested_expression() {
    // (x + 1) * 2 avec x = 4 → 10
    let pdv = pdv_with(vec![(num_var("x"), Value::Num(4.0))]);
    let e = bin(
        BinaryOp::Mul,
        bin(BinaryOp::Add, var("x"), num(1.0)),
        num(2.0),
    );
    let (v, _) = ev(&e, &pdv);
    assert_eq!(v, Value::Num(10.0));
}

#[test]
fn mixed_comparison_converts_char_side() {
    let pdv = Pdv::new();
    // '12' = 12 → conversion auto char→num → vrai, avec note.
    let e = bin(BinaryOp::Eq, str_("12"), num(12.0));
    let (v, ctx) = ev(&e, &pdv);
    assert_eq!(v, Value::Num(1.0));
    assert!(ctx.note_char_to_num);
    // ' ' < 0 → char blanc converti en missing → . < 0 vrai.
    let e = bin(BinaryOp::Lt, str_(" "), num(0.0));
    let (v, _) = ev(&e, &pdv);
    assert_eq!(v, Value::Num(1.0));
    // 'abc' = 5 → conversion invalide → missing ≠ 5 → faux + flags.
    let e = bin(BinaryOp::Eq, str_("abc"), num(5.0));
    let (v, ctx) = ev(&e, &pdv);
    assert_eq!(v, Value::Num(0.0));
    assert!(ctx.invalid_data > 0);
    assert!(ctx.error_flag);
}
