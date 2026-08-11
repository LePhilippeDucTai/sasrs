// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

use super::*;
use crate::value::{MissingKind, Value};

fn ctx() -> EvalCtx {
    EvalCtx::default()
}

fn num(f: f64) -> Value {
    Value::Num(f)
}

fn miss() -> Value {
    Value::missing()
}

fn miss_a() -> Value {
    Value::Missing(MissingKind::Letter(0))
}

fn chr(s: &str) -> Value {
    Value::Char(s.to_string())
}

fn invoke(name: &str, args: &[Value]) -> Value {
    let mut c = ctx();
    call(name, args, &mut c).expect("function should be known")
}

fn invoke_ctx(name: &str, args: &[Value], c: &mut EvalCtx) -> Value {
    call(name, args, c).expect("function should be known")
}

// ── INPUT / PUT with user-defined formats & informats (M18.2) ────────────

fn make_ctx_with_grade_informat() -> EvalCtx {
    use crate::formats::userdef::{Bound, InformatRange, InformatValue, UserInformat};
    let mut cat = crate::formats::FormatCatalog::default();
    cat.define_informat(
        "GRADE",
        UserInformat {
            is_char_result: false,
            ranges: vec![
                InformatRange {
                    from: Bound::Char("A".to_string()),
                    to: Bound::Char("A".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Num(4.0),
                },
                InformatRange {
                    from: Bound::Char("B".to_string()),
                    to: Bound::Char("B".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Num(3.0),
                },
                InformatRange {
                    from: Bound::Char("F".to_string()),
                    to: Bound::Char("F".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Num(0.0),
                },
            ],
            other: Some(InformatValue::Missing(".".to_string())),
        },
    );
    EvalCtx {
        format_catalog: std::rc::Rc::new(cat),
        ..EvalCtx::default()
    }
}

fn make_ctx_with_size_char_informat() -> EvalCtx {
    use crate::formats::userdef::{Bound, InformatRange, InformatValue, UserInformat};
    let mut cat = crate::formats::FormatCatalog::default();
    cat.define_informat(
        "$SIZE",
        UserInformat {
            is_char_result: true,
            ranges: vec![
                InformatRange {
                    from: Bound::Char("S".to_string()),
                    to: Bound::Char("S".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Char("Small".to_string()),
                },
                InformatRange {
                    from: Bound::Char("L".to_string()),
                    to: Bound::Char("L".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Char("Large".to_string()),
                },
            ],
            other: Some(InformatValue::Char("Unknown".to_string())),
        },
    );
    EvalCtx {
        format_catalog: std::rc::Rc::new(cat),
        ..EvalCtx::default()
    }
}

// ── INTCK ─────────────────────────────────────────────────────────────────

fn sas_day(y: i64, m: i64, d: i64) -> f64 {
    days_since_1960(y, m, d) as f64
}

// ── Probability distribution functions (M15.4) ─────────────────────────────

/// Numeric value of a function result, panicking if missing.
fn val(v: &Value) -> f64 {
    coerce_num(v, &mut ctx()).expect("expected numeric result")
}

fn approx(name: &str, args: &[Value], expected: f64, tol: f64) {
    let got = val(&invoke(name, args));
    assert!(
        (got - expected).abs() < tol,
        "{name}: got {got}, expected {expected} (tol {tol})"
    );
}

// ── M15.5 : Random variate generation ────────────────────────────────────

// Helper: extract f64 from a Value::Num, panic otherwise.
fn num_val(v: Value) -> f64 {
    match v {
        Value::Num(f) => f,
        other => panic!("expected Num, got {other:?}"),
    }
}

mod intnx;
mod probbnml;
mod prx;
mod sinh;
mod substr;
mod unknown;
mod whichc;
