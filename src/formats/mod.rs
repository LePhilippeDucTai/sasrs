//! Moteur de formats/informats SAS (jalon M4).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! - `FormatSpec::parse("DATE9.")` → { name:"DATE", w:Some(9), d:None } ;
//!   `"8.2"` → { name:"", w:8, d:2 } ; `"$CHAR10."` → { name:"$CHAR",
//!   w:10 }. Un format se reconnaît au `.` final dans la source — le
//!   parser fournit la chaîne déjà assemblée.
//! - `FormatCatalog` : formats utilisateur (PROC FORMAT) par nom upcase,
//!   en session uniquement (pas de catalogue persistant — limitation
//!   documentée).
//! - `format(value, spec)` : ordre de résolution — format utilisateur,
//!   sinon builtin, sinon fallback BESTw. / $w. Missings spéciaux →
//!   `A`..`Z`/`_`, `.` → `.` (à respecter dans TOUS les formats
//!   numériques).
//! - `informat(s, spec)` : symétrique pour INPUT().

#![allow(unused_variables, dead_code)]

pub mod builtin;
pub mod userdef;

use crate::value::{format_best, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct FormatSpec {
    /// Nom sans largeur ("DATE", "$CHAR", "" pour w.d), upcase.
    pub name: String,
    pub w: Option<u16>,
    pub d: Option<u16>,
}

impl FormatSpec {
    /// Parse a SAS format token. Handles forms like:
    ///   "DATE9."  -> name="DATE", w=9, d=None
    ///   "DATE9"   -> name="DATE", w=9, d=None  (trailing dot optional)
    ///   "8.2"     -> name="",     w=8, d=2
    ///   "8."      -> name="",     w=8, d=None
    ///   "$CHAR10."-> name="$CHAR",w=10,d=None
    ///   "$10."    -> name="$",    w=10,d=None
    ///   "COMMA12.2"->name="COMMA",w=12,d=2
    ///   "BEST12." -> name="BEST", w=12,d=None
    pub fn parse(s: &str) -> Option<FormatSpec> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }

        // Strip trailing dot(s) — but be careful: "8.2" has a dot in the
        // middle, so we only strip a trailing dot AFTER we know there's no
        // decimal part following it.
        // Strategy: work character by character.
        let chars: Vec<char> = s.chars().collect();
        let mut pos = 0;

        // 1. Collect the name: leading '$' is allowed, then alphabetic chars.
        if pos < chars.len() && chars[pos] == '$' {
            pos += 1;
        }
        while pos < chars.len() && chars[pos].is_ascii_alphabetic() {
            pos += 1;
        }
        let name: String = s[..pos].to_uppercase();

        // 2. Collect optional width digits.
        let w_start = pos;
        while pos < chars.len() && chars[pos].is_ascii_digit() {
            pos += 1;
        }
        let w: Option<u16> = if pos > w_start {
            s[w_start..pos].parse().ok()
        } else {
            None
        };

        // 3. Optional '.' then optional decimal digits.
        let d: Option<u16> = if pos < chars.len() && chars[pos] == '.' {
            pos += 1; // consume the dot
            let d_start = pos;
            while pos < chars.len() && chars[pos].is_ascii_digit() {
                pos += 1;
            }
            if pos > d_start {
                s[d_start..pos].parse().ok()
            } else {
                None
            }
        } else {
            None
        };

        // Ignore any remaining trailing characters (e.g. stray dot).

        // Must have at least a name or a width.
        if name.is_empty() && w.is_none() {
            return None;
        }

        Some(FormatSpec { name, w, d })
    }
}

#[derive(Default)]
pub struct FormatCatalog {
    user: HashMap<String, userdef::UserFormat>,
}

impl FormatCatalog {
    /// Register a user-defined format (from PROC FORMAT) keyed by upcased name.
    pub fn define(&mut self, name: &str, fmt: userdef::UserFormat) {
        self.user.insert(name.to_uppercase(), fmt);
    }

