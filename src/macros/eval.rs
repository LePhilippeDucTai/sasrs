//! Évaluation d'expressions macro : `%eval` (entier) et `%sysevalf` (flottant).
//!
//! Tokenizer partagé (`tokenize_eval`) puis deux analyseurs récursifs-descendants
//! (`EvalParser` entier, `FloatParser` flottant) sur la même grammaire.

use super::*;

impl MacroEngine {
    /// Formate le résultat flottant de `%sysevalf` selon la conversion demandée.
    pub(super) fn format_sysevalf(v: f64, conv: Option<&str>) -> String {
        match conv {
            Some("BOOLEAN") => {
                if v != 0.0 && !v.is_nan() {
                    "1".to_string()
                } else {
                    "0".to_string()
                }
            }
            Some("CEIL") => Self::format_float(v.ceil()),
            Some("FLOOR") => Self::format_float(v.floor()),
            Some("INTEGER") => Self::format_float(v.trunc()),
            _ => Self::format_float(v),
        }
    }

    /// Formate un `f64` en texte façon SAS : un entier exact perd ses décimales
    /// (`3.0` → `"3"`), sinon on emploie une représentation compacte sans zéros
    /// finaux superflus.
    pub(super) fn format_float(v: f64) -> String {
        if v.is_nan() {
            return String::new();
        }
        if v == v.trunc() && v.abs() < 1e15 {
            return format!("{}", v as i64);
        }
        // Représentation compacte : `{}` sur f64 rend déjà la plus courte forme
        // fidèle sans zéros finaux superflus.
        format!("{v}")
    }

    /// Évalue une expression arithmétique FLOTTANTE (pour `%sysevalf`). Supporte
    /// `+ - * / **`, parenthèses, comparaisons (`= ne < <= > >= eq …` → 1/0),
    /// logique (`and or not & | ^`) et l'unaire `+`/`-`. Tout est calculé en
    /// `f64` (division réelle, `**` réelle). Un opérande non numérique → erreur.
    pub(super) fn eval_float(expr: &str) -> Result<f64, MacroError> {
        let toks = Self::tokenize_eval(expr)?;
        let mut p = FloatParser { toks: &toks, pos: 0 };
        let v = p.parse_expr()?;
        if p.pos != p.toks.len() {
            return Err(MacroError::new(format!(
                "ERROR: A syntax error was detected in the %SYSEVALF expression: {expr}"
            )));
        }
        Ok(v)
    }

    /// Évalue une condition `%if` : résout d'abord les `&refs` et tout
    /// `%eval`/macro imbriqué, puis applique `macro_eval`. Truthy = non nul.
    pub(super) fn eval_condition(&mut self, cond: &str) -> Result<bool, MacroError> {
        let resolved = self.resolve_value(cond);
        let expanded = self.process_impl(&resolved);
        Ok(self.macro_eval(expanded.trim())? != 0)
    }

    /// Comme `eval_condition` mais rend l'entier (pour les bornes `%to`/`%by`).
    pub(super) fn eval_condition_int(&mut self, expr: &str) -> Result<i64, MacroError> {
        let resolved = self.resolve_value(expr);
        let expanded = self.process_impl(&resolved);
        self.macro_eval(expanded.trim())
    }
}

/// Jeton de l'expression macro pour `%eval`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EvalTok {
    Int(i64),
    /// Opérande non entier (rencontré tel quel) : déclenche l'erreur SAS
    /// "A character operand was found..." si utilisé dans un contexte
    /// arithmétique. Conservé pour égalité textuelle dans les comparaisons.
    Word(String),
    Plus,
    Minus,
    Star,
    Slash,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    LParen,
    RParen,
}

