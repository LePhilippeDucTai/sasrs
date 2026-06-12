//! Évaluateur d'expressions (tree-walking) sur le PDV.
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : MOYENNE-ÉLEVÉE)
//!
//! ## Règles de coercition / missing (fidélité SAS)
//! - Arithmétique (`+ - * / **`) : si UN opérande est missing → résultat
//!   `.` + incrément `ctx.missing_generated` (PAS d'erreur).
//! - Division par zéro → `.` + note dédiée + `_ERROR_`.
//! - `0 ** 0` = 1 ; `(-2) ** 0.5` → missing + note (SAS).
//! - Comparaisons : via `Value::sas_cmp` → 1.0/0.0. ATTENTION : les
//!   missings SONT comparables (`. = .` vrai, `. < 0` vrai). Comparaison
//!   num/char → en SAS c'est une ERREUR de compilation ; ici : note +
//!   conversion automatique (cf. ci-dessous) pour rester permissif.
//! - `AND`/`OR`/`NOT` : `truthy()` de chaque opérande → 1.0/0.0 (pas de
//!   court-circuit nécessaire, pas d'effets de bord).
//! - `||` : opérandes num convertis en char via BEST12. JUSTIFIÉ À
//!   DROITE sur 12 (oui, avec les espaces de tête — fidèle à SAS) + note
//!   "Numeric values have been converted to character values..." UNE
//!   fois par étape.
//! - Conversion char→num automatique (char utilisé en contexte
//!   numérique) : trim puis parse f64 ; vide/invalide → `.` + note
//!   "Invalid numeric data" + `_ERROR_`. Note "Character values have
//!   been converted to numeric values..." une fois par étape.
//! - `IN` : égalités successives via sas_cmp.
//! - `Call` : déléguer à `functions::call` ; fonction inconnue → erreur
//!   de compilation en SAS ; ici ERROR à la première évaluation.
//!
//! ## EvalCtx
//! Collecte les notes uniques (conversions), les compteurs (missing
//! generated, division par zéro avec n° de ligne plus tard), et le flag
//! `_ERROR_` à reporter au PDV par l'exécuteur.

use super::functions;
use super::pdv::Pdv;
use crate::ast::{BinaryOp, Expr, UnaryOp};
use crate::value::Value;

#[derive(Default)]
pub struct EvalCtx {
    pub missing_generated: u32,
    pub division_by_zero: u32,
    pub note_num_to_char: bool,
    pub note_char_to_num: bool,
    pub invalid_data: u32,
    pub error_flag: bool,
    /// Erreur fatale (fonction inconnue...) — stoppe l'étape.
    pub fatal: Option<String>,
}

/// Coerce une `Value` en f64 pour un CONTEXTE NUMÉRIQUE (arithmétique,
/// comparaison après conversion, etc.). Suit fidèlement SAS :
/// - `Num` → la valeur.
/// - `Missing` → `None` (le missing se propage, sans note ni compteur :
///   c'est l'opération arithmétique englobante qui décide d'incrémenter
///   `missing_generated`).
/// - `Char` → trim puis parse. La NOTE "Character values have been
///   converted to numeric values..." apparaît dès qu'une conversion
///   automatique est TENTÉE (réussie ou non), donc `note_char_to_num`
///   passe à `true` dans tous les cas char. Chaîne vide → `.` +
///   `missing_generated`. Chaîne invalide → `.` + `invalid_data` +
///   `error_flag`.
///
/// Le `bool` renvoyé indique si l'opérande source était missing (Num ou
/// Char vide/invalide tombés à `None`) — utile pour distinguer un missing
/// d'entrée d'une simple absence dans les agrégats. Ici on renvoie juste
/// l'`Option<f64>` ; `None` couvre les deux cas (missing propagé).
fn coerce_num(v: &Value, ctx: &mut EvalCtx) -> Option<f64> {
    match v {
        Value::Num(f) => Some(*f),
        Value::Missing(_) => None,
        Value::Char(s) => {
            // Toute conversion char→num automatique déclenche la NOTE SAS,
            // qu'elle réussisse ou non.
            ctx.note_char_to_num = true;
            let trimmed = s.trim();
            if trimmed.is_empty() {
                ctx.missing_generated += 1;
                None
            } else {
                match trimmed.parse::<f64>() {
                    Ok(f) => Some(f),
                    Err(_) => {
                        ctx.invalid_data += 1;
                        ctx.error_flag = true;
                        None
                    }
                }
            }
        }
    }
}

