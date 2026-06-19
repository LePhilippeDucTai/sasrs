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
//! - M28a.3 : algèbre linéaire — `INV`, `SOLVE`, `EIGVAL` (symétrique),
//!   `CHOL` (upper, convention SAS), `CALL QR(Q, R, A)`,
//!   `CALL SVDCD(U, D, V, A)` (méthode ATA-Jacobi). Différés : `EIGVEC`,
//!   `DET`, `CALL EIGEN`.
//! - M28a.4 : I/O datasets — `CREATE ds FROM mat[COLNAME=cn]`, `APPEND FROM`,
//!   `CLOSE`, `USE`, `READ ALL VAR {..} INTO mat`. Différés : `READ NEXT`,
//!   `WHERE`, `LOAD`/`STORE`/`SHOW`.
//! - Différés v1 (erreur propre) : `SHAPE`, sous-matrices `a[1:2, 1:2]`.

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
    /// CALL routine(args). Les arguments de sortie (CALL QR/SVDCD/EIGEN) sont
    /// des lvalues (noms de matrices à affecter) — résolus à l'exécution.
    Call { func: String, args: Vec<ImlExpr> },
    /// `CREATE ds FROM mat [COLNAME=cn];`
    Create { ds: String, from: String, colname: Option<ImlExpr> },
    /// `APPEND FROM mat;`
    Append { from: String },
    /// `CLOSE ds;`
    Close { ds: String },
    /// `USE ds;`
    Use { ds: String },
    /// `READ ALL VAR {vars} INTO mat;`
    ReadAll { vars: Vec<String>, into: String },
    /// Statements I/O non encore implémentés (erreur propre à l'exécution).
    UnsupportedIo { msg: String },
}

#[derive(Debug, Clone)]
pub enum ImlPrintItem {
    Var(String),
    StringLiteral(String),
}

