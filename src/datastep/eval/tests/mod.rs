use super::*;
use crate::ast::{BinaryOp, Expr, UnaryOp};
use crate::datastep::pdv::PdvVar;
use crate::value::{MissingKind, Value, VarType};

// ── Helpers ──────────────────────────────────────────────────────────

fn num(n: f64) -> Expr {
    Expr::Num(n)
}

fn str_(s: &str) -> Expr {
    Expr::Str(s.to_string())
}

fn miss() -> Expr {
    Expr::Missing(MissingKind::Dot)
}

fn var(name: &str) -> Expr {
    Expr::Var(name.to_string())
}

fn bin(op: BinaryOp, l: Expr, r: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(l),
        right: Box::new(r),
    }
}

fn unary(op: UnaryOp, e: Expr) -> Expr {
    Expr::Unary {
        op,
        expr: Box::new(e),
    }
}

fn num_var(name: &str) -> PdvVar {
    PdvVar {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        retained: false,
        from_input: false,
        format: None,
        temporary: false,
    }
}

fn char_var(name: &str, length: usize) -> PdvVar {
    PdvVar {
        name: name.to_string(),
        ty: VarType::Char,
        length,
        retained: false,
        from_input: false,
        format: None,
        temporary: false,
    }
}

/// Construit un PDV peuplé pour les tests.
fn pdv_with(vars: Vec<(PdvVar, Value)>) -> Pdv {
    let mut pdv = Pdv::new();
    for (v, val) in vars {
        let slot = pdv.add_var(v);
        pdv.set(slot, val);
    }
    pdv
}

fn ev(e: &Expr, pdv: &Pdv) -> (Value, EvalCtx) {
    let mut ctx = EvalCtx::default();
    let v = eval(e, pdv, &mut ctx);
    (v, ctx)
}

fn ev_bare(e: &Expr) -> (Value, EvalCtx) {
    ev(e, &Pdv::new())
}

// ── M15.7 : couverture complémentaire LAG/LAGn/DIF/DIFn ───────────────

/// Helper : exécute le site `e` une fois par valeur de `x`, et compare la
/// suite des retours à `expected`. Réutilise le MÊME `Expr` et le MÊME
/// `EvalCtx` (= un site lexical à travers la boucle implicite).
fn run_site(e: &Expr, inputs: &[Value], expected: &[Value]) {
    assert_eq!(inputs.len(), expected.len());
    let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
    let slot = pdv.slot("x").unwrap();
    let mut ctx = EvalCtx::default();
    for (i, (inp, exp)) in inputs.iter().zip(expected.iter()).enumerate() {
        pdv.set(slot, inp.clone());
        assert_eq!(&eval(e, &pdv, &mut ctx), exp, "appel #{}", i + 1);
    }
}

fn call(name: &str) -> Expr {
    Expr::Call {
        name: name.to_string(),
        args: vec![var("x")],
    }
}

mod literal;
mod parse;
