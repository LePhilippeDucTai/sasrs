//! Syntaxe des options de dataset sur l'étape DATA — en particulier
//! `WHERE=` et le jeu de parenthèses qui l'entoure.
//!
//! SAS est libre en espacement : `set t (where = (x = 1));` est identique à
//! `set t(where=(x=1));`. Ces tests verrouillent cette indifférence (espaces,
//! sauts de ligne, casse), les parenthèses imbriquées DANS l'expression, et
//! les formes qui doivent échouer (`==` n'est pas un opérateur SAS).

use super::*;
use crate::ast::BinaryOp;

/// L'expression `where=` attendue par la plupart des tests ci-dessous :
/// `x = 1`.
fn x_eq_1() -> Expr {
    Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::Var("x".to_string())),
        right: Box::new(Expr::Num(1.0)),
    }
}

/// La spec du (premier) dataset lu par le SET de l'étape.
fn set_spec(ast: &DataStepAst) -> &DatasetSpec {
    let DsStmt::Set { specs, .. } = &ast.stmts[0] else {
        panic!("expected a SET statement");
    };
    &specs[0]
}

// ── Espacement autour des options ────────────────────────────────────

/// Cas de la demande : espace entre le nom du dataset et la parenthèse
/// d'options, et espaces autour du `=` de `where`. Le lexeur travaillant
/// en tokens, l'espacement est indifférent — la spec porte bien l'option.
#[test]
fn set_where_option_ignores_spaces_around_name_and_equals() {
    let ast = parse("data test; set table (where = (x = 1)); run;").unwrap();
    let spec = set_spec(&ast);
    assert_eq!(spec.dref, dsref("table"));
    assert_eq!(spec.options.where_, Some(x_eq_1()));
}

/// Même programme sans aucun espace : la spec produite est IDENTIQUE.
#[test]
fn set_where_option_spacing_is_irrelevant() {
    let spaced = parse("data test; set table (where = (x = 1)); run;").unwrap();
    let tight = parse("data test; set table(where=(x=1)); run;").unwrap();
    assert_eq!(set_spec(&spaced), set_spec(&tight));
}

/// Un saut de ligne entre le nom du dataset et ses options ne coupe pas la
/// spec (le statement ne se termine qu'au `;`).
#[test]
fn set_where_option_across_newline() {
    let ast = parse("data test;\n  set table\n    (where = (x = 1));\nrun;").unwrap();
    assert_eq!(set_spec(&ast).options.where_, Some(x_eq_1()));
}

/// Le nom de l'option est insensible à la casse (`WHERE=`, `Where=`).
#[test]
fn set_where_option_name_is_case_insensitive() {
    for src in [
        "data test; set table(WHERE=(x = 1)); run;",
        "data test; set table(Where = (x = 1)); run;",
    ] {
        let ast = parse(src).unwrap();
        assert_eq!(set_spec(&ast).options.where_, Some(x_eq_1()), "src: {src}");
    }
}

// ── Parenthèses ──────────────────────────────────────────────────────

/// Les parenthèses INTERNES à l'expression sont rendues au parseur
/// d'expressions : seule la parenthèse fermante de plus haut niveau clôt
/// l'option, la liste d'options continue après.
#[test]
fn set_where_option_accepts_nested_parentheses() {
    let ast = parse("data test; set table(where=((x = 1) or (x = 3)) keep=x); run;").unwrap();
    let spec = set_spec(&ast);
    assert_eq!(
        spec.options.where_,
        Some(Expr::Binary {
            op: BinaryOp::Or,
            left: Box::new(x_eq_1()),
            right: Box::new(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Var("x".to_string())),
                right: Box::new(Expr::Num(3.0)),
            }),
        })
    );
    assert_eq!(spec.options.keep, Some(vec!["x".to_string()]));
}

/// Parenthèses redondantes autour de l'expression entière : `where=((x=1))`
/// ≡ `where=(x=1)` (la paire externe est celle de l'option).
#[test]
fn set_where_option_redundant_parentheses() {
    let ast = parse("data test; set table(where=((x = 1))); run;").unwrap();
    assert_eq!(set_spec(&ast).options.where_, Some(x_eq_1()));
}

/// Parenthèses vides : l'option exige une expression.
#[test]
fn set_where_option_empty_parentheses_errors() {
    let err = parse("data test; set table(where=()); run;").unwrap_err();
    assert!(
        err.to_string().contains("expected an expression"),
        "got: {err}"
    );
}

