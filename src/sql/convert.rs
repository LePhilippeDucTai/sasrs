use super::*;

// ----------------------------------------------------------------------------
// INSERT
// ----------------------------------------------------------------------------

/// Évaluateur de littéraux pour `INSERT ... VALUES`. Les expressions doivent
/// être constantes : `Num`, `Str`, `Missing` ou un moins unaire sur un `Num`.
/// Tout le reste → ERROR propre.
pub(super) fn expr_to_value(e: &Expr) -> Result<Value> {
    match e {
        Expr::Num(n) => Ok(Value::Num(*n)),
        Expr::Str(s) => Ok(Value::Char(s.clone())),
        Expr::Missing(k) => Ok(Value::Missing(*k)),
        Expr::Unary {
            op: UnaryOp::Minus,
            expr,
        } => match expr.as_ref() {
            Expr::Num(n) => Ok(Value::Num(-n)),
            _ => Err(SasError::runtime(
                "Only constant expressions are supported in INSERT ... VALUES.",
            )),
        },
        Expr::Unary {
            op: UnaryOp::Plus,
            expr,
        } => match expr.as_ref() {
            Expr::Num(n) => Ok(Value::Num(*n)),
            _ => Err(SasError::runtime(
                "Only constant expressions are supported in INSERT ... VALUES.",
            )),
        },
        _ => Err(SasError::runtime(
            "Only constant expressions are supported in INSERT ... VALUES.",
        )),
    }
}

/// Décode chaque colonne d'un dataset en Vec<Value> (downcast par colonne).
pub(super) fn decode_columns(ds: &SasDataset) -> Result<Vec<Vec<Value>>> {
    let mut cols: Vec<Vec<Value>> = Vec::with_capacity(ds.vars.len());
    for (i, v) in ds.vars.iter().enumerate() {
        let series = ds.df.get_columns()[i].as_materialized_series();
        let values: Vec<Value> = match v.ty {
            VarType::Num => series.f64()?.iter().map(num_to_value).collect(),
            VarType::Char => series
                .str()?
                .iter()
                .map(|o| Value::Char(o.unwrap_or("").to_string()))
                .collect(),
        };
        cols.push(values);
    }
    Ok(cols)
}

/// Coerce une Value à la cible (char/num) selon le VarMeta. Pour une cible
/// char, tronque à la longueur de stockage ; pour une cible num, garde le
/// nombre/missing (un littéral char vers une num → missing).
pub(super) fn coerce_to_target(v: Value, meta: &VarMeta) -> Value {
    match meta.ty {
        VarType::Char => {
            let s = match v {
                Value::Char(s) => s,
                Value::Num(_) | Value::Missing(_) => String::new(),
            };
            let truncated: String = s.chars().take(meta.length.max(1)).collect();
            Value::Char(truncated)
        }
        VarType::Num => match v {
            Value::Num(_) | Value::Missing(_) => v,
            Value::Char(_) => Value::missing(),
        },
    }
}

/// Coercition d'assignation UPDATE (M20.4), proche de la sémantique SAS du
/// signe `=` : char→num parse une chaîne numérique (sinon missing), num→char
/// formate en BEST12. puis tronque à la longueur déclarée, missing reste
/// missing. Distincte de `coerce_to_target` (INSERT, conversion stricte) pour
/// ne pas altérer les snapshots existants.
pub(super) fn coerce_update_target(v: Value, meta: &VarMeta) -> Value {
    match meta.ty {
        VarType::Char => {
            let s = match v {
                Value::Char(s) => s,
                Value::Num(f) => format_best(f, 12),
                Value::Missing(_) => String::new(),
            };
            let truncated: String = s.chars().take(meta.length.max(1)).collect();
            Value::Char(truncated)
        }
        VarType::Num => match v {
            Value::Num(_) | Value::Missing(_) => v,
            Value::Char(s) => {
                let t = s.trim();
                if t.is_empty() {
                    Value::missing()
                } else {
                    match t.parse::<f64>() {
                        Ok(f) => Value::Num(f),
                        Err(_) => Value::missing(),
                    }
                }
            }
        },
    }
}

/// Reconstruit un DataFrame depuis des colonnes de Value alignées sur les
/// VarMeta (num → Float64, char → String).
pub(super) fn build_dataframe(vars: &[VarMeta], cols: &[Vec<Value>]) -> Result<DataFrame> {
    let mut series: Vec<Column> = Vec::with_capacity(vars.len());
    for (i, v) in vars.iter().enumerate() {
        let col = &cols[i];
        let s = match v.ty {
            VarType::Num => {
                let ca: Float64Chunked = col
                    .iter()
                    .map(value_to_num)
                    .collect::<Float64Chunked>()
                    .with_name(v.name.as_str().into());
                ca.into_series()
            }
            VarType::Char => {
                let ca: StringChunked = col
                    .iter()
                    .map(|val| match val {
                        Value::Char(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect::<StringChunked>()
                    .with_name(v.name.as_str().into());
                ca.into_series()
            }
        };
        series.push(s.into());
    }
    Ok(DataFrame::new(series)?)
}

// ----------------------------------------------------------------------------
// UPDATE ... SET ... [WHERE] (M20.4)
// ----------------------------------------------------------------------------

/// Décode une série Polars arbitraire (Float64 / String, ou tout type coercible)
/// en Vec<Value>. Les colonnes calculées d'un UPDATE peuvent ressortir en
/// Float64 (arithmétique) ou String (concaténation/littéral) ; on ramène
/// chaque cellule à une `Value` SAS canonique.
pub(super) fn decode_series(series: &Series) -> Vec<Value> {
    match series.dtype() {
        DataType::Float64 => series
            .f64()
            .map(|ca| ca.iter().map(num_to_value).collect())
            .unwrap_or_default(),
        DataType::String => series
            .str()
            .map(|ca| {
                ca.iter()
                    .map(|o| match o {
                        Some(s) => Value::Char(s.to_string()),
                        None => Value::Char(String::new()),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => {
            // Type natif (entiers, booléens...) : on caste en Float64 puis décode.
            match series.cast(&DataType::Float64) {
                Ok(f) => f
                    .f64()
                    .map(|ca| ca.iter().map(num_to_value).collect())
                    .unwrap_or_default(),
                Err(_) => vec![Value::missing(); series.len()],
            }
        }
    }
}
