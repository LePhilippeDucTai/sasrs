use super::*;

fn stream(src: &SourceFile) -> StatementStream<'_> {
    StatementStream::new(src).unwrap()
}

#[test]
fn peek_next_eof() {
    let src = SourceFile::new("x = 1;");
    let mut ts = stream(&src);
    assert!(ts.peek().is_kw("x"));
    assert!(ts.next().is_kw("x"));
    assert_eq!(ts.next().kind, TokenKind::Eq);
    assert_eq!(ts.next().kind, TokenKind::Num(1.0));
    assert_eq!(ts.next().kind, TokenKind::Semi);
    assert!(ts.at_eof());
    // next() reste sur Eof.
    assert_eq!(ts.next().kind, TokenKind::Eof);
    assert_eq!(ts.next().kind, TokenKind::Eof);
}

#[test]
fn expect_semi_ok_and_err() {
    let src = SourceFile::new("; x");
    let mut ts = stream(&src);
    assert!(ts.expect_semi().is_ok());
    assert!(ts.expect_semi().is_err());
}

#[test]
fn skip_to_semi_consumes_semi() {
    let src = SourceFile::new("a b c ; next");
    let mut ts = stream(&src);
    ts.skip_to_semi();
    assert!(ts.peek().is_kw("next"));
    // À EOF : no-op.
    ts.skip_to_semi();
    ts.next();
    ts.skip_to_semi();
    assert!(ts.at_eof());
}

#[test]
fn dataset_ref_one_and_two_level() {
    let src = SourceFile::new("a mylib.b ;");
    let mut ts = stream(&src);
    let r1 = ts.parse_dataset_ref().unwrap();
    assert_eq!(r1, DatasetRef { libref: None, name: "a".into() });
    let r2 = ts.parse_dataset_ref().unwrap();
    assert_eq!(
        r2,
        DatasetRef { libref: Some("mylib".into()), name: "b".into() }
    );
    assert_eq!(ts.peek().kind, TokenKind::Semi);
}

#[test]
fn dataset_ref_rejects_long_names() {
    let long = "x".repeat(33);
    let src1 = SourceFile::new(format!("{long};"));
    assert!(stream(&src1).parse_dataset_ref().is_err());
    let src2 = SourceFile::new("librefnine.a;");
    assert!(stream(&src2).parse_dataset_ref().is_err());
    let src3 = SourceFile::new("lib.;");
    assert!(stream(&src3).parse_dataset_ref().is_err());
}

#[test]
fn name_list_until_semi() {
    let src = SourceFile::new("a b c;");
    let mut ts = stream(&src);
    assert_eq!(ts.parse_name_list().unwrap(), vec!["a", "b", "c"]);
    assert_eq!(ts.peek().kind, TokenKind::Semi);
    // Liste vide → erreur.
    assert!(ts.parse_name_list().is_err());
}

#[test]
fn next_block_none_on_empty_and_inert() {
    let src = SourceFile::new("");
    assert!(stream(&src).next_block().is_none());
    let src = SourceFile::new(" ;;  ; ");
    assert!(stream(&src).next_block().is_none());
    let src = SourceFile::new("* just a comment statement ;");
    assert!(stream(&src).next_block().is_none());
}

#[test]
fn lone_run_is_empty_block() {
    let src = SourceFile::new("run;");
    let mut ts = stream(&src);
    let (block, span) = ts.next_block().unwrap();
    assert!(matches!(block.unwrap(), Block::Empty));
    assert_eq!(span, Span::new(0, 4)); // couvre `run;`
    assert!(ts.next_block().is_none());
}

#[test]
fn macro_call_errors_and_recovers() {
    let src = SourceFile::new("%let x = 1; run;");
    let mut ts = stream(&src);
    let (block, _) = ts.next_block().unwrap();
    let Err(err) = block else { panic!("expected an error") };
    assert!(err.to_string().contains("macro facility"));
    // Récupération : le bloc suivant est le run; isolé.
    let (block, _) = ts.next_block().unwrap();
    assert!(matches!(block.unwrap(), Block::Empty));
}

#[test]
fn unknown_statement_errors_and_recovers() {
    let src = SourceFile::new("frobnicate a b; run;");
    let mut ts = stream(&src);
    let (block, _) = ts.next_block().unwrap();
    let Err(err) = block else { panic!("expected an error") };
    assert!(err.to_string().contains("FROBNICATE"));
    let (block, _) = ts.next_block().unwrap();
    assert!(matches!(block.unwrap(), Block::Empty));
}

#[test]
fn non_ident_head_errors() {
    let src = SourceFile::new("= 1; run;");
    let mut ts = stream(&src);
    let (block, _) = ts.next_block().unwrap();
    assert!(block.is_err());
    let (block, _) = ts.next_block().unwrap();
    assert!(matches!(block.unwrap(), Block::Empty));
}

#[test]
fn proc_without_name_errors() {
    let src = SourceFile::new("proc ; run;");
    let mut ts = stream(&src);
    let (block, _) = ts.next_block().unwrap();
    assert!(block.is_err());
    // skip_to_step_boundary a consommé le run; de récupération.
    assert!(ts.next_block().is_none());
}

#[test]
fn step_boundary_stops_before_block_heads() {
    let src = SourceFile::new("x = 1; y = 2; data b;");
    let mut ts = stream(&src);
    ts.skip_to_step_boundary();
    assert!(ts.peek().is_kw("data"));

    let src = SourceFile::new("garbage tokens run; data b;");
    let mut ts = stream(&src);
    ts.skip_to_step_boundary();
    assert!(ts.peek().is_kw("data"));

    let src = SourceFile::new("x = 1; title 'boundary';");
    let mut ts = stream(&src);
    ts.skip_to_step_boundary();
    assert!(ts.peek().is_kw("title"));
}

#[test]
fn title_levels() {
    assert_eq!(title_level("title"), Some(1));
    assert_eq!(title_level("TITLE3"), Some(3));
    assert_eq!(title_level("title9"), Some(9));
    assert_eq!(title_level("title0"), None);
    assert_eq!(title_level("title10"), None);
    assert_eq!(title_level("titles"), None);
    assert_eq!(title_level("data"), None);
}

#[test]
fn footnote_levels() {
    assert_eq!(footnote_level("footnote"), Some(1));
    assert_eq!(footnote_level("FOOTNOTE3"), Some(3));
    assert_eq!(footnote_level("footnote9"), Some(9));
    assert_eq!(footnote_level("footnote0"), None);
    assert_eq!(footnote_level("footnote10"), None);
    assert_eq!(footnote_level("footnotes"), None);
    assert_eq!(footnote_level("title"), None);
}
