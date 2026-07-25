use super::*;

// ───────────────────────── Lexer IML ─────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Tok {
    Num(f64),
    Ident(String),
    Str(String),
    LBrace, RBrace, LBracket, RBracket, LParen, RParen,
    Comma, Semi, Star, Slash, Plus, Minus, Hash, At, Quote, Colon, Dot,
    Eq, Ne, Lt, Le, Gt, Ge,
    Eof,
}

/// Lexe le corps IML brut. L'apostrophe `'` est **toujours** un token Quote
/// (transposée) sauf en position de chaîne PRINT — mais dans cette grammaire
/// les chaînes utilisent les guillemets doubles `"..."` (cf. fixtures). On
/// supporte aussi `'...'` comme chaîne UNIQUEMENT si l'apostrophe ouvre en
/// position de début d'item PRINT ; pour simplifier et lever l'ambiguïté, on
/// traite ici `'` collé à la fin d'une expression comme une transposée et on
/// réserve les chaînes aux guillemets doubles. Les chaînes simples `'...'` ne
/// sont donc pas supportées (documenté ; les fixtures utilisent `"..."`).
pub(super) fn lex(src: &str) -> Result<Vec<Tok>> {
    let b = src.as_bytes();
    let mut i = 0;
    let n = b.len();
    let mut out = Vec::new();
    while i < n {
        let c = b[i];
        // Espaces.
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Commentaires SAS /* ... */.
        if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < n && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        match c {
            b'0'..=b'9' | b'.' if c != b'.' || (i + 1 < n && b[i + 1].is_ascii_digit()) => {
                let start = i;
                while i < n && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i < n && b[i] == b'.' {
                    i += 1;
                    while i < n && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i < n && (b[i] == b'e' || b[i] == b'E') {
                    let mark = i;
                    i += 1;
                    if i < n && (b[i] == b'+' || b[i] == b'-') {
                        i += 1;
                    }
                    if i < n && b[i].is_ascii_digit() {
                        while i < n && b[i].is_ascii_digit() {
                            i += 1;
                        }
                    } else {
                        i = mark;
                    }
                }
                let txt = &src[start..i];
                let v: f64 = txt.parse().map_err(|_| {
                    SasError::runtime(format!("IML: invalid number '{txt}'"))
                })?;
                out.push(Tok::Num(v));
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = i;
                while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                out.push(Tok::Ident(src[start..i].to_string()));
            }
            b'"' => {
                i += 1;
                let start = i;
                let mut s = String::new();
                while i < n {
                    if b[i] == b'"' {
                        if i + 1 < n && b[i + 1] == b'"' {
                            s.push('"');
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    s.push(b[i] as char);
                    i += 1;
                }
                let _ = start;
                if i >= n {
                    return Err(SasError::runtime("IML: unterminated string"));
                }
                i += 1; // closing quote
                out.push(Tok::Str(s));
            }
            b'{' => { out.push(Tok::LBrace); i += 1; }
            b'}' => { out.push(Tok::RBrace); i += 1; }
            b'[' => { out.push(Tok::LBracket); i += 1; }
            b']' => { out.push(Tok::RBracket); i += 1; }
            b'(' => { out.push(Tok::LParen); i += 1; }
            b')' => { out.push(Tok::RParen); i += 1; }
            b',' => { out.push(Tok::Comma); i += 1; }
            b';' => { out.push(Tok::Semi); i += 1; }
            b'*' => { out.push(Tok::Star); i += 1; }
            b'/' => { out.push(Tok::Slash); i += 1; }
            b'+' => { out.push(Tok::Plus); i += 1; }
            b'-' => { out.push(Tok::Minus); i += 1; }
            b'#' => { out.push(Tok::Hash); i += 1; }
            b'@' => { out.push(Tok::At); i += 1; }
            b'\'' => { out.push(Tok::Quote); i += 1; }
            b':' => { out.push(Tok::Colon); i += 1; }
            b'.' => { out.push(Tok::Dot); i += 1; }
            b'=' => { out.push(Tok::Eq); i += 1; }
            b'<' => {
                if i + 1 < n && b[i + 1] == b'=' { out.push(Tok::Le); i += 2; }
                else { out.push(Tok::Lt); i += 1; }
            }
            b'>' => {
                if i + 1 < n && b[i + 1] == b'=' { out.push(Tok::Ge); i += 2; }
                else { out.push(Tok::Gt); i += 1; }
            }
            b'^' | b'~' => {
                if i + 1 < n && b[i + 1] == b'=' { out.push(Tok::Ne); i += 2; }
                else { return Err(SasError::runtime(format!("IML: unexpected character '{}'", c as char))); }
            }
            other => {
                return Err(SasError::runtime(format!(
                    "IML: unexpected character '{}'",
                    other as char
                )));
            }
        }
    }
    out.push(Tok::Eof);
    Ok(out)
}
