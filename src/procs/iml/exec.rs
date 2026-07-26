use super::*;

// ───────────────────────── Exécution + listing ─────────────────────────

pub(super) fn exec_stmts(
    stmts: &[ImlStmt],
    env: &mut Env,
    out: &mut Vec<PrintOp>,
    session: &mut Session,
) -> Result<()> {
    for s in stmts {
        exec_stmt(s, env, out, session)?;
    }
    Ok(())
}

/// Opération de PRINT capturée pendant l'exécution, rendue ensuite dans le
/// listing.
pub(super) enum PrintOp {
    Matrix { name: String, m: Matrix },
    Text(String),
}

pub(super) fn exec_stmt(
    s: &ImlStmt,
    env: &mut Env,
    out: &mut Vec<PrintOp>,
    session: &mut Session,
) -> Result<()> {
    match s {
        ImlStmt::Assign { var, expr } => {
            // Une liste de chaînes est stockée dans str_vars, pas dans vars.
            if let ImlExpr::StrList(strs) = expr {
                env.str_vars.insert(var.to_ascii_uppercase(), strs.clone());
                env.vars.remove(&var.to_ascii_uppercase());
                return Ok(());
            }
            let m = eval_expr(expr, env)?;
            env.str_vars.remove(&var.to_ascii_uppercase());
            env.vars.insert(var.to_ascii_uppercase(), m);
            Ok(())
        }
        ImlStmt::Print { items } => {
            for it in items {
                match it {
                    ImlPrintItem::StringLiteral(s) => out.push(PrintOp::Text(s.clone())),
                    ImlPrintItem::Var(name) => {
                        let m = env
                            .vars
                            .get(&name.to_ascii_uppercase())
                            .cloned()
                            .ok_or_else(|| {
                                SasError::runtime(format!(
                                    "IML: matrix {} has not been set to a value.",
                                    name.to_uppercase()
                                ))
                            })?;
                        out.push(PrintOp::Matrix {
                            name: name.to_ascii_uppercase(),
                            m,
                        });
                    }
                }
            }
            Ok(())
        }
        ImlStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            let c = eval_expr(cond, env)?;
            if matrix_truthy(&c) {
                exec_stmts(then_body, env, out, session)
            } else {
                exec_stmts(else_body, env, out, session)
            }
        }
        ImlStmt::DoLoop {
            var,
            from,
            to,
            by,
            body,
        } => {
            let f = as_scalar(&eval_expr(from, env)?)?;
            let t = as_scalar(&eval_expr(to, env)?)?;
            let step = match by {
                Some(e) => as_scalar(&eval_expr(e, env)?)?,
                None => 1.0,
            };
            if step == 0.0 {
                return Err(SasError::runtime("IML: DO loop BY value cannot be zero."));
            }
            let mut i = f;
            let mut guard = 0u64;
            loop {
                if step > 0.0 && i > t + 1e-9 {
                    break;
                }
                if step < 0.0 && i < t - 1e-9 {
                    break;
                }
                env.vars.insert(var.to_ascii_uppercase(), scalar(i));
                exec_stmts(body, env, out, session)?;
                i += step;
                guard += 1;
                if guard > 10_000_000 {
                    return Err(SasError::runtime(
                        "IML: DO loop exceeded the iteration guard.",
                    ));
                }
            }
            Ok(())
        }
        ImlStmt::DoWhile { cond, body } => {
            let mut guard = 0u64;
            while matrix_truthy(&eval_expr(cond, env)?) {
                exec_stmts(body, env, out, session)?;
                guard += 1;
                if guard > 10_000_000 {
                    return Err(SasError::runtime(
                        "IML: DO WHILE loop exceeded the iteration guard.",
                    ));
                }
            }
            Ok(())
        }
        ImlStmt::DoUntil { cond, body } => {
            let mut guard = 0u64;
            loop {
                exec_stmts(body, env, out, session)?;
                if matrix_truthy(&eval_expr(cond, env)?) {
                    break;
                }
                guard += 1;
                if guard > 10_000_000 {
                    return Err(SasError::runtime(
                        "IML: DO UNTIL loop exceeded the iteration guard.",
                    ));
                }
            }
            Ok(())
        }
        ImlStmt::Call { func, args } => exec_call(func, args, env),
        ImlStmt::Create { ds, from, colname } => exec_create(ds, from, colname.as_ref(), env),
        ImlStmt::Append { from } => exec_append(from, env),
        ImlStmt::Close { ds } => exec_close(ds, env, session),
        ImlStmt::Use { ds } => exec_use(ds, env, session),
        ImlStmt::ReadAll { vars, into } => exec_read_all(vars, into, env, session),
        ImlStmt::UnsupportedIo { msg } => Err(SasError::runtime(msg.clone())),
    }
}