impl MacroEngine {
    /// Évalue une expression macro `%eval` selon la sémantique ENTIÈRE de SAS.
    /// Le texte fourni doit déjà avoir ses `&vars` résolus (l'appelant le fait).
    ///
    /// Grammaire (par précédence croissante, récursive-descente) :
    /// ```text
    /// expr        := or_expr
    /// or_expr     := and_expr ( ('|' | 'or') and_expr )*
    /// and_expr    := not_expr ( ('&' | 'and') not_expr )*
    /// not_expr    := ('^' | '~' | 'not')* cmp_expr
    /// cmp_expr    := add_expr ( cmp_op add_expr )?
    /// add_expr    := mul_expr ( ('+' | '-') mul_expr )*
    /// mul_expr    := pow_expr ( ('*' | '/') pow_expr )*
    /// pow_expr    := unary ( '**' pow_expr )?         // associatif à droite
    /// unary       := ('+' | '-')* primary
    /// primary     := INT | '(' expr ')'
    /// ```
    /// Sémantique : opérandes entiers ; division ENTIÈRE tronquée vers zéro ;
    /// `**` puissance entière ; comparaisons → 1/0 ; logiques → 1/0 (vrai =
    /// non nul). Un opérande non entier dans un contexte arithmétique est une
    /// erreur ("A character operand was found in the %EVAL function...").
    pub(super) fn macro_eval(&self, expr: &str) -> Result<i64, MacroError> {
        let toks = Self::tokenize_eval(expr)?;
        let mut p = EvalParser { toks: &toks, pos: 0 };
        let v = p.parse_expr()?;
        if p.pos != p.toks.len() {
            return Err(MacroError::new(format!(
                "ERROR: A syntax error was detected in the %EVAL expression: {expr}"
            )));
        }
        Ok(v)
    }

