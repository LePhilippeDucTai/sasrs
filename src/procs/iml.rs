//! PROC IML — Interactive Matrix Language (M28a.1 + M28a.2).
//!
//! PROC IML est un **sous-langage à part entière** : il possède son propre
//! lexer/parser/évaluateur, indépendants du parser SAS principal. Ce choix est
//! imposé par des collisions de surface avec la syntaxe SAS :
//!
//! - `'` en fin d'expression = **transposée** (pas un délimiteur de chaîne) ;
//! - `*` = **produit matriciel** ; `#` = **produit de Hadamard** ;
//! - `{1 2, 3 4}` = littéral matriciel (espace = élément, `,` = nouvelle ligne) ;
//! - `QUIT;` termine le bloc (pas `RUN;`).
//!
//! Le lexer SAS (`src/lexer.rs`) reconnaît `proc iml`, capture le texte brut du
//! corps jusqu'au `quit;`, et l'émet comme `TokenKind::ImlBody(String)`. Ce
//! module re-lexe cette chaîne avec sa propre grammaire.
//!
//! **Type de valeur IML** : toujours une matrice `Vec<Vec<f64>>` (les scalaires
//! sont des matrices 1×1).
//!
//! ## Périmètre
//! - M28a.1 : littéraux matriciels, indexation 1-basée (scalaire / ligne `*` /
//!   colonne `*`), opérateurs (`'` transposée, `-` unaire, `@` Kronecker, `*`
//!   produit, `#` Hadamard, `/` division scalaire, `+`/`-`, comparaisons),
//!   fonctions `NROW`/`NCOL`/`DIM`/`T`, statement d'assignation, `PRINT`.
//! - M28a.2 : contrôle de flux (`IF/THEN/ELSE`, `DO i=a TO b [BY c]`,
//!   `DO WHILE`, `DO UNTIL`) et fonctions statistiques élémentaires (`MEAN`,
//!   `SUM`, `STD`, `MIN`, `MAX`, `ABS`, `SQRT`, `EXP`, `LOG`).
//! - Différés v1 (erreur propre) : `SHAPE`, sous-matrices `a[1:2, 1:2]`, et les
//!   statements I/O `CREATE`/`APPEND`/`CLOSE`/`READ`/`CALL` (M28a.3/.4).

use crate::error::{Result, SasError};
use crate::session::Session;
use std::collections::HashMap;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct ImlProgram {
    pub stmts: Vec<ImlStmt>,
}

#[derive(Debug, Clone)]
pub enum ImlStmt {
    Assign { var: String, expr: ImlExpr },
    Print { items: Vec<ImlPrintItem> },
    If { cond: ImlExpr, then_body: Vec<ImlStmt>, else_body: Vec<ImlStmt> },
    DoLoop { var: String, from: ImlExpr, to: ImlExpr, by: Option<ImlExpr>, body: Vec<ImlStmt> },
    DoWhile { cond: ImlExpr, body: Vec<ImlStmt> },
    DoUntil { cond: ImlExpr, body: Vec<ImlStmt> },
    /// CALL / CREATE / APPEND / CLOSE / READ : parsés mais non exécutés en v1.
    Call { func: String, args: Vec<ImlExpr> },
}

#[derive(Debug, Clone)]
pub enum ImlPrintItem {
    Var(String),
    StringLiteral(String),
}

#[derive(Debug, Clone)]
pub enum ImlExpr {
    Literal(Vec<Vec<f64>>),
    Var(String),
    BinOp { op: ImlOp, left: Box<ImlExpr>, right: Box<ImlExpr> },
    Unary { op: UnaryOp, expr: Box<ImlExpr> },
    Transpose(Box<ImlExpr>),
    Subscript { mat: Box<ImlExpr>, row: ImlIndex, col: ImlIndex },
    FnCall { name: String, args: Vec<ImlExpr> },
}

