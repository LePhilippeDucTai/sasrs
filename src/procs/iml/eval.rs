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
            .ok_or_else(|| SasError::runtime(format!("IML: matrix {} has not been set to a value.", name.to_uppercase()))),
        ImlExpr::Unary { op: UnaryOp::Neg, expr } => {
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
            eval_binop(*op, &l, &r)
        }
        ImlExpr::FnCall { name, args } => eval_fn(name, args, env),
        ImlExpr::Subscript { mat, row, col } => {
            let m = eval_expr(mat, env)?;
            eval_subscript(&m, row, col, env)
        }
    }
}

pub(super) fn eval_binop(op: ImlOp, l: &Matrix, r: &Matrix) -> Result<Matrix> {
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
            // Division par scalaire (ou élément par élément si même dim).
            if rr == 1 && rc == 1 {
                let d = r[0][0];
                Ok(l.iter().map(|row| row.iter().map(|v| v / d).collect()).collect())
            } else {
                elementwise(l, r, |a, b| a / b)
            }
        }
        ImlOp::Mul => {
            // Produit matriciel ; si l'un est scalaire, multiplication scalaire.
            if lr == 1 && lc == 1 {
                let s = l[0][0];
                return Ok(r.iter().map(|row| row.iter().map(|v| v * s).collect()).collect());
            }
            if rr == 1 && rc == 1 {
                let s = r[0][0];
                return Ok(l.iter().map(|row| row.iter().map(|v| v * s).collect()).collect());
            }
            if lc != rr {
                return Err(SasError::runtime(format!(
                    "IML: matrices do not conform for multiplication ({lr}x{lc} * {rr}x{rc})."
                )));
            }
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

pub(super) fn eval_subscript(m: &Matrix, row: &ImlIndex, col: &ImlIndex, env: &Env) -> Result<Matrix> {
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
            .ok_or_else(|| SasError::runtime(format!("IML: {} requires more arguments.", name.to_uppercase())))
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
        "sum" => Ok(scalar(all_elems(&arg(0)?).iter().sum())),
        "mean" => {
            let v = all_elems(&arg(0)?);
            if v.is_empty() {
                return Err(SasError::runtime("IML: MEAN of an empty matrix."));
            }
            Ok(scalar(v.iter().sum::<f64>() / v.len() as f64))
        }
        "std" => {
            let v = all_elems(&arg(0)?);
            if v.len() < 2 {
                return Err(SasError::runtime("IML: STD requires at least two elements."));
            }
            let m = v.iter().sum::<f64>() / v.len() as f64;
            let ss: f64 = v.iter().map(|x| (x - m) * (x - m)).sum();
            Ok(scalar((ss / (v.len() as f64 - 1.0)).sqrt()))
        }
        "min" => {
            let v = all_elems(&arg(0)?);
            v.iter().cloned().fold(None, |acc, x| Some(acc.map_or(x, |a: f64| a.min(x))))
                .map(scalar)
                .ok_or_else(|| SasError::runtime("IML: MIN of an empty matrix."))
        }
        "max" => {
            let v = all_elems(&arg(0)?);
            v.iter().cloned().fold(None, |acc, x| Some(acc.map_or(x, |a: f64| a.max(x))))
                .map(scalar)
                .ok_or_else(|| SasError::runtime("IML: MAX of an empty matrix."))
        }
        "abs" => Ok(map_elems(&arg(0)?, f64::abs)),
        "sqrt" => Ok(map_elems(&arg(0)?, f64::sqrt)),
        "exp" => Ok(map_elems(&arg(0)?, f64::exp)),
        "log" => Ok(map_elems(&arg(0)?, f64::ln)),
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