/// Exécute un `CALL routine(...)`. Les routines `QR`/`SVDCD` ont des arguments
/// de sortie (lvalues) suivis d'arguments d'entrée.
pub(super) fn exec_call(func: &str, args: &[ImlExpr], env: &mut Env) -> Result<()> {
    let lname = func.to_ascii_lowercase();
    // Extrait le nom (lvalue) d'un argument de sortie.
    let out_name = |e: &ImlExpr| -> Result<String> {
        match e {
            ImlExpr::Var(n) => Ok(n.to_ascii_uppercase()),
            _ => Err(SasError::runtime(format!(
                "IML: CALL {} output arguments must be variable names.",
                func.to_uppercase()
            ))),
        }
    };
    match lname.as_str() {
        "qr" => {
            if args.len() != 3 {
                return Err(SasError::runtime(
                    "IML: CALL QR requires 3 arguments: CALL QR(Q, R, A).",
                ));
            }
            let q_name = out_name(&args[0])?;
            let r_name = out_name(&args[1])?;
            let a = eval_expr(&args[2], env)?;
            let (q, r) = crate::stat::linalg::qr_decomposition(&a)?;
            env.vars.insert(q_name, q);
            env.vars.insert(r_name, r);
            Ok(())
        }
        "svdcd" => {
            if args.len() != 4 {
                return Err(SasError::runtime(
                    "IML: CALL SVDCD requires 4 arguments: CALL SVDCD(U, D, V, A).",
                ));
            }
            let u_name = out_name(&args[0])?;
            let d_name = out_name(&args[1])?;
            let v_name = out_name(&args[2])?;
            let a = eval_expr(&args[3], env)?;
            let (u, d, v) = iml_svdcd(&a)?;
            env.vars.insert(u_name, u);
            env.vars.insert(d_name, d);
            env.vars.insert(v_name, v);
            Ok(())
        }
        "eigen" => {
            // CALL EIGEN(values, vectors, A): values = column vector (descending),
            // vectors = matrix of eigenvectors (columns), A symmetric.
            if args.len() != 3 {
                return Err(SasError::runtime(
                    "IML: CALL EIGEN requires 3 arguments: CALL EIGEN(values, vectors, A).",
                ));
            }
            let val_name = out_name(&args[0])?;
            let vec_name = out_name(&args[1])?;
            let a = eval_expr(&args[2], env)?;
            let (vecs, vals) = symmetric_eigen(&a, "EIGEN")?;
            let val_col: Matrix = vals.into_iter().map(|v| vec![v]).collect();
            env.vars.insert(val_name, val_col);
            env.vars.insert(vec_name, vecs);
            Ok(())
        }
        other => Err(SasError::runtime(format!(
            "IML: the {} subroutine is not yet implemented.",
            other.to_uppercase()
        ))),
    }
}

/// Sépare un nom canonique `LIB.NAME` (ou `NAME`) en (libref, table) MAJUSCULES.
/// Défaut WORK si non qualifié.
pub(super) fn split_ds_name(name: &str) -> (String, String) {
    match name.split_once('.') {
        Some((lib, tbl)) => (lib.to_uppercase(), tbl.to_uppercase()),
        None => ("WORK".to_string(), name.to_uppercase()),
    }
}

/// `CREATE ds FROM mat [COLNAME=cn];` — prépare le tampon (colonnes seulement).
pub(super) fn exec_create(
    ds: &str,
    from: &str,
    colname: Option<&ImlExpr>,
    env: &mut Env,
) -> Result<()> {
    let mat = env
        .vars
        .get(&from.to_ascii_uppercase())
        .cloned()
        .ok_or_else(|| {
            SasError::runtime(format!(
                "IML: matrix {} has not been set to a value.",
                from.to_uppercase()
            ))
        })?;
    let ncol = dims(&mat).1;
    let colnames: Vec<String> = match colname {
        Some(ImlExpr::StrList(s)) => s.iter().map(|x| x.to_string()).collect(),
        Some(ImlExpr::Var(v)) => env
            .str_vars
            .get(&v.to_ascii_uppercase())
            .cloned()
            .ok_or_else(|| {
                SasError::runtime(format!(
                    "IML: COLNAME= must reference a string list; '{}' is not a character matrix.",
                    v.to_uppercase()
                ))
            })?,
        Some(_) => {
            return Err(SasError::runtime(
                "IML: COLNAME= must be a string literal list, e.g. {\"x\" \"y\"}.",
            ));
        }
        None => (1..=ncol).map(|j| format!("COL{j}")).collect(),
    };
    if colnames.len() != ncol {
        return Err(SasError::runtime(format!(
            "IML: COLNAME= has {} names but the matrix has {} columns.",
            colnames.len(),
            ncol
        )));
    }
    env.open_writes.insert(
        ds.to_uppercase(),
        OpenWrite {
            colnames,
            rows: Vec::new(),
        },
    );
    Ok(())
}

