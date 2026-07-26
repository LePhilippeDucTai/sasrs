use super::*;
use crate::source::SourceFile;

/// Parse une expression complète et vérifie qu'elle consomme tout
/// jusqu'à EOF.
fn parse(src: &str) -> Result<Expr> {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file)?;
    let expr = parse_expr(&mut ts)?;
    Ok(expr)
}

fn ok(src: &str) -> Expr {
    parse(src).unwrap_or_else(|e| panic!("parse of {src:?} failed: {e}"))
}

// Helpers de construction pour des assertions de STRUCTURE lisibles.
fn bin(op: BinaryOp, l: Expr, r: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(l),
        right: Box::new(r),
    }
}
fn un(op: UnaryOp, e: Expr) -> Expr {
    Expr::Unary {
        op,
        expr: Box::new(e),
    }
}
fn num(n: f64) -> Expr {
    Expr::Num(n)
}
fn var(s: &str) -> Expr {
    Expr::Var(s.to_string())
}

#[test]
fn first_last_dot_become_canonical_vars() {
    // Insensible à la casse, nom canonique MAJUSCULE.
    assert_eq!(ok("first.grp"), var("FIRST.GRP"));
    assert_eq!(ok("LAST.Age"), var("LAST.AGE"));
    // L'adjacence n'est pas requise.
    assert_eq!(ok("first . grp"), var("FIRST.GRP"));
    // Combinable dans une expression.
    assert_eq!(
        ok("first.grp and last.grp"),
        bin(BinaryOp::And, var("FIRST.GRP"), var("LAST.GRP"))
    );
    // `first` seul reste une variable ordinaire ; `first(x)` un appel.
    assert_eq!(ok("first"), var("first"));
    assert_eq!(
        ok("first(x)"),
        Expr::Call {
            name: "first".to_string(),
            args: vec![var("x")],
        }
    );
}

#[test]
fn other_ident_followed_by_dot_is_not_merged() {
    // Pas de lib.member en expression : seul `lib` est consommé — le
    // `.` orphelin fera échouer le statement appelant (expect_semi),
    // cf. test côté parser de l'étape DATA.
    assert_eq!(ok("lib.member"), var("lib"));
    // FIRST. sans nom de variable : erreur dédiée.
    let err = parse("first. + 1").unwrap_err();
    assert!(err.to_string().contains("BY variable"), "got: {err}");
}

#[test]
fn power_is_right_associative_structure() {
    // 2**3**2 = 2**(3**2), pas (2**3)**2.
    let e = ok("2 ** 3 ** 2");
    assert_eq!(
        e,
        bin(
            BinaryOp::Power,
            num(2.0),
            bin(BinaryOp::Power, num(3.0), num(2.0))
        )
    );
}

#[test]
fn power_binds_tighter_than_unary_minus() {
    // -2**2 = -(2**2).
    let e = ok("-2 ** 2");
    assert_eq!(
        e,
        un(UnaryOp::Minus, bin(BinaryOp::Power, num(2.0), num(2.0)))
    );
}

#[test]
fn not_binds_tighter_than_comparison() {
    // not x = 1  ≡  (not x) = 1.
    let e = ok("not x = 1");
    assert_eq!(e, bin(BinaryOp::Eq, un(UnaryOp::Not, var("x")), num(1.0)));
}

#[test]
fn arithmetic_precedence() {
    // 1 + 2 * 3  ≡  1 + (2*3).
    let e = ok("1 + 2 * 3");
    assert_eq!(
        e,
        bin(
            BinaryOp::Add,
            num(1.0),
            bin(BinaryOp::Mul, num(2.0), num(3.0))
        )
    );
    // 2 * 3 + 1  ≡  (2*3) + 1.
    let e = ok("2 * 3 + 1");
    assert_eq!(
        e,
        bin(
            BinaryOp::Add,
            bin(BinaryOp::Mul, num(2.0), num(3.0)),
            num(1.0)
        )
    );
}

#[test]
fn add_sub_left_associative() {
    // 10 - 3 - 2 ≡ (10-3)-2.
    let e = ok("10 - 3 - 2");
    assert_eq!(
        e,
        bin(
            BinaryOp::Sub,
            bin(BinaryOp::Sub, num(10.0), num(3.0)),
            num(2.0)
        )
    );
}

#[test]
fn comparison_vs_arithmetic() {
    // a + b = c  ≡  (a+b) = c (arithmetic binds tighter than compare).
    let e = ok("a + b = c");
    assert_eq!(
        e,
        bin(
            BinaryOp::Eq,
            bin(BinaryOp::Add, var("a"), var("b")),
            var("c")
        )
    );
}