    /// Découpe l'expression en jetons. Les espaces séparent ; les mots
    /// alphabétiques sont reconnus comme opérateurs textuels (`eq`, `and`,
    /// `not`, ...) sinon conservés comme `Word` (opérande non entier).
    pub(super) fn tokenize_eval(expr: &str) -> Result<Vec<EvalTok>, MacroError> {
        let chars: Vec<char> = expr.chars().collect();
        let mut toks = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            match c {
                '+' => {
                    toks.push(EvalTok::Plus);
                    i += 1;
                }
                '-' => {
                    toks.push(EvalTok::Minus);
                    i += 1;
                }
                '*' => {
                    if chars.get(i + 1) == Some(&'*') {
                        toks.push(EvalTok::Pow);
                        i += 2;
                    } else {
                        toks.push(EvalTok::Star);
                        i += 1;
                    }
                }
                '/' => {
                    toks.push(EvalTok::Slash);
                    i += 1;
                }
                '(' => {
                    toks.push(EvalTok::LParen);
                    i += 1;
                }
                ')' => {
                    toks.push(EvalTok::RParen);
                    i += 1;
                }
                '=' => {
                    toks.push(EvalTok::Eq);
                    i += 1;
                }
                '&' => {
                    // `&&` ou `&` -> AND logique.
                    if chars.get(i + 1) == Some(&'&') {
                        i += 2;
                    } else {
                        i += 1;
                    }
                    toks.push(EvalTok::And);
                }
                '|' => {
                    if chars.get(i + 1) == Some(&'|') {
                        i += 2;
                    } else {
                        i += 1;
                    }
                    toks.push(EvalTok::Or);
                }
                '<' => {
                    if chars.get(i + 1) == Some(&'=') {
                        toks.push(EvalTok::Le);
                        i += 2;
                    } else if chars.get(i + 1) == Some(&'>') {
                        // `<>` = NE en contexte de comparaison macro SAS.
                        toks.push(EvalTok::Ne);
                        i += 2;
                    } else {
                        toks.push(EvalTok::Lt);
                        i += 1;
                    }
                }
                '>' => {
                    if chars.get(i + 1) == Some(&'=') {
                        toks.push(EvalTok::Ge);
                        i += 2;
                    } else {
                        toks.push(EvalTok::Gt);
                        i += 1;
                    }
                }
                '^' | '~' => {
                    if chars.get(i + 1) == Some(&'=') {
                        toks.push(EvalTok::Ne);
                        i += 2;
                    } else {
                        toks.push(EvalTok::Not);
                        i += 1;
                    }
                }
                _ if c.is_ascii_digit() || c == '.' => {
                    let start = i;
                    while matches!(chars.get(i), Some(d) if d.is_ascii_digit()) {
                        i += 1;
                    }
                    // Partie fractionnaire / exposant : marque un littéral FLOTTANT
                    // (`7.5`, `.5`, `1e3`). `%eval` (entier) le verra comme un
                    // `Word` et émettra l'erreur « character operand » ; `%sysevalf`
                    // (flottant) le parse en `f64`.
                    let mut is_float = false;
                    if chars.get(i) == Some(&'.') {
                        is_float = true;
                        i += 1;
                        while matches!(chars.get(i), Some(d) if d.is_ascii_digit()) {
                            i += 1;
                        }
                    }
                    if matches!(chars.get(i), Some('e' | 'E'))
                        && matches!(
                            chars.get(i + 1),
                            Some(d) if d.is_ascii_digit()
                                || ((*d == '+' || *d == '-')
                                    && matches!(chars.get(i + 2), Some(e) if e.is_ascii_digit()))
                        )
                    {
                        is_float = true;
                        i += 1; // 'e'
                        if matches!(chars.get(i), Some('+' | '-')) {
                            i += 1;
                        }
                        while matches!(chars.get(i), Some(d) if d.is_ascii_digit()) {
                            i += 1;
                        }
                    }
                    // Un opérande alphanumérique mixte (ex. `3a`) est un mot.
                    if matches!(chars.get(i), Some(d) if d.is_ascii_alphabetic() || *d == '_') {
                        let wstart = start;
                        while matches!(chars.get(i), Some(d) if d.is_ascii_alphanumeric() || *d == '_') {
                            i += 1;
                        }
                        let w: String = chars[wstart..i].iter().collect();
                        toks.push(EvalTok::Word(w));
                    } else if is_float {
                        // Littéral flottant : porté comme `Word` (entier le rejette).
                        toks.push(EvalTok::Word(chars[start..i].iter().collect()));
                    } else {
                        let s: String = chars[start..i].iter().collect();
                        match s.parse::<i64>() {
                            Ok(n) => toks.push(EvalTok::Int(n)),
                            Err(_) => {
                                return Err(MacroError::new(format!(
                                    "ERROR: Overflow in the %EVAL function: {s}"
                                )))
                            }
                        }
                    }
                }
                _ if c.is_ascii_alphabetic() || c == '_' => {
                    let start = i;
                    while matches!(chars.get(i), Some(d) if d.is_ascii_alphanumeric() || *d == '_') {
                        i += 1;
                    }
                    let w: String = chars[start..i].iter().collect();
                    match w.to_ascii_lowercase().as_str() {
                        "eq" => toks.push(EvalTok::Eq),
                        "ne" => toks.push(EvalTok::Ne),
                        "lt" => toks.push(EvalTok::Lt),
                        "le" => toks.push(EvalTok::Le),
                        "gt" => toks.push(EvalTok::Gt),
                        "ge" => toks.push(EvalTok::Ge),
                        "and" => toks.push(EvalTok::And),
                        "or" => toks.push(EvalTok::Or),
                        "not" => toks.push(EvalTok::Not),
                        _ => toks.push(EvalTok::Word(w)),
                    }
                }
                other => {
                    return Err(MacroError::new(format!(
                        "ERROR: A syntax error was detected in the %EVAL expression near '{other}'"
                    )))
                }
            }
        }
        Ok(toks)
    }
}

/// Analyseur récursif-descendant pour l'expression `%eval`.
pub(super) struct EvalParser<'a> {
    pub(super) toks: &'a [EvalTok],
    pub(super) pos: usize,
}