    /// PUT: value → formatted string (SAS-justified, width spec.w).
    ///
    /// Resolution order:
    ///   1. User format (spec.name upcased) — consults the HashMap
    ///   2. builtin::format_builtin
    ///   3. Fallback: Char → left-justified/truncated to w;
    ///                Missing → right-justified missing char to w;
    ///                Num → format_best right-justified to w (default 12).
    pub fn format(&self, v: &Value, spec: &FormatSpec) -> String {
        // Intercept numeric missings first — applies before ANY numeric format.
        if let Value::Missing(k) = v {
            let ch = k.display();
            let w = spec.w.unwrap_or(1) as usize;
            return right_justify(&ch, w);
        }

        // 1. Try user format.
        let uname = spec.name.to_uppercase();
        if let Some(uf) = self.user.get(&uname) {
            if let Some(label) = uf.lookup(v) {
                let s = label.to_string();
                return match spec.w {
                    Some(w) => right_justify(&s, w as usize),
                    None => s,
                };
            }
        }

        // 2. Try builtin.
        if let Some(s) = builtin::format_builtin(v, spec) {
            return s;
        }

        // 3. Fallback.
        match v {
            Value::Char(s) => {
                match spec.w {
                    None => s.clone(),
                    Some(w) => {
                        let w = w as usize;
                        // Left-justify: truncate or pad with spaces.
                        let mut out = s.clone();
                        out.truncate(w);
                        while out.len() < w {
                            out.push(' ');
                        }
                        out
                    }
                }
            }
            Value::Num(n) => {
                let w = spec.w.unwrap_or(12) as usize;
                let s = format_best(*n, w);
                right_justify(&s, w)
            }
            Value::Missing(_) => unreachable!("handled above"),
        }
    }

    /// INPUT: string → value.
    ///
    /// Resolution order:
    ///   1. builtin::informat_builtin
    ///   2. Fallback: trim; empty/"." → missing; parse as f64 → Num; else Char.
    pub fn informat(&self, s: &str, spec: &FormatSpec) -> Value {
        // 1. Try builtin.
        if let Some(v) = builtin::informat_builtin(s, spec) {
            return v;
        }

        // 2. Fallback.
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed == "." {
            return Value::missing();
        }
        if let Ok(f) = trimmed.parse::<f64>() {
            return Value::Num(f);
        }
        Value::Char(trimmed.to_string())
    }
}

