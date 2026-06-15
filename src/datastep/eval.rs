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
use std::collections::HashMap;

pub struct EvalCtx {
    pub missing_generated: u32,
    pub division_by_zero: u32,
    pub note_num_to_char: bool,
    pub note_char_to_num: bool,
    pub invalid_data: u32,
    pub error_flag: bool,
    /// Erreur fatale (fonction inconnue, indice d'array hors bornes...) —
    /// stoppe l'étape.
    pub fatal: Option<String>,
    /// Arrays de l'étape : nom UPPERCASE → définition (slots + dimensions)
    /// (copié depuis `StepProgram.arrays` par l'exécuteur).
    pub arrays: HashMap<String, super::ArrayDef>,
    /// Flags de groupe BY `(nom UPPERCASE, first, last)`, dans l'ordre du
    /// BY — mis à jour par le Runner à chaque observation servie par
    /// l'interclassement. Servent les variables automatiques FIRST.x /
    /// LAST.x (jamais de slot PDV, donc jamais écrites en sortie).
    pub by_flags: Vec<(String, bool, bool)>,
    /// Flags IN= du MERGE `(nom UPPERCASE, valeur 0/1)` : 1 si le dataset
    /// associé a participé au groupe de clé BY de l'observation courante.
    /// Mis à jour par le Runner à chaque obs de sortie du MERGE. Servent
    /// les variables automatiques IN= (jamais de slot PDV).
    pub in_flags: Vec<(String, bool)>,
    /// Files FIFO de LAG/DIF, une PAR SITE D'APPEL lexical (clé = pointeur
    /// du slice d'arguments de l'AST, stable d'une itération de la boucle
    /// implicite à l'autre). Voir PLAN.md §Checklist pitfall #8 : LAGn /
    /// DIFn renvoient la valeur d'il y a `n` exécutions du MÊME site, pas la
    /// valeur d'il y a `n` lignes de la variable.
    pub lag_queues: HashMap<usize, std::collections::VecDeque<Value>>,
    /// CALL SYMPUT (M11.5) : écritures DIFFÉRÉES vers la table macro,
    /// `(nom, valeur)` dans l'ordre d'exécution. La visibilité SAS impose
    /// que le symbole ne soit posé qu'APRÈS le RUN de l'étape : on
    /// accumule donc ici et le drain est fait par `exec::execute` une fois
    /// la boucle implicite terminée (là où `&mut Session` est disponible).
    /// Sous le build par défaut ce vecteur se remplit toujours mais le
    /// drain est un no-op (l'engine identité n'a pas de table).
    pub symput_writes: Vec<(String, String)>,
    /// SYMGET (M11.5) : instantané de la table macro pris au DÉBUT de
    /// l'étape (`MacroEngine::symbols_snapshot`), clés en MAJUSCULES. Sous
    /// la feature `macros` il reflète l'état des `%let`/symput antérieurs ;
    /// sous le build par défaut il est vide (aucune résolution macro).
    pub macro_symbols: HashMap<String, String>,
    /// RNG state for RAND*, RANUNI, RANNOR, RANEXP, RANBIN, CALL STREAMINIT
    /// (M15.5). Uses a simple LCG seeded at construction time. CALL STREAMINIT
    /// resets it. Box-Muller stores a spare normal variate in `rng_spare`.
    pub rng_state: u64,
    /// Cached spare normal variate from Box-Muller (set when a pair is
    /// generated; consumed on the next RANNOR call).
    pub rng_spare: Option<f64>,
    /// `DO OVER` actifs (M16.3) : nom d'array UPPERCASE → slot PDV de
    /// l'élément courant. Une référence NUE au nom de l'array (lecture ou
    /// écriture) y est redirigée. Empilé/dépilé par le Runner à chaque tour.
    pub do_over: HashMap<String, usize>,
    /// Variable END= du SET (M16.4) : `(nom UPPERCASE, valeur 0/1)`. Mise à
    /// jour par le Runner après chaque lecture (1 = dernière obs lue). Servie
    /// comme variable automatique (jamais de slot PDV).
    pub end_flag: Option<(String, f64)>,
    /// Objets hash de l'étape (M17.1) : nom UPPERCASE → objet (clés, données,
    /// lignes). Copié depuis `StepProgram.hash_objects` par l'exécuteur ;
    /// defineKey/defineData/defineDone (et M17.2 find/add/...) y opèrent.
    pub hashes: HashMap<String, super::HashObject>,
}

