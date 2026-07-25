use super::*;

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
    pub(super) fn range_matches(&self, range: &InformatRange, trimmed: &str) -> bool {
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
    pub(super) fn resolve_result(&self, iv: &InformatValue, input: &str) -> Value {
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
