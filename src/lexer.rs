use crate::error::{Result, SasError};
use crate::token::{Span, StrSuffix, Token, TokenKind};

/// Hand-written lexer for SAS source. Word operators (eq, ne, lt, le, gt, ge,
/// and, or, not, in) are mapped to operator tokens; everything else
/// identifier-shaped stays an `Ident` and is matched contextually by parsers.
pub struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    /// Vrai en début de statement (début de source ou après `;`) : un `*`
    /// y ouvre un commentaire-statement `* texte ;`, consommé comme trivia
    /// (son contenu peut contenir n'importe quoi sauf `;`, y compris des
    /// caractères qui ne se lexent pas — fidèle à SAS).
    at_stmt_start: bool,
    /// `DATALINES`/`CARDS` (M14.1) : armé après l'émission du `;` qui ferme
    /// le statement `datalines;`/`cards;`. `Some(is4)` = la prochaine demande
    /// de token doit CAPTURER les lignes de données brutes jusqu'au
    /// terminateur (`;` seul, ou `;;;;` si `is4`). Le contenu n'est JAMAIS
    /// lexé comme du SAS — c'est ce qui permet d'y trouver `@`, des quotes
    /// déséquilibrées, etc. sans erreur de lexing.
    capture_datalines: Option<bool>,
    /// Mémorise le mot-clé de tête du statement courant QUAND il est en
    /// début de statement : sert à détecter `datalines;`/`cards;` (et les
    /// variantes `datalines4`/`cards4`) lorsqu'on émet leur `;`.
    pending_datalines_kw: Option<bool>,
}

