use super::*;

fn num_single(v: f64, label: &str) -> Range {
    Range {
        from: Bound::Num(v),
        to: Bound::Num(v),
        from_exclusive: false,
        to_exclusive: false,
        label: label.to_string(),
    }
}

fn num_range(lo: f64, hi: f64, from_excl: bool, to_excl: bool, label: &str) -> Range {
    Range {
        from: Bound::Num(lo),
        to: Bound::Num(hi),
        from_exclusive: from_excl,
        to_exclusive: to_excl,
        label: label.to_string(),
    }
}

// ── UserInformat tests (M18.2) ────────────────────────────────────────────

fn invalue_range(from: &str, to: &str, result: InformatValue) -> InformatRange {
    InformatRange {
        from: Bound::Char(from.to_string()),
        to: Bound::Char(to.to_string()),
        from_exclusive: false,
        to_exclusive: false,
        result,
    }
}

fn invalue_single(key: &str, result: InformatValue) -> InformatRange {
    invalue_range(key, key, result)
}

// ── UserPicture tests (M18.3) ─────────────────────────────────────────────

fn pic_low_high(template: &str, dir: PictureDirectives) -> UserPicture {
    UserPicture {
        ranges: vec![PictureRange {
            from: Bound::Low,
            to: Bound::High,
            from_exclusive: false,
            to_exclusive: false,
            template: template.to_string(),
            directives: dir,
        }],
        other: None,
    }
}

mod numeric;
mod picture;