impl<'a> EvalParser<'a> {
    fn peek(&self) -> Option<&EvalTok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<&EvalTok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_expr(&mut self) -> Result<i64, MacroError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<i64, MacroError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(EvalTok::Or)) {
            self.bump();
            let right = self.parse_and()?;
            left = ((left != 0) || (right != 0)) as i64;
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<i64, MacroError> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), Some(EvalTok::And)) {
            self.bump();
            let right = self.parse_not()?;
            left = ((left != 0) && (right != 0)) as i64;
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<i64, MacroError> {
        let mut negs = 0;
        while matches!(self.peek(), Some(EvalTok::Not)) {
            self.bump();
            negs += 1;
        }
        let v = self.parse_cmp()?;
        if negs % 2 == 1 {
            Ok((v == 0) as i64)
        } else {
            Ok(v)
        }
    }

    fn parse_cmp(&mut self) -> Result<i64, MacroError> {
        let left = self.parse_add()?;
        if let Some(op) = self.peek().cloned() {
            let is_cmp = matches!(
                op,
                EvalTok::Eq
                    | EvalTok::Ne
                    | EvalTok::Lt
                    | EvalTok::Le
                    | EvalTok::Gt
                    | EvalTok::Ge
            );
            if is_cmp {
                self.bump();
                let right = self.parse_add()?;
                let r = match op {
                    EvalTok::Eq => left == right,
                    EvalTok::Ne => left != right,
                    EvalTok::Lt => left < right,
                    EvalTok::Le => left <= right,
                    EvalTok::Gt => left > right,
                    EvalTok::Ge => left >= right,
                    _ => unreachable!(),
                };
                return Ok(r as i64);
            }
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<i64, MacroError> {
        let mut left = self.parse_mul()?;
        loop {
            match self.peek() {
                Some(EvalTok::Plus) => {
                    self.bump();
                    left = left.wrapping_add(self.parse_mul()?);
                }
                Some(EvalTok::Minus) => {
                    self.bump();
                    left = left.wrapping_sub(self.parse_mul()?);
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<i64, MacroError> {
        let mut left = self.parse_pow()?;
        loop {
            match self.peek() {
                Some(EvalTok::Star) => {
                    self.bump();
                    left = left.wrapping_mul(self.parse_pow()?);
                }
                Some(EvalTok::Slash) => {
                    self.bump();
                    let right = self.parse_pow()?;
                    if right == 0 {
                        return Err(MacroError::new(
                            "ERROR: Division by zero detected in the %EVAL expression",
                        ));
                    }
                    // Division entière tronquée vers zéro (sémantique Rust `/`).
                    left /= right;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_pow(&mut self) -> Result<i64, MacroError> {
        let base = self.parse_unary()?;
        if matches!(self.peek(), Some(EvalTok::Pow)) {
            self.bump();
            // Associatif à droite.
            let exp = self.parse_pow()?;
            return Ok(Self::ipow(base, exp));
        }
        Ok(base)
    }

    /// Puissance entière ; exposant négatif -> 0 (sémantique entière, comme SAS
    /// qui tronque le résultat fractionnaire vers 0 sauf base ±1).
    fn ipow(base: i64, exp: i64) -> i64 {
        if exp < 0 {
            return match base {
                1 => 1,
                -1 => {
                    if (-exp) % 2 == 0 {
                        1
                    } else {
                        -1
                    }
                }
                _ => 0,
            };
        }
        let mut result: i64 = 1;
        let mut b = base;
        let mut e = exp;
        while e > 0 {
            if e & 1 == 1 {
                result = result.wrapping_mul(b);
            }
            e >>= 1;
            if e > 0 {
                b = b.wrapping_mul(b);
            }
        }
        result
    }

    fn parse_unary(&mut self) -> Result<i64, MacroError> {
        match self.peek() {
            Some(EvalTok::Plus) => {
                self.bump();
                self.parse_unary()
            }
            Some(EvalTok::Minus) => {
                self.bump();
                Ok(self.parse_unary()?.wrapping_neg())
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<i64, MacroError> {
        match self.bump() {
            Some(EvalTok::Int(n)) => Ok(*n),
            Some(EvalTok::LParen) => {
                let v = self.parse_expr()?;
                match self.bump() {
                    Some(EvalTok::RParen) => Ok(v),
                    _ => Err(MacroError::new(
                        "ERROR: A syntax error was detected in the %EVAL expression: expected ')'",
                    )),
                }
            }
            Some(EvalTok::Word(w)) => Err(MacroError::new(format!(
                "ERROR: A character operand was found in the %EVAL function or %IF condition where a numeric operand is required. The condition was: {w}"
            ))),
            other => Err(MacroError::new(format!(
                "ERROR: A syntax error was detected in the %EVAL expression near {other:?}"
            ))),
        }
    }
}

/// Analyseur récursif-descendant FLOTTANT pour `%sysevalf` (M19.1). Même
/// grammaire que [`EvalParser`] mais en `f64` : division réelle, `**` réelle,
/// comparaisons/logique rendant `1.0`/`0.0`. Réutilise les `EvalTok` produits
/// par `MacroEngine::tokenize_eval` ; un littéral flottant arrive comme
/// `EvalTok::Word` (que cet analyseur parse en nombre, contrairement à
/// l'analyseur entier qui le rejette).
pub(super) struct FloatParser<'a> {
    pub(super) toks: &'a [EvalTok],
    pub(super) pos: usize,
}

impl FloatParser<'_> {
    fn peek(&self) -> Option<&EvalTok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<&EvalTok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_expr(&mut self) -> Result<f64, MacroError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<f64, MacroError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(EvalTok::Or)) {
            self.bump();
            let right = self.parse_and()?;
            left = ((left != 0.0) || (right != 0.0)) as i64 as f64;
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<f64, MacroError> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), Some(EvalTok::And)) {
            self.bump();
            let right = self.parse_not()?;
            left = ((left != 0.0) && (right != 0.0)) as i64 as f64;
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<f64, MacroError> {
        let mut negs = 0;
        while matches!(self.peek(), Some(EvalTok::Not)) {
            self.bump();
            negs += 1;
        }
        let v = self.parse_cmp()?;
        if negs % 2 == 1 {
            Ok((v == 0.0) as i64 as f64)
        } else {
            Ok(v)
        }
    }

    fn parse_cmp(&mut self) -> Result<f64, MacroError> {
        let left = self.parse_add()?;
        if let Some(op) = self.peek().cloned() {
            let is_cmp = matches!(
                op,
                EvalTok::Eq
                    | EvalTok::Ne
                    | EvalTok::Lt
                    | EvalTok::Le
                    | EvalTok::Gt
                    | EvalTok::Ge
            );
            if is_cmp {
                self.bump();
                let right = self.parse_add()?;
                let r = match op {
                    EvalTok::Eq => left == right,
                    EvalTok::Ne => left != right,
                    EvalTok::Lt => left < right,
                    EvalTok::Le => left <= right,
                    EvalTok::Gt => left > right,
                    EvalTok::Ge => left >= right,
                    _ => unreachable!(),
                };
                return Ok(r as i64 as f64);
            }
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<f64, MacroError> {
        let mut left = self.parse_mul()?;
        loop {
            match self.peek() {
                Some(EvalTok::Plus) => {
                    self.bump();
                    left += self.parse_mul()?;
                }
                Some(EvalTok::Minus) => {
                    self.bump();
                    left -= self.parse_mul()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<f64, MacroError> {
        let mut left = self.parse_pow()?;
        loop {
            match self.peek() {
                Some(EvalTok::Star) => {
                    self.bump();
                    left *= self.parse_pow()?;
                }
                Some(EvalTok::Slash) => {
                    self.bump();
                    let right = self.parse_pow()?;
                    if right == 0.0 {
                        return Err(MacroError::new(
                            "ERROR: Division by zero detected in the %SYSEVALF expression",
                        ));
                    }
                    // Division RÉELLE (≠ %eval qui tronque).
                    left /= right;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_pow(&mut self) -> Result<f64, MacroError> {
        let base = self.parse_unary()?;
        if matches!(self.peek(), Some(EvalTok::Pow)) {
            self.bump();
            // Associatif à droite.
            let exp = self.parse_pow()?;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    fn parse_unary(&mut self) -> Result<f64, MacroError> {
        match self.peek() {
            Some(EvalTok::Plus) => {
                self.bump();
                self.parse_unary()
            }
            Some(EvalTok::Minus) => {
                self.bump();
                Ok(-self.parse_unary()?)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<f64, MacroError> {
        match self.bump() {
            Some(EvalTok::Int(n)) => Ok(*n as f64),
            Some(EvalTok::Word(w)) => w.parse::<f64>().map_err(|_| {
                MacroError::new(format!(
                    "ERROR: A character operand was found in the %SYSEVALF function where a numeric operand is required: {w}"
                ))
            }),
            Some(EvalTok::LParen) => {
                let v = self.parse_expr()?;
                match self.bump() {
                    Some(EvalTok::RParen) => Ok(v),
                    _ => Err(MacroError::new(
                        "ERROR: A syntax error was detected in the %SYSEVALF expression: expected ')'",
                    )),
                }
            }
            other => Err(MacroError::new(format!(
                "ERROR: A syntax error was detected in the %SYSEVALF expression near {other:?}"
            ))),
        }
    }
}
