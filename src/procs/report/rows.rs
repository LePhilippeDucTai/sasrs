// MQ7.2c — `needless_range_loop` assumé dans ce module : l'indice EST le
// langage du domaine (`a[i][j] * b[j][k]`, parcours colonne-major, triangle
// d'une matrice symétrique). La forme itérateur y coûte plus en lisibilité
// qu'elle n'en rend, et la revue a préféré garder les indices explicites.
#![allow(clippy::needless_range_loop)]

use super::*;

/// Detail report: one listing row per (surviving) observation.
pub(super) fn build_detail_rows(
    plan: &[ColPlan],
    decoded: &[Vec<Value>],
    n_obs: usize,
) -> Vec<RowOut> {
    let mut value_rows: Vec<RowOut> = Vec::new();
    for r in 0..n_obs {
        value_rows.push(RowOut {
            kind: RowKind::Detail,
            vals: detail_row_values(plan, decoded, r),
        });
    }
    value_rows
}

/// Values of the detail row of observation `r` (every column as read).
fn detail_row_values(plan: &[ColPlan], decoded: &[Vec<Value>], r: usize) -> Vec<Value> {
    (0..plan.len()).map(|ci| decoded[ci][r].clone()).collect()
}

/// Key column of a GROUP/ORDER item: its formatted values when the item has a
/// format (rows are formed on the formatted values, J02-P7), else the values
/// as read.
fn group_key_column(
    col: &ColPlan,
    values: &[Value],
    catalog: &crate::formats::FormatCatalog,
) -> Vec<Value> {
    match col
        .format
        .as_deref()
        .and_then(crate::formats::FormatSpec::parse)
    {
        Some(spec) => values
            .iter()
            .map(|v| Value::Char(catalog.format(v, &spec).trim().to_string()))
            .collect(),
        None => values.to_vec(),
    }
}