/// `APPEND FROM mat;` — ajoute les lignes de `mat` au (seul) dataset ouvert.
pub(super) fn exec_append(from: &str, env: &mut Env) -> Result<()> {
    let mat = env
        .vars
        .get(&from.to_ascii_uppercase())
        .cloned()
        .ok_or_else(|| {
            SasError::runtime(format!(
                "IML: matrix {} has not been set to a value.",
                from.to_uppercase()
            ))
        })?;
    // SAS APPEND s'applique au dataset courant en écriture. Ici on exige qu'il
    // y en ait exactement un d'ouvert.
    if env.open_writes.len() != 1 {
        return Err(SasError::runtime(
            "IML: APPEND requires exactly one open output data set (use CREATE first).",
        ));
    }
    let key = env.open_writes.keys().next().cloned().unwrap();
    let buf = env.open_writes.get_mut(&key).unwrap();
    let ncol = buf.colnames.len();
    for row in &mat {
        if row.len() != ncol {
            return Err(SasError::runtime(format!(
                "IML: APPEND row has {} columns but the data set expects {}.",
                row.len(),
                ncol
            )));
        }
        buf.rows.push(row.clone());
    }
    Ok(())
}

/// `CLOSE ds;` — écrit le dataset accumulé dans la bibliothèque cible.
pub(super) fn exec_close(ds: &str, env: &mut Env, session: &mut Session) -> Result<()> {
    let key = ds.to_uppercase();
    if let Some(buf) = env.open_writes.remove(&key) {
        use crate::dataset::{SasDataset, VarMeta};
        use crate::value::VarType;
        use polars::prelude::*;
        let (libref, table) = split_ds_name(&key);
        let ncol = buf.colnames.len();
        let nrow = buf.rows.len();
        // Construire une colonne f64 par variable.
        let mut columns: Vec<Column> = Vec::with_capacity(ncol);
        let mut vars: Vec<VarMeta> = Vec::with_capacity(ncol);
        for j in 0..ncol {
            let col: Vec<f64> = (0..nrow).map(|i| buf.rows[i][j]).collect();
            columns.push(Series::new(buf.colnames[j].as_str().into(), col).into());
            vars.push(VarMeta {
                name: buf.colnames[j].clone(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            });
        }
        let df = DataFrame::new(columns)?;
        let out_ds = SasDataset { df, vars };
        let display = format!("{libref}.{table}");
        session.libs.get(&libref)?.write(&table, &out_ds)?;
        session.last_dataset = Some(display.clone());
        session.log.note(&format!(
            "The data set {display} has {nrow} observations and {ncol} variables."
        ));
        return Ok(());
    }
    // Fermeture d'un dataset ouvert en lecture : best-effort.
    env.open_reads.remove(&key);
    Ok(())
}

/// `USE ds;` — ouvre un dataset en lecture (marque l'ouverture).
pub(super) fn exec_use(ds: &str, env: &mut Env, session: &mut Session) -> Result<()> {
    let key = ds.to_uppercase();
    let (libref, table) = split_ds_name(&key);
    let provider = session.libs.get(&libref)?;
    if !provider.exists(&table) {
        return Err(SasError::runtime(format!(
            "IML: data set {libref}.{table} does not exist."
        )));
    }
    env.open_reads.insert(key);
    Ok(())
}

/// `READ ALL VAR {vars} INTO mat;` — lit les colonnes demandées dans une matrice.
pub(super) fn exec_read_all(
    vars: &[String],
    into: &str,
    env: &mut Env,
    session: &mut Session,
) -> Result<()> {
    use crate::procs::common::decode_column;
    use crate::value::Value;
    // Choisir le dataset ouvert en lecture (exactement un attendu).
    if env.open_reads.len() != 1 {
        return Err(SasError::runtime(
            "IML: READ requires exactly one open input data set (use USE first).",
        ));
    }
    let key = env.open_reads.iter().next().cloned().unwrap();
    let (libref, table) = split_ds_name(&key);
    let (ds, notes) = session.libs.get(&libref)?.read(&table)?;
    for note in notes {
        session.log.forward(&note);
    }
    // Indices des colonnes demandées.
    let mut col_idx = Vec::with_capacity(vars.len());
    for vname in vars {
        let idx = ds
            .vars
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case(vname))
            .ok_or_else(|| {
                SasError::runtime(format!(
                    "IML: variable {} not found in data set {libref}.{table}.",
                    vname
                ))
            })?;
        col_idx.push(idx);
    }
    let nrow = ds.n_obs();
    // Décoder chaque colonne demandée en f64.
    let mut cols: Vec<Vec<f64>> = Vec::with_capacity(col_idx.len());
    for &ci in &col_idx {
        let decoded = decode_column(&ds, ci)?;
        let mut c = Vec::with_capacity(nrow);
        for v in decoded {
            let x = match v {
                Value::Num(x) => x,
                Value::Missing(_) => f64::NAN,
                Value::Char(_) => {
                    return Err(SasError::runtime(format!(
                        "IML: variable {} is character; READ INTO requires numeric variables.",
                        ds.vars[ci].name
                    )));
                }
            };
            c.push(x);
        }
        cols.push(c);
    }
    // Assembler la matrice nrow × ncol.
    let mut mat = vec![vec![0.0; col_idx.len()]; nrow];
    for (j, c) in cols.iter().enumerate() {
        for (i, &x) in c.iter().enumerate() {
            mat[i][j] = x;
        }
    }
    env.vars.insert(into.to_ascii_uppercase(), mat);
    Ok(())
}