/// Convertit une `Value` en chaîne pour le CONTEXTE CARACTÈRE de `||`.
/// Un opérande numérique est rendu via BEST12. puis JUSTIFIÉ À DROITE sur
/// 12 colonnes (avec les espaces de tête, fidèle à SAS) et lève la NOTE
/// "Numeric values have been converted to character values...".
fn concat_operand(v: &Value, ctx: &mut EvalCtx) -> String {
    match v {
        Value::Char(s) => s.clone(),
        Value::Num(f) => {
            ctx.note_num_to_char = true;
            format!("{:>12}", crate::value::format_best(*f, 12))
        }
        Value::Missing(_) => {
            // Un missing numérique en contexte caractère devient 12 blancs
            // (BEST12. d'un `.` est un point cadré à droite ; SAS imprime un
            // simple "." cadré à droite). On reste fidèle au cadrage à
            // droite sur 12.
            ctx.note_num_to_char = true;
            format!("{:>12}", ".")
        }
    }
}

pub fn eval(expr: &Expr, pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    match expr {
        Expr::Num(n) => Value::Num(*n),
        Expr::Str(s) => Value::Char(s.clone()),
        Expr::Missing(k) => Value::Missing(*k),
        Expr::Var(name) => eval_var(name, pdv, ctx),
        Expr::Unary { op, expr } => eval_unary(op, expr, pdv, ctx),
        Expr::Binary { op, left, right } => eval_binary(*op, left, right, pdv, ctx),
        Expr::In { expr, list } => eval_in(expr, list, pdv, ctx),
        Expr::Call { name, args } => eval_call(name, args, pdv, ctx),
    }
}

fn eval_var(name: &str, pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    let upper = name.to_uppercase();
    if upper == "_N_" {
        return Value::Num(pdv.n_ as f64);
    }
    if upper == "_ERROR_" {
        return Value::Num(if pdv.error_ { 1.0 } else { 0.0 });
    }
    match pdv.slot(name) {
        Some(slot) => pdv.get(slot).clone(),
        None => {
            // Ne devrait pas arriver : la compilation a déjà créé toutes les
            // variables référencées au PDV. Si cela arrive, c'est fatal.
            ctx.fatal = Some(format!(
                "ERROR: Variable {name} is not on the program data vector."
            ));
            Value::missing()
        }
    }
}

