//! Formats et informats utilisateur définis par PROC FORMAT (jalons M4/M18.2).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc format; value sexfmt 1='Male' 2='Female' other='?'; run;`
//! produit un `UserFormat` à plages :
//! - plage = valeur unique, `low-<high` / `low<-high` (bornes exclusives
//!   côté `<`), `LOW`/`HIGH` symboliques, ou liste `1,2,3='x'`.
//! - `VALUE` (num→label), `VALUE $fmt` (char→label) ; `INVALUE` pour les
//!   informats utilisateur (M18.2).
//! - Résolution : première plage qui matche dans l'ordre de tri SAS des
//!   bornes ; sinon `other` ; sinon la valeur formatée en BEST./$.
//!
//! ## INVALUE (M18.2)
//! `invalue grade 'A'=4 'B'=3 'C'=2 'D'=1 'F'=0;`
//! produit un `UserInformat` : les CLÉS sont des chaînes (plages de chaînes,
//! comme les formats char), la VALEUR de résultat est `Value::Num` ou
//! `Value::Char` selon que le nom porte `$` ou non.
//! - `invalue name` → résultat numérique ; clés = chaînes brutes à matcher.
//! - `invalue $name` → résultat caractère.
//! - La valeur de résultat peut être : littéral numérique `=4`, littéral
//!   chaîne `='Small'`, ou le mot-clé `_SAME_` (copie l'entrée non modifiée).
//! - `other=value` : valeur de repli si aucune plage ne correspond.
//! - Ranges de chaînes : `'A'-'F'` inclusive, `low-'C'`, `'D'-high`, avec
//!   bornes exclusives `<` ; comparison via `str::trim_end()` (insensible aux
//!   blancs finaux, fidèle SAS).
//! - Valeur inconnue (aucune plage, pas d'other) → missing.

#![allow(unused_variables, dead_code)]

use crate::value::Value;

#[derive(Debug)]
pub enum Bound {
    Low,
    High,
    Num(f64),
    Char(String),
}

impl Clone for Bound {
    fn clone(&self) -> Self {
        match self {
            Bound::Low => Bound::Low,
            Bound::High => Bound::High,
            Bound::Num(n) => Bound::Num(*n),
            Bound::Char(s) => Bound::Char(s.clone()),
        }
    }
}

#[derive(Clone)]
pub struct Range {
    pub from: Bound,
    pub to: Bound,
    pub from_exclusive: bool,
    pub to_exclusive: bool,
    pub label: String,
}

#[derive(Clone)]
pub struct UserFormat {
    pub is_char: bool,
    pub ranges: Vec<Range>,
    pub other: Option<String>,
}

impl UserFormat {
    pub fn lookup(&self, v: &Value) -> Option<&str> {
        for range in &self.ranges {
            if self.range_matches(range, v) {
                return Some(&range.label);
            }
        }
        self.other.as_deref()
    }

