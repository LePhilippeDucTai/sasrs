use super::*;

/// Jeton de l'expression macro pour `%eval`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EvalTok {
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

/// Analyseur récursif-descendant pour l'expression `%eval`.
pub(super) struct EvalParser<'a> {
    pub(super) toks: &'a [EvalTok],
    pub(super) pos: usize,
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
    pub(super) fn peek(&self) -> Option<&EvalTok> {
        self.toks.get(self.pos)
    }

    pub(super) fn bump(&mut self) -> Option<&EvalTok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    pub(super) fn parse_expr(&mut self) -> Result<f64, MacroError> {
        self.parse_or()
    }

    pub(super) fn parse_or(&mut self) -> Result<f64, MacroError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(EvalTok::Or)) {
            self.bump();
            let right = self.parse_and()?;
            left = ((left != 0.0) || (right != 0.0)) as i64 as f64;
        }
        Ok(left)
    }

    pub(super) fn parse_and(&mut self) -> Result<f64, MacroError> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), Some(EvalTok::And)) {
            self.bump();
            let right = self.parse_not()?;
            left = ((left != 0.0) && (right != 0.0)) as i64 as f64;
        }
        Ok(left)
    }

    pub(super) fn parse_not(&mut self) -> Result<f64, MacroError> {
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

    pub(super) fn parse_cmp(&mut self) -> Result<f64, MacroError> {
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

    pub(super) fn parse_add(&mut self) -> Result<f64, MacroError> {
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

    pub(super) fn parse_mul(&mut self) -> Result<f64, MacroError> {
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

    pub(super) fn parse_pow(&mut self) -> Result<f64, MacroError> {
        let base = self.parse_unary()?;
        if matches!(self.peek(), Some(EvalTok::Pow)) {
            self.bump();
            // Associatif à droite.
            let exp = self.parse_pow()?;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    pub(super) fn parse_unary(&mut self) -> Result<f64, MacroError> {
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

    pub(super) fn parse_primary(&mut self) -> Result<f64, MacroError> {
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
