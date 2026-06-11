use std::cmp::Ordering;

/// SAS numeric missing values. Sort order: `._` < `.` < `.A` < ... < `.Z`,
/// and every missing sorts before every number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MissingKind {
    Underscore,
    Dot,
    /// 0 = .A ... 25 = .Z
    Letter(u8),
}

impl MissingKind {
    pub fn from_letter(c: char) -> Option<MissingKind> {
        match c {
            '_' => Some(MissingKind::Underscore),
            'a'..='z' => Some(MissingKind::Letter(c as u8 - b'a')),
            'A'..='Z' => Some(MissingKind::Letter(c as u8 - b'A')),
            _ => None,
        }
    }

    /// How the value prints in listings/logs: `.`, `_`, or `A`..`Z`.
    pub fn display(&self) -> String {
        match self {
            MissingKind::Underscore => "_".to_string(),
            MissingKind::Dot => ".".to_string(),
            MissingKind::Letter(i) => ((b'A' + i) as char).to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarType {
    Num,
    Char,
}

/// A SAS data value: the language has exactly two types, numeric (f64,
/// with 28 distinct missing values) and character (fixed length, blank
/// padded — we store trimmed and enforce length at assignment).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Missing(MissingKind),
    Char(String),
}

impl Value {
    pub fn missing() -> Value {
        Value::Missing(MissingKind::Dot)
    }

    pub fn var_type(&self) -> VarType {
        match self {
            Value::Char(_) => VarType::Char,
            _ => VarType::Num,
        }
    }

    /// SAS missing test: numeric missings, or blank character value.
    pub fn is_missing(&self) -> bool {
        match self {
            Value::Missing(_) => true,
            Value::Char(s) => s.trim().is_empty(),
            Value::Num(_) => false,
        }
    }

    /// SAS boolean context: missing and 0 are false, everything else true.
    /// Character values are true when non-blank (SAS would flag a type
    /// error; we are lenient here).
    pub fn truthy(&self) -> bool {
        match self {
            Value::Num(n) => *n != 0.0,
            Value::Missing(_) => false,
            Value::Char(s) => !s.trim().is_empty(),
        }
    }

    /// SAS total order. Numeric: missings sort below all numbers with
    /// `._ < . < .A < ... < .Z`; note `. = .` is TRUE in SAS, which this
    /// ordering encodes. Character: comparison ignores trailing blanks
    /// (SAS pads the shorter operand). Mixed types compare Num < Char
    /// only to keep the order total; real SAS raises an error, which the
    /// evaluator reports before ever calling this.
    pub fn sas_cmp(&self, other: &Value) -> Ordering {
        match (self, other) {
            (Value::Missing(a), Value::Missing(b)) => a.cmp(b),
            (Value::Missing(_), Value::Num(_)) => Ordering::Less,
            (Value::Num(_), Value::Missing(_)) => Ordering::Greater,
            (Value::Num(a), Value::Num(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Value::Char(a), Value::Char(b)) => a.trim_end().cmp(b.trim_end()),
            (Value::Char(_), _) => Ordering::Greater,
            (_, Value::Char(_)) => Ordering::Less,
        }
    }
}

/// Approximation of the BESTw. format: the most readable representation
/// that fits in `w` columns. Integers print without decimals; otherwise
/// we use the shortest round-trip representation, degrading precision and
/// finally scientific notation to fit.
pub fn format_best(v: f64, w: usize) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        let s = format!("{}", v as i64);
        if s.len() <= w {
            return s;
        }
    }
    let s = format!("{v}");
    if s.len() <= w {
        return s;
    }
    // Reduce decimal precision until it fits.
    for prec in (1..=w.saturating_sub(2)).rev() {
        let s = format!("{v:.prec$}");
        if s.len() <= w {
            let trimmed = s.trim_end_matches('0').trim_end_matches('.');
            return trimmed.to_string();
        }
    }
    // Last resort: scientific notation.
    let s = format!("{v:E}");
    if s.len() <= w {
        s
    } else {
        format!("{v:.2E}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_order_is_total_sas_order() {
        let underscore = Value::Missing(MissingKind::Underscore);
        let dot = Value::Missing(MissingKind::Dot);
        let a = Value::Missing(MissingKind::Letter(0));
        let z = Value::Missing(MissingKind::Letter(25));
        let neg = Value::Num(-1e300);

        assert_eq!(underscore.sas_cmp(&dot), Ordering::Less);
        assert_eq!(dot.sas_cmp(&a), Ordering::Less);
        assert_eq!(a.sas_cmp(&z), Ordering::Less);
        assert_eq!(z.sas_cmp(&neg), Ordering::Less);
        // `. = .` is true in SAS.
        assert_eq!(dot.sas_cmp(&Value::missing()), Ordering::Equal);
    }

    #[test]
    fn char_compare_ignores_trailing_blanks() {
        let a = Value::Char("abc".into());
        let b = Value::Char("abc   ".into());
        assert_eq!(a.sas_cmp(&b), Ordering::Equal);
    }

    #[test]
    fn best_format() {
        assert_eq!(format_best(3.0, 12), "3");
        assert_eq!(format_best(-42.0, 12), "-42");
        assert_eq!(format_best(0.5, 12), "0.5");
        assert_eq!(format_best(1.0 / 3.0, 12), "0.3333333333");
    }
}
