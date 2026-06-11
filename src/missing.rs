//! Mapping between SAS missing values and the Polars/parquet representation.
//!
//! Ordinary missing `.` is a Polars `null`. Special missings `._`, `.A`..`.Z`
//! are encoded as quiet NaNs with a payload (1 = `._`, 2..=27 = `.A`..`.Z`)
//! in the low mantissa bits; parquet stores doubles bit-exactly, so these
//! round-trip through files. Any NaN with an unrecognized payload decodes
//! as ordinary missing.
//!
//! IMPORTANT: before a column is handed to native Polars compute
//! (aggregations, joins, SQL), special-missing NaNs must be converted to
//! nulls — Polars treats NaN as a regular float value. That conversion
//! lives here (`nullify_specials`) so it cannot be reimplemented ad hoc.

use crate::value::{MissingKind, Value};
use polars::prelude::*;

const QNAN_BASE: u64 = 0x7FF8_0000_0000_0000;
const PAYLOAD_MASK: u64 = 0x0000_0000_0000_00FF;

pub fn encode_special(kind: MissingKind) -> f64 {
    let payload: u64 = match kind {
        MissingKind::Underscore => 1,
        // `.` is represented as null, never as NaN, but keep a total mapping.
        MissingKind::Dot => 0,
        MissingKind::Letter(i) => 2 + i as u64,
    };
    f64::from_bits(QNAN_BASE | payload)
}

pub fn decode_nan(v: f64) -> MissingKind {
    if !v.is_nan() {
        return MissingKind::Dot;
    }
    match v.to_bits() & PAYLOAD_MASK {
        1 => MissingKind::Underscore,
        p @ 2..=27 => MissingKind::Letter((p - 2) as u8),
        _ => MissingKind::Dot,
    }
}

/// Numeric cell from Polars to a SAS value.
pub fn num_to_value(v: Option<f64>) -> Value {
    match v {
        None => Value::missing(),
        Some(f) if f.is_nan() => Value::Missing(decode_nan(f)),
        Some(f) => Value::Num(f),
    }
}

/// SAS numeric value to a Polars cell (None = null = `.`).
pub fn value_to_num(v: &Value) -> Option<f64> {
    match v {
        Value::Num(f) => Some(*f),
        Value::Missing(MissingKind::Dot) => None,
        Value::Missing(k) => Some(encode_special(*k)),
        Value::Char(_) => None,
    }
}

/// Replace special-missing NaNs by nulls so Polars compute treats them as
/// missing. Apply to every Float64 column before native aggregation/joins.
pub fn nullify_specials(df: &DataFrame) -> PolarsResult<DataFrame> {
    let mut out = df.clone();
    let names: Vec<String> = df
        .get_columns()
        .iter()
        .filter(|c| c.dtype() == &DataType::Float64)
        .map(|c| c.name().to_string())
        .collect();
    for name in names {
        let s = out.column(&name)?.f64()?;
        let cleaned: Float64Chunked = s
            .iter()
            .map(|opt| opt.filter(|v| !v.is_nan()))
            .collect();
        out.replace(&name, cleaned.into_series())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_missing_roundtrip() {
        for kind in [
            MissingKind::Underscore,
            MissingKind::Letter(0),
            MissingKind::Letter(25),
        ] {
            let f = encode_special(kind);
            assert!(f.is_nan());
            assert_eq!(decode_nan(f), kind);
        }
    }

    #[test]
    fn plain_nan_decodes_as_dot() {
        assert_eq!(decode_nan(f64::NAN), MissingKind::Dot);
    }
}
