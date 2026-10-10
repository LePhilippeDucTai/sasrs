use super::*;

pub(super) fn eval_expr(e: &ImlExpr, env: &Env) -> Result<Matrix> {
    match e {
        ImlExpr::Literal(m) => Ok(m.clone()),
        ImlExpr::StrList(_) => Err(SasError::runtime(
            "IML: a character matrix cannot be used in a numeric expression.",
        )),
        ImlExpr::Var(name) => env
            .vars
            .get(&name.to_ascii_uppercase())
            .cloned()
            .ok_or_else(|| {
                SasError::runtime(format!(
                    "IML: matrix {} has not been set to a value.",
                    name.to_uppercase()
                ))
            }),
        ImlExpr::Unary {
            op: UnaryOp::Neg,
            expr,
        } => {
            let m = eval_expr(expr, env)?;
            Ok(m.iter().map(|r| r.iter().map(|v| -v).collect()).collect())
        }
        ImlExpr::Transpose(inner) => {
            let m = eval_expr(inner, env)?;
            Ok(transpose(&m))
        }
        ImlExpr::BinOp { op, left, right } => {
            let l = eval_expr(left, env)?;
            let r = eval_expr(right, env)?;
            eval_binop(*op, &l, &r, env)
        }
        ImlExpr::FnCall { name, args } => eval_fn(name, args, env),
        ImlExpr::Subscript { mat, row, col } => {
            let m = eval_expr(mat, env)?;
            eval_subscript(&m, row, col, env)
        }
    }
}

/// WARNING of the SAS/IML division operator for a zero divisor.
pub(super) const DIVISION_BY_ZERO: &str =
    "Division by zero, result set to missing value.\noperation : /";

pub(super) fn eval_binop(op: ImlOp, l: &Matrix, r: &Matrix, env: &Env) -> Result<Matrix> {
    let (lr, lc) = dims(l);
    let (rr, rc) = dims(r);
    match op {
        ImlOp::Add | ImlOp::Sub => {
            // Élément par élément ; scalaire diffusé.
            let f = |a: f64, b: f64| if op == ImlOp::Add { a + b } else { a - b };
            elementwise(l, r, f)
        }
        ImlOp::Hadamard => elementwise(l, r, |a, b| a * b),
        ImlOp::Div => {
            // Élément par élément, scalaire diffusé. J02-P5 — SAS/IML 9.4,
            // Division Operator : un opérande manquant donne un quotient
            // manquant ; « If a divisor is zero, the operation displays a
            // warning and assigns a missing value for the corresponding
            // element in the result » (inf/NaN silencieux auparavant).
            let zero_divisor = std::cell::Cell::new(false);
            let quotient = |a: f64, b: f64| {
                if a.is_nan() || b.is_nan() {
                    f64::NAN
                } else if b == 0.0 {
                    zero_divisor.set(true);
                    f64::NAN
                } else {
                    a / b
                }
            };
            let out = elementwise(l, r, quotient)?;
            if zero_divisor.get() {
                env.warn(DIVISION_BY_ZERO);
            }
            Ok(out)
        }
        ImlOp::Mul => {
            // Produit matriciel ; si l'un est scalaire, multiplication scalaire.
            if lr == 1 && lc == 1 {
                let s = l[0][0];
                return Ok(r
                    .iter()
                    .map(|row| row.iter().map(|v| v * s).collect())
                    .collect());
            }
            if rr == 1 && rc == 1 {
                let s = r[0][0];
                return Ok(l
                    .iter()
                    .map(|row| row.iter().map(|v| v * s).collect())
                    .collect());
            }
            if lc != rr {
                return Err(SasError::runtime(format!(
                    "IML: matrices do not conform for multiplication ({lr}x{lc} * {rr}x{rc})."
                )));
            }
            // « Matrix multiplication with missing values is not supported »
            // (SAS/IML 9.4, Missing Values) : ERROR plutôt que des NaN.
            require_nonmissing(l, "*")?;
            require_nonmissing(r, "*")?;
            let mut out = vec![vec![0.0; rc]; lr];
            for i in 0..lr {
                for j in 0..rc {
                    let mut s = 0.0;
                    for k in 0..lc {
                        s += l[i][k] * r[k][j];
                    }
                    out[i][j] = s;
                }
            }
            Ok(out)
        }
        ImlOp::Kronecker => Ok(kronecker(l, r)),
        ImlOp::Eq | ImlOp::Ne | ImlOp::Lt | ImlOp::Le | ImlOp::Gt | ImlOp::Ge => {
            // Comparaisons : si les deux sont scalaires → 1×1 booléen.
            // Sinon élément par élément (diffusion scalaire).
            let cmp = |a: f64, b: f64| -> f64 {
                let t = match op {
                    ImlOp::Eq => a == b,
                    ImlOp::Ne => a != b,
                    ImlOp::Lt => a < b,
                    ImlOp::Le => a <= b,
                    ImlOp::Gt => a > b,
                    ImlOp::Ge => a >= b,
                    _ => unreachable!(),
                };
                if t { 1.0 } else { 0.0 }
            };
            elementwise(l, r, cmp)
        }
    }
}