fn eval_unary(op: &UnaryOp, expr: &Expr, pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
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

fn eval_binary(op: BinaryOp, left: &Expr, right: &Expr, pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
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
fn normalize_comparison(l: Value, r: Value, ctx: &mut EvalCtx) -> (Value, Value) {
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

/// Comparaison fidèle SAS : on traduit l'`Ordering` de `sas_cmp` en
/// booléen numérique 1.0/0.0. Les missings sont comparables (`. = .` vrai,
/// `. < 0` vrai) : c'est `sas_cmp` qui encode cet ordre total.
fn eval_comparison(op: BinaryOp, l: &Value, r: &Value) -> Value {
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
fn eval_arith(op: BinaryOp, l: &Value, r: &Value, ctx: &mut EvalCtx) -> Value {
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
fn eval_in(expr: &Expr, list: &[Expr], pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
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

/// `Call` : déléguer à `functions::call`. Fonction inconnue → ERROR fatal.
fn eval_call(name: &str, args: &[Expr], pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    let mut arg_vals = Vec::with_capacity(args.len());
    for a in args {
        let v = eval(a, pdv, ctx);
        if ctx.fatal.is_some() {
            return Value::missing();
        }
        arg_vals.push(v);
    }
    match functions::call(name, &arg_vals, ctx) {
        Some(v) => v,
        None => {
            ctx.fatal = Some(format!(
                "ERROR: Function {} is unknown.",
                name.to_uppercase()
            ));
            Value::missing()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinaryOp, Expr, UnaryOp};
    use crate::value::{MissingKind, VarType, Value};
    use crate::datastep::pdv::PdvVar;

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

    // ── Littéraux ────────────────────────────────────────────────────────

    #[test]
    fn literal_num() {
        let (v, _) = ev_bare(&num(42.0));
        assert_eq!(v, Value::Num(42.0));
    }

    #[test]
    fn literal_str() {
        let (v, _) = ev_bare(&str_("hi"));
        assert_eq!(v, Value::Char("hi".into()));
    }

    #[test]
    fn literal_missing() {
        let (v, _) = ev_bare(&miss());
        assert_eq!(v, Value::missing());
    }

    // ── Comparaisons : sas_cmp PARTOUT ───────────────────────────────────

    #[test]
    fn dot_eq_dot_is_true() {
        // `. = .` → 1.0
        let (v, _) = ev_bare(&bin(BinaryOp::Eq, miss(), miss()));
        assert_eq!(v, Value::Num(1.0));
    }

    #[test]
    fn dot_lt_zero_is_true() {
        // `. < 0` → 1.0 (missing trie sous tous les nombres)
        let (v, _) = ev_bare(&bin(BinaryOp::Lt, miss(), num(0.0)));
        assert_eq!(v, Value::Num(1.0));
    }

    #[test]
    fn comparisons_full_matrix() {
        let cases = [
            (BinaryOp::Eq, 1.0, 1.0, 1.0),
            (BinaryOp::Eq, 1.0, 2.0, 0.0),
            (BinaryOp::Ne, 1.0, 2.0, 1.0),
            (BinaryOp::Lt, 1.0, 2.0, 1.0),
            (BinaryOp::Le, 2.0, 2.0, 1.0),
            (BinaryOp::Gt, 3.0, 2.0, 1.0),
            (BinaryOp::Ge, 2.0, 2.0, 1.0),
            (BinaryOp::Ge, 1.0, 2.0, 0.0),
        ];
        for (op, a, b, expected) in cases {
            let (v, _) = ev_bare(&bin(op, num(a), num(b)));
            assert_eq!(v, Value::Num(expected), "{op:?} {a} {b}");
        }
    }

    #[test]
    fn char_comparison_ignores_trailing_blanks() {
        let (v, _) = ev_bare(&bin(BinaryOp::Eq, str_("abc"), str_("abc   ")));
        assert_eq!(v, Value::Num(1.0));
    }

    // ── Logique ──────────────────────────────────────────────────────────

    #[test]
    fn not_missing_is_true() {
        // `not .` → 1.0 (missing est falsy)
        let (v, _) = ev_bare(&unary(UnaryOp::Not, miss()));
        assert_eq!(v, Value::Num(1.0));
    }

    #[test]
    fn not_zero_is_true() {
        let (v, _) = ev_bare(&unary(UnaryOp::Not, num(0.0)));
        assert_eq!(v, Value::Num(1.0));
    }

    #[test]
    fn not_nonzero_is_false() {
        let (v, _) = ev_bare(&unary(UnaryOp::Not, num(5.0)));
        assert_eq!(v, Value::Num(0.0));
    }

    #[test]
    fn and_or_truth_table() {
        let (v, _) = ev_bare(&bin(BinaryOp::And, num(1.0), num(1.0)));
        assert_eq!(v, Value::Num(1.0));
        let (v, _) = ev_bare(&bin(BinaryOp::And, num(1.0), num(0.0)));
        assert_eq!(v, Value::Num(0.0));
        let (v, _) = ev_bare(&bin(BinaryOp::Or, num(0.0), num(0.0)));
        assert_eq!(v, Value::Num(0.0));
        let (v, _) = ev_bare(&bin(BinaryOp::Or, num(0.0), num(3.0)));
        assert_eq!(v, Value::Num(1.0));
        // missing est falsy.
        let (v, _) = ev_bare(&bin(BinaryOp::And, num(1.0), miss()));
        assert_eq!(v, Value::Num(0.0));
    }

    // ── Arithmétique nominale ────────────────────────────────────────────

    #[test]
    fn arithmetic_nominal() {
        let (v, _) = ev_bare(&bin(BinaryOp::Add, num(2.0), num(3.0)));
        assert_eq!(v, Value::Num(5.0));
        let (v, _) = ev_bare(&bin(BinaryOp::Sub, num(2.0), num(3.0)));
        assert_eq!(v, Value::Num(-1.0));
        let (v, _) = ev_bare(&bin(BinaryOp::Mul, num(2.0), num(3.0)));
        assert_eq!(v, Value::Num(6.0));
        let (v, _) = ev_bare(&bin(BinaryOp::Div, num(6.0), num(3.0)));
        assert_eq!(v, Value::Num(2.0));
    }

    #[test]
    fn power_nominal() {
        // 2 ** 3 = 8
        let (v, _) = ev_bare(&bin(BinaryOp::Power, num(2.0), num(3.0)));
        assert_eq!(v, Value::Num(8.0));
    }

    #[test]
    fn power_zero_zero_is_one() {
        let (v, _) = ev_bare(&bin(BinaryOp::Power, num(0.0), num(0.0)));
        assert_eq!(v, Value::Num(1.0));
    }

    #[test]
    fn power_negative_base_fractional_exponent_is_missing() {
        // (-2) ** 0.5 → missing + missing_generated, PAS d'error_flag.
        let (v, ctx) = ev_bare(&bin(BinaryOp::Power, num(-2.0), num(0.5)));
        assert_eq!(v, Value::missing());
        assert_eq!(ctx.missing_generated, 1);
        assert!(!ctx.error_flag);
    }

    #[test]
    fn unary_minus_on_power_ast() {
        // AST déjà -(2 ** 2) : -(4) = -4 (le parser produit ce nœud).
        let e = unary(UnaryOp::Minus, bin(BinaryOp::Power, num(2.0), num(2.0)));
        let (v, _) = ev_bare(&e);
        assert_eq!(v, Value::Num(-4.0));
    }

    #[test]
    fn unary_plus_nominal() {
        let (v, _) = ev_bare(&unary(UnaryOp::Plus, num(5.0)));
        assert_eq!(v, Value::Num(5.0));
    }

    // ── Arithmétique avec missing ───────────────────────────────────────

    #[test]
    fn missing_plus_one_is_missing_and_counts() {
        // `. + 1` → missing + missing_generated, pas d'erreur.
        let (v, ctx) = ev_bare(&bin(BinaryOp::Add, miss(), num(1.0)));
        assert_eq!(v, Value::missing());
        assert_eq!(ctx.missing_generated, 1);
        assert!(!ctx.error_flag);
    }

    #[test]
    fn one_minus_missing_is_missing_and_counts() {
        let (v, ctx) = ev_bare(&bin(BinaryOp::Sub, num(1.0), miss()));
        assert_eq!(v, Value::missing());
        assert_eq!(ctx.missing_generated, 1);
    }

    #[test]
    fn unary_minus_on_missing_generates_missing() {
        let (v, ctx) = ev_bare(&unary(UnaryOp::Minus, miss()));
        assert_eq!(v, Value::missing());
        assert_eq!(ctx.missing_generated, 1);
    }

    #[test]
    fn unary_plus_on_missing_generates_missing() {
        let (v, ctx) = ev_bare(&unary(UnaryOp::Plus, miss()));
        assert_eq!(v, Value::missing());
        assert_eq!(ctx.missing_generated, 1);
    }

    // ── Division par zéro ───────────────────────────────────────────────

    #[test]
    fn division_by_zero_is_missing_with_counter_and_error() {
        // `1 / 0` → missing + division_by_zero + error_flag.
        let (v, ctx) = ev_bare(&bin(BinaryOp::Div, num(1.0), num(0.0)));
        assert_eq!(v, Value::missing());
        assert_eq!(ctx.division_by_zero, 1);
        assert!(ctx.error_flag);
        // Une division par zéro n'est pas comptée comme missing arithmétique.
        assert_eq!(ctx.missing_generated, 0);
    }

    // ── Concaténation || ────────────────────────────────────────────────

    #[test]
    fn concat_char_char() {
        let (v, ctx) = ev_bare(&bin(BinaryOp::Concat, str_("ab"), str_("cd")));
        assert_eq!(v, Value::Char("abcd".into()));
        assert!(!ctx.note_num_to_char);
    }

    #[test]
    fn concat_num_num_best12_right_justified() {
        // 2 || 3 : chaque opérande BEST12 justifié droite 12 →
        // "           2" + "           3" concaténés.
        let (v, ctx) = ev_bare(&bin(BinaryOp::Concat, num(2.0), num(3.0)));
        let expected = format!("{:>12}{:>12}", "2", "3");
        assert_eq!(v, Value::Char(expected.clone()));
        assert_eq!(expected, "           2           3");
        assert!(ctx.note_num_to_char);
    }

    #[test]
    fn concat_mixed_num_char() {
        // num justifié droite 12, char tel quel.
        let (v, ctx) = ev_bare(&bin(BinaryOp::Concat, num(5.0), str_("x")));
        assert_eq!(v, Value::Char(format!("{:>12}x", "5")));
        assert!(ctx.note_num_to_char);
    }

    // ── Char en contexte numérique ──────────────────────────────────────

    #[test]
    fn char_numeric_string_in_arith_converts_with_note() {
        // '12 ' en contexte num → 12 + note_char_to_num.
        let (v, ctx) = ev_bare(&bin(BinaryOp::Add, str_("12 "), num(0.0)));
        assert_eq!(v, Value::Num(12.0));
        assert!(ctx.note_char_to_num);
        assert_eq!(ctx.invalid_data, 0);
        assert!(!ctx.error_flag);
    }

    #[test]
    fn char_invalid_in_arith_is_missing_with_invalid_and_error() {
        // 'abc' → missing + invalid_data + error_flag (+ note tentée).
        let (v, ctx) = ev_bare(&bin(BinaryOp::Add, str_("abc"), num(1.0)));
        assert_eq!(v, Value::missing());
        assert!(ctx.note_char_to_num);
        assert_eq!(ctx.invalid_data, 1);
        assert!(ctx.error_flag);
        // Le missing arithmétique généré par l'opération est aussi compté.
        assert_eq!(ctx.missing_generated, 1);
    }

    #[test]
    fn char_empty_in_arith_is_missing_generated() {
        // chaîne vide en contexte num → missing + missing_generated (note tentée).
        let (v, ctx) = ev_bare(&bin(BinaryOp::Mul, str_("   "), num(2.0)));
        assert_eq!(v, Value::missing());
        assert!(ctx.note_char_to_num);
        // coerce_num incrémente missing_generated pour la chaîne vide, puis
        // l'opération arithmétique l'incrémente une seconde fois.
        assert_eq!(ctx.missing_generated, 2);
        assert_eq!(ctx.invalid_data, 0);
        assert!(!ctx.error_flag);
    }

    // ── Variables / automatiques ────────────────────────────────────────

    #[test]
    fn var_lookup_num() {
        let pdv = pdv_with(vec![(num_var("Age"), Value::Num(14.0))]);
        let (v, _) = ev(&var("age"), &pdv);
        assert_eq!(v, Value::Num(14.0));
    }

    #[test]
    fn var_lookup_char() {
        let pdv = pdv_with(vec![(char_var("Name", 10), Value::Char("Alice".into()))]);
        let (v, _) = ev(&var("NAME"), &pdv);
        assert_eq!(v, Value::Char("Alice".into()));
    }

    #[test]
    fn var_in_arithmetic() {
        let pdv = pdv_with(vec![(num_var("x"), Value::Num(10.0))]);
        let (v, _) = ev(&bin(BinaryOp::Add, var("x"), num(5.0)), &pdv);
        assert_eq!(v, Value::Num(15.0));
    }

    #[test]
    fn automatic_n_variable() {
        let mut pdv = Pdv::new();
        pdv.n_ = 7;
        let (v, _) = ev(&var("_N_"), &pdv);
        assert_eq!(v, Value::Num(7.0));
        let (v, _) = ev(&var("_n_"), &pdv);
        assert_eq!(v, Value::Num(7.0));
    }

    #[test]
    fn automatic_error_variable() {
        let mut pdv = Pdv::new();
        let (v, _) = ev(&var("_ERROR_"), &pdv);
        assert_eq!(v, Value::Num(0.0));
        pdv.error_ = true;
        let (v, _) = ev(&var("_error_"), &pdv);
        assert_eq!(v, Value::Num(1.0));
    }

    #[test]
    fn unknown_variable_is_fatal() {
        let pdv = Pdv::new();
        let (v, ctx) = ev(&var("nosuch"), &pdv);
        assert_eq!(v, Value::missing());
        assert!(ctx.fatal.is_some());
        assert!(ctx.fatal.unwrap().contains("program data vector"));
    }

    // ── IN ───────────────────────────────────────────────────────────────

    #[test]
    fn in_match() {
        let e = Expr::In {
            expr: Box::new(num(2.0)),
            list: vec![num(1.0), num(2.0), num(3.0)],
        };
        let (v, _) = ev_bare(&e);
        assert_eq!(v, Value::Num(1.0));
    }

    #[test]
    fn in_no_match() {
        let e = Expr::In {
            expr: Box::new(num(9.0)),
            list: vec![num(1.0), num(2.0)],
        };
        let (v, _) = ev_bare(&e);
        assert_eq!(v, Value::Num(0.0));
    }

    #[test]
    fn in_missing_matches_missing() {
        // `. in (.)` → 1.0 (sas_cmp : `. = .` vrai)
        let e = Expr::In {
            expr: Box::new(miss()),
            list: vec![num(1.0), miss()],
        };
        let (v, _) = ev_bare(&e);
        assert_eq!(v, Value::Num(1.0));
    }

    #[test]
    fn in_char() {
        let e = Expr::In {
            expr: Box::new(str_("b")),
            list: vec![str_("a"), str_("b")],
        };
        let (v, _) = ev_bare(&e);
        assert_eq!(v, Value::Num(1.0));
    }

    // ── Call ─────────────────────────────────────────────────────────────

    #[test]
    fn call_known_function() {
        let e = Expr::Call {
            name: "sum".to_string(),
            args: vec![num(1.0), num(2.0), num(3.0)],
        };
        let (v, ctx) = ev_bare(&e);
        assert_eq!(v, Value::Num(6.0));
        assert!(ctx.fatal.is_none());
    }

    #[test]
    fn call_function_propagates_args_from_pdv() {
        let pdv = pdv_with(vec![
            (num_var("a"), Value::Num(10.0)),
            (num_var("b"), Value::Num(20.0)),
        ]);
        let e = Expr::Call {
            name: "MEAN".to_string(),
            args: vec![var("a"), var("b")],
        };
        let (v, _) = ev(&e, &pdv);
        assert_eq!(v, Value::Num(15.0));
    }

    #[test]
    fn call_unknown_function_is_fatal() {
        let e = Expr::Call {
            name: "NOSUCHFN".to_string(),
            args: vec![],
        };
        let (v, ctx) = ev_bare(&e);
        assert_eq!(v, Value::missing());
        assert!(ctx.fatal.is_some());
        let msg = ctx.fatal.unwrap();
        assert!(msg.contains("ERROR"));
        assert!(msg.contains("NOSUCHFN"));
    }

    #[test]
    fn fatal_short_circuits_outer_expression() {
        // Une fonction inconnue dans un sous-arbre stoppe l'évaluation.
        let inner = Expr::Call {
            name: "BOGUS".to_string(),
            args: vec![],
        };
        let e = bin(BinaryOp::Add, inner, num(1.0));
        let (v, ctx) = ev_bare(&e);
        assert_eq!(v, Value::missing());
        assert!(ctx.fatal.is_some());
    }

    // ── Composition / précédence (telle qu'encodée par l'AST) ───────────

    #[test]
    fn nested_expression() {
        // (x + 1) * 2 avec x = 4 → 10
        let pdv = pdv_with(vec![(num_var("x"), Value::Num(4.0))]);
        let e = bin(
            BinaryOp::Mul,
            bin(BinaryOp::Add, var("x"), num(1.0)),
            num(2.0),
        );
        let (v, _) = ev(&e, &pdv);
        assert_eq!(v, Value::Num(10.0));
    }

    #[test]
    fn mixed_comparison_converts_char_side() {
        let pdv = Pdv::new();
        // '12' = 12 → conversion auto char→num → vrai, avec note.
        let e = bin(BinaryOp::Eq, str_("12"), num(12.0));
        let (v, ctx) = ev(&e, &pdv);
        assert_eq!(v, Value::Num(1.0));
        assert!(ctx.note_char_to_num);
        // ' ' < 0 → char blanc converti en missing → . < 0 vrai.
        let e = bin(BinaryOp::Lt, str_(" "), num(0.0));
        let (v, _) = ev(&e, &pdv);
        assert_eq!(v, Value::Num(1.0));
        // 'abc' = 5 → conversion invalide → missing ≠ 5 → faux + flags.
        let e = bin(BinaryOp::Eq, str_("abc"), num(5.0));
        let (v, ctx) = ev(&e, &pdv);
        assert_eq!(v, Value::Num(0.0));
        assert!(ctx.invalid_data > 0);
        assert!(ctx.error_flag);
    }
}
