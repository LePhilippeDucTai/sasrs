use super::*;

/// ORDER= PROC/CLASS option (J07-P2) — how CLASS levels are ordered in the
/// listing and in the OUT= dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClassOrder {
    /// ORDER=INTERNAL (default): ascending `sas_cmp` order of the raw values.
    #[default]
    Internal,
    /// ORDER=DATA: order of first appearance in the input data.
    Data,
    /// ORDER=FORMATTED: ascending order of the FORMATTED values (ties →
    /// internal). Unformatted values fall back to their raw representation,
    /// so FORMATTED degrades gracefully to INTERNAL.
    Formatted,
    /// ORDER=FREQ: descending frequency count (ties → internal ascending).
    Freq,
}

impl ClassOrder {
    /// Parse an ORDER= value (already lowercased). `None` → unknown keyword.
    pub(super) fn parse(s: &str) -> Option<ClassOrder> {
        match s {
            "internal" => Some(ClassOrder::Internal),
            "data" => Some(ClassOrder::Data),
            "formatted" => Some(ClassOrder::Formatted),
            "freq" => Some(ClassOrder::Freq),
            _ => None,
        }
    }
}

/// One observed CLASS level with the data ORDER= needs: the number of
/// analysis rows carrying it and the position of its first appearance.
pub(super) struct LevelInfo {
    pub value: Value,
    pub count: usize,
    pub first_seen: usize,
}

/// Rank the observed levels of ONE class variable under `order`
/// (rank 0 = first in the listing / OUT=).
pub(super) fn rank_levels(order: ClassOrder, levels: &[LevelInfo]) -> Vec<Value> {
    let mut sorted: Vec<&LevelInfo> = levels.iter().collect();
    match order {
        ClassOrder::Internal => {
            sorted.sort_by(|a, b| a.value.sas_cmp(&b.value));
        }
        ClassOrder::Data => {
            sorted.sort_by_key(|l| l.first_seen);
        }
        ClassOrder::Formatted => {
            // Formatted representation: the char text itself, `format_best`
            // for numerics — the closest available analogue of SAS formats
            // when no format is attached. Ties break on internal order.
            sorted.sort_by(|a, b| {
                let fa = crate::procs::means::report::class_cell(&a.value);
                let fb = crate::procs::means::report::class_cell(&b.value);
                fa.cmp(&fb).then_with(|| a.value.sas_cmp(&b.value))
            });
        }
        ClassOrder::Freq => {
            // Descending frequency count, ties in internal ascending order.
            sorted.sort_by(|a, b| {
                b.count
                    .cmp(&a.count)
                    .then_with(|| a.value.sas_cmp(&b.value))
            });
        }
    }
    sorted.into_iter().map(|l| l.value.clone()).collect()
}

/// Sort `(key, rows)` groups by the per-variable level ranks (ORDER=), used
/// when `order != Internal`. `ranks[i]` lists the levels of class variable
/// `i` in display order; `active` maps each group-key position to its class
/// variable index. Falls back to `sas_cmp` for values absent from `ranks`
/// (e.g. COMPLETETYPES cells never observed — SAS leaves unobserved levels
/// out of the ranking entirely).
pub(super) fn sort_groups_by_ranks(
    groups: &mut [(Vec<Value>, Vec<usize>)],
    ranks: &[Vec<Value>],
    active: &[usize],
) {
    let key_of = |value: &Value, class_idx: usize| -> (usize, Value) {
        let pos = ranks[class_idx]
            .iter()
            .position(|v| v.sas_cmp(value) == Ordering::Equal);
        // Unranked levels sort last, by internal order.
        (pos.unwrap_or(usize::MAX), value.clone())
    };
    groups.sort_by(|(ka, _), (kb, _)| {
        for (pos, &ci) in active.iter().enumerate() {
            let (ra, va) = key_of(&ka[pos], ci);
            let (rb, vb) = key_of(&kb[pos], ci);
            match ra.cmp(&rb) {
                Ordering::Equal => {}
                other => return other,
            }
            match va.sas_cmp(&vb) {
                Ordering::Equal => {}
                other => return other,
            }
        }
        Ordering::Equal
    });
}

/// Is `v` a missing CLASS value? Ordinary/special numeric missing, or the
/// empty character string (a null char cell decodes as `""`).
pub(super) fn is_missing_class(v: &Value) -> bool {
    match v {
        Value::Missing(_) => true,
        Value::Char(s) => s.is_empty(),
        _ => false,
    }
}

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