pub(super) fn eval_subscript(
    m: &Matrix,
    row: &ImlIndex,
    col: &ImlIndex,
    env: &Env,
) -> Result<Matrix> {
    let (nr, nc) = dims(m);
    // Resolve an index expression to the explicit (0-based) list of positions.
    let resolve = |idx: &ImlIndex, max: usize| -> Result<Vec<usize>> {
        let check = |v: f64| -> Result<usize> {
            let i = v.round() as i64;
            if i < 1 || i as usize > max {
                return Err(SasError::runtime(format!(
                    "IML: subscript {i} is out of range 1..{max}."
                )));
            }
            Ok((i as usize) - 1)
        };
        match idx {
            ImlIndex::All => Ok((0..max).collect()),
            ImlIndex::Scalar(e) => {
                let v = as_scalar(&eval_expr(e, env)?)?;
                Ok(vec![check(v)?])
            }
            ImlIndex::Range(a, b) => {
                let lo = check(as_scalar(&eval_expr(a, env)?)?)?;
                let hi = check(as_scalar(&eval_expr(b, env)?)?)?;
                // Inclusive range; support both ascending and descending bounds.
                if lo <= hi {
                    Ok((lo..=hi).collect())
                } else {
                    Ok((hi..=lo).rev().collect())
                }
            }
        }
    };
    let rows = resolve(row, nr)?;
    let cols = resolve(col, nc)?;
    let mut out = Vec::with_capacity(rows.len());
    for &i in &rows {
        let mut r = Vec::with_capacity(cols.len());
        for &j in &cols {
            r.push(m[i][j]);
        }
        out.push(r);
    }
    Ok(out)
}

