//! Formats utilisateur définis par PROC FORMAT (jalon M4).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc format; value sexfmt 1='Male' 2='Female' other='?'; run;`
//! produit un `UserFormat` à plages :
//! - plage = valeur unique, `low-<high` / `low<-high` (bornes exclusives
//!   côté `<`), `LOW`/`HIGH` symboliques, ou liste `1,2,3='x'`.
//! - `VALUE` (num→label), `VALUE $fmt` (char→label) ; `INVALUE` pour les
//!   informats utilisateur (M4+).
//! - Résolution : première plage qui matche dans l'ordre de tri SAS des
//!   bornes ; sinon `other` ; sinon la valeur formatée en BEST./$.

#![allow(unused_variables, dead_code)]

use crate::value::Value;

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
}
