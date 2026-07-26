use super::*;
use chrono::NaiveDate;

// Helper: make a spec
fn spec(name: &str, w: Option<u16>, d: Option<u16>) -> FormatSpec {
    FormatSpec {
        name: name.to_string(),
        w,
        d,
    }
}

// ── Date day-number computation (verify by chrono, not hardcoded) ─────────

fn day_num(y: i32, m: u32, d: u32) -> f64 {
    let epoch = NaiveDate::from_ymd_opt(1960, 1, 1).unwrap();
    let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
    date.signed_duration_since(epoch).num_days() as f64
}

mod day;
mod fract;
mod informat;