pub(super) fn eval_fn(name: &str, args: &[ImlExpr], env: &Env) -> Result<Matrix> {
    let lname = name.to_ascii_lowercase();
    let arg = |i: usize| -> Result<Matrix> {
        args.get(i)
            .ok_or_else(|| {
                SasError::runtime(format!(
                    "IML: {} requires more arguments.",
                    name.to_uppercase()
                ))
            })
            .and_then(|e| eval_expr(e, env))
    };
    match lname.as_str() {
        "nrow" => Ok(scalar(dims(&arg(0)?).0 as f64)),
        "ncol" => Ok(scalar(dims(&arg(0)?).1 as f64)),
        "dim" => {
            let (nr, nc) = dims(&arg(0)?);
            Ok(vec![vec![nr as f64, nc as f64]])
        }
        "t" => Ok(transpose(&arg(0)?)),
        "shape" => {
            // SHAPE(x, nrow [, ncol]) — reshape row-major, recycling elements.
            // nrow=0 → infer from element count and ncol; ncol omitted/0 → infer.
            let src = arg(0)?;
            let nrow = as_scalar(&arg(1)?)?.round() as i64;
            let ncol = match args.get(2) {
                Some(e) => as_scalar(&eval_expr(e, env)?)?.round() as i64,
                None => 0,
            };
            iml_shape(&src, nrow, ncol)
        }
        // J02-P5 — SUM, MIN et MAX portent sur TOUS leurs arguments (seul le
        // premier était lu) et excluent les valeurs manquantes (SAS/IML 9.4 :
        // « The operators SUM, SSQ, MAX, and MIN check for and exclude missing
        // values » ; SUM rend 0 si tout est manquant, MIN « the machine's
        // largest representable number », MAX la plus négative).
        "sum" => {
            let mut total = 0.0;
            for e in args {
                total += all_elems(&eval_expr(e, env)?)
                    .into_iter()
                    .filter(|v| !v.is_nan())
                    .sum::<f64>();
            }
            Ok(scalar(total))
        }
        "min" | "max" => {
            let is_min = lname == "min";
            let mut best: Option<f64> = None;
            for e in args {
                for v in all_elems(&eval_expr(e, env)?)
                    .into_iter()
                    .filter(|v| !v.is_nan())
                {
                    best = Some(match best {
                        Some(b) if is_min => b.min(v),
                        Some(b) => b.max(v),
                        None => v,
                    });
                }
            }
            Ok(scalar(best.unwrap_or(if is_min {
                f64::MAX
            } else {
                -f64::MAX
            })))
        }
        // J02-P5 — MEAN et STD sont des statistiques PAR COLONNE : une matrice
        // n×p donne un vecteur ligne 1×p (SAS/IML 9.4, MEAN et STD Functions),
        // et non plus un scalaire sur tous les éléments. Les valeurs
        // manquantes d'une colonne sont exclues ; une colonne sans valeur
        // (MEAN) ou avec moins de deux valeurs non manquantes (STD) donne une
        // valeur manquante.
        "mean" => Ok(column_stat(&arg(0)?, |xs| {
            if xs.is_empty() {
                f64::NAN
            } else {
                xs.iter().sum::<f64>() / xs.len() as f64
            }
        })),
        "std" => Ok(column_stat(&arg(0)?, |xs| {
            let n = xs.len();
            if n < 2 {
                return f64::NAN;
            }
            let m = xs.iter().sum::<f64>() / n as f64;
            let ss: f64 = xs.iter().map(|x| (x - m) * (x - m)).sum();
            (ss / (n as f64 - 1.0)).sqrt()
        })),
        "abs" => Ok(map_elems(&arg(0)?, f64::abs)),
        // J02-P5 — SQRT d'un négatif et LOG d'un argument ≤ 0 rendaient NaN /
        // -inf en silence : ERROR d'exécution SAS/IML « Invalid argument to
        // function » (une valeur manquante reste manquante).
        "sqrt" => checked_map(&arg(0)?, "SQRT", |v| v >= 0.0, f64::sqrt),
        "exp" => Ok(map_elems(&arg(0)?, f64::exp)),
        "log" => checked_map(&arg(0)?, "LOG", |v| v > 0.0, f64::ln),
        // ── M28a.3 : algèbre linéaire ──
        "inv" => iml_inv(&arg(0)?),
        "solve" => iml_solve(&arg(0)?, &arg(1)?),
        "eigval" => iml_eigval(&arg(0)?),
        "chol" => iml_chol(&arg(0)?),
        "eigvec" => iml_eigvec(&arg(0)?),
        "det" => Ok(scalar(iml_det(&arg(0)?)?)),
        _ => Err(SasError::runtime(format!(
            "IML: the function {} is not yet implemented.",
            name.to_uppercase()
        ))),
    }
}