#[derive(Debug, Clone)]
pub enum ImlIndex {
    All,
    Scalar(Box<ImlExpr>),
    /// `a:b` — différé en v1 (erreur propre à l'exécution).
    Range(Box<ImlExpr>, Box<ImlExpr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImlOp {
    Add, Sub, Mul, Hadamard, Div, Kronecker,
    Eq, Ne, Lt, Le, Gt, Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
}

// ───────────────────────── Lexer IML ─────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Str(String),
    LBrace, RBrace, LBracket, RBracket, LParen, RParen,
    Comma, Semi, Star, Slash, Plus, Minus, Hash, At, Quote, Colon,
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
fn lex(src: &str) -> Result<Vec<Tok>> {
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

// ───────────────────────── Parser IML ─────────────────────────

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos]
    }
    fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].clone();
        if t != Tok::Eof {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.next();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, t: &Tok, what: &str) -> Result<()> {
        if self.eat(t) {
            Ok(())
        } else {
            Err(SasError::runtime(format!(
                "IML: expected {what}, found {:?}",
                self.peek()
            )))
        }
    }

    fn parse_program(&mut self) -> Result<ImlProgram> {
        let mut stmts = Vec::new();
        while self.peek() != &Tok::Eof {
            // Tolérer les `;` vides.
            if self.eat(&Tok::Semi) {
                continue;
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(ImlProgram { stmts })
    }

    /// Parse un statement (sans le `;` final consommé, sauf flux qui gèrent `end`).
    fn parse_stmt(&mut self) -> Result<ImlStmt> {
        let tok = self.peek().clone();
        let Tok::Ident(kw) = tok else {
            return Err(SasError::runtime(format!(
                "IML: expected a statement, found {:?}",
                self.peek()
            )));
        };
        match kw.to_ascii_lowercase().as_str() {
            "print" => {
                self.next();
                let items = self.parse_print_items()?;
                self.expect(&Tok::Semi, "';' after PRINT")?;
                Ok(ImlStmt::Print { items })
            }
            "if" => self.parse_if(),
            "do" => self.parse_do(),
            "call" => {
                self.next();
                let name = self.expect_ident("a routine name after CALL")?;
                let args = if self.eat(&Tok::LParen) {
                    let a = self.parse_arg_list()?;
                    self.expect(&Tok::RParen, "')'")?;
                    a
                } else {
                    Vec::new()
                };
                self.expect(&Tok::Semi, "';' after CALL")?;
                Ok(ImlStmt::Call { func: name, args })
            }
            // CREATE/APPEND/CLOSE/READ/QUIT/RESET/FINISH/START/etc. : on les
            // consume jusqu'au `;` et on les modélise comme Call non exécutés.
            "create" | "append" | "close" | "read" | "edit" | "use"
            | "show" | "reset" | "free" | "store" | "load" | "remove" => {
                self.next();
                // Consommer les arguments jusqu'au `;` (best-effort).
                while self.peek() != &Tok::Semi && self.peek() != &Tok::Eof {
                    self.next();
                }
                self.expect(&Tok::Semi, "';'")?;
                Ok(ImlStmt::Call { func: kw, args: Vec::new() })
            }
            _ => {
                // Assignation : ident [subscript] = expr ;
                let var = self.expect_ident("a variable name")?;
                self.expect(&Tok::Eq, "'=' in an assignment")?;
                let expr = self.parse_expr()?;
                self.expect(&Tok::Semi, "';' after an assignment")?;
                Ok(ImlStmt::Assign { var, expr })
            }
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<String> {
        match self.next() {
            Tok::Ident(s) => Ok(s),
            other => Err(SasError::runtime(format!("IML: expected {what}, found {other:?}"))),
        }
    }

    fn parse_print_items(&mut self) -> Result<Vec<ImlPrintItem>> {
        let mut items = Vec::new();
        while self.peek() != &Tok::Semi && self.peek() != &Tok::Eof {
            match self.peek().clone() {
                Tok::Str(s) => {
                    self.next();
                    items.push(ImlPrintItem::StringLiteral(s));
                }
                Tok::Ident(name) => {
                    self.next();
                    // Option [label='...'] : parser et ignorer.
                    if self.eat(&Tok::LBracket) {
                        let mut depth = 1;
                        while depth > 0 && self.peek() != &Tok::Eof {
                            match self.next() {
                                Tok::LBracket => depth += 1,
                                Tok::RBracket => depth -= 1,
                                _ => {}
                            }
                        }
                    }
                    items.push(ImlPrintItem::Var(name));
                }
                other => {
                    return Err(SasError::runtime(format!(
                        "IML: unexpected token in PRINT: {other:?}"
                    )));
                }
            }
        }
        Ok(items)
    }

    fn parse_if(&mut self) -> Result<ImlStmt> {
        self.next(); // if
        let cond = self.parse_expr()?;
        // then
        let then_kw = self.expect_ident("THEN")?;
        if !then_kw.eq_ignore_ascii_case("then") {
            return Err(SasError::runtime("IML: expected THEN after IF condition"));
        }
        let then_body = self.parse_then_or_block()?;
        let else_body = if matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("else")) {
            self.next(); // else
            self.parse_then_or_block()?
        } else {
            Vec::new()
        };
        Ok(ImlStmt::If { cond, then_body, else_body })
    }

    /// Après THEN/ELSE : soit `DO; ... END;`, soit un statement unique.
    fn parse_then_or_block(&mut self) -> Result<Vec<ImlStmt>> {
        if matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("do"))
            && self.peek_is_bare_do()
        {
            self.next(); // do
            self.expect(&Tok::Semi, "';' after DO")?;
            let body = self.parse_block_until_end()?;
            // Un `;` après END est optionnel ici (consommé par l'appelant ou non).
            self.eat(&Tok::Semi);
            Ok(body)
        } else {
            Ok(vec![self.parse_stmt()?])
        }
    }

    /// Vrai si le token courant `do` est un DO « nu » (`do;`) et non un DO
    /// itératif/while/until (`do i=...`, `do while(...)`, `do until(...)`).
    fn peek_is_bare_do(&self) -> bool {
        // toks[pos] == do ; regarder toks[pos+1].
        matches!(self.toks.get(self.pos + 1), Some(Tok::Semi))
    }

    fn parse_do(&mut self) -> Result<ImlStmt> {
        self.next(); // do
        // DO; (bloc nu) — non attendu au niveau statement, mais tolérons-le.
        if self.eat(&Tok::Semi) {
            let body = self.parse_block_until_end()?;
            self.expect(&Tok::Semi, "';' after END")?;
            // Bloc nu ≡ exécution séquentielle : on le rend comme un IF vrai.
            return Ok(ImlStmt::If {
                cond: ImlExpr::Literal(vec![vec![1.0]]),
                then_body: body,
                else_body: Vec::new(),
            });
        }
        // DO WHILE (cond) / DO UNTIL (cond)
        if let Tok::Ident(s) = self.peek().clone() {
            if s.eq_ignore_ascii_case("while") || s.eq_ignore_ascii_case("until") {
                let is_while = s.eq_ignore_ascii_case("while");
                self.next();
                self.expect(&Tok::LParen, "'(' after DO WHILE/UNTIL")?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen, "')'")?;
                self.expect(&Tok::Semi, "';' after DO WHILE/UNTIL")?;
                let body = self.parse_block_until_end()?;
                self.expect(&Tok::Semi, "';' after END")?;
                return Ok(if is_while {
                    ImlStmt::DoWhile { cond, body }
                } else {
                    ImlStmt::DoUntil { cond, body }
                });
            }
        }
        // DO i = from TO to [BY by];
        let var = self.expect_ident("a loop variable after DO")?;
        self.expect(&Tok::Eq, "'=' in a DO loop")?;
        let from = self.parse_expr()?;
        let to_kw = self.expect_ident("TO")?;
        if !to_kw.eq_ignore_ascii_case("to") {
            return Err(SasError::runtime("IML: expected TO in a DO loop"));
        }
        let to = self.parse_expr()?;
        let by = if matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("by")) {
            self.next();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(&Tok::Semi, "';' after a DO loop header")?;
        let body = self.parse_block_until_end()?;
        self.expect(&Tok::Semi, "';' after END")?;
        Ok(ImlStmt::DoLoop { var, from, to, by, body })
    }

    /// Parse des statements jusqu'à `END` (consommé), sans le `;` final.
    fn parse_block_until_end(&mut self) -> Result<Vec<ImlStmt>> {
        let mut body = Vec::new();
        loop {
            if self.eat(&Tok::Semi) {
                continue;
            }
            if matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("end")) {
                self.next(); // end
                return Ok(body);
            }
            if self.peek() == &Tok::Eof {
                return Err(SasError::runtime("IML: missing END for a DO block"));
            }
            body.push(self.parse_stmt()?);
        }
    }

    fn parse_arg_list(&mut self) -> Result<Vec<ImlExpr>> {
        let mut args = Vec::new();
        if self.peek() == &Tok::RParen {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        Ok(args)
    }

    // ── Expressions (par niveaux de précédence) ──

    fn parse_expr(&mut self) -> Result<ImlExpr> {
        self.parse_compare()
    }

    fn parse_compare(&mut self) -> Result<ImlExpr> {
        let left = self.parse_add()?;
        let op = match self.peek() {
            Tok::Eq => Some(ImlOp::Eq),
            Tok::Ne => Some(ImlOp::Ne),
            Tok::Lt => Some(ImlOp::Lt),
            Tok::Le => Some(ImlOp::Le),
            Tok::Gt => Some(ImlOp::Gt),
            Tok::Ge => Some(ImlOp::Ge),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            let right = self.parse_add()?;
            Ok(ImlExpr::BinOp { op, left: Box::new(left), right: Box::new(right) })
        } else {
            Ok(left)
        }
    }

    fn parse_add(&mut self) -> Result<ImlExpr> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => ImlOp::Add,
                Tok::Minus => ImlOp::Sub,
                _ => break,
            };
            self.next();
            let right = self.parse_mul()?;
            left = ImlExpr::BinOp { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<ImlExpr> {
        let mut left = self.parse_kron()?;
        loop {
            let op = match self.peek() {
                Tok::Star => ImlOp::Mul,
                Tok::Hash => ImlOp::Hadamard,
                Tok::Slash => ImlOp::Div,
                _ => break,
            };
            self.next();
            let right = self.parse_kron()?;
            left = ImlExpr::BinOp { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_kron(&mut self) -> Result<ImlExpr> {
        let mut left = self.parse_unary()?;
        while self.peek() == &Tok::At {
            self.next();
            let right = self.parse_unary()?;
            left = ImlExpr::BinOp { op: ImlOp::Kronecker, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<ImlExpr> {
        if self.peek() == &Tok::Minus {
            self.next();
            let e = self.parse_unary()?;
            return Ok(ImlExpr::Unary { op: UnaryOp::Neg, expr: Box::new(e) });
        }
        self.parse_postfix()
    }

    /// Postfix : transposée `'` et indexation `[...]`, en boucle.
    fn parse_postfix(&mut self) -> Result<ImlExpr> {
        let mut e = self.parse_primary()?;
        loop {
            match self.peek() {
                Tok::Quote => {
                    self.next();
                    e = ImlExpr::Transpose(Box::new(e));
                }
                Tok::LBracket => {
                    self.next();
                    let (row, col) = self.parse_subscript()?;
                    self.expect(&Tok::RBracket, "']'")?;
                    e = ImlExpr::Subscript { mat: Box::new(e), row, col };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    /// `[ row , col ]` ou `[ idx ]` (1-D, non supporté → erreur à l'exec).
    fn parse_subscript(&mut self) -> Result<(ImlIndex, ImlIndex)> {
        let row = self.parse_index()?;
        if self.eat(&Tok::Comma) {
            let col = self.parse_index()?;
            Ok((row, col))
        } else {
            // Indexation 1-D : différée v1.
            Ok((row, ImlIndex::All))
        }
    }

    fn parse_index(&mut self) -> Result<ImlIndex> {
        if self.peek() == &Tok::Star {
            self.next();
            return Ok(ImlIndex::All);
        }
        let e = self.parse_add()?;
        if self.eat(&Tok::Colon) {
            let e2 = self.parse_add()?;
            return Ok(ImlIndex::Range(Box::new(e), Box::new(e2)));
        }
        Ok(ImlIndex::Scalar(Box::new(e)))
    }

    fn parse_primary(&mut self) -> Result<ImlExpr> {
        match self.peek().clone() {
            Tok::Num(_) | Tok::Minus => {
                // Un nombre nu hors littéral n'est pas valide en IML pur, mais
                // on l'accepte comme matrice 1×1 pour les expressions de flux.
                if let Tok::Num(v) = self.peek().clone() {
                    self.next();
                    return Ok(ImlExpr::Literal(vec![vec![v]]));
                }
                unreachable!()
            }
            Tok::LBrace => self.parse_matrix_literal(),
            Tok::LParen => {
                self.next();
                let e = self.parse_expr()?;
                self.expect(&Tok::RParen, "')'")?;
                Ok(e)
            }
            Tok::Ident(name) => {
                self.next();
                if self.eat(&Tok::LParen) {
                    let args = self.parse_arg_list()?;
                    self.expect(&Tok::RParen, "')'")?;
                    Ok(ImlExpr::FnCall { name, args })
                } else {
                    Ok(ImlExpr::Var(name))
                }
            }
            other => Err(SasError::runtime(format!(
                "IML: unexpected token in an expression: {other:?}"
            ))),
        }
    }

    /// `{ a b , c d }` — espace = élément, virgule = nouvelle ligne.
    fn parse_matrix_literal(&mut self) -> Result<ImlExpr> {
        self.expect(&Tok::LBrace, "'{'")?;
        let mut rows: Vec<Vec<f64>> = Vec::new();
        let mut cur: Vec<f64> = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::RBrace => {
                    self.next();
                    rows.push(cur);
                    break;
                }
                Tok::Comma => {
                    self.next();
                    rows.push(std::mem::take(&mut cur));
                }
                Tok::Minus => {
                    self.next();
                    let v = self.expect_number()?;
                    cur.push(-v);
                }
                Tok::Num(v) => {
                    self.next();
                    cur.push(v);
                }
                other => {
                    return Err(SasError::runtime(format!(
                        "IML: matrix literals support only numeric constants, found {other:?}"
                    )));
                }
            }
        }
        // Valider la rectangularité.
        let ncol = rows.first().map(|r| r.len()).unwrap_or(0);
        if rows.iter().any(|r| r.len() != ncol) {
            return Err(SasError::runtime(
                "IML: all rows of a matrix literal must have the same number of elements",
            ));
        }
        if ncol == 0 {
            return Err(SasError::runtime("IML: empty matrix literal"));
        }
        Ok(ImlExpr::Literal(rows))
    }

    fn expect_number(&mut self) -> Result<f64> {
        match self.next() {
            Tok::Num(v) => Ok(v),
            other => Err(SasError::runtime(format!("IML: expected a number, found {other:?}"))),
        }
    }
}

/// Parse le corps brut d'un bloc IML.
pub fn parse_body(src: &str) -> Result<ImlProgram> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0 };
    p.parse_program()
}

/// Entrée appelée par `parse_proc` : `ts` est positionné sur le token
/// `ImlBody`. On le consomme et on parse son contenu.
pub fn parse(ts: &mut crate::parser::StatementStream) -> Result<ImlProgram> {
    use crate::token::TokenKind;
    // Le statement `proc iml;` se termine par `;` : le consommer. PROC IML
    // n'accepte pas d'options dans notre périmètre, donc tout token avant le
    // `;` est ignoré (best-effort).
    while !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::ImlBody(_) | TokenKind::Eof) {
        ts.next();
    }
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    let tok = ts.peek().clone();
    if let TokenKind::ImlBody(body) = tok.kind {
        ts.next();
        parse_body(&body)
    } else {
        // PROC IML sans corps capturé (pas de quit;) : corps vide.
        Err(SasError::parse(
            "PROC IML requires a QUIT; to terminate the block.",
            tok.span,
        ))
    }
}

// ───────────────────────── Évaluateur ─────────────────────────

type Matrix = Vec<Vec<f64>>;

struct Env {
    vars: HashMap<String, Matrix>,
}

fn scalar(v: f64) -> Matrix {
    vec![vec![v]]
}

fn as_scalar(m: &Matrix) -> Result<f64> {
    if m.len() == 1 && m[0].len() == 1 {
        Ok(m[0][0])
    } else {
        Err(SasError::runtime("IML: expected a scalar (1x1 matrix)"))
    }
}

fn dims(m: &Matrix) -> (usize, usize) {
    (m.len(), m.first().map(|r| r.len()).unwrap_or(0))
}

fn eval_expr(e: &ImlExpr, env: &Env) -> Result<Matrix> {
    match e {
        ImlExpr::Literal(m) => Ok(m.clone()),
        ImlExpr::Var(name) => env
            .vars
            .get(&name.to_ascii_uppercase())
            .cloned()
            .ok_or_else(|| SasError::runtime(format!("IML: matrix {} has not been set to a value.", name.to_uppercase()))),
        ImlExpr::Unary { op: UnaryOp::Neg, expr } => {
            let m = eval_expr(expr, env)?;
            Ok(m.iter().map(|r| r.iter().map(|v| -v).collect()).collect())
        }
        ImlExpr::Transpose(inner) => {
            let m = eval_expr(inner, env)?;
            Ok(transpose(&m))
        }
        ImlExpr::BinOp { op, left, right } => {
            let l = eval_expr(left, env)?;
            let r = eval_expr(right, env)?;
            eval_binop(*op, &l, &r)
        }
        ImlExpr::FnCall { name, args } => eval_fn(name, args, env),
        ImlExpr::Subscript { mat, row, col } => {
            let m = eval_expr(mat, env)?;
            eval_subscript(&m, row, col, env)
        }
    }
}

fn transpose(m: &Matrix) -> Matrix {
    let (nr, nc) = dims(m);
    let mut out = vec![vec![0.0; nr]; nc];
    for i in 0..nr {
        for j in 0..nc {
            out[j][i] = m[i][j];
        }
    }
    out
}

fn eval_binop(op: ImlOp, l: &Matrix, r: &Matrix) -> Result<Matrix> {
    let (lr, lc) = dims(l);
    let (rr, rc) = dims(r);
    match op {
        ImlOp::Add | ImlOp::Sub => {
            // Élément par élément ; scalaire diffusé.
            let f = |a: f64, b: f64| if op == ImlOp::Add { a + b } else { a - b };
            elementwise(l, r, f)
        }
        ImlOp::Hadamard => elementwise(l, r, |a, b| a * b),
        ImlOp::Div => {
            // Division par scalaire (ou élément par élément si même dim).
            if rr == 1 && rc == 1 {
                let d = r[0][0];
                Ok(l.iter().map(|row| row.iter().map(|v| v / d).collect()).collect())
            } else {
                elementwise(l, r, |a, b| a / b)
            }
        }
        ImlOp::Mul => {
            // Produit matriciel ; si l'un est scalaire, multiplication scalaire.
            if lr == 1 && lc == 1 {
                let s = l[0][0];
                return Ok(r.iter().map(|row| row.iter().map(|v| v * s).collect()).collect());
            }
            if rr == 1 && rc == 1 {
                let s = r[0][0];
                return Ok(l.iter().map(|row| row.iter().map(|v| v * s).collect()).collect());
            }
            if lc != rr {
                return Err(SasError::runtime(format!(
                    "IML: matrices do not conform for multiplication ({lr}x{lc} * {rr}x{rc})."
                )));
            }
            let mut out = vec![vec![0.0; rc]; lr];
            for i in 0..lr {
                for j in 0..rc {
                    let mut s = 0.0;
                    for k in 0..lc {
                        s += l[i][k] * r[k][j];
                    }
                    out[i][j] = s;
                }
            }
            Ok(out)
        }
        ImlOp::Kronecker => Ok(kronecker(l, r)),
        ImlOp::Eq | ImlOp::Ne | ImlOp::Lt | ImlOp::Le | ImlOp::Gt | ImlOp::Ge => {
            // Comparaisons : si les deux sont scalaires → 1×1 booléen.
            // Sinon élément par élément (diffusion scalaire).
            let cmp = |a: f64, b: f64| -> f64 {
                let t = match op {
                    ImlOp::Eq => a == b,
                    ImlOp::Ne => a != b,
                    ImlOp::Lt => a < b,
                    ImlOp::Le => a <= b,
                    ImlOp::Gt => a > b,
                    ImlOp::Ge => a >= b,
                    _ => unreachable!(),
                };
                if t { 1.0 } else { 0.0 }
            };
            elementwise(l, r, cmp)
        }
    }
}

fn elementwise(l: &Matrix, r: &Matrix, f: impl Fn(f64, f64) -> f64) -> Result<Matrix> {
    let (lr, lc) = dims(l);
    let (rr, rc) = dims(r);
    // Diffusion scalaire.
    if rr == 1 && rc == 1 {
        let s = r[0][0];
        return Ok(l.iter().map(|row| row.iter().map(|v| f(*v, s)).collect()).collect());
    }
    if lr == 1 && lc == 1 {
        let s = l[0][0];
        return Ok(r.iter().map(|row| row.iter().map(|v| f(s, *v)).collect()).collect());
    }
    if lr != rr || lc != rc {
        return Err(SasError::runtime(format!(
            "IML: matrices do not conform ({lr}x{lc} and {rr}x{rc})."
        )));
    }
    let mut out = vec![vec![0.0; lc]; lr];
    for i in 0..lr {
        for j in 0..lc {
            out[i][j] = f(l[i][j], r[i][j]);
        }
    }
    Ok(out)
}

fn kronecker(l: &Matrix, r: &Matrix) -> Matrix {
    let (lr, lc) = dims(l);
    let (rr, rc) = dims(r);
    let mut out = vec![vec![0.0; lc * rc]; lr * rr];
    for i in 0..lr {
        for j in 0..lc {
            for p in 0..rr {
                for q in 0..rc {
                    out[i * rr + p][j * rc + q] = l[i][j] * r[p][q];
                }
            }
        }
    }
    out
}

fn eval_subscript(m: &Matrix, row: &ImlIndex, col: &ImlIndex, env: &Env) -> Result<Matrix> {
    let (nr, nc) = dims(m);
    let resolve = |idx: &ImlIndex, max: usize| -> Result<IndexSel> {
        match idx {
            ImlIndex::All => Ok(IndexSel::All),
            ImlIndex::Scalar(e) => {
                let v = as_scalar(&eval_expr(e, env)?)?;
                let i = v.round() as i64;
                if i < 1 || i as usize > max {
                    return Err(SasError::runtime(format!(
                        "IML: subscript {i} is out of range 1..{max}."
                    )));
                }
                Ok(IndexSel::One((i as usize) - 1))
            }
            ImlIndex::Range(_, _) => Err(SasError::runtime(
                "IML: range subscripts (a:b) are not yet implemented.",
            )),
        }
    };
    let rsel = resolve(row, nr)?;
    let csel = resolve(col, nc)?;
    let rows: Vec<usize> = match rsel {
        IndexSel::All => (0..nr).collect(),
        IndexSel::One(i) => vec![i],
    };
    let cols: Vec<usize> = match csel {
        IndexSel::All => (0..nc).collect(),
        IndexSel::One(j) => vec![j],
    };
    let mut out = Vec::with_capacity(rows.len());
    for &i in &rows {
        let mut r = Vec::with_capacity(cols.len());
        for &j in &cols {
            r.push(m[i][j]);
        }
        out.push(r);
    }
    Ok(out)
}

enum IndexSel {
    All,
    One(usize),
}

fn eval_fn(name: &str, args: &[ImlExpr], env: &Env) -> Result<Matrix> {
    let lname = name.to_ascii_lowercase();
    let arg = |i: usize| -> Result<Matrix> {
        args.get(i)
            .ok_or_else(|| SasError::runtime(format!("IML: {} requires more arguments.", name.to_uppercase())))
            .and_then(|e| eval_expr(e, env))
    };
    match lname.as_str() {
        "nrow" => Ok(scalar(dims(&arg(0)?).0 as f64)),
        "ncol" => Ok(scalar(dims(&arg(0)?).1 as f64)),
        "dim" => {
            let (nr, nc) = dims(&arg(0)?);
            Ok(vec![vec![nr as f64, nc as f64]])
        }
        "t" => Ok(transpose(&arg(0)?)),
        "shape" => Err(SasError::runtime(
            "IML: the SHAPE function is not yet implemented.",
        )),
        "sum" => Ok(scalar(all_elems(&arg(0)?).iter().sum())),
        "mean" => {
            let v = all_elems(&arg(0)?);
            if v.is_empty() {
                return Err(SasError::runtime("IML: MEAN of an empty matrix."));
            }
            Ok(scalar(v.iter().sum::<f64>() / v.len() as f64))
        }
        "std" => {
            let v = all_elems(&arg(0)?);
            if v.len() < 2 {
                return Err(SasError::runtime("IML: STD requires at least two elements."));
            }
            let m = v.iter().sum::<f64>() / v.len() as f64;
            let ss: f64 = v.iter().map(|x| (x - m) * (x - m)).sum();
            Ok(scalar((ss / (v.len() as f64 - 1.0)).sqrt()))
        }
        "min" => {
            let v = all_elems(&arg(0)?);
            v.iter().cloned().fold(None, |acc, x| Some(acc.map_or(x, |a: f64| a.min(x))))
                .map(scalar)
                .ok_or_else(|| SasError::runtime("IML: MIN of an empty matrix."))
        }
        "max" => {
            let v = all_elems(&arg(0)?);
            v.iter().cloned().fold(None, |acc, x| Some(acc.map_or(x, |a: f64| a.max(x))))
                .map(scalar)
                .ok_or_else(|| SasError::runtime("IML: MAX of an empty matrix."))
        }
        "abs" => Ok(map_elems(&arg(0)?, f64::abs)),
        "sqrt" => Ok(map_elems(&arg(0)?, f64::sqrt)),
        "exp" => Ok(map_elems(&arg(0)?, f64::exp)),
        "log" => Ok(map_elems(&arg(0)?, f64::ln)),
        _ => Err(SasError::runtime(format!(
            "IML: the function {} is not yet implemented.",
            name.to_uppercase()
        ))),
    }
}

fn all_elems(m: &Matrix) -> Vec<f64> {
    m.iter().flat_map(|r| r.iter().cloned()).collect()
}

fn map_elems(m: &Matrix, f: impl Fn(f64) -> f64) -> Matrix {
    m.iter().map(|r| r.iter().map(|v| f(*v)).collect()).collect()
}

// ───────────────────────── Exécution + listing ─────────────────────────

fn exec_stmts(stmts: &[ImlStmt], env: &mut Env, out: &mut Vec<PrintOp>) -> Result<()> {
    for s in stmts {
        exec_stmt(s, env, out)?;
    }
    Ok(())
}

/// Opération de PRINT capturée pendant l'exécution, rendue ensuite dans le
/// listing.
enum PrintOp {
    Matrix { name: String, m: Matrix },
    Text(String),
}

fn exec_stmt(s: &ImlStmt, env: &mut Env, out: &mut Vec<PrintOp>) -> Result<()> {
    match s {
        ImlStmt::Assign { var, expr } => {
            let m = eval_expr(expr, env)?;
            env.vars.insert(var.to_ascii_uppercase(), m);
            Ok(())
        }
        ImlStmt::Print { items } => {
            for it in items {
                match it {
                    ImlPrintItem::StringLiteral(s) => out.push(PrintOp::Text(s.clone())),
                    ImlPrintItem::Var(name) => {
                        let m = env
                            .vars
                            .get(&name.to_ascii_uppercase())
                            .cloned()
                            .ok_or_else(|| SasError::runtime(format!(
                                "IML: matrix {} has not been set to a value.",
                                name.to_uppercase()
                            )))?;
                        out.push(PrintOp::Matrix { name: name.to_ascii_uppercase(), m });
                    }
                }
            }
            Ok(())
        }
        ImlStmt::If { cond, then_body, else_body } => {
            let c = eval_expr(cond, env)?;
            if matrix_truthy(&c) {
                exec_stmts(then_body, env, out)
            } else {
                exec_stmts(else_body, env, out)
            }
        }
        ImlStmt::DoLoop { var, from, to, by, body } => {
            let f = as_scalar(&eval_expr(from, env)?)?;
            let t = as_scalar(&eval_expr(to, env)?)?;
            let step = match by {
                Some(e) => as_scalar(&eval_expr(e, env)?)?,
                None => 1.0,
            };
            if step == 0.0 {
                return Err(SasError::runtime("IML: DO loop BY value cannot be zero."));
            }
            let mut i = f;
            let mut guard = 0u64;
            loop {
                if step > 0.0 && i > t + 1e-9 {
                    break;
                }
                if step < 0.0 && i < t - 1e-9 {
                    break;
                }
                env.vars.insert(var.to_ascii_uppercase(), scalar(i));
                exec_stmts(body, env, out)?;
                i += step;
                guard += 1;
                if guard > 10_000_000 {
                    return Err(SasError::runtime("IML: DO loop exceeded the iteration guard."));
                }
            }
            Ok(())
        }
        ImlStmt::DoWhile { cond, body } => {
            let mut guard = 0u64;
            while matrix_truthy(&eval_expr(cond, env)?) {
                exec_stmts(body, env, out)?;
                guard += 1;
                if guard > 10_000_000 {
                    return Err(SasError::runtime("IML: DO WHILE loop exceeded the iteration guard."));
                }
            }
            Ok(())
        }
        ImlStmt::DoUntil { cond, body } => {
            let mut guard = 0u64;
            loop {
                exec_stmts(body, env, out)?;
                if matrix_truthy(&eval_expr(cond, env)?) {
                    break;
                }
                guard += 1;
                if guard > 10_000_000 {
                    return Err(SasError::runtime("IML: DO UNTIL loop exceeded the iteration guard."));
                }
            }
            Ok(())
        }
        ImlStmt::Call { func, .. } => Err(SasError::runtime(format!(
            "IML: the {} statement is not yet implemented.",
            func.to_uppercase()
        ))),
    }
}

/// Une matrice est « vraie » si elle est 1×1 et non nulle (sémantique SAS IML
/// des conditions IF/WHILE : la condition doit être un scalaire).
fn matrix_truthy(m: &Matrix) -> bool {
    if m.len() == 1 && m[0].len() == 1 {
        m[0][0] != 0.0 && !m[0][0].is_nan()
    } else {
        // Toute la matrice doit être non nulle (sémantique IML : ALL).
        !m.is_empty() && m.iter().all(|r| r.iter().all(|v| *v != 0.0))
    }
}

/// Formate une valeur numérique pour le listing IML (logique BEST. : entiers
/// sans décimale, flottants tronqués à 4 décimales, trailing zeros enlevés).
fn fmt_val(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.4}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn render_matrix(name: &str, m: &Matrix, session: &mut Session) {
    let (nr, nc) = dims(m);
    session.listing.write_line(name);
    session.listing.blank();

    // Cellules formatées.
    let cells: Vec<Vec<String>> = m.iter().map(|r| r.iter().map(|v| fmt_val(*v)).collect()).collect();

    let show_col_hdr = nc >= 2;
    let show_row_hdr = nr >= 2;

    // Largeur de chaque colonne : max(4, valeurs formatées, en-tête COLk).
    let mut widths = vec![4usize; nc];
    for (j, w) in widths.iter_mut().enumerate() {
        if show_col_hdr {
            *w = (*w).max(format!("COL{}", j + 1).len());
        }
        for row in &cells {
            *w = (*w).max(row[j].len());
        }
    }

    // Largeur de l'étiquette de ligne.
    let row_label_w = if show_row_hdr {
        (0..nr).map(|i| format!("ROW{}", i + 1).len()).max().unwrap_or(0)
    } else {
        0
    };

    let gap = 2usize;

    // En-tête de colonnes.
    if show_col_hdr {
        let mut line = String::new();
        if show_row_hdr {
            line.push_str(&" ".repeat(row_label_w));
        }
        for (j, w) in widths.iter().enumerate() {
            line.push_str(&" ".repeat(gap));
            let hdr = format!("COL{}", j + 1);
            line.push_str(&format!("{hdr:>w$}", w = *w));
        }
        session.listing.write_line(&line);
        session.listing.blank();
    }

    // Lignes.
    for (i, row) in cells.iter().enumerate() {
        let mut line = String::new();
        if show_row_hdr {
            let lbl = format!("ROW{}", i + 1);
            line.push_str(&format!("{lbl:<w$}", w = row_label_w));
        }
        for (j, w) in widths.iter().enumerate() {
            line.push_str(&" ".repeat(gap));
            line.push_str(&format!("{:>w$}", row[j], w = *w));
        }
        session.listing.write_line(&line);
    }
    session.listing.blank();
}

/// Exécute un programme IML.
pub fn execute(prog: &ImlProgram, session: &mut Session) -> Result<()> {
    let mut env = Env { vars: HashMap::new() };
    let mut ops: Vec<PrintOp> = Vec::new();
    exec_stmts(&prog.stmts, &mut env, &mut ops)?;

    session.listing.page_header();
    let ls = session.listing.ls();
    let pad = ls.saturating_sub("The IML Procedure".len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), "The IML Procedure"));
    session.listing.blank();

    for op in &ops {
        match op {
            PrintOp::Text(t) => {
                session.listing.write_line(t);
                session.listing.blank();
            }
            PrintOp::Matrix { name, m } => render_matrix(name, m, session),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_one(src: &str) -> Matrix {
        // Enveloppe : assigne à `r` et renvoie sa valeur.
        let prog = parse_body(&format!("r = {src};")).unwrap();
        let mut env = Env { vars: HashMap::new() };
        let mut ops = Vec::new();
        exec_stmts(&prog.stmts, &mut env, &mut ops).unwrap();
        env.vars.get("R").unwrap().clone()
    }

    fn run_get(src: &str, var: &str) -> Matrix {
        let prog = parse_body(src).unwrap();
        let mut env = Env { vars: HashMap::new() };
        let mut ops = Vec::new();
        exec_stmts(&prog.stmts, &mut env, &mut ops).unwrap();
        env.vars.get(&var.to_ascii_uppercase()).unwrap().clone()
    }

    #[test]
    fn lit_row_vector() {
        assert_eq!(eval_one("{1 2 3}"), vec![vec![1.0, 2.0, 3.0]]);
    }

    #[test]
    fn lit_col_vector() {
        assert_eq!(eval_one("{1, 2, 3}"), vec![vec![1.0], vec![2.0], vec![3.0]]);
    }

    #[test]
    fn lit_2x2() {
        assert_eq!(eval_one("{1 2, 3 4}"), vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
    }

    #[test]
    fn matrix_product() {
        assert_eq!(
            eval_one("{1 2,3 4}*{5 6,7 8}"),
            vec![vec![19.0, 22.0], vec![43.0, 50.0]]
        );
    }

    #[test]
    fn transpose_op() {
        assert_eq!(eval_one("{1 2,3 4}'"), vec![vec![1.0, 3.0], vec![2.0, 4.0]]);
    }

    #[test]
    fn hadamard() {
        assert_eq!(
            eval_one("{1 2,3 4}#{5 6,7 8}"),
            vec![vec![5.0, 12.0], vec![21.0, 32.0]]
        );
    }

    #[test]
    fn nrow_ncol() {
        assert_eq!(eval_one("nrow({1 2 3, 4 5 6})"), scalar(2.0));
        assert_eq!(eval_one("ncol({1 2 3, 4 5 6})"), scalar(3.0));
    }

    #[test]
    fn sum_fn() {
        assert_eq!(eval_one("sum({2 4 6 8 10})"), scalar(30.0));
    }

    #[test]
    fn std_fn() {
        let s = as_scalar(&eval_one("std({2 4 6 8 10})")).unwrap();
        assert!((s - 3.1622776601).abs() < 0.001, "std = {s}");
    }

    #[test]
    fn do_loop_accumulates() {
        let m = run_get("total = {0}; do i = 1 to 5; total = total + i; end;", "total");
        assert_eq!(m, scalar(15.0));
    }

    #[test]
    fn if_then_else() {
        let m = run_get("if 15 > {10} then big = {1}; else big = {0};", "big");
        assert_eq!(m, scalar(1.0));
    }

    #[test]
    fn subscript_scalar() {
        assert_eq!(eval_one("{1 2,3 4}[2,1]"), scalar(3.0));
    }

    #[test]
    fn subscript_row() {
        assert_eq!(eval_one("{1 2,3 4}[1,*]"), vec![vec![1.0, 2.0]]);
    }

    #[test]
    fn subscript_col() {
        assert_eq!(eval_one("{1 2,3 4}[*,2]"), vec![vec![2.0], vec![4.0]]);
    }

    #[test]
    fn quit_parsed_whole_block() {
        // parse_body reçoit le corps SANS le quit; (retiré par le lexer SAS).
        let prog = parse_body("a = {1};").unwrap();
        assert_eq!(prog.stmts.len(), 1);
    }

    #[test]
    fn print_generates_listing_section() {
        use crate::session::Session;
        use std::path::PathBuf;
        let mut session = Session::new(None, PathBuf::from("."), true).unwrap();
        let prog = parse_body("a = {1 2, 3 4}; print a;").unwrap();
        execute(&prog, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("The IML Procedure"), "listing: {listing}");
        assert!(listing.contains("COL1"), "listing: {listing}");
        assert!(listing.contains("ROW1"), "listing: {listing}");
    }

    #[test]
    fn kronecker_2x2() {
        // {1 0,0 1} @ {1 2,3 4} = block diag.
        let m = eval_one("{1 0,0 1}@{1 2,3 4}");
        assert_eq!(
            m,
            vec![
                vec![1.0, 2.0, 0.0, 0.0],
                vec![3.0, 4.0, 0.0, 0.0],
                vec![0.0, 0.0, 1.0, 2.0],
                vec![0.0, 0.0, 3.0, 4.0],
            ]
        );
    }

    #[test]
    fn negative_and_decimal_literals() {
        assert_eq!(eval_one("{1.5 -2, 0 3.7}"), vec![vec![1.5, -2.0], vec![0.0, 3.7]]);
    }
}
