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
    /// Mode DATALINES/CARDS armé (M14) : `Some(true)` pour les variantes `4`
    /// (`datalines4`/`cards4`, terminateur `;;;;`), `Some(false)` pour les
    /// variantes simples (terminateur = ligne ne contenant qu'un `;`). Armé
    /// quand un Ident de tête de statement est l'un de ces mots-clés ;
    /// déclenche la capture verbatim AU `;` qui termine ce statement.
    datalines_armed: Option<bool>,
    /// Lignes verbatim en attente d'émission (M14) : capturées juste après le
    /// `;` d'un `datalines;`/`cards;`, émises au token suivant sous forme de
    /// `TokenKind::DataLines`.
    pending_datalines: Option<Vec<String>>,
    /// Vrai quand le dernier Ident émis en tête de statement était `proc`
    /// (M28a) : sert à reconnaître la séquence `proc iml` sur deux tokens.
    prev_ident_was_proc: bool,
    /// Mode PROC IML armé (M28a) : déclenché quand l'Ident `iml` suit `proc`.
    /// La capture verbatim du corps IML est lancée au `;` qui termine le
    /// statement `proc iml`.
    iml_armed: bool,
    /// Corps IML verbatim en attente d'émission (M28a) : capturé juste après le
    /// `;` du statement `proc iml`, émis au token suivant sous forme de
    /// `TokenKind::ImlBody`.
    pending_iml_body: Option<String>,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            at_stmt_start: true,
            datalines_armed: None,
            pending_datalines: None,
            prev_ident_was_proc: false,
            iml_armed: false,
            pending_iml_body: None,
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
        // Données verbatim en attente (capturées juste après le `;` d'un
        // `datalines;`/`cards;`) : les émettre AVANT de relexer normalement.
        if let Some(lines) = self.pending_datalines.take() {
            let span = Span::new(self.pos, self.pos);
            // Le `*` d'un commentaire-statement ne doit pas s'ouvrir juste
            // après les données : on reste « début de statement » comme après
            // un `;`.
            self.at_stmt_start = true;
            return Ok(Token {
                kind: TokenKind::DataLines(lines),
                span,
            });
        }
        // Corps IML verbatim en attente (capturé juste après le `;` du
        // statement `proc iml`) : l'émettre AVANT de relexer normalement.
        if let Some(body) = self.pending_iml_body.take() {
            let span = Span::new(self.pos, self.pos);
            self.at_stmt_start = true;
            return Ok(Token {
                kind: TokenKind::ImlBody(body),
                span,
            });
        }
        let tok = self.next_token_inner()?;
        // Un `*` en tête du PROCHAIN statement ouvrira un commentaire.
        self.at_stmt_start = tok.kind == TokenKind::Semi;
        // Le `;` qui termine un statement `datalines`/`cards`/`datalines4`/
        // `cards4` déclenche la capture verbatim : on lit les lignes brutes
        // jusqu'au terminateur (exclu) et on les met en attente.
        if tok.kind == TokenKind::Semi {
            if let Some(four) = self.datalines_armed.take() {
                let lines = self.capture_datalines(four);
                self.pending_datalines = Some(lines);
            }
            if self.iml_armed {
                self.iml_armed = false;
                let body = self.capture_iml_body();
                self.pending_iml_body = Some(body);
            }
        }
        Ok(tok)
    }

    /// Capture les lignes verbatim d'un bloc DATALINES/CARDS. À l'entrée,
    /// `self.pos` est juste APRÈS le `;` qui a terminé le statement. On
    /// avance jusqu'au début de la ligne suivante (les éventuels caractères
    /// restants sur la ligne du `;` sont ignorés — fidèle à SAS qui exige
    /// `datalines;` seul sur sa ligne), puis on capture chaque ligne jusqu'au
    /// terminateur : pour les variantes simples (`four == false`) une ligne ne
    /// contenant qu'un `;` (espaces tolérés), pour les variantes `4`
    /// (`four == true`) une ligne contenant `;;;;`. Le terminateur est
    /// consommé mais N'EST PAS une donnée.
    fn capture_datalines(&mut self, four: bool) -> Vec<String> {
        // Aller à la fin de la ligne courante (celle du `datalines;`).
        while self.peek().is_some_and(|c| c != b'\n') {
            self.pos += 1;
        }
        if self.peek() == Some(b'\n') {
            self.pos += 1;
        }
        let mut lines = Vec::new();
        loop {
            if self.peek().is_none() {
                // EOF avant le terminateur : on prend ce qui reste.
                return lines;
            }
            let line_start = self.pos;
            while self.peek().is_some_and(|c| c != b'\n') {
                self.pos += 1;
            }
            // Ligne SANS le `\n` final ; un éventuel `\r` de fin est retiré.
            let mut line = &self.src[line_start..self.pos];
            if line.ends_with('\r') {
                line = &line[..line.len() - 1];
            }
            // Consommer le `\n`.
            if self.peek() == Some(b'\n') {
                self.pos += 1;
            }
            let trimmed = line.trim();
            let is_terminator = if four {
                trimmed == ";;;;"
            } else {
                trimmed == ";"
            };
            if is_terminator {
                return lines;
            }
            lines.push(line.to_string());
        }
    }

    /// Capture le corps verbatim d'un bloc `PROC IML ... QUIT;` (M28a). À
    /// l'entrée, `self.pos` est juste APRÈS le `;` qui a terminé le statement
    /// `proc iml`. On scanne le texte BRUT (sans le lexer SAS — l'apostrophe
    /// `'` y est une transposée, `#` un produit de Hadamard) jusqu'au mot-clé
    /// `quit` de niveau supérieur suivi (espaces tolérés) d'un `;`. Le `quit;`
    /// est consommé mais N'EST PAS inclus dans le corps. Si aucun `quit;` n'est
    /// trouvé (EOF), on prend tout le reste — le parser IML signalera l'erreur.
    ///
    /// Frontières de mot pour `quit` : précédé d'un non-identifiant (ou début)
    /// et suivi d'un non-identifiant. Les commentaires `/* */` et les chaînes
    /// `'...'`/`"..."` ne sont PAS interprétés ici : un `quit` à l'intérieur
    /// d'une chaîne IML est improbable et hors périmètre v1 (documenté).
    fn capture_iml_body(&mut self) -> String {
        let body_start = self.pos;
        let n = self.bytes.len();
        while self.pos < n {
            // Frontière gauche : début ou caractère non-identifiant.
            let left_ok = self.pos == 0 || {
                let p = self.bytes[self.pos - 1];
                !(p.is_ascii_alphanumeric() || p == b'_')
            };
            if left_ok && self.matches_kw_ci("quit") {
                let after = self.pos + 4;
                let right_ok = match self.bytes.get(after) {
                    Some(c) => !(c.is_ascii_alphanumeric() || *c == b'_'),
                    None => true,
                };
                if right_ok {
                    let body_end = self.pos;
                    // Avancer après `quit` puis jusqu'au `;` inclus.
                    self.pos = after;
                    while self.pos < n && self.bytes[self.pos] != b';' {
                        self.pos += 1;
                    }
                    if self.pos < n {
                        self.pos += 1; // le `;`
                    }
                    return self.src[body_start..body_end].to_string();
                }
            }
            self.pos += 1;
        }
        // Pas de `quit;` : tout le reste forme le corps.
        self.src[body_start..n].to_string()
    }

    /// Vrai si `self.bytes[self.pos..]` commence par `kw` (insensible casse).
    fn matches_kw_ci(&self, kw: &str) -> bool {
        let kb = kw.as_bytes();
        if self.pos + kb.len() > self.bytes.len() {
            return false;
        }
        self.bytes[self.pos..self.pos + kb.len()]
            .iter()
            .zip(kb)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
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
        let lower = raw.to_ascii_lowercase();
        // Armement du mode DATALINES/CARDS : seulement en tête de statement
        // (sinon `cards` pourrait être un nom de variable en plein milieu
        // d'une expression). La capture verbatim est déclenchée au `;` qui
        // termine ce statement (cf. `next_token`).
        if self.at_stmt_start {
            match lower.as_str() {
                "datalines" | "cards" => self.datalines_armed = Some(false),
                "datalines4" | "cards4" => self.datalines_armed = Some(true),
                _ => {}
            }
        }
        // Reconnaissance de `proc iml` sur deux Idents (M28a). `proc` n'est pas
        // un mot-clé réservé ici : on n'arme que si `proc` apparaît en tête de
        // statement, puis `iml` immédiatement après.
        let was_proc = self.prev_ident_was_proc;
        self.prev_ident_was_proc = self.at_stmt_start && lower == "proc";
        if was_proc && lower == "iml" {
            self.iml_armed = true;
        }
        let kind = match lower.as_str() {
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
        assert_eq!(
            v,
            &vec!["a;b".to_string(), "; not the end".to_string()]
        );
        assert!(k.contains(&TokenKind::Ident("run".into())));
    }

    #[test]
    fn cards_keyword_also_captures() {
        let src = "cards;\nx\n;\n";
        let k = kinds(src);
        assert!(k
            .iter()
            .any(|t| matches!(t, TokenKind::DataLines(v) if v == &vec!["x".to_string()])));
    }

    #[test]
    fn cards_as_variable_name_not_armed() {
        // `cards` en plein milieu d'un statement n'arme PAS le mode verbatim.
        let k = kinds("x = cards + 1;");
        assert!(!k.iter().any(|t| matches!(t, TokenKind::DataLines(_))));
        assert!(k.contains(&TokenKind::Ident("cards".into())));
    }
}
