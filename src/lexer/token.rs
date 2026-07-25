use super::*;

impl<'a> Lexer<'a> {
    pub(super) fn next_token(&mut self) -> Result<Token> {
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
    pub(super) fn capture_datalines(&mut self, four: bool) -> Vec<String> {
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
    pub(super) fn capture_iml_body(&mut self) -> String {
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
    pub(super) fn matches_kw_ci(&self, kw: &str) -> bool {
        let kb = kw.as_bytes();
        if self.pos + kb.len() > self.bytes.len() {
            return false;
        }
        self.bytes[self.pos..self.pos + kb.len()]
            .iter()
            .zip(kb)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
    }

    pub(super) fn next_token_inner(&mut self) -> Result<Token> {
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
}