    fn range_matches(&self, range: &Range, v: &Value) -> bool {
        if self.is_char {
            // Character format: match Value::Char against Bound::Char bounds.
            let s = match v {
                Value::Char(s) => s.trim_end(),
                _ => return false,
            };
            let from_ok = match &range.from {
                Bound::Low => true,
                Bound::High => false,
                Bound::Char(c) => {
                    let c = c.trim_end();
                    if range.from_exclusive { s > c } else { s >= c }
                }
                Bound::Num(_) => false,
            };
            if !from_ok {
                return false;
            }
            let to_ok = match &range.to {
                Bound::High => true,
                Bound::Low => false,
                Bound::Char(c) => {
                    let c = c.trim_end();
                    if range.to_exclusive { s < c } else { s <= c }
                }
                Bound::Num(_) => false,
            };
            to_ok
        } else {
            // Numeric format: match Value::Num against numeric bounds.
            // Missing values don't match numeric ranges unless there is a
            // special handling; here we treat them as no-match (falls to `other`).
            let n = match v {
                Value::Num(n) => *n,
                _ => return false,
            };
            let from_ok = match &range.from {
                Bound::Low => true,
                Bound::High => false,
                Bound::Num(lo) => {
                    if range.from_exclusive { n > *lo } else { n >= *lo }
                }
                Bound::Char(_) => false,
            };
            if !from_ok {
                return false;
            }
            let to_ok = match &range.to {
                Bound::High => true,
                Bound::Low => false,
                Bound::Num(hi) => {
                    if range.to_exclusive { n < *hi } else { n <= *hi }
                }
                Bound::Char(_) => false,
            };
            to_ok
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// UserInformat (M18.2) — INVALUE maps string keys → Value results
// ─────────────────────────────────────────────────────────────────────────────

/// The result value of a single INVALUE mapping.
#[derive(Clone, Debug)]
pub enum InformatValue {
    /// A numeric literal result (e.g. `'A'=4`).
    Num(f64),
    /// A character literal result (e.g. `'S'='Small'`).
    Char(String),
    /// `_SAME_` — return the input string unchanged (as Char or Num depending
    /// on the informat type).
    Same,
    /// Missing (`.` alone or `._` / `.A`..`.Z`). Encodes the missing kind as
    /// a string: `"."` = standard, `"_"` = underscore, `"A"`..`"Z"` = letter.
    Missing(String),
}

/// A single range entry in a `UserInformat` (string key range → result value).
#[derive(Clone, Debug)]
pub struct InformatRange {
    /// Lower bound (character string or Low sentinel). For a single-value entry
    /// `from == to` and both have the same `Bound::Char(…)` value.
    pub from: Bound,
    /// Upper bound.
    pub to: Bound,
    pub from_exclusive: bool,
    pub to_exclusive: bool,
    /// The result to produce when this range matches.
    pub result: InformatValue,
}

/// User-defined informat (from PROC FORMAT INVALUE statement). Maps an input
/// string to a `Value` result.
///
/// The lookup key is always a string (the raw text being read). The result
/// type is determined by whether the informat name has a `$` prefix:
///   - `invalue grade` → numeric result
///   - `invalue $size` → character result
#[derive(Clone, Debug)]
pub struct UserInformat {
    /// `true` if the name had a `$` prefix → character result.
    pub is_char_result: bool,
    pub ranges: Vec<InformatRange>,
    /// Fallback for unmatched input.
    pub other: Option<InformatValue>,
}

impl UserInformat {
    /// Perform the informat lookup. Returns `None` if no range matched AND
    /// there is no `other` fallback — the caller should use missing in that
    /// case.
    pub fn lookup(&self, input: &str) -> Option<Value> {
        let trimmed = input.trim_end();
        for range in &self.ranges {
            if self.range_matches(range, trimmed) {
                return Some(self.resolve_result(&range.result, input));
            }
        }
        self.other.as_ref().map(|r| self.resolve_result(r, input))
    }

    /// Check whether the trimmed input string falls within a range's bounds.
    fn range_matches(&self, range: &InformatRange, trimmed: &str) -> bool {
        let from_ok = match &range.from {
            Bound::Low => true,
            Bound::High => false,
            Bound::Char(c) => {
                let c = c.trim_end();
                if range.from_exclusive { trimmed > c } else { trimmed >= c }
            }
            Bound::Num(_) => false,
        };
        if !from_ok {
            return false;
        }
        match &range.to {
            Bound::High => true,
            Bound::Low => false,
            Bound::Char(c) => {
                let c = c.trim_end();
                if range.to_exclusive { trimmed < c } else { trimmed <= c }
            }
            Bound::Num(_) => false,
        }
    }

    /// Convert an `InformatValue` to a `Value`, using the raw input where
    /// `_SAME_` is specified.
    fn resolve_result(&self, iv: &InformatValue, input: &str) -> Value {
        match iv {
            InformatValue::Num(n) => Value::Num(*n),
            InformatValue::Char(s) => Value::Char(s.clone()),
            InformatValue::Same => {
                if self.is_char_result {
                    Value::Char(input.trim_end().to_string())
                } else {
                    // Try to parse as f64; fallback to missing.
                    let t = input.trim();
                    if t.is_empty() || t == "." {
                        Value::missing()
                    } else if let Ok(f) = t.parse::<f64>() {
                        Value::Num(f)
                    } else {
                        Value::missing()
                    }
                }
            }
            InformatValue::Missing(kind) => {
                use crate::value::MissingKind;
                match kind.as_str() {
                    "." | "" => Value::missing(),
                    "_" => Value::Missing(MissingKind::Underscore),
                    s if s.len() == 1 => {
                        let ch = s.chars().next().unwrap().to_ascii_uppercase();
                        if ch.is_ascii_uppercase() {
                            Value::Missing(MissingKind::Letter(ch as u8 - b'A'))
                        } else {
                            Value::missing()
                        }
                    }
                    _ => Value::missing(),
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn num_single(v: f64, label: &str) -> Range {
        Range {
            from: Bound::Num(v),
            to: Bound::Num(v),
            from_exclusive: false,
            to_exclusive: false,
            label: label.to_string(),
        }
    }

    fn num_range(lo: f64, hi: f64, from_excl: bool, to_excl: bool, label: &str) -> Range {
        Range {
            from: Bound::Num(lo),
            to: Bound::Num(hi),
            from_exclusive: from_excl,
            to_exclusive: to_excl,
            label: label.to_string(),
        }
    }

    // --- numeric single-value lookup ---

    #[test]
    fn numeric_single_value_match() {
        let uf = UserFormat {
            is_char: false,
            ranges: vec![num_single(1.0, "Male"), num_single(2.0, "Female")],
            other: None,
        };
        assert_eq!(uf.lookup(&Value::Num(1.0)), Some("Male"));
        assert_eq!(uf.lookup(&Value::Num(2.0)), Some("Female"));
        assert_eq!(uf.lookup(&Value::Num(3.0)), None);
    }

    #[test]
    fn numeric_single_value_list() {
        // Multiple single-value ranges simulate a list like 1,2,3='Group'
        let uf = UserFormat {
            is_char: false,
            ranges: vec![
                num_single(1.0, "Group"),
                num_single(2.0, "Group"),
                num_single(3.0, "Group"),
            ],
            other: Some("Other".to_string()),
        };
        assert_eq!(uf.lookup(&Value::Num(1.0)), Some("Group"));
        assert_eq!(uf.lookup(&Value::Num(2.0)), Some("Group"));
        assert_eq!(uf.lookup(&Value::Num(3.0)), Some("Group"));
        assert_eq!(uf.lookup(&Value::Num(99.0)), Some("Other"));
    }

    #[test]
    fn numeric_inclusive_range() {
        // 1-3 = 'Low'
        let uf = UserFormat {
            is_char: false,
            ranges: vec![num_range(1.0, 3.0, false, false, "Low")],
            other: Some("High".to_string()),
        };
        assert_eq!(uf.lookup(&Value::Num(1.0)), Some("Low"));
        assert_eq!(uf.lookup(&Value::Num(2.5)), Some("Low"));
        assert_eq!(uf.lookup(&Value::Num(3.0)), Some("Low"));
        assert_eq!(uf.lookup(&Value::Num(4.0)), Some("High"));
        assert_eq!(uf.lookup(&Value::Num(0.9)), Some("High"));
    }

    #[test]
    fn numeric_low_to_exclusive_upper() {
        // low -< 5 = 'Below5'
        let uf = UserFormat {
            is_char: false,
            ranges: vec![Range {
                from: Bound::Low,
                to: Bound::Num(5.0),
                from_exclusive: false,
                to_exclusive: true,
                label: "Below5".to_string(),
            }],
            other: Some("AtLeast5".to_string()),
        };
        assert_eq!(uf.lookup(&Value::Num(-100.0)), Some("Below5"));
        assert_eq!(uf.lookup(&Value::Num(4.999)), Some("Below5"));
        assert_eq!(uf.lookup(&Value::Num(5.0)), Some("AtLeast5"));
        assert_eq!(uf.lookup(&Value::Num(6.0)), Some("AtLeast5"));
    }

    #[test]
    fn numeric_exclusive_lower_to_high() {
        // 5 <-high = 'Above5'
        let uf = UserFormat {
            is_char: false,
            ranges: vec![Range {
                from: Bound::Num(5.0),
                to: Bound::High,
                from_exclusive: true,
                to_exclusive: false,
                label: "Above5".to_string(),
            }],
            other: Some("AtMost5".to_string()),
        };
        assert_eq!(uf.lookup(&Value::Num(5.001)), Some("Above5"));
        assert_eq!(uf.lookup(&Value::Num(100.0)), Some("Above5"));
        assert_eq!(uf.lookup(&Value::Num(5.0)), Some("AtMost5"));
        assert_eq!(uf.lookup(&Value::Num(0.0)), Some("AtMost5"));
    }

    #[test]
    fn other_fallback() {
        let uf = UserFormat {
            is_char: false,
            ranges: vec![num_single(1.0, "One")],
            other: Some("Unknown".to_string()),
        };
        assert_eq!(uf.lookup(&Value::Num(99.0)), Some("Unknown"));
    }

    #[test]
    fn missing_value_falls_to_other() {
        let uf = UserFormat {
            is_char: false,
            ranges: vec![num_single(1.0, "One")],
            other: Some("Miss".to_string()),
        };
        assert_eq!(uf.lookup(&Value::missing()), Some("Miss"));
    }

    #[test]
    fn char_single_value_format() {
        let uf = UserFormat {
            is_char: true,
            ranges: vec![
                Range {
                    from: Bound::Char("PAR".to_string()),
                    to: Bound::Char("PAR".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    label: "Paris".to_string(),
                },
                Range {
                    from: Bound::Char("NYC".to_string()),
                    to: Bound::Char("NYC".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    label: "New York".to_string(),
                },
            ],
            other: Some("Unknown City".to_string()),
        };
        assert_eq!(uf.lookup(&Value::Char("PAR".to_string())), Some("Paris"));
        assert_eq!(uf.lookup(&Value::Char("NYC".to_string())), Some("New York"));
        assert_eq!(uf.lookup(&Value::Char("LON".to_string())), Some("Unknown City"));
        // trailing-blank insensitive
        assert_eq!(uf.lookup(&Value::Char("PAR   ".to_string())), Some("Paris"));
    }

    // ── UserInformat tests (M18.2) ────────────────────────────────────────────

    fn invalue_range(from: &str, to: &str, result: InformatValue) -> InformatRange {
        InformatRange {
            from: Bound::Char(from.to_string()),
            to: Bound::Char(to.to_string()),
            from_exclusive: false,
            to_exclusive: false,
            result,
        }
    }

    fn invalue_single(key: &str, result: InformatValue) -> InformatRange {
        invalue_range(key, key, result)
    }

    #[test]
    fn user_informat_numeric_single_values() {
        // invalue grade 'A'=4 'B'=3 'C'=2 'D'=1 'F'=0;
        let ui = UserInformat {
            is_char_result: false,
            ranges: vec![
                invalue_single("A", InformatValue::Num(4.0)),
                invalue_single("B", InformatValue::Num(3.0)),
                invalue_single("C", InformatValue::Num(2.0)),
                invalue_single("D", InformatValue::Num(1.0)),
                invalue_single("F", InformatValue::Num(0.0)),
            ],
            other: None,
        };
        assert_eq!(ui.lookup("A"), Some(Value::Num(4.0)));
        assert_eq!(ui.lookup("B"), Some(Value::Num(3.0)));
        assert_eq!(ui.lookup("F"), Some(Value::Num(0.0)));
        // Unmatched with no other → None.
        assert_eq!(ui.lookup("X"), None);
    }

    #[test]
    fn user_informat_char_result() {
        // invalue $size 'S'='Small' 'M'='Medium' 'L'='Large';
        let ui = UserInformat {
            is_char_result: true,
            ranges: vec![
                invalue_single("S", InformatValue::Char("Small".to_string())),
                invalue_single("M", InformatValue::Char("Medium".to_string())),
                invalue_single("L", InformatValue::Char("Large".to_string())),
            ],
            other: Some(InformatValue::Char("Unknown".to_string())),
        };
        assert_eq!(ui.lookup("S"), Some(Value::Char("Small".to_string())));
        assert_eq!(ui.lookup("M"), Some(Value::Char("Medium".to_string())));
        assert_eq!(ui.lookup("L"), Some(Value::Char("Large".to_string())));
        assert_eq!(ui.lookup("XL"), Some(Value::Char("Unknown".to_string())));
    }

    #[test]
    fn user_informat_trailing_blanks_ignored() {
        // Keys with trailing blanks should still match (trim_end comparison).
        let ui = UserInformat {
            is_char_result: false,
            ranges: vec![invalue_single("A", InformatValue::Num(4.0))],
            other: None,
        };
        assert_eq!(ui.lookup("A   "), Some(Value::Num(4.0)));
    }

    #[test]
    fn user_informat_range_matching() {
        // invalue score 'A'-'C'=1 'D'-'F'=0;
        let ui = UserInformat {
            is_char_result: false,
            ranges: vec![
                invalue_range("A", "C", InformatValue::Num(1.0)),
                invalue_range("D", "F", InformatValue::Num(0.0)),
            ],
            other: Some(InformatValue::Missing(".".to_string())),
        };
        assert_eq!(ui.lookup("A"), Some(Value::Num(1.0)));
        assert_eq!(ui.lookup("B"), Some(Value::Num(1.0)));
        assert_eq!(ui.lookup("C"), Some(Value::Num(1.0)));
        assert_eq!(ui.lookup("D"), Some(Value::Num(0.0)));
        assert_eq!(ui.lookup("F"), Some(Value::Num(0.0)));
        // "G" doesn't match either range — falls to other (missing).
        let result = ui.lookup("G").unwrap();
        assert_eq!(result, Value::missing());
    }

    #[test]
    fn user_informat_exclusive_range() {
        // 'A'-<'C' → matches A, B but NOT C.
        let ui = UserInformat {
            is_char_result: false,
            ranges: vec![InformatRange {
                from: Bound::Char("A".to_string()),
                to: Bound::Char("C".to_string()),
                from_exclusive: false,
                to_exclusive: true,
                result: InformatValue::Num(1.0),
            }],
            other: Some(InformatValue::Num(0.0)),
        };
        assert_eq!(ui.lookup("A"), Some(Value::Num(1.0)));
        assert_eq!(ui.lookup("B"), Some(Value::Num(1.0)));
        assert_eq!(ui.lookup("C"), Some(Value::Num(0.0))); // excluded
    }

    #[test]
    fn user_informat_low_high_bounds() {
        // low-'M'=1 'N'-high=2
        let ui = UserInformat {
            is_char_result: false,
            ranges: vec![
                InformatRange {
                    from: Bound::Low,
                    to: Bound::Char("M".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Num(1.0),
                },
                InformatRange {
                    from: Bound::Char("N".to_string()),
                    to: Bound::High,
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Num(2.0),
                },
            ],
            other: None,
        };
        assert_eq!(ui.lookup("A"), Some(Value::Num(1.0)));
        assert_eq!(ui.lookup("M"), Some(Value::Num(1.0)));
        assert_eq!(ui.lookup("N"), Some(Value::Num(2.0)));
        assert_eq!(ui.lookup("Z"), Some(Value::Num(2.0)));
    }

    #[test]
    fn user_informat_other_fallback() {
        let ui = UserInformat {
            is_char_result: false,
            ranges: vec![invalue_single("A", InformatValue::Num(1.0))],
            other: Some(InformatValue::Num(99.0)),
        };
        assert_eq!(ui.lookup("Z"), Some(Value::Num(99.0)));
    }

    #[test]
    fn user_informat_same_keyword_char_result() {
        // _SAME_ for char informat → copy input verbatim.
        let ui = UserInformat {
            is_char_result: true,
            ranges: vec![InformatRange {
                from: Bound::Low,
                to: Bound::High,
                from_exclusive: false,
                to_exclusive: false,
                result: InformatValue::Same,
            }],
            other: None,
        };
        assert_eq!(ui.lookup("Hello"), Some(Value::Char("Hello".to_string())));
        // Trailing blanks stripped.
        assert_eq!(ui.lookup("Hi  "), Some(Value::Char("Hi".to_string())));
    }

    #[test]
    fn user_informat_same_keyword_num_result() {
        // _SAME_ for numeric informat → parse input as f64.
        let ui = UserInformat {
            is_char_result: false,
            ranges: vec![InformatRange {
                from: Bound::Low,
                to: Bound::High,
                from_exclusive: false,
                to_exclusive: false,
                result: InformatValue::Same,
            }],
            other: None,
        };
        assert_eq!(ui.lookup("42"), Some(Value::Num(42.0)));
        // Non-numeric → missing.
        assert_eq!(ui.lookup("abc"), Some(Value::missing()));
    }

    #[test]
    fn user_informat_missing_result() {
        let ui = UserInformat {
            is_char_result: false,
            ranges: vec![invalue_single(".", InformatValue::Missing(".".to_string()))],
            other: None,
        };
        assert_eq!(ui.lookup("."), Some(Value::missing()));
    }

    #[test]
    fn user_informat_no_match_no_other_returns_none() {
        let ui = UserInformat {
            is_char_result: false,
            ranges: vec![invalue_single("A", InformatValue::Num(1.0))],
            other: None,
        };
        assert_eq!(ui.lookup("Z"), None);
    }
}