/// Right-justify `s` in a field of width `w`, truncating if longer.
pub(crate) fn right_justify(s: &str, w: usize) -> String {
    if w == 0 {
        return String::new();
    }
    if s.len() >= w {
        // Truncate from the right (keep rightmost w chars, SAS overflow rule).
        // Actually SAS fills with * on overflow; but for missing/name we truncate.
        s[s.len().saturating_sub(w)..].to_string()
    } else {
        format!("{:>width$}", s, width = w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::MissingKind;

    // -------------------------------------------------------------------------
    // FormatSpec::parse
    // -------------------------------------------------------------------------

    #[test]
    fn parse_date9_dot() {
        let spec = FormatSpec::parse("DATE9.").unwrap();
        assert_eq!(spec.name, "DATE");
        assert_eq!(spec.w, Some(9));
        assert_eq!(spec.d, None);
    }

    #[test]
    fn parse_date9_no_dot() {
        let spec = FormatSpec::parse("DATE9").unwrap();
        assert_eq!(spec.name, "DATE");
        assert_eq!(spec.w, Some(9));
        assert_eq!(spec.d, None);
    }

    #[test]
    fn parse_wd_8_2() {
        let spec = FormatSpec::parse("8.2").unwrap();
        assert_eq!(spec.name, "");
        assert_eq!(spec.w, Some(8));
        assert_eq!(spec.d, Some(2));
    }

    #[test]
    fn parse_wd_8_dot() {
        let spec = FormatSpec::parse("8.").unwrap();
        assert_eq!(spec.name, "");
        assert_eq!(spec.w, Some(8));
        assert_eq!(spec.d, None);
    }

    #[test]
    fn parse_dollar_char10() {
        let spec = FormatSpec::parse("$CHAR10.").unwrap();
        assert_eq!(spec.name, "$CHAR");
        assert_eq!(spec.w, Some(10));
        assert_eq!(spec.d, None);
    }

    #[test]
    fn parse_dollar_10() {
        let spec = FormatSpec::parse("$10.").unwrap();
        assert_eq!(spec.name, "$");
        assert_eq!(spec.w, Some(10));
        assert_eq!(spec.d, None);
    }

    #[test]
    fn parse_comma12_2() {
        let spec = FormatSpec::parse("COMMA12.2").unwrap();
        assert_eq!(spec.name, "COMMA");
        assert_eq!(spec.w, Some(12));
        assert_eq!(spec.d, Some(2));
    }

    #[test]
    fn parse_best12() {
        let spec = FormatSpec::parse("BEST12.").unwrap();
        assert_eq!(spec.name, "BEST");
        assert_eq!(spec.w, Some(12));
        assert_eq!(spec.d, None);
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(FormatSpec::parse("").is_none());
    }

    #[test]
    fn parse_lowercase_is_upcased() {
        let spec = FormatSpec::parse("date9.").unwrap();
        assert_eq!(spec.name, "DATE");
    }

    #[test]
    fn parse_name_only_no_width() {
        let spec = FormatSpec::parse("BEST.").unwrap();
        assert_eq!(spec.name, "BEST");
        assert_eq!(spec.w, None);
        assert_eq!(spec.d, None);
    }

    // -------------------------------------------------------------------------
    // FormatCatalog::format — fallback paths
    // -------------------------------------------------------------------------

    fn catalog() -> FormatCatalog {
        FormatCatalog::default()
    }

    #[test]
    fn format_missing_dot_right_justified() {
        let cat = catalog();
        let spec = FormatSpec { name: "".into(), w: Some(5), d: None };
        let result = cat.format(&Value::missing(), &spec);
        assert_eq!(result, "    .");
    }

    #[test]
    fn format_missing_letter_a() {
        let cat = catalog();
        let spec = FormatSpec { name: "".into(), w: Some(3), d: None };
        let result = cat.format(&Value::Missing(MissingKind::Letter(0)), &spec);
        assert_eq!(result, "  A");
    }

    #[test]
    fn format_missing_underscore() {
        let cat = catalog();
        let spec = FormatSpec { name: "".into(), w: Some(3), d: None };
        let result = cat.format(&Value::Missing(MissingKind::Underscore), &spec);
        assert_eq!(result, "  _");
    }

    #[test]
    fn format_char_left_justified_padded() {
        let cat = catalog();
        let spec = FormatSpec { name: "$".into(), w: Some(8), d: None };
        let result = cat.format(&Value::Char("abc".into()), &spec);
        // fallback: not a known builtin name for $, but builtin handles $ too
        // Let's test the exact fallback only if builtin doesn't claim it.
        // Actually builtin handles "$", so test truncation via builtin.
        assert_eq!(result.len(), 8);
        assert!(result.starts_with("abc"));
    }

    #[test]
    fn format_num_fallback_best12() {
        let cat = catalog();
        let spec = FormatSpec { name: "UNKNOWNFORMAT".into(), w: Some(12), d: None };
        let result = cat.format(&Value::Num(42.0), &spec);
        assert_eq!(result, "          42");
    }

    // -------------------------------------------------------------------------
    // FormatCatalog::informat — fallback paths
    // -------------------------------------------------------------------------

    #[test]
    fn informat_empty_gives_missing() {
        let cat = catalog();
        let spec = FormatSpec { name: "UNKNOWNFORMAT".into(), w: None, d: None };
        let result = cat.informat("  ", &spec);
        assert_eq!(result, Value::missing());
    }

    #[test]
    fn informat_dot_gives_missing() {
        let cat = catalog();
        let spec = FormatSpec { name: "UNKNOWNFORMAT".into(), w: None, d: None };
        let result = cat.informat(".", &spec);
        assert_eq!(result, Value::missing());
    }

    #[test]
    fn informat_numeric_string_gives_num() {
        let cat = catalog();
        let spec = FormatSpec { name: "UNKNOWNFORMAT".into(), w: None, d: None };
        let result = cat.informat("3.14", &spec);
        assert_eq!(result, Value::Num(3.14));
    }

    #[test]
    fn informat_text_gives_char() {
        let cat = catalog();
        let spec = FormatSpec { name: "UNKNOWNFORMAT".into(), w: None, d: None };
        let result = cat.informat("hello", &spec);
        assert_eq!(result, Value::Char("hello".into()));
    }
}