#[derive(Debug, Clone)]
pub enum ImlExpr {
    Literal(Vec<Vec<f64>>),
    /// Littéral de liste de chaînes : `{"x" "y"}`. Utilisé pour COLNAME=.
    StrList(Vec<String>),
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
            "create" => self.parse_create(),
            "append" => self.parse_append(),
            "close" => self.parse_close(),
            "use" => self.parse_use(),
            "read" => self.parse_read(),
            // Statements I/O différés : erreur propre à l'exécution.
            "store" | "load" | "show" => {
                self.next();
                while self.peek() != &Tok::Semi && self.peek() != &Tok::Eof {
                    self.next();
                }
                self.expect(&Tok::Semi, "';'")?;
                Ok(ImlStmt::UnsupportedIo {
                    msg: format!(
                        "{} is not yet implemented in PROC IML",
                        kw.to_uppercase()
                    ),
                })
            }
            // Autres statements de gestion : consommés sans effet (best-effort).
            "edit" | "reset" | "free" | "remove" => {
                self.next();
                while self.peek() != &Tok::Semi && self.peek() != &Tok::Eof {
                    self.next();
                }
                self.expect(&Tok::Semi, "';'")?;
                Ok(ImlStmt::UnsupportedIo {
                    msg: format!(
                        "the {} statement is not yet implemented in PROC IML",
                        kw.to_uppercase()
                    ),
                })
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

    /// Parse un nom de dataset possiblement qualifié : `name` ou `lib.name`.
    /// Retourne la forme canonique en MAJUSCULES (`LIB.NAME` ou `NAME`).
    fn parse_dataset_name(&mut self, what: &str) -> Result<String> {
        let first = self.expect_ident(what)?;
        if self.eat(&Tok::Dot) {
            let second = self.expect_ident("a dataset name after '.'")?;
            Ok(format!("{}.{}", first.to_uppercase(), second.to_uppercase()))
        } else {
            Ok(first.to_uppercase())
        }
    }

    /// `CREATE ds FROM mat [COLNAME=cn];`
    fn parse_create(&mut self) -> Result<ImlStmt> {
        self.next(); // create
        let ds = self.parse_dataset_name("a dataset name after CREATE")?;
        let from_kw = self.expect_ident("FROM")?;
        if !from_kw.eq_ignore_ascii_case("from") {
            return Err(SasError::runtime("IML: expected FROM in a CREATE statement"));
        }
        let from = self.expect_ident("a matrix name after FROM")?.to_uppercase();
        // Option [COLNAME=cn] ou [colname=cn].
        let mut colname = None;
        if self.eat(&Tok::LBracket) {
            let opt = self.expect_ident("an option name (COLNAME)")?;
            if !opt.eq_ignore_ascii_case("colname") {
                return Err(SasError::runtime(format!(
                    "IML: unsupported CREATE option '{opt}' (only COLNAME= is supported)"
                )));
            }
            self.expect(&Tok::Eq, "'=' after COLNAME")?;
            colname = Some(self.parse_primary()?);
            self.expect(&Tok::RBracket, "']'")?;
        }
        self.expect(&Tok::Semi, "';' after CREATE")?;
        Ok(ImlStmt::Create { ds, from, colname })
    }

    /// `APPEND FROM mat;`
    fn parse_append(&mut self) -> Result<ImlStmt> {
        self.next(); // append
        let from_kw = self.expect_ident("FROM")?;
        if !from_kw.eq_ignore_ascii_case("from") {
            return Err(SasError::runtime("IML: expected FROM in an APPEND statement"));
        }
        let from = self.expect_ident("a matrix name after FROM")?.to_uppercase();
        self.expect(&Tok::Semi, "';' after APPEND")?;
        Ok(ImlStmt::Append { from })
    }

    /// `CLOSE ds;`
    fn parse_close(&mut self) -> Result<ImlStmt> {
        self.next(); // close
        let ds = self.parse_dataset_name("a dataset name after CLOSE")?;
        self.expect(&Tok::Semi, "';' after CLOSE")?;
        Ok(ImlStmt::Close { ds })
    }

    /// `USE ds;`
    fn parse_use(&mut self) -> Result<ImlStmt> {
        self.next(); // use
        let ds = self.parse_dataset_name("a dataset name after USE")?;
        self.expect(&Tok::Semi, "';' after USE")?;
        Ok(ImlStmt::Use { ds })
    }

    /// `READ ALL VAR {vars} INTO mat;` (autres formes → erreur propre).
    fn parse_read(&mut self) -> Result<ImlStmt> {
        self.next(); // read
        let mode = self.expect_ident("ALL or NEXT after READ")?;
        if mode.eq_ignore_ascii_case("next") {
            while self.peek() != &Tok::Semi && self.peek() != &Tok::Eof {
                self.next();
            }
            self.expect(&Tok::Semi, "';'")?;
            return Ok(ImlStmt::UnsupportedIo {
                msg: "READ NEXT not yet implemented; use READ ALL instead".to_string(),
            });
        }
        if !mode.eq_ignore_ascii_case("all") {
            return Err(SasError::runtime("IML: expected ALL or NEXT after READ"));
        }
        let var_kw = self.expect_ident("VAR after READ ALL")?;
        if !var_kw.eq_ignore_ascii_case("var") {
            return Err(SasError::runtime("IML: expected VAR after READ ALL"));
        }
        // Liste de variables : `{ "x" "y" }` ou `{ x y }`.
        let vars = self.parse_var_list()?;
        // INTO mat ou WHERE ... .
        let kw = self.expect_ident("INTO or WHERE after the variable list")?;
        if kw.eq_ignore_ascii_case("where") {
            while self.peek() != &Tok::Semi && self.peek() != &Tok::Eof {
                self.next();
            }
            self.expect(&Tok::Semi, "';'")?;
            return Ok(ImlStmt::UnsupportedIo {
                msg: "WHERE clause in READ not yet implemented".to_string(),
            });
        }
        if !kw.eq_ignore_ascii_case("into") {
            return Err(SasError::runtime("IML: expected INTO after the variable list"));
        }
        let into = self.expect_ident("a matrix name after INTO")?.to_uppercase();
        self.expect(&Tok::Semi, "';' after READ")?;
        Ok(ImlStmt::ReadAll { vars, into })
    }

    /// Liste de variables `{ "x" "y" }` ou `{ x y }` (noms en MAJUSCULES).
    fn parse_var_list(&mut self) -> Result<Vec<String>> {
        self.expect(&Tok::LBrace, "'{' to begin a variable list")?;
        let mut out = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::Str(s) => {
                    self.next();
                    out.push(s.to_uppercase());
                }
                Tok::Ident(s) => {
                    self.next();
                    out.push(s.to_uppercase());
                }
                Tok::RBrace => {
                    self.next();
                    break;
                }
                other => {
                    return Err(SasError::runtime(format!(
                        "IML: unexpected token in a variable list: {other:?}"
                    )));
                }
            }
        }
        if out.is_empty() {
            return Err(SasError::runtime("IML: empty variable list in READ"));
        }
        Ok(out)
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
    /// `{ "x" "y" }` — liste de chaînes (pour COLNAME=, READ VAR, etc.).
    fn parse_matrix_literal(&mut self) -> Result<ImlExpr> {
        self.expect(&Tok::LBrace, "'{'")?;
        // Liste de chaînes : `{ "x" "y" ... }`.
        if matches!(self.peek(), Tok::Str(_)) {
            let mut strs = Vec::new();
            loop {
                match self.peek().clone() {
                    Tok::Str(s) => {
                        self.next();
                        strs.push(s);
                    }
                    Tok::RBrace => {
                        self.next();
                        break;
                    }
                    other => {
                        return Err(SasError::runtime(format!(
                            "IML: a string literal list may only contain strings, found {other:?}"
                        )));
                    }
                }
            }
            return Ok(ImlExpr::StrList(strs));
        }
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

/// Tampon d'un dataset ouvert en écriture par CREATE/APPEND/CLOSE.
struct OpenWrite {
    colnames: Vec<String>,
    rows: Vec<Vec<f64>>,
}

struct Env {
    vars: HashMap<String, Matrix>,
    /// Matrices de chaînes (listes de noms), p.ex. `cn = {"x" "y"}`. Stockées à
    /// part car la valeur IML numérique est `Vec<Vec<f64>>`.
    str_vars: HashMap<String, Vec<String>>,
    /// Datasets ouverts en écriture (CREATE … APPEND … CLOSE), clé = nom canonique.
    open_writes: HashMap<String, OpenWrite>,
    /// Datasets ouverts en lecture (USE … READ … CLOSE), clé = nom canonique.
    open_reads: std::collections::HashSet<String>,
}

impl Env {
    fn new() -> Self {
        Env {
            vars: HashMap::new(),
            str_vars: HashMap::new(),
            open_writes: HashMap::new(),
            open_reads: std::collections::HashSet::new(),
        }
    }
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
        ImlExpr::StrList(_) => Err(SasError::runtime(
            "IML: a character matrix cannot be used in a numeric expression.",
        )),
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
        // ── M28a.3 : algèbre linéaire ──
        "inv" => iml_inv(&arg(0)?),
        "solve" => iml_solve(&arg(0)?, &arg(1)?),
        "eigval" => iml_eigval(&arg(0)?),
        "chol" => iml_chol(&arg(0)?),
        "eigvec" => Err(SasError::runtime(
            "EIGVEC: use `CALL EIGEN` for eigenvectors (not yet implemented)",
        )),
        "det" => Err(SasError::runtime("DET not yet implemented in PROC IML")),
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

// ───────────────────────── M28a.3 : algèbre linéaire ─────────────────────────

/// Vérifie qu'une matrice est carrée ; renvoie sa dimension.
fn require_square(m: &Matrix, fname: &str) -> Result<usize> {
    let (nr, nc) = dims(m);
    if nr == 0 || nr != nc {
        return Err(SasError::runtime(format!(
            "IML: {fname} requires a square matrix (got {nr}x{nc})."
        )));
    }
    Ok(nr)
}

/// `INV(A)` → A⁻¹ via `invert_matrix`.
fn iml_inv(a: &Matrix) -> Result<Matrix> {
    require_square(a, "INV")?;
    crate::stat::linalg::invert_matrix(a)
}

/// `SOLVE(A, b)` → x tel que A*x = b, colonne par colonne via `least_squares`.
fn iml_solve(a: &Matrix, b: &Matrix) -> Result<Matrix> {
    let (an, _) = dims(a);
    let (bn, bc) = dims(b);
    if an != bn {
        return Err(SasError::runtime(format!(
            "IML: SOLVE dimensions do not conform (A has {an} rows, b has {bn} rows)."
        )));
    }
    let ncols_x = a[0].len();
    // Résoudre chaque colonne de b ; assembler la solution (n×bc).
    let mut sols: Vec<Vec<f64>> = Vec::with_capacity(bc);
    for j in 0..bc {
        let col: Vec<f64> = (0..bn).map(|i| b[i][j]).collect();
        sols.push(crate::stat::linalg::least_squares(a, &col)?);
    }
    let mut out = vec![vec![0.0; bc]; ncols_x];
    for (j, sol) in sols.iter().enumerate() {
        for (i, &v) in sol.iter().enumerate() {
            out[i][j] = v;
        }
    }
    Ok(out)
}

/// `EIGVAL(A)` → vecteur colonne des valeurs propres (ordre décroissant).
/// SAS n'accepte que les matrices symétriques.
fn iml_eigval(a: &Matrix) -> Result<Matrix> {
    let n = require_square(a, "EIGVAL")?;
    for i in 0..n {
        for j in (i + 1)..n {
            if (a[i][j] - a[j][i]).abs() > 1e-10 {
                return Err(SasError::runtime(
                    "ERROR: The argument to the EIGVAL function must be a symmetric matrix.",
                ));
            }
        }
    }
    let vals = crate::stat::linalg::eigenvalues_jacobi(a)?;
    Ok(vals.into_iter().map(|v| vec![v]).collect())
}

/// `CHOL(A)` → U upper triangular telle que U'*U = A (SAS convention).
/// `cholesky` renvoie L (lower) avec L*L'=A ; on transpose.
fn iml_chol(a: &Matrix) -> Result<Matrix> {
    require_square(a, "CHOL")?;
    let l = crate::stat::linalg::cholesky(a)?;
    Ok(transpose(&l))
}

/// `CALL SVDCD(U, D, V, A)` : A = U*diag(D)*V', via la méthode ATA-Jacobi.
/// Renvoie (U, D, V) où D est un vecteur colonne des valeurs singulières.
fn iml_svdcd(a: &Matrix) -> Result<(Matrix, Matrix, Matrix)> {
    let (m, n) = dims(a);
    if m == 0 || n == 0 {
        return Err(SasError::runtime("IML: SVDCD requires a non-empty matrix."));
    }
    if m < n {
        return Err(SasError::runtime(
            "IML: SVDCD currently requires rows >= columns (m >= n).",
        ));
    }
    // S = A'*A (n×n, symétrique).
    let at = transpose(a);
    let s = crate::stat::linalg::matrix_mult(&at, a);
    // Eigendécomposition : S = V diag(λ) V', λ décroissants.
    let (v, lambda) = crate::stat::linalg::eigenvectors_jacobi(&s)?;
    // σᵢ = sqrt(max(λᵢ, 0)).
    let sigma: Vec<f64> = lambda.iter().map(|&l| l.max(0.0).sqrt()).collect();
    // U[:,i] = A * V[:,i] / σᵢ.
    let mut u = vec![vec![0.0; n]; m];
    for i in 0..n {
        if sigma[i] > 1e-12 {
            // colonne i de V.
            let vi: Vec<f64> = (0..n).map(|r| v[r][i]).collect();
            let avi = crate::stat::linalg::matrix_vec_mult(a, &vi);
            for r in 0..m {
                u[r][i] = avi[r] / sigma[i];
            }
        } else {
            return Err(SasError::runtime(
                "IML: SVDCD: rank-deficient matrix; orthonormal completion not yet implemented.",
            ));
        }
    }
    let d: Matrix = sigma.into_iter().map(|s| vec![s]).collect();
    Ok((u, d, v))
}

// ───────────────────────── Exécution + listing ─────────────────────────

fn exec_stmts(
    stmts: &[ImlStmt],
    env: &mut Env,
    out: &mut Vec<PrintOp>,
    session: &mut Session,
) -> Result<()> {
    for s in stmts {
        exec_stmt(s, env, out, session)?;
    }
    Ok(())
}

/// Opération de PRINT capturée pendant l'exécution, rendue ensuite dans le
/// listing.
enum PrintOp {
    Matrix { name: String, m: Matrix },
    Text(String),
}

fn exec_stmt(
    s: &ImlStmt,
    env: &mut Env,
    out: &mut Vec<PrintOp>,
    session: &mut Session,
) -> Result<()> {
    match s {
        ImlStmt::Assign { var, expr } => {
            // Une liste de chaînes est stockée dans str_vars, pas dans vars.
            if let ImlExpr::StrList(strs) = expr {
                env.str_vars
                    .insert(var.to_ascii_uppercase(), strs.clone());
                env.vars.remove(&var.to_ascii_uppercase());
                return Ok(());
            }
            let m = eval_expr(expr, env)?;
            env.str_vars.remove(&var.to_ascii_uppercase());
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
                exec_stmts(then_body, env, out, session)
            } else {
                exec_stmts(else_body, env, out, session)
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
                exec_stmts(body, env, out, session)?;
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
                exec_stmts(body, env, out, session)?;
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
                exec_stmts(body, env, out, session)?;
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
        ImlStmt::Call { func, args } => exec_call(func, args, env),
        ImlStmt::Create { ds, from, colname } => exec_create(ds, from, colname.as_ref(), env),
        ImlStmt::Append { from } => exec_append(from, env),
        ImlStmt::Close { ds } => exec_close(ds, env, session),
        ImlStmt::Use { ds } => exec_use(ds, env, session),
        ImlStmt::ReadAll { vars, into } => exec_read_all(vars, into, env, session),
        ImlStmt::UnsupportedIo { msg } => Err(SasError::runtime(msg.clone())),
    }
}

/// Exécute un `CALL routine(...)`. Les routines `QR`/`SVDCD` ont des arguments
/// de sortie (lvalues) suivis d'arguments d'entrée.
fn exec_call(func: &str, args: &[ImlExpr], env: &mut Env) -> Result<()> {
    let lname = func.to_ascii_lowercase();
    // Extrait le nom (lvalue) d'un argument de sortie.
    let out_name = |e: &ImlExpr| -> Result<String> {
        match e {
            ImlExpr::Var(n) => Ok(n.to_ascii_uppercase()),
            _ => Err(SasError::runtime(format!(
                "IML: CALL {} output arguments must be variable names.",
                func.to_uppercase()
            ))),
        }
    };
    match lname.as_str() {
        "qr" => {
            if args.len() != 3 {
                return Err(SasError::runtime(
                    "IML: CALL QR requires 3 arguments: CALL QR(Q, R, A).",
                ));
            }
            let q_name = out_name(&args[0])?;
            let r_name = out_name(&args[1])?;
            let a = eval_expr(&args[2], env)?;
            let (q, r) = crate::stat::linalg::qr_decomposition(&a)?;
            env.vars.insert(q_name, q);
            env.vars.insert(r_name, r);
            Ok(())
        }
        "svdcd" => {
            if args.len() != 4 {
                return Err(SasError::runtime(
                    "IML: CALL SVDCD requires 4 arguments: CALL SVDCD(U, D, V, A).",
                ));
            }
            let u_name = out_name(&args[0])?;
            let d_name = out_name(&args[1])?;
            let v_name = out_name(&args[2])?;
            let a = eval_expr(&args[3], env)?;
            let (u, d, v) = iml_svdcd(&a)?;
            env.vars.insert(u_name, u);
            env.vars.insert(d_name, d);
            env.vars.insert(v_name, v);
            Ok(())
        }
        "eigen" => Err(SasError::runtime(
            "CALL EIGEN not yet implemented in PROC IML",
        )),
        other => Err(SasError::runtime(format!(
            "IML: the {} subroutine is not yet implemented.",
            other.to_uppercase()
        ))),
    }
}

/// Sépare un nom canonique `LIB.NAME` (ou `NAME`) en (libref, table) MAJUSCULES.
/// Défaut WORK si non qualifié.
fn split_ds_name(name: &str) -> (String, String) {
    match name.split_once('.') {
        Some((lib, tbl)) => (lib.to_uppercase(), tbl.to_uppercase()),
        None => ("WORK".to_string(), name.to_uppercase()),
    }
}

/// `CREATE ds FROM mat [COLNAME=cn];` — prépare le tampon (colonnes seulement).
fn exec_create(
    ds: &str,
    from: &str,
    colname: Option<&ImlExpr>,
    env: &mut Env,
) -> Result<()> {
    let mat = env
        .vars
        .get(&from.to_ascii_uppercase())
        .cloned()
        .ok_or_else(|| SasError::runtime(format!(
            "IML: matrix {} has not been set to a value.",
            from.to_uppercase()
        )))?;
    let ncol = dims(&mat).1;
    let colnames: Vec<String> = match colname {
        Some(ImlExpr::StrList(s)) => s.iter().map(|x| x.to_string()).collect(),
        Some(ImlExpr::Var(v)) => env
            .str_vars
            .get(&v.to_ascii_uppercase())
            .cloned()
            .ok_or_else(|| SasError::runtime(format!(
                "IML: COLNAME= must reference a string list; '{}' is not a character matrix.",
                v.to_uppercase()
            )))?,
        Some(_) => {
            return Err(SasError::runtime(
                "IML: COLNAME= must be a string literal list, e.g. {\"x\" \"y\"}.",
            ));
        }
        None => (1..=ncol).map(|j| format!("COL{j}")).collect(),
    };
    if colnames.len() != ncol {
        return Err(SasError::runtime(format!(
            "IML: COLNAME= has {} names but the matrix has {} columns.",
            colnames.len(),
            ncol
        )));
    }
    env.open_writes.insert(
        ds.to_uppercase(),
        OpenWrite { colnames, rows: Vec::new() },
    );
    Ok(())
}

/// `APPEND FROM mat;` — ajoute les lignes de `mat` au (seul) dataset ouvert.
fn exec_append(from: &str, env: &mut Env) -> Result<()> {
    let mat = env
        .vars
        .get(&from.to_ascii_uppercase())
        .cloned()
        .ok_or_else(|| SasError::runtime(format!(
            "IML: matrix {} has not been set to a value.",
            from.to_uppercase()
        )))?;
    // SAS APPEND s'applique au dataset courant en écriture. Ici on exige qu'il
    // y en ait exactement un d'ouvert.
    if env.open_writes.len() != 1 {
        return Err(SasError::runtime(
            "IML: APPEND requires exactly one open output data set (use CREATE first).",
        ));
    }
    let key = env.open_writes.keys().next().cloned().unwrap();
    let buf = env.open_writes.get_mut(&key).unwrap();
    let ncol = buf.colnames.len();
    for row in &mat {
        if row.len() != ncol {
            return Err(SasError::runtime(format!(
                "IML: APPEND row has {} columns but the data set expects {}.",
                row.len(),
                ncol
            )));
        }
        buf.rows.push(row.clone());
    }
    Ok(())
}

/// `CLOSE ds;` — écrit le dataset accumulé dans la bibliothèque cible.
fn exec_close(ds: &str, env: &mut Env, session: &mut Session) -> Result<()> {
    let key = ds.to_uppercase();
    if let Some(buf) = env.open_writes.remove(&key) {
        use crate::dataset::{SasDataset, VarMeta};
        use crate::value::VarType;
        use polars::prelude::*;
        let (libref, table) = split_ds_name(&key);
        let ncol = buf.colnames.len();
        let nrow = buf.rows.len();
        // Construire une colonne f64 par variable.
        let mut columns: Vec<Column> = Vec::with_capacity(ncol);
        let mut vars: Vec<VarMeta> = Vec::with_capacity(ncol);
        for j in 0..ncol {
            let col: Vec<f64> = (0..nrow).map(|i| buf.rows[i][j]).collect();
            columns.push(Series::new(buf.colnames[j].as_str().into(), col).into());
            vars.push(VarMeta {
                name: buf.colnames[j].clone(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            });
        }
        let df = DataFrame::new(columns)?;
        let out_ds = SasDataset { df, vars };
        let display = format!("{libref}.{table}");
        session.libs.get(&libref)?.write(&table, &out_ds)?;
        session.last_dataset = Some(display.clone());
        session.log.note(&format!(
            "The data set {display} has {nrow} observations and {ncol} variables."
        ));
        return Ok(());
    }
    // Fermeture d'un dataset ouvert en lecture : best-effort.
    env.open_reads.remove(&key);
    Ok(())
}

/// `USE ds;` — ouvre un dataset en lecture (marque l'ouverture).
fn exec_use(ds: &str, env: &mut Env, session: &mut Session) -> Result<()> {
    let key = ds.to_uppercase();
    let (libref, table) = split_ds_name(&key);
    let provider = session.libs.get(&libref)?;
    if !provider.exists(&table) {
        return Err(SasError::runtime(format!(
            "IML: data set {libref}.{table} does not exist."
        )));
    }
    env.open_reads.insert(key);
    Ok(())
}

/// `READ ALL VAR {vars} INTO mat;` — lit les colonnes demandées dans une matrice.
fn exec_read_all(
    vars: &[String],
    into: &str,
    env: &mut Env,
    session: &mut Session,
) -> Result<()> {
    use crate::procs::common::decode_column;
    use crate::value::Value;
    // Choisir le dataset ouvert en lecture (exactement un attendu).
    if env.open_reads.len() != 1 {
        return Err(SasError::runtime(
            "IML: READ requires exactly one open input data set (use USE first).",
        ));
    }
    let key = env.open_reads.iter().next().cloned().unwrap();
    let (libref, table) = split_ds_name(&key);
    let (ds, notes) = session.libs.get(&libref)?.read(&table)?;
    for note in notes {
        session.log.forward(&note);
    }
    // Indices des colonnes demandées.
    let mut col_idx = Vec::with_capacity(vars.len());
    for vname in vars {
        let idx = ds
            .vars
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case(vname))
            .ok_or_else(|| SasError::runtime(format!(
                "IML: variable {} not found in data set {libref}.{table}.",
                vname
            )))?;
        col_idx.push(idx);
    }
    let nrow = ds.n_obs();
    // Décoder chaque colonne demandée en f64.
    let mut cols: Vec<Vec<f64>> = Vec::with_capacity(col_idx.len());
    for &ci in &col_idx {
        let decoded = decode_column(&ds, ci)?;
        let mut c = Vec::with_capacity(nrow);
        for v in decoded {
            let x = match v {
                Value::Num(x) => x,
                Value::Missing(_) => f64::NAN,
                Value::Char(_) => {
                    return Err(SasError::runtime(format!(
                        "IML: variable {} is character; READ INTO requires numeric variables.",
                        ds.vars[ci].name
                    )));
                }
            };
            c.push(x);
        }
        cols.push(c);
    }
    // Assembler la matrice nrow × ncol.
    let mut mat = vec![vec![0.0; col_idx.len()]; nrow];
    for (j, c) in cols.iter().enumerate() {
        for (i, &x) in c.iter().enumerate() {
            mat[i][j] = x;
        }
    }
    env.vars.insert(into.to_ascii_uppercase(), mat);
    Ok(())
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
    let mut env = Env::new();
    let mut ops: Vec<PrintOp> = Vec::new();
    exec_stmts(&prog.stmts, &mut env, &mut ops, session)?;

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

    fn test_session() -> Session {
        use std::path::PathBuf;
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn eval_one(src: &str) -> Matrix {
        // Enveloppe : assigne à `r` et renvoie sa valeur.
        let prog = parse_body(&format!("r = {src};")).unwrap();
        let mut env = Env::new();
        let mut ops = Vec::new();
        let mut session = test_session();
        exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();
        env.vars.get("R").unwrap().clone()
    }

    fn eval_try(src: &str) -> Result<Matrix> {
        let prog = parse_body(&format!("r = {src};"))?;
        let mut env = Env::new();
        let mut ops = Vec::new();
        let mut session = test_session();
        exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session)?;
        Ok(env.vars.get("R").unwrap().clone())
    }

    fn run_get(src: &str, var: &str) -> Matrix {
        let prog = parse_body(src).unwrap();
        let mut env = Env::new();
        let mut ops = Vec::new();
        let mut session = test_session();
        exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();
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

    // ───────────────────── M28a.3 : algèbre linéaire ─────────────────────

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn inv_2x2() {
        let m = eval_one("inv({4 7, 2 6})");
        // inv = {0.6 -0.7, -0.2 0.4}
        assert!(approx(m[0][0], 0.6, 1e-3), "m={m:?}");
        assert!(approx(m[0][1], -0.7, 1e-3), "m={m:?}");
        assert!(approx(m[1][0], -0.2, 1e-3), "m={m:?}");
        assert!(approx(m[1][1], 0.4, 1e-3), "m={m:?}");
    }

    #[test]
    fn solve_diagonal() {
        let m = eval_one("solve({2 0, 0 3}, {6, 9})");
        // x = {3, 3} (column vector)
        assert_eq!(dims(&m), (2, 1));
        assert!(approx(m[0][0], 3.0, 1e-3), "m={m:?}");
        assert!(approx(m[1][0], 3.0, 1e-3), "m={m:?}");
    }

    #[test]
    fn eigval_symmetric() {
        let m = eval_one("eigval({4 2, 2 1})");
        // {5, 0} descending, column vector
        assert_eq!(dims(&m), (2, 1));
        assert!(approx(m[0][0], 5.0, 1e-3), "m={m:?}");
        assert!(approx(m[1][0], 0.0, 1e-3), "m={m:?}");
    }

    #[test]
    fn eigval_nonsymmetric_errors() {
        let e = eval_try("eigval({1 2, 3 4})");
        assert!(e.is_err());
        let msg = e.err().unwrap().to_string();
        assert!(msg.contains("symmetric"), "msg={msg}");
    }

    #[test]
    fn chol_upper() {
        let m = eval_one("chol({4 2, 2 3})");
        // U = {2 1, 0 1.4142}
        assert!(approx(m[0][0], 2.0, 1e-3), "m={m:?}");
        assert!(approx(m[0][1], 1.0, 1e-3), "m={m:?}");
        assert!(approx(m[1][0], 0.0, 1e-3), "m={m:?}");
        assert!(approx(m[1][1], std::f64::consts::SQRT_2, 1e-3), "m={m:?}");
    }

    #[test]
    fn chol_not_spd_errors() {
        // {1 2, 2 1} is indefinite (det = 1 - 4 = -3 < 0).
        let e = eval_try("chol({1 2, 2 1})");
        assert!(e.is_err(), "expected error for non-SPD matrix");
    }

    #[test]
    fn call_qr_dimensions() {
        let prog = parse_body("call qr(q, r, {1 2, 3 4, 5 6});").unwrap();
        let mut env = Env::new();
        let mut ops = Vec::new();
        let mut session = test_session();
        exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();
        let q = env.vars.get("Q").unwrap();
        let r = env.vars.get("R").unwrap();
        assert_eq!(dims(q), (3, 2), "Q dims");
        assert_eq!(dims(r), (2, 2), "R dims");
        // Q*R ≈ original.
        let qr = eval_binop(ImlOp::Mul, q, r).unwrap();
        assert!(approx(qr[0][0], 1.0, 1e-6) && approx(qr[2][1], 6.0, 1e-6), "qr={qr:?}");
    }

    #[test]
    fn call_svdcd_singular_values() {
        let prog = parse_body("call svdcd(u, d, v, {1 2, 3 4});").unwrap();
        let mut env = Env::new();
        let mut ops = Vec::new();
        let mut session = test_session();
        exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();
        let d = env.vars.get("D").unwrap();
        assert_eq!(dims(d), (2, 1), "D should be a column vector");
        assert!(approx(d[0][0], 5.4651, 1e-2), "σ1={}", d[0][0]);
        assert!(approx(d[1][0], 0.3660, 1e-2), "σ2={}", d[1][0]);
        // Reconstruction: A = U diag(D) V'.
        let u = env.vars.get("U").unwrap().clone();
        let vmat = env.vars.get("V").unwrap().clone();
        let ud: Matrix = u
            .iter()
            .map(|row| row.iter().enumerate().map(|(j, &x)| x * d[j][0]).collect())
            .collect();
        let recon = eval_binop(ImlOp::Mul, &ud, &transpose(&vmat)).unwrap();
        assert!(approx(recon[0][0], 1.0, 1e-4) && approx(recon[1][1], 4.0, 1e-4), "recon={recon:?}");
    }

    #[test]
    fn deferred_eigvec_and_det_errors() {
        assert!(eval_try("eigvec({1 0, 0 1})").is_err());
        assert!(eval_try("det({1 0, 0 1})").is_err());
    }

    // ───────────────────── M28a.4 : I/O datasets ─────────────────────

    #[test]
    fn create_append_close_writes_dataset() {
        let src = r#"
            mat_out = {1 10, 2 20, 3 30};
            cn = {"id" "val"};
            create work.iml_out from mat_out[colname=cn];
            append from mat_out;
            close work.iml_out;
        "#;
        let prog = parse_body(src).unwrap();
        let mut env = Env::new();
        let mut ops = Vec::new();
        let mut session = test_session();
        exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();

        let (ds, _) = session.libs.get("WORK").unwrap().read("IML_OUT").unwrap();
        assert_eq!(ds.n_obs(), 3, "expected 3 rows");
        let names: Vec<String> = ds.vars.iter().map(|v| v.name.to_lowercase()).collect();
        assert_eq!(names, vec!["id".to_string(), "val".to_string()]);
    }

    #[test]
    fn use_read_all_close_reads_dataset() {
        // First write a dataset, then USE/READ it back.
        let mut session = test_session();
        {
            use crate::dataset::{SasDataset, VarMeta};
            use crate::value::VarType;
            use polars::prelude::*;
            let df = df!["x" => [1.0_f64, 2.0, 3.0], "y" => [10.0_f64, 20.0, 30.0]].unwrap();
            let vars = vec![
                VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
                VarMeta { name: "y".into(), ty: VarType::Num, length: 8, format: None, label: None },
            ];
            session.libs.get("WORK").unwrap().write("IML_IN", &SasDataset { df, vars }).unwrap();
        }
        let src = r#"
            use work.iml_in;
            read all var {"x" "y"} into m;
            close work.iml_in;
        "#;
        let prog = parse_body(src).unwrap();
        let mut env = Env::new();
        let mut ops = Vec::new();
        exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();
        let m = env.vars.get("M").unwrap();
        assert_eq!(dims(m), (3, 2), "m={m:?}");
        assert!(approx(m[0][0], 1.0, 1e-9) && approx(m[2][1], 30.0, 1e-9), "m={m:?}");
    }

    #[test]
    fn read_next_deferred_error() {
        let prog = parse_body("read next into m;").unwrap();
        let mut env = Env::new();
        let mut ops = Vec::new();
        let mut session = test_session();
        let e = exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session);
        assert!(e.is_err());
        assert!(e.err().unwrap().to_string().contains("READ NEXT"));
    }
}
