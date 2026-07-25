use super::*;

/// Référence d'array indexée `arr{i}` / `arr{i,j,k}` (rvalue). Chaque indice
/// est coercé en numérique puis ARRONDI au plus proche ; missing, hors
/// bornes, ou nombre d'indices invalide → erreur fatale "Array subscript out
/// of range." qui stoppe l'étape (comme SAS). Un index unique sur un array
/// multi-dim est interprété linéairement (row-major).
pub(super) fn eval_array_ref(name: &str, indices: &[Expr], pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    let mut idxs: Vec<i64> = Vec::with_capacity(indices.len());
    for index in indices {
        let idx_val = eval(index, pdv, ctx);
        if ctx.fatal.is_some() {
            return Value::missing();
        }
        match coerce_num(&idx_val, ctx).map(f64::round) {
            Some(i) => idxs.push(i as i64),
            None => {
                ctx.fatal = Some(SasError::runtime("Array subscript out of range."));
                return Value::missing();
            }
        }
    }
    let Some(def) = ctx.arrays.get(&name.to_uppercase()) else {
        // Impossible après compile() ; garde-fou.
        ctx.fatal = Some(SasError::runtime(format!(
            "Undeclared array referenced: {name}."
        )));
        return Value::missing();
    };
    match def.linear_index(&idxs) {
        Some(lin) => pdv.get(def.slots[lin]).clone(),
        None => {
            ctx.fatal = Some(SasError::runtime("Array subscript out of range."));
            Value::missing()
        }
    }
}

pub(super) fn eval_var(name: &str, pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
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
                ctx.fatal = Some(SasError::runtime(format!(
                    "Variable {name} is not on the program data vector."
                )));
                Value::missing()
            }
        };
    }
    if let Some(var) = upper.strip_prefix("LAST.") {
        return match ctx.by_flags.iter().find(|(n, _, _)| n == var) {
            Some((_, _, last)) => Value::Num(if *last { 1.0 } else { 0.0 }),
            None => {
                ctx.fatal = Some(SasError::runtime(format!(
                    "Variable {name} is not on the program data vector."
                )));
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
            ctx.fatal = Some(SasError::runtime(format!(
                "Variable {name} is not on the program data vector."
            )));
            Value::missing()
        }
    }
}

/// `Call` : `dim(arr)` et les références d'array à parenthèses sont
/// interceptés AVANT l'évaluation des arguments (un nom d'array n'est pas
/// une variable du PDV) ; sinon déléguer à `functions::call`. Fonction
/// inconnue → ERROR fatal.
pub(super) fn eval_call(name: &str, args: &[Expr], pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    // Nom normalisé UNE seule fois (lookups d'arrays, dispatch, messages) —
    // évite les `to_uppercase()` répétés à chaque appel de fonction.
    let upper = name.to_uppercase();
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
                    ctx.fatal = Some(SasError::runtime(format!(
                        "Invalid dimension argument to {upper}."
                    )));
                    return Value::missing();
                }
            }
        } else {
            1
        };
        if which > def.dims.len() {
            ctx.fatal = Some(SasError::runtime(format!(
                "Invalid dimension argument to {upper}."
            )));
            return Value::missing();
        }
        if is_lbound {
            return Value::Num(1.0);
        }
        // DIM et HBOUND coïncident (borne inférieure = 1).
        return Value::Num(def.dims[which - 1] as f64);
    }
    // `arr(i)` / `arr(i,j)` : l'array masque la fonction homonyme (SAS).
    if !args.is_empty() && ctx.arrays.contains_key(&upper) {
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
    // `upper` est déjà normalisé : `functions::call` ne ré-alloue pas.
    match functions::call(&upper, &arg_vals, ctx) {
        Some(v) => v,
        None => {
            ctx.fatal = Some(SasError::runtime(format!("Function {upper} is unknown.")));
            Value::missing()
        }
    }
}

/// Reconnaît `LAG`, `LAG1`, `LAG2`, … et `DIF`, `DIF1`, … (insensible à la
/// casse). Renvoie `(n, is_dif)` où `n` est le décalage (1 par défaut quand
/// aucun chiffre ne suit). Un suffixe non entièrement numérique → None.
pub(super) fn parse_lag_dif(name: &str) -> Option<(usize, bool)> {
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
pub(super) fn eval_lag_dif(n: usize, is_dif: bool, args: &[Expr], pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
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
