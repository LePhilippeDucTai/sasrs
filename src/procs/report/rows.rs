use super::*;

/// Detail report: one listing row per (surviving) observation.
pub(super) fn build_detail_rows(
    plan: &[ColPlan],
    decoded: &[Vec<Value>],
    n_obs: usize,
) -> Vec<RowOut> {
    let mut value_rows: Vec<RowOut> = Vec::new();
    for r in 0..n_obs {
        let vals: Vec<Value> = (0..plan.len()).map(|ci| decoded[ci][r].clone()).collect();
        value_rows.push(RowOut {
            kind: RowKind::Detail,
            vals,
        });
    }
    value_rows
}

/// Summary report: group by GROUP+ORDER key columns and emit one row per
/// group, plus BREAK sub-totals and the RBREAK grand total.
pub(super) fn build_summary_rows(
    ast: &ReportAst,
    ds: &crate::dataset::SasDataset,
    plan: &[ColPlan],
    decoded: &[Vec<Value>],
    group_positions: &[usize],
    n_obs: usize,
) -> Vec<RowOut> {
    let mut value_rows: Vec<RowOut> = Vec::new();

    let key_refs: Vec<&Vec<Value>> = group_positions.iter().map(|&p| &decoded[p]).collect();
    let mut groups = group_by_keys(&key_refs, n_obs);

    // Apply DESCENDING direction lexicographically over the key tuple.
    let dirs: Vec<OrderDir> = group_positions.iter().map(|&p| plan[p].dir).collect();
    groups.sort_by(|(a, _), (b, _)| {
        for ((x, y), dir) in a.iter().zip(b).zip(&dirs) {
            let mut c = x.sas_cmp(y);
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
                .position(|&p| {
                    plan[p].idx != usize::MAX && ds.vars[plan[p].idx].name.eq_ignore_ascii_case(vn)
                })
                .map(|pos| (pos, b))
        })
        .collect();

    for (gi, (key, grp_rows)) in groups.iter().enumerate() {
        let vals = summary_row_values(plan, decoded, grp_rows);
        value_rows.push(RowOut {
            kind: RowKind::Group,
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
