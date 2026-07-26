use super::*;

/// Reconstruit le PDV en marquant `retained` les slots donnés (les autres
/// attributs sont copiés tels quels ; les indices de slots sont stables).
pub(super) fn rebuild_with_retained(pdv: &Pdv, retained: &HashSet<usize>) -> Pdv {
    let mut rebuilt = Pdv::new();
    for (i, v) in pdv.vars().iter().enumerate() {
        let slot = rebuilt.add_var(PdvVar {
            name: v.name.clone(),
            ty: v.ty,
            length: v.length,
            retained: v.retained || retained.contains(&i),
            from_input: v.from_input,
            format: v.format.clone(),
            temporary: v.temporary,
        });
        debug_assert_eq!(slot, i, "rebuild must preserve slot indices");
    }
    rebuilt
}

/// Évalue une valeur initiale CONSTANTE d'un statement ARRAY (`(1, 2, 'x')`)
/// à la compilation. N'accepte que des littéraux (num, chaîne, missing) et
/// `-num`. La valeur est coercée vers le type de l'array (num→char =
/// formaté BEST ; char→num = parse).
pub(super) fn const_eval_initial(expr: &crate::ast::Expr, ty: VarType) -> Result<Value> {
    use crate::ast::{Expr, UnaryOp};
    let v = match expr {
        Expr::Num(n) => Value::Num(*n),
        Expr::Str(s) => Value::Char(s.clone()),
        Expr::Missing(k) => Value::Missing(*k),
        Expr::Unary {
            op: UnaryOp::Minus,
            expr,
        } => match const_eval_initial(expr, VarType::Num)? {
            Value::Num(n) => Value::Num(-n),
            other => other,
        },
        _ => {
            return Err(SasError::runtime(
                "Array initial values must be constants (numbers or quoted strings).",
            ));
        }
    };
    // Coercition vers le type déclaré de l'array.
    Ok(match (ty, &v) {
        (VarType::Char, Value::Num(n)) => Value::Char(crate::value::format_best(*n, 12)),
        (VarType::Num, Value::Char(s)) => match s.trim().parse::<f64>() {
            Ok(n) => Value::Num(n),
            Err(_) => Value::missing(),
        },
        _ => v,
    })
}

/// Type, longueur et valeur d'un littéral d'init RETAIN. Le parser ne
/// produit que `Num` (le `-` unaire y est replié), `Str` ou `Missing` ;
/// tout autre nœud est un garde-fou.
/// Type/longueur de l'index d'un `DO sur liste de valeurs` (M16.3). SAS
/// infère caractère ssi la liste contient au moins une valeur chaîne ; sinon
/// numérique. La longueur caractère est la plus grande des chaînes
/// littérales (défaut 8 si aucune n'est un littéral). Les ranges sont
/// numériques par construction.
pub(super) fn do_list_index_type(items: &[DoListItem]) -> (VarType, usize) {
    let mut is_char = false;
    let mut max_len = 0usize;
    for item in items {
        if let DoListItem::Value(Expr::Str(s)) = item {
            is_char = true;
            max_len = max_len.max(s.chars().count());
        }
    }
    if is_char {
        (VarType::Char, max_len.max(1))
    } else {
        (VarType::Num, 8)
    }
}

pub(super) fn retain_literal(expr: &Expr) -> Result<(VarType, usize, Value)> {
    match expr {
        Expr::Num(n) => Ok((VarType::Num, 8, Value::Num(*n))),
        Expr::Missing(k) => Ok((VarType::Num, 8, Value::Missing(*k))),
        Expr::Str(s) => Ok((
            VarType::Char,
            s.chars().count().max(1),
            Value::Char(s.clone()),
        )),
        _ => Err(SasError::runtime("RETAIN initial values must be literals.")),
    }
}

/// Type et longueur d'une variable d'INPUT (M14). Caractère si `$` OU si
/// l'informat porte un `$` (ex. `$char10.`) ; longueur = largeur de
/// l'informat, sinon 8 par défaut. Numérique : longueur 8 (métadonnée).
pub(super) fn input_var_type(is_char: bool, informat: Option<&str>) -> Result<(VarType, usize)> {
    let spec = informat
        .map(|tok| {
            crate::formats::FormatSpec::parse(tok)
                .ok_or_else(|| SasError::runtime(format!("The informat {tok} is not valid.")))
        })
        .transpose()?;
    let char_informat = spec.as_ref().is_some_and(|s| s.name.starts_with('$'));
    let char = is_char || char_informat;
    if char {
        // Longueur = largeur de l'informat caractère, défaut 8.
        let len = spec
            .as_ref()
            .and_then(|s| s.w)
            .map(|w| w as usize)
            .unwrap_or(8);
        Ok((VarType::Char, len.max(1)))
    } else {
        Ok((VarType::Num, 8))
    }
}

/// Largeur du format d'un `put(x, fmt)` : chiffres finaux du nom du format
/// (`best12` → 12), sinon 200. Le parser M1 ne produit pas encore de
/// littéral de format complet — best-effort.
pub(super) fn put_width(args: &[Expr]) -> usize {
    let Some(fmt) = args.get(1) else { return 200 };
    let name = match fmt {
        Expr::Var(n) => n.as_str(),
        Expr::Str(s) => s.as_str(),
        _ => return 200,
    };
    // La largeur du résultat de PUT est la largeur `w` du format, PAS les
    // chiffres finaux du token : pour `dollar10.2` c'est 10 (pas 2, le nombre
    // de décimales). On s'appuie donc sur le parseur de FormatSpec.
    crate::formats::FormatSpec::parse(name)
        .and_then(|spec| spec.w)
        .map(|w| w as usize)
        .unwrap_or(200)
}
