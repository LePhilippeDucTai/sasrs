use super::*;

/// `_TYPE_` bitmask for a set of ACTIVE class positions `active` (indices into
/// the CLASS list, 0-based, left→right) given `k` CLASS variables. The LSB
/// corresponds to the LAST class variable — identical convention to the OUTPUT
/// path. Empty `active` → 0 (the overall row).
pub(super) fn type_mask(active: &[usize], k: usize) -> u64 {
    let mut ty: u64 = 0;
    for &i in active {
        ty |= 1u64 << (k - 1 - i);
    }
    ty
}

/// Resolve the WAYS/TYPES restrictions (M33.3) into the SET of `_TYPE_` values
/// to keep. Returns `None` when neither WAYS nor TYPES is given (no
/// restriction — every `_TYPE_` is kept, preserving the default path). `k` is
/// the number of CLASS variables; `class` the CLASS names (for TYPES lookups).
pub(super) fn allowed_types(
    ast: &MeansAst,
    class: &[String],
    k: usize,
) -> Result<Option<std::collections::BTreeSet<u64>>> {
    if ast.ways.is_empty() && ast.types.is_empty() {
        return Ok(None);
    }
    let mut set: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();

    // WAYS n: keep every _TYPE_ whose number of active CLASS vars == n. Enumerate
    // all 2^k subsets and select those whose popcount matches a requested way.
    for &w in &ast.ways {
        for mask in 0u32..(1u32 << k) {
            let active: Vec<usize> = (0..k).filter(|&i| (mask >> i) & 1 == 1).collect();
            if active.len() == w {
                set.insert(type_mask(&active, k));
            }
        }
    }

    // TYPES (crossing ...): keep the specific _TYPE_ for each named crossing.
    for crossing in &ast.types {
        let mut active: Vec<usize> = Vec::with_capacity(crossing.len());
        for name in crossing {
            let pos = class
                .iter()
                .position(|c| c.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    SasError::runtime(format!(
                        "The variable {} in the TYPES statement is not a CLASS variable.",
                        name.to_uppercase()
                    ))
                })?;
            active.push(pos);
        }
        set.insert(type_mask(&active, k));
    }

    Ok(Some(set))
}

/// Like `group_by_keys`, but only considers `rows` (a subset of all rows),
/// grouping by the class-value tuple in `sas_cmp` order. Used so CLASS
/// grouping happens *within* a BY group.
pub(super) fn group_by_keys_subset(
    class_values: &[&Vec<Value>],
    rows: &[usize],
) -> Vec<(Vec<Value>, Vec<usize>)> {
    let mut groups: Vec<(Vec<Value>, Vec<usize>)> = Vec::new();
    for &row in rows {
        let key: Vec<Value> = class_values.iter().map(|c| c[row].clone()).collect();
        let pos = groups.iter().position(|(k, _)| {
            k.len() == key.len()
                && k.iter()
                    .zip(&key)
                    .all(|(a, b)| a.sas_cmp(b) == Ordering::Equal)
        });
        match pos {
            Some(p) => groups[p].1.push(row),
            None => groups.push((key, vec![row])),
        }
    }
    groups.sort_by(|(a, _), (b, _)| {
        for (x, y) in a.iter().zip(b) {
            let c = x.sas_cmp(y);
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    });
    groups
}
