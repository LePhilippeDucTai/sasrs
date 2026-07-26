use super::*;

// ───────────────────────── WHERE evaluation ─────────────────────────

/// Decode every dataset variable into a name→Values map (lowercased names) so a
/// WHERE predicate can reference any variable. Returns a Vec of (name, column)
/// to preserve simple lookup without pulling in a HashMap.
pub(super) fn decode_named_columns(
    ds: &crate::dataset::SasDataset,
) -> Result<Vec<(String, Vec<Value>)>> {
    let mut out = Vec::with_capacity(ds.vars.len());
    for (i, m) in ds.vars.iter().enumerate() {
        out.push((m.name.to_ascii_lowercase(), decode_column(ds, i)?));
    }
    Ok(out)
}

/// Look up a column's value for row `r` by (case-insensitive) name.
pub(super) fn lookup_var<'a>(
    cols: &'a [(String, Vec<Value>)],
    name: &str,
    r: usize,
) -> Option<&'a Value> {
    let lname = name.to_ascii_lowercase();
    cols.iter()
        .find(|(n, _)| *n == lname)
        .map(|(_, col)| &col[r])
}

/// Self-contained, faithful-SAS evaluation of a WHERE/COMPUTE expression for a
/// single row, over decoded columns. Comparisons go through `Value::sas_cmp`
/// (so `. = .` is true and char compares ignore trailing blanks); logical ops
/// use SAS truthiness (missing/0 = false). Unsupported constructs (function
/// calls, arrays, hash methods) evaluate to a guard missing rather than panic.
pub(super) fn eval_row_expr(expr: &Expr, cols: &[(String, Vec<Value>)], r: usize) -> Value {
    use crate::ast::UnaryOp;
    match expr {
        Expr::Num(n) => Value::Num(*n),
        Expr::Str(s) => Value::Char(s.clone()),
        Expr::Missing(k) => Value::Missing(*k),
        Expr::Var(name) => lookup_var(cols, name, r)
            .cloned()
            .unwrap_or(Value::missing()),
        Expr::Unary { op, expr } => {
            let v = eval_row_expr(expr, cols, r);
            match op {
                UnaryOp::Not => Value::Num(if v.truthy() { 0.0 } else { 1.0 }),
                UnaryOp::Plus => match value_to_num(&v) {
                    Some(f) => Value::Num(f),
                    None => Value::missing(),
                },
                UnaryOp::Minus => match value_to_num(&v) {
                    Some(f) => Value::Num(-f),
                    None => Value::missing(),
                },
            }
        }
        Expr::Binary { op, left, right } => {
            let l = eval_row_expr(left, cols, r);
            let rr = eval_row_expr(right, cols, r);
            eval_row_binary(*op, &l, &rr)
        }
        Expr::In { expr, list } => {
            let v = eval_row_expr(expr, cols, r);
            let found = list.iter().any(|e| {
                let item = eval_row_expr(e, cols, r);
                v.sas_cmp(&item) == Ordering::Equal
            });
            Value::Num(if found { 1.0 } else { 0.0 })
        }
        // Unsupported in this lightweight evaluator (documented): guard missing.
        _ => Value::missing(),
    }
}

pub(super) fn eval_row_binary(op: crate::ast::BinaryOp, l: &Value, r: &Value) -> Value {
    use crate::ast::BinaryOp::*;
    match op {
        Lt | Le | Gt | Ge | Eq | Ne => {
            let ord = l.sas_cmp(r);
            let res = match op {
                Eq => ord == Ordering::Equal,
                Ne => ord != Ordering::Equal,
                Lt => ord == Ordering::Less,
                Le => ord != Ordering::Greater,
                Gt => ord == Ordering::Greater,
                Ge => ord != Ordering::Less,
                _ => unreachable!(),
            };
            Value::Num(if res { 1.0 } else { 0.0 })
        }
        And => Value::Num(if l.truthy() && r.truthy() { 1.0 } else { 0.0 }),
        Or => Value::Num(if l.truthy() || r.truthy() { 1.0 } else { 0.0 }),
        Concat => {
            let ls = value_to_disp(l);
            let rs = value_to_disp(r);
            Value::Char(format!("{ls}{rs}"))
        }
        Add | Sub | Mul | Div | Power => match (value_to_num(l), value_to_num(r)) {
            (Some(a), Some(b)) => {
                let v = match op {
                    Add => a + b,
                    Sub => a - b,
                    Mul => a * b,
                    Div => {
                        if b == 0.0 {
                            return Value::missing();
                        }
                        a / b
                    }
                    Power => a.powf(b),
                    _ => unreachable!(),
                };
                Value::Num(v)
            }
            _ => Value::missing(),
        },
    }
}

/// Plain string rendering of a Value for concatenation / LINE output.
pub(super) fn value_to_disp(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim_end().to_string(),
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(_) => String::new(),
    }
}

// ───────────────────────── COMPUTE / LINE ─────────────────────────

/// Apply simple `compute <col>; <col> = <expr>; endcomp;` assignments to each
/// produced row. The expression may reference any report column by name (its
/// per-row value). Computes targeting `after`/`before` are handled separately
/// (LINE rendering); non-column targets are skipped here.
pub(super) fn apply_row_computes(ast: &ReportAst, plan: &[ColPlan], rows: &mut [RowOut]) {
    for comp in &ast.computes {
        // Only column-targeted computes assign into a cell.
        let target_ci = plan
            .iter()
            .position(|c| c.header.eq_ignore_ascii_case(&comp.target));
        // Build the per-row column context lazily inside the loop.
        for ro in rows.iter_mut() {
            // Context: each plan column referenced by its header AND by the
            // positional alias `_Cn_` (1-based COLUMN index, M33.5).
            let cols = compute_row_context(plan, &ro.vals);
            for st in &comp.stmts {
                if let ComputeStmt::Assign { col, expr } = st {
                    let v = eval_row_expr(expr, &cols, 0);
                    // Assign into the named column if it matches a plan column,
                    // else into the compute target column.
                    let dest = plan
                        .iter()
                        .position(|c| c.header.eq_ignore_ascii_case(col))
                        .or(target_ci);
                    if let Some(d) = dest {
                        ro.vals[d] = v;
                    }
                }
            }
        }
    }
}

/// Build the per-row COMPUTE/LINE evaluation context: each plan column is
/// addressable by its (lowercased) header AND by the positional alias `_Cn_`
/// (1-based COLUMN index), matching SAS's `_C1_`/`_C2_` report-column refs
/// (M33.5). Each column holds a single value (the current report row).
pub(super) fn compute_row_context(plan: &[ColPlan], vals: &[Value]) -> Vec<(String, Vec<Value>)> {
    let mut cols: Vec<(String, Vec<Value>)> = Vec::with_capacity(plan.len() * 2);
    for (ci, c) in plan.iter().enumerate() {
        cols.push((c.header.to_ascii_lowercase(), vec![vals[ci].clone()]));
        cols.push((format!("_c{}_", ci + 1), vec![vals[ci].clone()]));
    }
    cols
}