#[test]
fn and_or_precedence() {
    // a or b and c  ≡  a or (b and c).
    let e = ok("a or b and c");
    assert_eq!(
        e,
        bin(
            BinaryOp::Or,
            var("a"),
            bin(BinaryOp::And, var("b"), var("c"))
        )
    );
}

#[test]
fn concat_precedence() {
    // a || b = c  ≡  (a||b) = c (concat binds tighter than compare).
    let e = ok("a || b = c");
    assert_eq!(
        e,
        bin(
            BinaryOp::Eq,
            bin(BinaryOp::Concat, var("a"), var("b")),
            var("c")
        )
    );
    // 1 + 2 || 3  ≡  (1+2) || 3 (add binds tighter than concat).
    let e = ok("1 + 2 || 3");
    assert_eq!(
        e,
        bin(
            BinaryOp::Concat,
            bin(BinaryOp::Add, num(1.0), num(2.0)),
            num(3.0)
        )
    );
}

#[test]
fn parentheses_override_precedence() {
    // (1 + 2) * 3.
    let e = ok("(1 + 2) * 3");
    assert_eq!(
        e,
        bin(
            BinaryOp::Mul,
            bin(BinaryOp::Add, num(1.0), num(2.0)),
            num(3.0)
        )
    );
}

#[test]
fn date_literal_epoch() {
    assert_eq!(ok("'01jan1960'd"), num(0.0));
    assert_eq!(ok("'02jan1960'd"), num(1.0));
    assert_eq!(ok("'01JAN2020'd"), num(21915.0));
}

#[test]
fn time_literal_seconds_from_midnight() {
    assert_eq!(ok("'12:30't"), num(12.0 * 3600.0 + 30.0 * 60.0));
    assert_eq!(ok("'12:30:45't"), num(12.0 * 3600.0 + 30.0 * 60.0 + 45.0));
}

#[test]
fn datetime_literal_seconds_from_1960() {
    // 1960-01-02 12:00:00 = 1 day + 12h.
    assert_eq!(ok("'02jan1960:12:00:00'dt"), num(86400.0 + 12.0 * 3600.0));
    // Epoch itself.
    assert_eq!(ok("'01jan1960:00:00:00'dt"), num(0.0));
}

#[test]
fn invalid_date_literal_errors() {
    assert!(parse("'32jan2020'd").is_err());
    assert!(parse("'01xxx2020'd").is_err());
    assert!(parse("'notadate'd").is_err());
    assert!(parse("'29feb2021'd").is_err()); // 2021 non bissextile
}

#[test]
fn invalid_time_literal_errors() {
    assert!(parse("'12:99't").is_err());
    assert!(parse("'noon't").is_err());
}

#[test]
fn string_literal_none_and_name() {
    assert_eq!(ok("'hello'"), Expr::Str("hello".to_string()));
    assert_eq!(ok("'my var'n"), Expr::Str("my var".to_string()));
}

#[test]
fn dot_alone_is_ordinary_missing() {
    assert_eq!(ok("."), Expr::Missing(MissingKind::Dot));
}

#[test]
fn dot_adjacent_letter_is_special_missing() {
    // `.a` (jointif) → Missing(Letter(0)).
    assert_eq!(ok(".a"), Expr::Missing(MissingKind::Letter(0)));
    assert_eq!(ok(".Z"), Expr::Missing(MissingKind::Letter(25)));
    assert_eq!(ok("._"), Expr::Missing(MissingKind::Underscore));
}

#[test]
fn dot_non_adjacent_is_ordinary_missing() {
    // `. a` (espace) → Missing(Dot) ; l'ident `a` reste pour l'appelant.
    let file = SourceFile::new(". a");
    let mut ts = StatementStream::new(&file).unwrap();
    let e = parse_expr(&mut ts).unwrap();
    assert_eq!(e, Expr::Missing(MissingKind::Dot));
    // L'ident `a` n'a pas été consommé.
    assert!(ts.peek().is_kw("a"));
}

#[test]
fn dot_followed_by_multiletter_ident_is_ordinary_missing() {
    // `.ab` n'est pas un missing spécial (ident de >1 lettre).
    let file = SourceFile::new(".ab");
    let mut ts = StatementStream::new(&file).unwrap();
    let e = parse_expr(&mut ts).unwrap();
    assert_eq!(e, Expr::Missing(MissingKind::Dot));
    assert!(ts.peek().is_kw("ab"));
}

#[test]
fn variable_reference() {
    assert_eq!(ok("age"), var("age"));
}

#[test]
fn function_call_zero_args() {
    assert_eq!(
        ok("today()"),
        Expr::Call {
            name: "today".to_string(),
            args: vec![]
        }
    );
}

