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

mod token;
mod literal;

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
}

#[cfg(test)]
mod tests;

