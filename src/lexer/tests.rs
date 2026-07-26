use super::*;

fn kinds(src: &str) -> Vec<TokenKind> {
    Lexer::new(src)
        .tokenize()
        .unwrap()
        .into_iter()
        .map(|t| t.kind)
        .collect()
}

#[test]
fn basic_statement() {
    let k = kinds("data work.a; x = 1.5; run;");
    assert_eq!(
        k,
        vec![
            TokenKind::Ident("data".into()),
            TokenKind::Ident("work".into()),
            TokenKind::Dot,
            TokenKind::Ident("a".into()),
            TokenKind::Semi,
            TokenKind::Ident("x".into()),
            TokenKind::Eq,
            TokenKind::Num(1.5),
            TokenKind::Semi,
            TokenKind::Ident("run".into()),
            TokenKind::Semi,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn word_operators() {
    let k = kinds("if x ge 2 and y ne 3 or not z");
    assert!(k.contains(&TokenKind::Ge));
    assert!(k.contains(&TokenKind::And));
    assert!(k.contains(&TokenKind::Ne));
    assert!(k.contains(&TokenKind::Or));
    assert!(k.contains(&TokenKind::Not));
}

#[test]
fn date_literal_and_strings() {
    let k = kinds("d = '01jan2020'd; s = \"it''s\";");
    assert!(k.contains(&TokenKind::Str {
        value: "01jan2020".into(),
        suffix: StrSuffix::Date
    }));
    // Doubled quote inside single-quoted string.
    let k2 = kinds("s = 'it''s';");
    assert!(k2.contains(&TokenKind::Str {
        value: "it's".into(),
        suffix: StrSuffix::None
    }));
}

#[test]
fn comments_and_power() {
    let k = kinds("x = /* note */ 2 ** 3;");
    assert!(k.contains(&TokenKind::Power));
    assert_eq!(k.iter().filter(|k| **k == TokenKind::Num(2.0)).count(), 1);
}

#[test]
fn star_comment_statement_is_trivia() {
    // Contenu arbitraire (`:`, apostrophe) toléré dans `* ... ;`.
    let k = kinds("* commentaire : avec l'apostrophe ; x = 1;");
    assert_eq!(
        k,
        vec![
            TokenKind::Ident("x".into()),
            TokenKind::Eq,
            TokenKind::Num(1.0),
            TokenKind::Semi,
            TokenKind::Eof,
        ]
    );
    // Après un `;`, donc en début de statement, y compris en fin de source.
    let k = kinds("run; * fini ;");
    assert_eq!(
        k,
        vec![
            TokenKind::Ident("run".into()),
            TokenKind::Semi,
            TokenKind::Eof
        ]
    );
    // `*` en PLEIN statement reste la multiplication.
    let k = kinds("x = 2 * 3;");
    assert!(k.contains(&TokenKind::Star));
}

#[test]
fn dollar_token_in_length_statement() {
    // `$` collé ou non au nombre : toujours un token Dollar distinct.
    let k = kinds("length a b $ 12 c 5;");
    assert_eq!(
        k,
        vec![
            TokenKind::Ident("length".into()),
            TokenKind::Ident("a".into()),
            TokenKind::Ident("b".into()),
            TokenKind::Dollar,
            TokenKind::Num(12.0),
            TokenKind::Ident("c".into()),
            TokenKind::Num(5.0),
            TokenKind::Semi,
            TokenKind::Eof,
        ]
    );
    let k = kinds("length x $20;");
    assert!(k.contains(&TokenKind::Dollar));
    assert!(k.contains(&TokenKind::Num(20.0)));
}

#[test]
fn braces_and_brackets_tokens() {
    // Les 4 délimiteurs d'array (M2) : accolades et crochets.
    let k = kinds("array a{3} b[2];");
    assert_eq!(
        k,
        vec![
            TokenKind::Ident("array".into()),
            TokenKind::Ident("a".into()),
            TokenKind::LBrace,
            TokenKind::Num(3.0),
            TokenKind::RBrace,
            TokenKind::Ident("b".into()),
            TokenKind::LBracket,
            TokenKind::Num(2.0),
            TokenKind::RBracket,
            TokenKind::Semi,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn missing_dot_vs_number() {
    let k = kinds("x = .; y = .5;");
    assert!(k.contains(&TokenKind::Dot));
    assert!(k.contains(&TokenKind::Num(0.5)));
}

#[test]
fn at_and_colon_tokens() {
    // `@` (pointeur de colonne) et `:` (modificateur d'informat) ne
    // tombent plus dans l'arme « caractère inattendu ».
    let k = kinds("input @5 x :date9.;");
    assert!(k.contains(&TokenKind::At));
    assert!(k.contains(&TokenKind::Colon));
}

#[test]
fn datalines_capture_simple() {
    // `datalines;` capture les lignes brutes jusqu'à la ligne `;`.
    let src = "input x y;\ndatalines;\n1 2\n3 4\n;\nrun;";
    let k = kinds(src);
    // Le token DataLines porte exactement les deux lignes de données.
    let dl: Vec<&Vec<String>> = k
        .iter()
        .filter_map(|t| match t {
            TokenKind::DataLines(v) => Some(v),
            _ => None,
        })
        .collect();
    assert_eq!(dl.len(), 1);
    assert_eq!(dl[0], &vec!["1 2".to_string(), "3 4".to_string()]);
    // `run;` suit normalement après les données.
    assert!(k.contains(&TokenKind::Ident("run".into())));
}

#[test]
fn datalines_preserves_internal_spacing() {
    // Les colonnes fixes exigent que les espaces internes soient gardés.
    let src = "datalines;\nAlice   14\nBob     16\n;\n";
    let k = kinds(src);
    let TokenKind::DataLines(v) = k
        .iter()
        .find(|t| matches!(t, TokenKind::DataLines(_)))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(v, &vec!["Alice   14".to_string(), "Bob     16".to_string()]);
}

#[test]
fn datalines4_terminator() {
    // Les variantes `4` se terminent par `;;;;` (les `;` isolés sont des
    // données ordinaires).
    let src = "datalines4;\na;b\n; not the end\n;;;;\nrun;";
    let k = kinds(src);
    let TokenKind::DataLines(v) = k
        .iter()
        .find(|t| matches!(t, TokenKind::DataLines(_)))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(v, &vec!["a;b".to_string(), "; not the end".to_string()]);
    assert!(k.contains(&TokenKind::Ident("run".into())));
}

#[test]
fn cards_keyword_also_captures() {
    let src = "cards;\nx\n;\n";
    let k = kinds(src);
    assert!(
        k.iter()
            .any(|t| matches!(t, TokenKind::DataLines(v) if v == &vec!["x".to_string()]))
    );
}

#[test]
fn cards_as_variable_name_not_armed() {
    // `cards` en plein milieu d'un statement n'arme PAS le mode verbatim.
    let k = kinds("x = cards + 1;");
    assert!(!k.iter().any(|t| matches!(t, TokenKind::DataLines(_))));
    assert!(k.contains(&TokenKind::Ident("cards".into())));
}