impl Default for EvalCtx {
    fn default() -> Self {
        EvalCtx {
            missing_generated: 0,
            division_by_zero: 0,
            note_num_to_char: false,
            note_char_to_num: false,
            invalid_data: 0,
            error_flag: false,
            fatal: None,
            arrays: HashMap::new(),
            by_flags: Vec::new(),
            in_flags: Vec::new(),
            lag_queues: HashMap::new(),
            symput_writes: Vec::new(),
            macro_symbols: HashMap::new(),
            // Default seed: 1960 (SAS epoch year), shifted to avoid zero.
            rng_state: 0x0000_0007_A120_1960_u64,
            rng_spare: None,
            do_over: HashMap::new(),
            end_flag: None,
            hashes: HashMap::new(),
        }
    }
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
pub(super) fn coerce_num(v: &Value, ctx: &mut EvalCtx) -> Option<f64> {
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
        Expr::Index { name, indices } => eval_array_ref(name, indices, pdv, ctx),
    }
}

/// Référence d'array indexée `arr{i}` / `arr{i,j,k}` (rvalue). Chaque indice
/// est coercé en numérique puis ARRONDI au plus proche ; missing, hors
/// bornes, ou nombre d'indices invalide → erreur fatale "Array subscript out
/// of range." qui stoppe l'étape (comme SAS). Un index unique sur un array
/// multi-dim est interprété linéairement (row-major).
fn eval_array_ref(name: &str, indices: &[Expr], pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    let mut idxs: Vec<i64> = Vec::with_capacity(indices.len());
    for index in indices {
        let idx_val = eval(index, pdv, ctx);
        if ctx.fatal.is_some() {
            return Value::missing();
        }
        match coerce_num(&idx_val, ctx).map(f64::round) {
            Some(i) => idxs.push(i as i64),
            None => {
                ctx.fatal = Some("ERROR: Array subscript out of range.".to_string());
                return Value::missing();
            }
        }
    }
    let Some(def) = ctx.arrays.get(&name.to_uppercase()) else {
        // Impossible après compile() ; garde-fou.
        ctx.fatal = Some(format!("ERROR: Undeclared array referenced: {name}."));
        return Value::missing();
    };
    match def.linear_index(&idxs) {
        Some(lin) => pdv.get(def.slots[lin]).clone(),
        None => {
            ctx.fatal = Some("ERROR: Array subscript out of range.".to_string());
            Value::missing()
        }
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
    // FIRST.x / LAST.x : servies depuis les flags BY du contexte (0/1).
    // La compilation a validé que x est une variable BY ; un nom inconnu
    // ici est un garde-fou.
    if let Some(var) = upper.strip_prefix("FIRST.") {
        return match ctx.by_flags.iter().find(|(n, _, _)| n == var) {
            Some((_, first, _)) => Value::Num(if *first { 1.0 } else { 0.0 }),
            None => {
                ctx.fatal = Some(format!(
                    "ERROR: Variable {name} is not on the program data vector."
                ));
                Value::missing()
            }
        };
    }
    if let Some(var) = upper.strip_prefix("LAST.") {
        return match ctx.by_flags.iter().find(|(n, _, _)| n == var) {
            Some((_, _, last)) => Value::Num(if *last { 1.0 } else { 0.0 }),
            None => {
                ctx.fatal = Some(format!(
                    "ERROR: Variable {name} is not on the program data vector."
                ));
                Value::missing()
            }
        };
    }
    // Variable IN= d'un MERGE : automatique 0/1 servie depuis le contexte.
    if let Some((_, flag)) = ctx.in_flags.iter().find(|(n, _)| *n == upper) {
        return Value::Num(if *flag { 1.0 } else { 0.0 });
    }
    // Variable END= du SET (M16.4) : automatique 0/1 servie depuis le contexte.
    if let Some((_, v)) = ctx.end_flag.as_ref().filter(|(n, _)| *n == upper) {
        return Value::Num(*v);
    }
    // `DO OVER arr` actif : une référence nue à `arr` désigne l'élément
    // courant (M16.3).
    if let Some(slot) = ctx.do_over.get(&upper) {
        return pdv.get(*slot).clone();
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

/// `Call` : `dim(arr)` et les références d'array à parenthèses sont
/// interceptés AVANT l'évaluation des arguments (un nom d'array n'est pas
/// une variable du PDV) ; sinon déléguer à `functions::call`. Fonction
/// inconnue → ERROR fatal.
fn eval_call(name: &str, args: &[Expr], pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    // `dim(arr)` / `hbound(arr[, n])` / `lbound(arr[, n])` : le 1er argument
    // nomme un array déclaré → fonctions de bornes. DIM/HBOUND renvoient la
    // borne supérieure de la dimension n (défaut 1) ; LBOUND = 1 (SAS).
    let is_dim = name.eq_ignore_ascii_case("dim");
    let is_hbound = name.eq_ignore_ascii_case("hbound");
    let is_lbound = name.eq_ignore_ascii_case("lbound");
    if (is_dim || is_hbound || is_lbound)
        && !args.is_empty()
        && let Expr::Var(n) | Expr::Index { name: n, .. } = &args[0]
        && let Some(def) = ctx.arrays.get(&n.to_uppercase()).cloned()
    {
        // Dimension demandée (argument 2 optionnel, défaut 1).
        let which = if args.len() >= 2 {
            let dv = eval(&args[1], pdv, ctx);
            if ctx.fatal.is_some() {
                return Value::missing();
            }
            match coerce_num(&dv, ctx).map(f64::round) {
                Some(d) if d >= 1.0 => d as usize,
                _ => {
                    ctx.fatal = Some(format!(
                        "ERROR: Invalid dimension argument to {}.",
                        name.to_uppercase()
                    ));
                    return Value::missing();
                }
            }
        } else {
            1
        };
        if which > def.dims.len() {
            ctx.fatal = Some(format!(
                "ERROR: Invalid dimension argument to {}.",
                name.to_uppercase()
            ));
            return Value::missing();
        }
        if is_lbound {
            return Value::Num(1.0);
        }
        // DIM et HBOUND coïncident (borne inférieure = 1).
        return Value::Num(def.dims[which - 1] as f64);
    }
    // `arr(i)` / `arr(i,j)` : l'array masque la fonction homonyme (SAS).
    if !args.is_empty() && ctx.arrays.contains_key(&name.to_uppercase()) {
        return eval_array_ref(name, args, pdv, ctx);
    }
    // LAGn / DIFn : NE PEUVENT PAS être de simples fonctions car elles ont
    // besoin de l'identité du SITE D'APPEL (chaque LAG/DIF lexical possède sa
    // propre file FIFO — PLAN.md §Checklist pitfall #8). On intercepte ici,
    // avant la délégation générique.
    if args.len() == 1
        && let Some((n, is_dif)) = parse_lag_dif(name)
    {
        return eval_lag_dif(n, is_dif, args, pdv, ctx);
    }
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

/// Reconnaît `LAG`, `LAG1`, `LAG2`, … et `DIF`, `DIF1`, … (insensible à la
/// casse). Renvoie `(n, is_dif)` où `n` est le décalage (1 par défaut quand
/// aucun chiffre ne suit). Un suffixe non entièrement numérique → None.
fn parse_lag_dif(name: &str) -> Option<(usize, bool)> {
    let upper = name.to_uppercase();
    let (prefix_len, is_dif) = if upper.starts_with("LAG") {
        (3, false)
    } else if upper.starts_with("DIF") {
        (3, true)
    } else {
        return None;
    };
    let suffix = &upper[prefix_len..];
    let n = if suffix.is_empty() {
        1
    } else if suffix.chars().all(|c| c.is_ascii_digit()) {
        suffix.parse::<usize>().ok()?
    } else {
        return None;
    };
    Some((n, is_dif))
}

/// Implémente LAGn / DIFn avec une file FIFO PAR SITE D'APPEL.
///
/// La clé de site est `args.as_ptr() as usize` : l'AST persiste pendant toute
/// l'étape (et `Runner.ctx` aussi), donc ce pointeur est STABLE pour un même
/// site lexical d'une itération de la boucle implicite à l'autre, et DISTINCT
/// entre deux sites différents. C'est le cœur de la sémantique (pitfall #8).
///
/// L'argument est évalué EXACTEMENT UNE FOIS. La file renvoie missing tant que
/// `n` exécutions n'ont pas eu lieu, puis la valeur d'il y a `n` exécutions.
fn eval_lag_dif(n: usize, is_dif: bool, args: &[Expr], pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    // Clé de site AVANT d'emprunter ctx de façon mutable pour l'évaluation.
    let key = args.as_ptr() as usize;
    // Évaluer l'argument UNE seule fois (emprunt mutable de ctx).
    let cur = eval(&args[0], pdv, ctx);
    if ctx.fatal.is_some() {
        return Value::missing();
    }
    // L'emprunt mutable ci-dessus est terminé : on peut emprunter la file.
    let q = ctx.lag_queues.entry(key).or_default();
    let lagged = if q.len() == n {
        q.pop_front().unwrap()
    } else {
        Value::missing()
    };
    q.push_back(cur.clone());

    if is_dif {
        // DIFn(x) = x - LAGn(x).
        if cur.is_missing() || lagged.is_missing() {
            Value::missing()
        } else {
            match (coerce_num(&cur, ctx), coerce_num(&lagged, ctx)) {
                (Some(a), Some(b)) => Value::Num(a - b),
                _ => Value::missing(),
            }
        }
    } else {
        lagged
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

    // ── LAG / DIF : files FIFO PAR SITE D'APPEL ──────────────────────────
    //
    // On réutilise le MÊME `Expr::Call` (donc le même `args.as_ptr()`) et le
    // MÊME `EvalCtx` à travers les appels successifs, en mutant la valeur de
    // `x` dans le PDV entre chaque exécution — c'est exactement ce que fait la
    // boucle implicite du DATA step sur un site lexical donné.

    /// Reconnaissance du suffixe numérique.
    #[test]
    fn parse_lag_dif_recognises_suffix() {
        assert_eq!(parse_lag_dif("LAG"), Some((1, false)));
        assert_eq!(parse_lag_dif("lag"), Some((1, false)));
        assert_eq!(parse_lag_dif("LAG2"), Some((2, false)));
        assert_eq!(parse_lag_dif("DIF"), Some((1, true)));
        assert_eq!(parse_lag_dif("Dif3"), Some((3, true)));
        // Pas un LAG/DIF.
        assert_eq!(parse_lag_dif("LOG"), None);
        // Suffixe non numérique → pas reconnu (laisse passer LAGUERRE & co).
        assert_eq!(parse_lag_dif("LAGX"), None);
    }

    #[test]
    fn lag1_returns_value_from_previous_call() {
        let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(10.0))]);
        let slot = pdv.slot("x").unwrap();
        let e = Expr::Call {
            name: "LAG".to_string(),
            args: vec![var("x")],
        };
        let mut ctx = EvalCtx::default();

        // Appel 1 : x = 10 → LAG renvoie missing (rien encore en file).
        pdv.set(slot, Value::Num(10.0));
        assert_eq!(eval(&e, &pdv, &mut ctx), Value::missing());
        // Appel 2 : x = 20 → LAG renvoie la valeur de l'appel 1 (10).
        pdv.set(slot, Value::Num(20.0));
        assert_eq!(eval(&e, &pdv, &mut ctx), Value::Num(10.0));
        // Appel 3 : x = 30 → LAG renvoie la valeur de l'appel 2 (20).
        pdv.set(slot, Value::Num(30.0));
        assert_eq!(eval(&e, &pdv, &mut ctx), Value::Num(20.0));
    }

    #[test]
    fn lag2_returns_value_from_two_calls_ago() {
        let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
        let slot = pdv.slot("x").unwrap();
        let e = Expr::Call {
            name: "LAG2".to_string(),
            args: vec![var("x")],
        };
        let mut ctx = EvalCtx::default();

        let seq = [1.0, 2.0, 3.0, 4.0];
        // n=2 : missing, missing, puis valeur d'il y a 2 exécutions.
        let expected = [
            Value::missing(),
            Value::missing(),
            Value::Num(1.0),
            Value::Num(2.0),
        ];
        for (v, exp) in seq.iter().zip(expected.iter()) {
            pdv.set(slot, Value::Num(*v));
            assert_eq!(&eval(&e, &pdv, &mut ctx), exp);
        }
    }

    #[test]
    fn dif1_computes_difference_with_missing_first() {
        let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
        let slot = pdv.slot("x").unwrap();
        let e = Expr::Call {
            name: "DIF".to_string(),
            args: vec![var("x")],
        };
        let mut ctx = EvalCtx::default();

        // x : 10, 25, 5 → DIF : ., 15, -20.
        let seq = [10.0, 25.0, 5.0];
        let expected = [Value::missing(), Value::Num(15.0), Value::Num(-20.0)];
        for (v, exp) in seq.iter().zip(expected.iter()) {
            pdv.set(slot, Value::Num(*v));
            assert_eq!(&eval(&e, &pdv, &mut ctx), exp);
        }
    }

    #[test]
    fn dif1_missing_current_yields_missing() {
        let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
        let slot = pdv.slot("x").unwrap();
        let e = Expr::Call {
            name: "DIF".to_string(),
            args: vec![var("x")],
        };
        let mut ctx = EvalCtx::default();

        pdv.set(slot, Value::Num(10.0));
        assert_eq!(eval(&e, &pdv, &mut ctx), Value::missing()); // 1er : pas de lag
        // x manquant → DIF missing même si un lag existe.
        pdv.set(slot, Value::missing());
        assert_eq!(eval(&e, &pdv, &mut ctx), Value::missing());
        // x de nouveau présent, mais le lag (.) est manquant → missing.
        pdv.set(slot, Value::Num(7.0));
        assert_eq!(eval(&e, &pdv, &mut ctx), Value::missing());
        // x = 8, lag = 7 → 1.
        pdv.set(slot, Value::Num(8.0));
        assert_eq!(eval(&e, &pdv, &mut ctx), Value::Num(1.0));
    }

    #[test]
    fn two_lag_sites_have_independent_queues() {
        // CRUX (pitfall #8) : deux sites LAG lexicaux DISTINCTS sur la MÊME
        // variable ont des files INDÉPENDANTES, indexées par args.as_ptr().
        let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
        let slot = pdv.slot("x").unwrap();
        let site_a = Expr::Call {
            name: "LAG".to_string(),
            args: vec![var("x")],
        };
        let site_b = Expr::Call {
            name: "LAG".to_string(),
            args: vec![var("x")],
        };
        // Les deux sites doivent avoir des pointeurs d'args distincts.
        if let (Expr::Call { args: a, .. }, Expr::Call { args: b, .. }) = (&site_a, &site_b) {
            assert_ne!(a.as_ptr() as usize, b.as_ptr() as usize);
        }
        let mut ctx = EvalCtx::default();

        // Appel 1 du site A avec x = 100.
        pdv.set(slot, Value::Num(100.0));
        assert_eq!(eval(&site_a, &pdv, &mut ctx), Value::missing());

        // Premier appel du site B avec x = 200 : DOIT renvoyer missing
        // (file propre au site B vide), PAS 100 (qui appartient au site A).
        pdv.set(slot, Value::Num(200.0));
        assert_eq!(eval(&site_b, &pdv, &mut ctx), Value::missing());

        // Deuxième appel du site A avec x = 300 → renvoie 100 (sa file à lui).
        pdv.set(slot, Value::Num(300.0));
        assert_eq!(eval(&site_a, &pdv, &mut ctx), Value::Num(100.0));

        // Deuxième appel du site B avec x = 400 → renvoie 200 (sa file à lui).
        pdv.set(slot, Value::Num(400.0));
        assert_eq!(eval(&site_b, &pdv, &mut ctx), Value::Num(200.0));

        // Deux files distinctes ont bien été créées.
        assert_eq!(ctx.lag_queues.len(), 2);
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

    /// parse_lag_dif : LAG sans chiffre vaut n=1, DIF idem.
    #[test]
    fn parse_lag_dif_bare_is_one() {
        assert_eq!(parse_lag_dif("LAG"), Some((1, false)));
        assert_eq!(parse_lag_dif("DIF"), Some((1, true)));
    }

    /// parse_lag_dif : un suffixe à plusieurs chiffres est lu en entier.
    #[test]
    fn parse_lag_dif_multidigit_suffix() {
        assert_eq!(parse_lag_dif("LAG10"), Some((10, false)));
        assert_eq!(parse_lag_dif("DIF250"), Some((250, true)));
    }

    /// parse_lag_dif : insensible à la casse, mixte inclus.
    #[test]
    fn parse_lag_dif_case_insensitive() {
        assert_eq!(parse_lag_dif("Lag3"), Some((3, false)));
        assert_eq!(parse_lag_dif("dIf4"), Some((4, true)));
    }

    /// parse_lag_dif : ne capture PAS les fonctions homonymes au préfixe
    /// (LAGUERRE n'existe pas mais une fonction LOGxxx ne doit pas matcher).
    #[test]
    fn parse_lag_dif_rejects_non_matching() {
        assert_eq!(parse_lag_dif("LOG"), None);
        assert_eq!(parse_lag_dif("DIFX"), None);
        assert_eq!(parse_lag_dif("LAG2A"), None);
        assert_eq!(parse_lag_dif("X"), None);
    }

    /// LAG3 : missing sur les 3 premiers appels, puis la valeur d'il y a 3.
    #[test]
    fn lag3_returns_value_from_three_calls_ago() {
        run_site(
            &call("LAG3"),
            &[
                Value::Num(1.0),
                Value::Num(2.0),
                Value::Num(3.0),
                Value::Num(4.0),
                Value::Num(5.0),
            ],
            &[
                Value::missing(),
                Value::missing(),
                Value::missing(),
                Value::Num(1.0),
                Value::Num(2.0),
            ],
        );
    }

    /// DIF2 : x - LAG2(x). Missing tant que LAG2 n'a pas de valeur.
    #[test]
    fn dif2_computes_second_difference() {
        // x : 1,2,4,7,11 → DIF2 : .,.,4-1=3,7-2=5,11-4=7
        run_site(
            &call("DIF2"),
            &[
                Value::Num(1.0),
                Value::Num(2.0),
                Value::Num(4.0),
                Value::Num(7.0),
                Value::Num(11.0),
            ],
            &[
                Value::missing(),
                Value::missing(),
                Value::Num(3.0),
                Value::Num(5.0),
                Value::Num(7.0),
            ],
        );
    }

    /// LAG propage un x manquant dans sa file : le missing ressort au bon rang.
    #[test]
    fn lag_propagates_missing_input() {
        // x : 5, ., 9 → LAG1 : ., 5, .
        run_site(
            &call("LAG"),
            &[Value::Num(5.0), Value::missing(), Value::Num(9.0)],
            &[Value::missing(), Value::Num(5.0), Value::missing()],
        );
    }

    /// DIF : si la valeur retardée est manquante (poussée plus tôt), le résultat
    /// est manquant même quand x courant est présent.
    #[test]
    fn dif_missing_lagged_yields_missing() {
        // x : ., 10 → DIF1 : . (rien en file), . (lag=., x=10)
        run_site(
            &call("DIF"),
            &[Value::missing(), Value::Num(10.0)],
            &[Value::missing(), Value::missing()],
        );
    }

    /// LAG conserve la valeur missing SPÉCIALE telle quelle (.A reste .A).
    #[test]
    fn lag_preserves_special_missing() {
        let special = Value::Missing(MissingKind::Letter(0)); // .A
        run_site(
            &call("LAG"),
            &[special.clone(), Value::Num(3.0)],
            &[Value::missing(), special.clone()],
        );
    }

    /// DIF : un missing spécial dans x courant rend le résultat manquant.
    #[test]
    fn dif_special_missing_current_yields_missing() {
        let special = Value::Missing(MissingKind::Letter(25)); // .Z
        run_site(
            &call("DIF"),
            &[Value::Num(4.0), special],
            &[Value::missing(), Value::missing()],
        );
    }

    /// LAG d'une expression (pas une simple variable) : argument évalué UNE fois,
    /// file FIFO sur la valeur calculée.
    #[test]
    fn lag_of_expression() {
        // site = LAG(x + 1) ; x : 1,2,3 → arg : 2,3,4 → LAG : .,2,3
        let e = Expr::Call {
            name: "LAG".to_string(),
            args: vec![bin(BinaryOp::Add, var("x"), num(1.0))],
        };
        run_site(
            &e,
            &[Value::Num(1.0), Value::Num(2.0), Value::Num(3.0)],
            &[Value::missing(), Value::Num(2.0), Value::Num(3.0)],
        );
    }

    /// DIF d'une constante : x - lag(x) = 0 dès que la file est amorcée.
    #[test]
    fn dif_of_constant_is_zero_after_warmup() {
        run_site(
            &call("DIF"),
            &[Value::Num(7.0), Value::Num(7.0), Value::Num(7.0)],
            &[Value::missing(), Value::Num(0.0), Value::Num(0.0)],
        );
    }

    /// LAG sur valeurs négatives et fractionnaires (pas de troncature).
    #[test]
    fn lag_handles_negative_and_fractional() {
        run_site(
            &call("LAG"),
            &[Value::Num(-1.5), Value::Num(2.25)],
            &[Value::missing(), Value::Num(-1.5)],
        );
    }

    /// DIF sur valeurs fractionnaires : différence exacte.
    #[test]
    fn dif_fractional() {
        run_site(
            &call("DIF"),
            &[Value::Num(1.5), Value::Num(4.0)],
            &[Value::missing(), Value::Num(2.5)],
        );
    }

    /// Trois sites LAG distincts → trois files indépendantes, indexées ptr.
    #[test]
    fn three_lag_sites_independent() {
        let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
        let slot = pdv.slot("x").unwrap();
        let a = call("LAG");
        let b = call("LAG");
        let c = call("LAG2");
        let mut ctx = EvalCtx::default();

        pdv.set(slot, Value::Num(1.0));
        assert_eq!(eval(&a, &pdv, &mut ctx), Value::missing());
        assert_eq!(eval(&b, &pdv, &mut ctx), Value::missing());
        assert_eq!(eval(&c, &pdv, &mut ctx), Value::missing());

        pdv.set(slot, Value::Num(2.0));
        assert_eq!(eval(&a, &pdv, &mut ctx), Value::Num(1.0));
        assert_eq!(eval(&b, &pdv, &mut ctx), Value::Num(1.0));
        assert_eq!(eval(&c, &pdv, &mut ctx), Value::missing());

        pdv.set(slot, Value::Num(3.0));
        assert_eq!(eval(&c, &pdv, &mut ctx), Value::Num(1.0));

        assert_eq!(ctx.lag_queues.len(), 3);
    }

    /// LAG et DIF au MÊME site lexical seraient impossibles (noms différents),
    /// mais LAG(x) et DIF(x) sont deux sites distincts → files distinctes,
    /// le DIF n'emprunte pas la file du LAG.
    #[test]
    fn lag_and_dif_sites_do_not_share_queue() {
        let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
        let slot = pdv.slot("x").unwrap();
        let lag = call("LAG");
        let dif = call("DIF");
        let mut ctx = EvalCtx::default();

        // Appel 1.
        pdv.set(slot, Value::Num(10.0));
        assert_eq!(eval(&lag, &pdv, &mut ctx), Value::missing());
        assert_eq!(eval(&dif, &pdv, &mut ctx), Value::missing());
        // Appel 2 : chacun amorcé sur sa propre file.
        pdv.set(slot, Value::Num(15.0));
        assert_eq!(eval(&lag, &pdv, &mut ctx), Value::Num(10.0));
        assert_eq!(eval(&dif, &pdv, &mut ctx), Value::Num(5.0));
        assert_eq!(ctx.lag_queues.len(), 2);
    }

    /// LAG sur une longue séquence : la file ne garde jamais plus de n éléments
    /// (invariant interne), et le retard est constant.
    #[test]
    fn lag1_long_sequence_constant_delay() {
        let e = call("LAG");
        let inputs: Vec<Value> = (1..=20).map(|i| Value::Num(i as f64)).collect();
        let mut expected = vec![Value::missing()];
        expected.extend((1..20).map(|i| Value::Num(i as f64)));
        run_site(&e, &inputs, &expected);
    }
}
