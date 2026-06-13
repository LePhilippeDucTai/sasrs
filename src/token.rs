/// Byte span into the submitted source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// Suffix on a string literal: '01jan2020'd, '12:00't, '01jan2020:12:00:00'dt, 'name'n.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrSuffix {
    None,
    Date,
    Time,
    DateTime,
    Name,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// Identifier or keyword; raw spelling preserved, matched case-insensitively.
    Ident(String),
    Num(f64),
    Str { value: String, suffix: StrSuffix },
    /// `%name` — reserved for the macro facility (later phase).
    MacroCall(String),
    Semi,
    LParen,
    RParen,
    /// `{` (array dimension/subscript delimiter)
    LBrace,
    /// `}`
    RBrace,
    /// `[` (array dimension/subscript delimiter)
    LBracket,
    /// `]`
    RBracket,
    Comma,
    Dot,
    Plus,
    Minus,
    Star,
    Slash,
    /// `**`
    Power,
    /// `||`
    Concat,
    Lt,
    Le,
    Gt,
    Ge,
    /// `$` (char marker in LENGTH/INPUT/FORMAT statements)
    Dollar,
    /// `@` (column pointer / line hold in INPUT/PUT — M14).
    At,
    /// `:` (format modifier in INPUT/PUT, label separator — M14).
    Colon,
    /// Données verbatim capturées après `datalines;`/`cards;` (M14). Le
    /// lexer bascule en mode verbatim après le `;` qui termine un statement
    /// `datalines`/`cards`/`datalines4`/`cards4` et émet ce token portant les
    /// lignes brutes (terminateur exclu).
    DataLines(Vec<String>),
    /// `=` (assignment or comparison depending on context)
    Eq,
    /// `^=`, `~=`, `ne`
    Ne,
    And,
    Or,
    Not,
    In,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    /// True if this token is the given identifier/keyword (case-insensitive).
    pub fn is_kw(&self, kw: &str) -> bool {
        matches!(&self.kind, TokenKind::Ident(s) if s.eq_ignore_ascii_case(kw))
    }

    pub fn ident(&self) -> Option<&str> {
        match &self.kind {
            TokenKind::Ident(s) => Some(s),
            _ => None,
        }
    }
}