/// Report with GROUP or ORDER variables: rows ordered by these variables
/// (COLUMN order, each in its direction), plus BREAK sub-totals and the RBREAK
/// grand total.
///
/// J02-P7 — with `consolidate` (GROUP variables and no ORDER or DISPLAY item)
/// one row per combination of the FORMATTED values of the GROUP variables;
/// otherwise one detail row per observation. SAS 9.4 REPORT, « Usage of
/// Variables in a Report »: « A report that contains one or more order
/// variables has a row for every observation in the input data set », and
/// GROUP variables that cannot consolidate are displayed as ORDER variables.
/// ORDER used to consolidate the rows like GROUP. A formatted key is ordered by
/// the smallest unformatted value it covers; ties keep the data order.
pub(super) fn build_summary_rows(
    ast: &ReportAst,
    plan: &[ColPlan],
    decoded: &[Vec<Value>],
    group_positions: &[usize],
    n_obs: usize,
    consolidate: bool,
    catalog: &crate::formats::FormatCatalog,
) -> Vec<RowOut> {
    let mut value_rows: Vec<RowOut> = Vec::new();

    let keys: Vec<Vec<Value>> = group_positions
        .iter()
        .map(|&p| group_key_column(&plan[p], &decoded[p][..n_obs], catalog))
        .collect();
    // Sort value of every formatted key: the smallest unformatted value of
    // the rows that share it (unformatted columns sort on the key itself).
    let reps: Vec<Option<std::collections::HashMap<String, Value>>> = group_positions
        .iter()
        .zip(&keys)
        .map(|(&p, key_col)| {
            plan[p].format.as_ref()?;
            let mut reps: std::collections::HashMap<String, Value> =
                std::collections::HashMap::new();
            for (k, v) in key_col.iter().zip(&decoded[p]) {
                let Value::Char(k) = k else { continue };
                match reps.get_mut(k) {
                    Some(rep) if v.sas_cmp(rep) == Ordering::Less => *rep = v.clone(),
                    Some(_) => {}
                    None => {
                        reps.insert(k.clone(), v.clone());
                    }
                }
            }
            Some(reps)
        })
        .collect();
    let sort_value = |pos: usize, key: &Value| -> Value {
        match (&reps[pos], key) {
            (Some(m), Value::Char(k)) => m.get(k).cloned().unwrap_or_else(|| key.clone()),
            _ => key.clone(),
        }
    };

    let mut groups: Vec<(Vec<Value>, Vec<usize>)> = if consolidate {
        let key_refs: Vec<&Vec<Value>> = keys.iter().collect();
        group_by_keys(&key_refs, n_obs)
    } else {
        (0..n_obs)
            .map(|r| (keys.iter().map(|k| k[r].clone()).collect(), vec![r]))
            .collect()
    };

    // Apply DESCENDING direction lexicographically over the key tuple (stable
    // sort: equal keys keep the data order).
    let dirs: Vec<OrderDir> = group_positions.iter().map(|&p| plan[p].dir).collect();
    groups.sort_by(|(a, _), (b, _)| {
        for (pos, ((x, y), dir)) in a.iter().zip(b).zip(&dirs).enumerate() {
            let mut c = sort_value(pos, x).sas_cmp(&sort_value(pos, y));
            if *dir == OrderDir::Descending {
                c = c.reverse();
            }
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    });

    // Which group var(s) trigger a BREAK? Map a break's var to its position
    // in `group_positions` (the deepest matching group level).
    let break_after: Vec<(usize, &Break)> = ast
        .breaks
        .iter()
        .filter_map(|b| {
            let vn = b.var.as_ref()?;
            group_positions
                .iter()
                .position(|&p| plan[p].name.eq_ignore_ascii_case(vn))
                .map(|pos| (pos, b))
        })
        .collect();

    for (gi, (key, grp_rows)) in groups.iter().enumerate() {
        let vals = if consolidate {
            summary_row_values(plan, decoded, grp_rows)
        } else {
            detail_row_values(plan, decoded, grp_rows[0])
        };
        value_rows.push(RowOut {
            kind: if consolidate {
                RowKind::Group
            } else {
                RowKind::Detail
            },
            vals,
        });

        // BREAK AFTER <var>: emit a sub-total line when the key value for
        // that level changes (or at the last group).
        for &(level_pos, brk) in &break_after {
            let is_last = gi + 1 == groups.len();
            let changes = is_last
                || groups[gi + 1]
                    .0
                    .get(level_pos)
                    .map(|nv| key[level_pos].sas_cmp(nv) != Ordering::Equal)
                    != Some(false);
            if changes && brk.summarize {
                // Range = all original rows whose key matches up to and
                // including `level_pos`. Collect across the contiguous run.
                let range = break_range_rows(&groups, gi, level_pos, key);
                let bvals = break_row_values(plan, decoded, &range, level_pos);
                value_rows.push(RowOut {
                    kind: RowKind::Break,
                    vals: bvals,
                });
            }
        }
    }

    // RBREAK AFTER / SUMMARIZE: grand-total line over all surviving rows.
    if let Some(rb) = &ast.rbreak
        && rb.summarize
    {
        let all: Vec<usize> = (0..n_obs).collect();
        let rvals = break_row_values(plan, decoded, &all, usize::MAX);
        value_rows.push(RowOut {
            kind: RowKind::Rbreak,
            vals: rvals,
        });
    }
    value_rows
}

/// A produced report row (typed values) and what kind of row it is.
pub(super) struct RowOut {
    pub(super) kind: RowKind,
    pub(super) vals: Vec<Value>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum RowKind {
    Detail,
    Group,
    Break,
    Rbreak,
}

/// Compute the typed cell values of a summary (group) row.
pub(super) fn summary_row_values(
    plan: &[ColPlan],
    decoded: &[Vec<Value>],
    grp_rows: &[usize],
) -> Vec<Value> {
    let mut vals = Vec::with_capacity(plan.len());
    for (ci, c) in plan.iter().enumerate() {
        let v = match &c.usage {
            Usage::Group | Usage::Order => decoded[ci][grp_rows[0]].clone(),
            Usage::Analysis(stat) => {
                let (xs, nmiss) = partition_numeric(&decoded[ci], grp_rows);
                means::compute(stat, &xs, nmiss, 0.05)
            }
            Usage::Display => {
                let first = &decoded[ci][grp_rows[0]];
                let constant = grp_rows
                    .iter()
                    .all(|&r| decoded[ci][r].sas_cmp(first) == Ordering::Equal);
                if constant {
                    first.clone()
                } else {
                    Value::Char(String::new())
                }
            }
            // COMPUTED / ACROSS columns are filled later / handled elsewhere.
            _ => Value::missing(),
        };
        vals.push(v);
    }
    vals
}

/// Compute the typed cell values of a BREAK/RBREAK summary row. The break key
/// columns up to and including `level_pos` keep their value; deeper group
/// columns are blanked; ANALYSIS columns are recomputed over `range`.
pub(super) fn break_row_values(
    plan: &[ColPlan],
    decoded: &[Vec<Value>],
    range: &[usize],
    level_pos_excl: usize,
) -> Vec<Value> {
    // Translate the group-level cutoff (an index into group_positions) into a
    // plan-column comparison: we keep GROUP/ORDER cells whose own group level
    // is <= level_pos_excl; here we simply keep the first matching value for
    // key columns and blank the rest, marking the first key column with a tag.
    let mut group_seen = 0usize;
    let mut vals = Vec::with_capacity(plan.len());
    let mut first_key_done = false;
    for (ci, c) in plan.iter().enumerate() {
        let v = match &c.usage {
            Usage::Group | Usage::Order => {
                let keep = group_seen <= level_pos_excl;
                group_seen += 1;
                if keep && !range.is_empty() {
                    if !first_key_done && level_pos_excl == usize::MAX {
                        // RBREAK: label the leading key column.
                        first_key_done = true;
                        Value::Char(String::new())
                    } else {
                        decoded[ci][range[0]].clone()
                    }
                } else {
                    Value::Char(String::new())
                }
            }
            Usage::Analysis(stat) => {
                let (xs, nmiss) = partition_numeric(&decoded[ci], range);
                means::compute(stat, &xs, nmiss, 0.05)
            }
            _ => Value::Char(String::new()),
        };
        vals.push(v);
    }
    vals
}

/// Collect the original (projected) row indices belonging to the contiguous run
/// of groups that share the same key prefix up to `level_pos` ending at `gi`.
pub(super) fn break_range_rows(
    groups: &[(Vec<Value>, Vec<usize>)],
    gi: usize,
    level_pos: usize,
    key: &[Value],
) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    // Walk backwards while the prefix matches, then forward — but since groups
    // are sorted, the run sharing this prefix is contiguous and ends at gi.
    let prefix_eq =
        |k: &[Value]| -> bool { (0..=level_pos).all(|p| key[p].sas_cmp(&k[p]) == Ordering::Equal) };
    let mut start = gi;
    while start > 0 && prefix_eq(&groups[start - 1].0) {
        start -= 1;
    }
    for g in &groups[start..=gi] {
        out.extend_from_slice(&g.1);
    }
    out
}
