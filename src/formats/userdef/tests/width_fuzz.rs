use super::*;

// ── M43.1 — UserFormat::effective_width ─────────────────────────────────────

fn uf_with(min: Option<u32>, max: Option<u32>, default: Option<u32>) -> UserFormat {
    UserFormat {
        is_char: false,
        ranges: vec![num_single(1.0, "AB"), num_single(2.0, "ABCDE")],
        other: Some("Z".to_string()),
        min,
        max,
        default,
        fuzz: None,
    }
}

#[test]
fn effective_width_no_options_matches_legacy_behavior_exactly() {
    // The "byte-identical" invariant: with MIN=/MAX=/DEFAULT= all unset,
    // effective_width must be EXACTLY spec.w when given, else None (no
    // truncation) — the pre-M43.1 calculation in `formats::FormatCatalog::format`.
    let uf = uf_with(None, None, None);
    assert_eq!(uf.effective_width(Some(8)), Some(8));
    assert_eq!(uf.effective_width(None), None);
}

#[test]
fn effective_width_default_used_when_no_explicit_width() {
    // DEFAULT=6, no explicit width at point of use → 6.
    let uf = uf_with(None, None, Some(6));
    assert_eq!(uf.effective_width(None), Some(6));
    // Explicit width still wins over DEFAULT=.
    assert_eq!(uf.effective_width(Some(10)), Some(10));
}

#[test]
fn effective_width_no_default_falls_back_to_max_label_length() {
    // No DEFAULT= → computed as the longest label ("ABCDE", len 5), which
    // includes the OTHER= label ("Z", len 1) in the comparison. MIN=1 is set
    // (so we're off the all-None fast path) but is a no-op here since 5 >= 1.
    let uf = uf_with(Some(1), None, None);
    assert_eq!(uf.effective_width(None), Some(5));
}

#[test]
fn effective_width_min_clamps_explicit_width() {
    let uf = uf_with(Some(10), None, None);
    assert_eq!(uf.effective_width(Some(3)), Some(10));
    // Width already >= MIN is untouched.
    assert_eq!(uf.effective_width(Some(20)), Some(20));
}

#[test]
fn effective_width_max_clamps_explicit_width() {
    let uf = uf_with(None, Some(4), None);
    assert_eq!(uf.effective_width(Some(20)), Some(4));
    assert_eq!(uf.effective_width(Some(2)), Some(2));
}

#[test]
fn effective_width_min_max_clamp_computed_default_width() {
    // No DEFAULT=, no explicit width → computed width = 5 ("ABCDE"), then
    // clamped by MIN=8 up to 8.
    let uf = uf_with(Some(8), None, None);
    assert_eq!(uf.effective_width(None), Some(8));

    // And clamped down by MAX=3.
    let uf2 = uf_with(None, Some(3), None);
    assert_eq!(uf2.effective_width(None), Some(3));
}

#[test]
fn effective_width_min_and_max_both_set() {
    let uf = uf_with(Some(3), Some(4), None);
    // Explicit width 1 → clamped up to MIN=3.
    assert_eq!(uf.effective_width(Some(1)), Some(3));
    // Explicit width 100 → clamped down to MAX=4.
    assert_eq!(uf.effective_width(Some(100)), Some(4));
    // Within range → untouched.
    assert_eq!(uf.effective_width(Some(3)), Some(3));
}

// ── M43.1 — FUZZ= boundary tolerance ────────────────────────────────────────

#[test]
fn fuzz_default_tiny_tolerance_does_not_affect_normal_integer_boundaries() {
    // No explicit FUZZ= → SAS's own tiny default (1e-12): integer-bound
    // fixtures (the overwhelming majority) are unaffected.
    let uf = UserFormat {
        is_char: false,
        ranges: vec![num_range(1.0, 3.0, false, false, "Low")],
        other: Some("High".to_string()),
        ..Default::default()
    };
    assert_eq!(uf.lookup(&Value::Num(3.0)), Some("Low"));
    assert_eq!(uf.lookup(&Value::Num(3.000001)), Some("High"));
}

#[test]
fn explicit_fuzz_makes_near_boundary_value_match_inclusive_range() {
    // low-<10 = 'Below', 10-high = 'AtLeast', with FUZZ=1e-6: a value that
    // is, due to floating point error, a hair under 10 (but within FUZZ)
    // should be treated as exactly 10 → matches 'AtLeast', not 'Below'.
    let uf = UserFormat {
        is_char: false,
        ranges: vec![
            Range {
                from: Bound::Low,
                to: Bound::Num(10.0),
                from_exclusive: false,
                to_exclusive: true,
                label: "Below".to_string(),
            },
            Range {
                from: Bound::Num(10.0),
                to: Bound::High,
                from_exclusive: false,
                to_exclusive: false,
                label: "AtLeast".to_string(),
            },
        ],
        other: None,
        fuzz: Some(1e-6),
        ..Default::default()
    };
    // 9.9999995 differs from 10 by 5e-7, within FUZZ=1e-6.
    assert_eq!(uf.lookup(&Value::Num(9.9999995)), Some("AtLeast"));
    // Without a FUZZ tolerance this large, a value clearly below the
    // boundary still falls in the exclusive-upper range.
    assert_eq!(uf.lookup(&Value::Num(9.9)), Some("Below"));
    // And clearly at/after the boundary still matches AtLeast.
    assert_eq!(uf.lookup(&Value::Num(10.0)), Some("AtLeast"));
    assert_eq!(uf.lookup(&Value::Num(15.0)), Some("AtLeast"));
}

#[test]
fn explicit_fuzz_excludes_near_boundary_value_from_exclusive_side() {
    // 5<-high = 'Above' (exclusive lower bound at 5), FUZZ=1e-6: a value a
    // hair above 5 (within FUZZ) is treated as AT the boundary, so it does
    // NOT match the exclusive range either — consistent "snap to boundary"
    // semantics on both sides.
    let uf = UserFormat {
        is_char: false,
        ranges: vec![Range {
            from: Bound::Num(5.0),
            to: Bound::High,
            from_exclusive: true,
            to_exclusive: false,
            label: "Above".to_string(),
        }],
        other: Some("AtMost".to_string()),
        fuzz: Some(1e-6),
        ..Default::default()
    };
    assert_eq!(uf.lookup(&Value::Num(5.0000005)), Some("AtMost"));
    assert_eq!(uf.lookup(&Value::Num(5.1)), Some("Above"));
}