/// Si `name` (insensible à la casse) est le mot-clé d'un statement
/// `DATALINES`/`CARDS`/`LINES` (ou leur variante `4`), renvoie `Some(is4)`.
fn datalines_keyword(name: &str) -> Option<bool> {
    match name.to_ascii_lowercase().as_str() {
        "datalines" | "cards" | "lines" => Some(false),
        "datalines4" | "cards4" | "lines4" => Some(true),
        _ => None,
    }
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            at_stmt_start: true,
            capture_datalines: None,
            pending_datalines_kw: None,
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>> {
        let mut out = Vec::new();
        loop {
            let tok = self.next_token()?;
            let eof = tok.kind == TokenKind::Eof;
            out.push(tok);
            if eof {
                return Ok(out);
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<u8> {
        self.bytes.get(self.pos + 1).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_trivia(&mut self) -> Result<()> {
        loop {
            match self.peek() {
                Some(b) if b.is_ascii_whitespace() => {
                    self.pos += 1;
                }
                Some(b'/') if self.peek2() == Some(b'*') => {
                    let start = self.pos;
                    self.pos += 2;
                    loop {
                        match self.peek() {
                            Some(b'*') if self.peek2() == Some(b'/') => {
                                self.pos += 2;
                                break;
                            }
                            Some(_) => self.pos += 1,
                            None => {
                                return Err(SasError::parse(
                                    "unterminated comment",
                                    Span::new(start, self.pos),
                                ));
                            }
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn next_token(&mut self) -> Result<Token> {
        // DATALINES/CARDS : si la capture est armée (le `;` du statement
        // `datalines;` vient d'être émis), lire les lignes brutes AVANT toute
        // tentative de lexing SAS.
        if let Some(is4) = self.capture_datalines.take() {
            return Ok(self.capture_data_lines(is4));
        }

        let at_start = self.at_stmt_start;
        let tok = self.next_token_inner()?;
        match &tok.kind {
            TokenKind::Ident(name) if at_start => {
                // Tête de statement : mémorise si c'est datalines/cards.
                self.pending_datalines_kw = datalines_keyword(name);
            }
            TokenKind::Semi => {
                // Fin de statement : si la tête était datalines/cards, armer
                // la capture pour le prochain appel.
                if let Some(is4) = self.pending_datalines_kw.take() {
                    self.capture_datalines = Some(is4);
                }
            }
            // Tout autre token entre la tête et le `;` invalide la détection
            // (ex. `datalines foo;` n'est pas un statement datalines pur — le
            // parser en fera une erreur de toute façon).
            _ => {
                self.pending_datalines_kw = None;
            }
        }
        // Un `*` en tête du PROCHAIN statement ouvrira un commentaire.
        self.at_stmt_start = tok.kind == TokenKind::Semi;
        Ok(tok)
    }

    /// Capture les lignes de données brutes après `datalines;`/`cards;`.
    /// Les données commencent à la LIGNE SUIVANT le `;` (on saute le reste de
    /// la ligne du `;`). On lit ligne par ligne jusqu'à une ligne dont le
    /// contenu trimé est exactement le terminateur : `;` (normal) ou `;;;;`
    /// (variante `4`). Le terminateur N'EST PAS inclus mais EST consommé (la
    /// position se place après lui), de sorte que le lexing reprend ensuite
    /// normalement. EOF avant terminateur : on capture ce qu'on a (best
    /// effort, comme SAS qui clôt l'étape).
    fn capture_data_lines(&mut self, is4: bool) -> Token {
        let start = self.pos;
        // Sauter jusqu'à la fin de la ligne courante (celle du `;`).
        while self.peek().is_some_and(|c| c != b'\n') {
            self.pos += 1;
        }
        if self.peek() == Some(b'\n') {
            self.pos += 1;
        }
        let terminator = if is4 { ";;;;" } else { ";" };
        let mut lines: Vec<String> = Vec::new();
        loop {
            if self.peek().is_none() {
                break;
            }
            // Lire une ligne (jusqu'au `\n` exclu).
            let line_start = self.pos;
            while self.peek().is_some_and(|c| c != b'\n') {
                self.pos += 1;
            }
            let raw = &self.src[line_start..self.pos];
            // Consommer le `\n` (s'il existe).
            if self.peek() == Some(b'\n') {
                self.pos += 1;
            }
            // Ligne sans CR final.
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if line.trim() == terminator {
                // Terminateur : consommé, non inclus.
                break;
            }
            lines.push(line.to_string());
        }
        // Le lexing reprend en début de statement.
        self.at_stmt_start = true;
        Token {
            kind: TokenKind::DataLines(lines),
            span: Span::new(start, self.pos),
        }
    }

    fn next_token_inner(&mut self) -> Result<Token> {
        self.skip_trivia()?;
        // Commentaire-statement : `* texte ;` en début de statement, consommé
        // jusqu'au `;` inclus (ou EOF), puis on recommence.
        while self.at_stmt_start && self.peek() == Some(b'*') {
            while self.peek().is_some_and(|c| c != b';') {
                self.pos += 1;
            }
            if self.peek() == Some(b';') {
                self.pos += 1;
            }
            self.skip_trivia()?;
        }
        let start = self.pos;
        let Some(b) = self.peek() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                span: Span::new(start, start),
            });
        };

        let kind = match b {
            b'\'' | b'"' => return self.lex_string(),
            b'0'..=b'9' => return self.lex_number(),
            b'.' if self.peek2().is_some_and(|c| c.is_ascii_digit()) => return self.lex_number(),
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => return Ok(self.lex_ident()),
            b'%' => {
                self.pos += 1;
                if self.peek().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_') {
                    let name_start = self.pos;
                    while self
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
                    {
                        self.pos += 1;
                    }
                    TokenKind::MacroCall(self.src[name_start..self.pos].to_string())
                } else {
                    return Err(SasError::parse(
                        "unexpected character '%'",
                        Span::new(start, self.pos),
                    ));
                }
            }
            b';' => {
                self.pos += 1;
                TokenKind::Semi
            }
            b'(' => {
                self.pos += 1;
                TokenKind::LParen
            }
            b')' => {
                self.pos += 1;
                TokenKind::RParen
            }
            b'{' => {
                self.pos += 1;
                TokenKind::LBrace
            }
            b'}' => {
                self.pos += 1;
                TokenKind::RBrace
            }
            b'[' => {
                self.pos += 1;
                TokenKind::LBracket
            }
            b']' => {
                self.pos += 1;
                TokenKind::RBracket
            }
            b',' => {
                self.pos += 1;
                TokenKind::Comma
            }
            b'.' => {
                self.pos += 1;
                TokenKind::Dot
            }
            b'+' => {
                self.pos += 1;
                TokenKind::Plus
            }
            b'-' => {
                self.pos += 1;
                TokenKind::Minus
            }
            b'*' => {
                self.pos += 1;
                if self.peek() == Some(b'*') {
                    self.pos += 1;
                    TokenKind::Power
                } else {
                    TokenKind::Star
                }
            }
            b'/' => {
                self.pos += 1;
                TokenKind::Slash
            }
            b'|' => {
                self.pos += 1;
                if self.peek() == Some(b'|') {
                    self.pos += 1;
                    TokenKind::Concat
                } else {
                    TokenKind::Or
                }
            }
            b'&' => {
                self.pos += 1;
                TokenKind::And
            }
            b'!' => {
                self.pos += 1;
                if self.peek() == Some(b'!') {
                    self.pos += 1;
                    TokenKind::Concat
                } else {
                    TokenKind::Or
                }
            }
            b'^' | b'~' => {
                self.pos += 1;
                if self.peek() == Some(b'=') {
                    self.pos += 1;
                    TokenKind::Ne
                } else {
                    TokenKind::Not
                }
            }
            b'<' => {
                self.pos += 1;
                if self.peek() == Some(b'=') {
                    self.pos += 1;
                    TokenKind::Le
                } else {
                    TokenKind::Lt
                }
            }
            b'>' => {
                self.pos += 1;
                if self.peek() == Some(b'=') {
                    self.pos += 1;
                    TokenKind::Ge
                } else {
                    TokenKind::Gt
                }
            }
            b'=' => {
                self.pos += 1;
                TokenKind::Eq
            }
            b'$' => {
                self.pos += 1;
                TokenKind::Dollar
            }
            b'@' => {
                self.pos += 1;
                TokenKind::At
            }
            b':' => {
                self.pos += 1;
                TokenKind::Colon
            }
            other => {
                self.pos += 1;
                return Err(SasError::parse(
                    format!("unexpected character '{}'", other as char),
                    Span::new(start, self.pos),
                ));
            }
        };

        Ok(Token {
            kind,
            span: Span::new(start, self.pos),
        })
    }

    fn lex_ident(&mut self) -> Token {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            self.pos += 1;
        }
        let raw = &self.src[start..self.pos];
        let kind = match raw.to_ascii_lowercase().as_str() {
            "eq" => TokenKind::Eq,
            "ne" => TokenKind::Ne,
            "lt" => TokenKind::Lt,
            "le" => TokenKind::Le,
            "gt" => TokenKind::Gt,
            "ge" => TokenKind::Ge,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            "not" => TokenKind::Not,
            "in" => TokenKind::In,
            _ => TokenKind::Ident(raw.to_string()),
        };
        Token {
            kind,
            span: Span::new(start, self.pos),
        }
    }

    fn lex_number(&mut self) -> Result<Token> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') && self.peek2().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if self.peek().is_some_and(|c| c == b'e' || c == b'E') {
            let mark = self.pos;
            self.pos += 1;
            if self.peek().is_some_and(|c| c == b'+' || c == b'-') {
                self.pos += 1;
            }
            if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
            } else {
                // Not an exponent after all (e.g. `1e` would be `1` then ident `e`).
                self.pos = mark;
            }
        }
        let text = &self.src[start..self.pos];
        let value: f64 = text.parse().map_err(|_| {
            SasError::parse(
                format!("invalid numeric literal '{text}'"),
                Span::new(start, self.pos),
            )
        })?;
        Ok(Token {
            kind: TokenKind::Num(value),
            span: Span::new(start, self.pos),
        })
    }

    fn lex_string(&mut self) -> Result<Token> {
        let start = self.pos;
        let quote = self.bump().unwrap();
        let mut value = String::new();
        loop {
            match self.bump() {
                Some(b) if b == quote => {
                    // Doubled quote is an escaped quote character.
                    if self.peek() == Some(quote) {
                        self.pos += 1;
                        value.push(quote as char);
                    } else {
                        break;
                    }
                }
                Some(b) => value.push(b as char),
                None => {
                    return Err(SasError::parse(
                        "unterminated string literal",
                        Span::new(start, self.pos),
                    ));
                }
            }
        }
        // Optional literal suffix: d, t, dt, n (case-insensitive), must be
        // immediately adjacent and not followed by more identifier characters.
        let suffix_start = self.pos;
        let mut suffix_text = String::new();
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            suffix_text.push(self.bump().unwrap() as char);
        }
        let suffix = match suffix_text.to_ascii_lowercase().as_str() {
            "" => StrSuffix::None,
            "d" => StrSuffix::Date,
            "t" => StrSuffix::Time,
            "dt" => StrSuffix::DateTime,
            "n" => StrSuffix::Name,
            other => {
                return Err(SasError::parse(
                    format!("invalid string literal suffix '{other}'"),
                    Span::new(suffix_start, self.pos),
                ));
            }
        };
        Ok(Token {
            kind: TokenKind::Str { value, suffix },
            span: Span::new(start, self.pos),
        })
    }
}

#[cfg(test)]
mod tests {
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

    // ---- M14.1 : DATALINES / CARDS capture, `@`, `:` ----

    #[test]
    fn datalines_captures_raw_lines() {
        let k = kinds("data a;\ninput x;\ndatalines;\n10\n20\n;\nrun;");
        // The DataLines token holds the two raw data lines.
        assert!(k.contains(&TokenKind::DataLines(vec!["10".into(), "20".into()])));
        // The terminator `;` line is consumed, not emitted as a Semi after it.
        // Lexing resumes with `run;`.
        let tail: Vec<&TokenKind> = k.iter().rev().take(3).collect();
        assert_eq!(tail[0], &TokenKind::Eof);
        assert_eq!(tail[1], &TokenKind::Semi);
        assert_eq!(tail[2], &TokenKind::Ident("run".into()));
    }

    #[test]
    fn datalines_content_never_lexed_as_sas() {
        // Content with `@`, unbalanced quote, `%` — would all fail normal
        // lexing, but inside datalines it is captured verbatim.
        let k = kinds("data a;\ninput x $;\ndatalines;\n@weird \"unbalanced %macro\n;\nrun;");
        assert!(k.contains(&TokenKind::DataLines(vec![
            "@weird \"unbalanced %macro".into()
        ])));
    }

    #[test]
    fn cards_is_an_alias() {
        let k = kinds("data a;\ninput x;\ncards;\n1\n;\nrun;");
        assert!(k.contains(&TokenKind::DataLines(vec!["1".into()])));
    }

    #[test]
    fn datalines4_terminator_is_four_semicolons() {
        let k = kinds("data a;\ninput x $;\ndatalines4;\nline;with;semis\n;;;;\nrun;");
        assert!(k.contains(&TokenKind::DataLines(vec!["line;with;semis".into()])));
    }

    #[test]
    fn at_and_colon_tokens() {
        let k = kinds("input @1 x :8.;");
        assert!(k.contains(&TokenKind::At));
        assert!(k.contains(&TokenKind::Colon));
    }

    #[test]
    fn datalines_not_armed_when_not_statement_head() {
        // `x = datalines;` — `datalines` is not at statement start, so no
        // capture is armed (it is just an identifier).
        let k = kinds("x = datalines; y = 1;");
        assert!(!k.iter().any(|t| matches!(t, TokenKind::DataLines(_))));
    }
}