#[test]
fn function_call_one_arg() {
    assert_eq!(
        ok("abs(x)"),
        Expr::Call {
            name: "abs".to_string(),
            args: vec![var("x")]
        }
    );
}

#[test]
fn function_call_two_args() {
    assert_eq!(
        ok("sum(a, b)"),
        Expr::Call {
            name: "sum".to_string(),
            args: vec![var("a"), var("b")]
        }
    );
    // Arguments composés.
    assert_eq!(
        ok("max(a + 1, b * 2)"),
        Expr::Call {
            name: "max".to_string(),
            args: vec![
                bin(BinaryOp::Add, var("a"), num(1.0)),
                bin(BinaryOp::Mul, var("b"), num(2.0)),
            ]
        }
    );
}

#[test]
fn in_operator_num_and_str() {
    let e = ok("x in (1, 2, 3)");
    assert_eq!(
        e,
        Expr::In {
            expr: Box::new(var("x")),
            list: vec![num(1.0), num(2.0), num(3.0)],
        }
    );
    let e = ok("sex in ('M', 'F')");
    assert_eq!(
        e,
        Expr::In {
            expr: Box::new(var("sex")),
            list: vec![Expr::Str("M".to_string()), Expr::Str("F".to_string())],
        }
    );
}

#[test]
fn in_with_negative_and_single_item() {
    let e = ok("x in (-1)");
    assert_eq!(
        e,
        Expr::In {
            expr: Box::new(var("x")),
            list: vec![num(-1.0)],
        }
    );
}

#[test]
fn in_binds_at_comparison_level() {
    // a and x in (1, 2)  ≡  a and (x in (1,2)).
    let e = ok("a and x in (1, 2)");
    assert_eq!(
        e,
        bin(
            BinaryOp::And,
            var("a"),
            Expr::In {
                expr: Box::new(var("x")),
                list: vec![num(1.0), num(2.0)],
            }
        )
    );
}

#[test]
fn unmatched_paren_errors() {
    assert!(parse("(1 + 2").is_err());
}

// ── Références d'array indexées (M2) ─────────────────────────────────

#[test]
fn index_with_braces_and_brackets() {
    let expected = Expr::Index {
        name: "a".to_string(),
        indices: vec![bin(BinaryOp::Add, var("i"), num(1.0))],
    };
    assert_eq!(ok("a{i + 1}"), expected);
    assert_eq!(ok("a[i + 1]"), expected);
}

#[test]
fn index_paren_form_stays_a_call() {
    // `a(i)` reste un Call : l'ambiguïté array/fonction est résolue à
    // l'évaluation.
    assert_eq!(
        ok("a(i)"),
        Expr::Call {
            name: "a".to_string(),
            args: vec![var("i")]
        }
    );
}

#[test]
fn index_in_larger_expression() {
    assert_eq!(
        ok("a{1} + a{2}"),
        bin(
            BinaryOp::Add,
            Expr::Index {
                name: "a".to_string(),
                indices: vec![num(1.0)]
            },
            Expr::Index {
                name: "a".to_string(),
                indices: vec![num(2.0)]
            },
        )
    );
}

#[test]
fn index_multi_dim_parses() {
    // M16.2 : `a{1, 2}` → deux indices.
    assert_eq!(
        ok("a{1, 2}"),
        Expr::Index {
            name: "a".to_string(),
            indices: vec![num(1.0), num(2.0)],
        }
    );
}

#[test]
fn index_mismatched_closer_errors() {
    assert!(parse("a{1]").is_err());
    assert!(parse("a[1}").is_err());
    assert!(parse("a{1").is_err());
}

#[test]
fn empty_input_errors() {
    assert!(parse("").is_err());
}

// ── PUT / INPUT : le 2e argument est un token de format (M4) ──────────

#[test]
fn put_parses_format_token_as_string_arg() {
    assert_eq!(
        ok("put(x, dollar8.2)"),
        Expr::Call {
            name: "put".to_string(),
            args: vec![var("x"), Expr::Str("dollar8.2".to_string())],
        }
    );
}

#[test]
fn input_parses_format_token_with_trailing_dot() {
    assert_eq!(
        ok("input(s, date9.)"),
        Expr::Call {
            name: "input".to_string(),
            args: vec![var("s"), Expr::Str("date9.".to_string())],
        }
    );
}

#[test]
fn put_with_bare_wd_format() {
    assert_eq!(
        ok("put(y, 8.2)"),
        Expr::Call {
            name: "put".to_string(),
            args: vec![var("y"), Expr::Str("8.2".to_string())],
        }
    );
}