/// Parenthèse fermante manquante : le `;` tombe au milieu de la liste
/// d'options → erreur de parsing (pas de spec silencieusement tronquée).
#[test]
fn set_where_option_unclosed_parenthesis_errors() {
    assert!(parse("data test; set table(where=(x = 1); run;").is_err());
    assert!(parse("data test; set table(where=(x = 1)); ru").is_err());
}

/// Liste d'options VIDE `t()` : acceptée, la spec est nue.
#[test]
fn set_empty_option_list_parses_as_plain_spec() {
    let ast = parse("data test; set table(); run;").unwrap();
    assert_eq!(*set_spec(&ast), dspec("table"));
}

// ── Opérateur d'égalité ──────────────────────────────────────────────

/// `==` n'existe pas en SAS : l'égalité s'écrit `=` (ou `eq`). La forme
/// `where=(x == 1)` doit donc être REFUSÉE, pas silencieusement acceptée.
#[test]
fn set_where_option_double_equals_errors() {
    let err = parse("data test; set table(where=(x == 1)); run;").unwrap_err();
    assert!(
        err.to_string().contains("expected an expression"),
        "got: {err}"
    );
}

/// L'opérateur mnémonique `eq` est en revanche accepté et produit le même
/// AST que `=`.
#[test]
fn set_where_option_eq_mnemonic_matches_equals_sign() {
    let ast = parse("data test; set table(where=(x eq 1)); run;").unwrap();
    assert_eq!(set_spec(&ast).options.where_, Some(x_eq_1()));
}

// ── Combinaisons et positions ────────────────────────────────────────

/// L'ordre des options dans la parenthèse est libre : `where=` avant ou
/// après `keep=` donne la même spec.
#[test]
fn set_option_order_is_free() {
    let a = parse("data test; set table(where=(x = 1) keep=x y); run;").unwrap();
    let b = parse("data test; set table(keep=x y where=(x = 1)); run;").unwrap();
    assert_eq!(set_spec(&a), set_spec(&b));
    assert_eq!(
        set_spec(&a).options.keep,
        Some(vec!["x".to_string(), "y".to_string()])
    );
}

/// Un `where=` sur le premier dataset ne déborde pas sur le suivant :
/// `set a(where=(x = 1)) b;` — deux specs, une seule option.
#[test]
fn set_where_option_does_not_leak_to_next_dataset() {
    let ast = parse("data test; set a (where = (x = 1)) b; run;").unwrap();
    let DsStmt::Set { specs, .. } = &ast.stmts[0] else {
        panic!("expected a SET statement");
    };
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].options.where_, Some(x_eq_1()));
    assert_eq!(specs[1], dspec("b"));
}

/// Chaque dataset porte SON `where=` — y compris deux lectures du même
/// dataset.
#[test]
fn set_where_option_is_per_dataset() {
    let ast = parse("data test; set t(where=(x = 1)) t(where=(x = 3)); run;").unwrap();
    let DsStmt::Set { specs, .. } = &ast.stmts[0] else {
        panic!("expected a SET statement");
    };
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].options.where_, Some(x_eq_1()));
    assert_eq!(
        specs[1].options.where_,
        Some(Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Var("x".to_string())),
            right: Box::new(Expr::Num(3.0)),
        })
    );
}

/// `where=` s'applique aussi à un dataset qualifié par un libref, avec les
/// mêmes libertés d'espacement.
#[test]
fn set_where_option_on_qualified_dataset() {
    let ast = parse("data test; set lib.table (where = (x = 1)); run;").unwrap();
    let spec = set_spec(&ast);
    assert_eq!(
        spec.dref,
        DatasetRef {
            libref: Some("lib".to_string()),
            name: "table".to_string(),
        }
    );
    assert_eq!(spec.options.where_, Some(x_eq_1()));
}

/// Le `where=` d'un MERGE se parse comme celui d'un SET.
#[test]
fn merge_where_option_parses() {
    let ast = parse("data test; merge a (where = (x = 1)) b; by x; run;").unwrap();
    let DsStmt::Merge(specs) = &ast.stmts[0] else {
        panic!("expected a MERGE statement");
    };
    assert_eq!(specs[0].options.where_, Some(x_eq_1()));
    assert_eq!(specs[1], dspec("b"));
}

/// Sur le statement DATA lui-même (dataset de SORTIE), `where=` se parse —
/// son rejet est prononcé à la compilation (cf. `where_on_output_dataset_errors`).
#[test]
fn data_statement_where_option_parses() {
    let ast = parse("data test (where = (x = 1)); set table; run;").unwrap();
    assert_eq!(ast.outputs.len(), 1);
    assert_eq!(ast.outputs[0].options.where_, Some(x_eq_1()));
}
