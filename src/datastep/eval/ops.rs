use super::*;

pub(super) fn eval_unary(op: &UnaryOp, expr: &Expr, pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    let v = eval(expr, pdv, ctx);
    if ctx.fatal.is_some() {
        return Value::missing();
    }
    match op {
        UnaryOp::Not => {
            // truthy() : missing et 0 → faux (1.0), sinon vrai (0.0).
            Value::Num(if v.truthy() { 0.0 } else { 1.0 })
        }
        UnaryOp::Plus => match coerce_num(&v, ctx) {
            // Le plus unaire sur un missing propage un missing.
            None => {
                ctx.missing_generated += 1;
                Value::missing()
            }
            Some(f) => Value::Num(f),
        },
        UnaryOp::Minus => match coerce_num(&v, ctx) {
            // Le moins unaire sur un missing propage un missing + note.
            None => {
                ctx.missing_generated += 1;
                Value::missing()
            }
            Some(f) => Value::Num(-f),
        },
    }
}

pub(super) fn eval_binary(op: BinaryOp, left: &Expr, right: &Expr, pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    let l = eval(left, pdv, ctx);
    if ctx.fatal.is_some() {
        return Value::missing();
    }
    // AND/OR utilisent truthy() — pas de court-circuit nécessaire en SAS
    // (pas d'effets de bord ici), mais on évalue tout de même la droite.
    let r = eval(right, pdv, ctx);
    if ctx.fatal.is_some() {
        return Value::missing();
    }

    match op {
        // ── Concaténation ────────────────────────────────────────────────
        BinaryOp::Concat => {
            let ls = concat_operand(&l, ctx);
            let rs = concat_operand(&r, ctx);
            Value::Char(format!("{ls}{rs}"))
        }
        // ── Comparaisons : TOUJOURS via sas_cmp ─────────────────────────
        // Types mixtes num/char : SAS en fait une erreur de compilation ;
        // ici (cf. en-tête) on reste permissif en convertissant le côté
        // char en numérique (note + compteurs via coerce_num).
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge | BinaryOp::Eq | BinaryOp::Ne => {
            let (l, r) = normalize_comparison(l, r, ctx);
            eval_comparison(op, &l, &r)
        }
        // ── Logique ─────────────────────────────────────────────────────
        BinaryOp::And => Value::Num(if l.truthy() && r.truthy() { 1.0 } else { 0.0 }),
        BinaryOp::Or => Value::Num(if l.truthy() || r.truthy() { 1.0 } else { 0.0 }),
        // ── Arithmétique ────────────────────────────────────────────────
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Power => {
            eval_arith(op, &l, &r, ctx)
        }
    }
}

/// Aligne les types d'une comparaison mixte : le côté char est converti en
/// numérique (conversion automatique SAS : note + compteurs). Les paires
/// homogènes passent inchangées.
pub(super) fn normalize_comparison(l: Value, r: Value, ctx: &mut EvalCtx) -> (Value, Value) {
    let to_num = |v: Value, ctx: &mut EvalCtx| match coerce_num(&v, ctx) {
        Some(f) => Value::Num(f),
        None => Value::missing(),
    };
    match (&l, &r) {
        (Value::Char(_), Value::Num(_) | Value::Missing(_)) => {
            let l = to_num(l, ctx);
            (l, r)
        }
        (Value::Num(_) | Value::Missing(_), Value::Char(_)) => {
            let r = to_num(r, ctx);
            (l, r)
        }
        _ => (l, r),
    }
}

/// Égalité fidèle SAS de deux valeurs déjà évaluées (M16.1, pour SELECT
/// sélecteur). Réutilise exactement la sémantique de l'opérateur `=` :
/// alignement des types mixtes via `normalize_comparison` (note de
/// conversion char→num le cas échéant) puis `sas_cmp` (`. = .` est vrai,
/// comparaison char insensible aux blancs finaux).
pub(crate) fn sas_values_equal(l: Value, r: Value, ctx: &mut EvalCtx) -> bool {
    let (l, r) = normalize_comparison(l, r, ctx);
    l.sas_cmp(&r) == std::cmp::Ordering::Equal
}

/// Comparaison fidèle SAS : on traduit l'`Ordering` de `sas_cmp` en
/// booléen numérique 1.0/0.0. Les missings sont comparables (`. = .` vrai,
/// `. < 0` vrai) : c'est `sas_cmp` qui encode cet ordre total.
pub(super) fn eval_comparison(op: BinaryOp, l: &Value, r: &Value) -> Value {
    use std::cmp::Ordering;
    let ord = l.sas_cmp(r);
    let result = match op {
        BinaryOp::Eq => ord == Ordering::Equal,
        BinaryOp::Ne => ord != Ordering::Equal,
        BinaryOp::Lt => ord == Ordering::Less,
        BinaryOp::Le => ord != Ordering::Greater,
        BinaryOp::Gt => ord == Ordering::Greater,
        BinaryOp::Ge => ord != Ordering::Less,
        _ => unreachable!("eval_comparison called with non-comparison op"),
    };
    Value::Num(if result { 1.0 } else { 0.0 })
}

/// Arithmétique fidèle SAS. Un opérande missing (ou char vide/invalide
/// converti à `None`) → `.` + `missing_generated`. Division par zéro → `.`
/// + `division_by_zero` + `error_flag`. `0 ** 0 = 1`. Base négative avec
/// exposant non entier → `.` + `missing_generated` (note SAS).
pub(super) fn eval_arith(op: BinaryOp, l: &Value, r: &Value, ctx: &mut EvalCtx) -> Value {
    let a = coerce_num(l, ctx);
    let b = coerce_num(r, ctx);
    let (a, b) = match (a, b) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            // Au moins un opérande missing → résultat missing + compteur.
            // (Une conversion char invalide a déjà incrémenté invalid_data
            // dans coerce_num ; ici on compte le missing généré par
            // l'opération arithmétique elle-même.)
            ctx.missing_generated += 1;
            return Value::missing();
        }
    };
    match op {
        BinaryOp::Add => Value::Num(a + b),
        BinaryOp::Sub => Value::Num(a - b),
        BinaryOp::Mul => Value::Num(a * b),
        BinaryOp::Div => {
            if b == 0.0 {
                ctx.division_by_zero += 1;
                ctx.error_flag = true;
                Value::missing()
            } else {
                Value::Num(a / b)
            }
        }
        BinaryOp::Power => {
            // 0 ** 0 = 1 (Rust f64::powf concorde déjà).
            if a < 0.0 && b.fract() != 0.0 {
                // Base négative, exposant non entier → racine d'un négatif :
                // missing + note SAS (résultat complexe). On ne lève PAS le
                // flag _ERROR_ : SAS n'émet qu'une NOTE de missing généré.
                ctx.missing_generated += 1;
                Value::missing()
            } else {
                Value::Num(a.powf(b))
            }
        }
        _ => unreachable!("eval_arith called with non-arithmetic op"),
    }
}

/// `IN` : `expr in (a, b, ...)` → 1.0 si une égalité sas_cmp matche.
pub(super) fn eval_in(expr: &Expr, list: &[Expr], pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    use std::cmp::Ordering;
    let target = eval(expr, pdv, ctx);
    if ctx.fatal.is_some() {
        return Value::missing();
    }
    for item in list {
        let v = eval(item, pdv, ctx);
        if ctx.fatal.is_some() {
            return Value::missing();
        }
        if target.sas_cmp(&v) == Ordering::Equal {
            return Value::Num(1.0);
        }
    }
    Value::Num(0.0)
}
