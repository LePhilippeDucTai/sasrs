use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// UserPicture (M18.3) — PICTURE maps numeric ranges → templated digit strings
// ─────────────────────────────────────────────────────────────────────────────

/// Optional directives attached to a picture template, written in parentheses
/// after the template string, e.g. `'000,009.99' (prefix='$' mult=100)`.
#[derive(Clone, Debug, Default)]
pub struct PictureDirectives {
    /// String prepended just before the first significant (printed) character.
    pub prefix: Option<String>,
    /// Multiplier applied to the value before extracting digits. SAS auto-derives
    /// a multiplier from the number of digit selectors after the decimal point
    /// when MULT= is not given; we replicate that (see [`UserPicture::render`]).
    pub mult: Option<f64>,
    /// Fill character used to pad leading positions when a `9` selector would
    /// otherwise print a (suppressed) leading zero. Default is a space.
    pub fill: Option<char>,
}

/// A single range in a PICTURE format: a numeric range → a picture template.
#[derive(Clone, Debug)]
pub struct PictureRange {
    pub from: Bound,
    pub to: Bound,
    pub from_exclusive: bool,
    pub to_exclusive: bool,
    /// The raw picture template, e.g. `"99/99/9999"` or `"000,009.99"`.
    pub template: String,
    pub directives: PictureDirectives,
}

/// User-defined PICTURE format (PROC FORMAT PICTURE). Applies to NUMERIC values.
///
/// # Template semantics (faithful v1)
///
/// A template is a string of *digit selectors* (`0` and `9`) interleaved with
/// *message characters* (anything else: `/`, `,`, `.`, `%`, space, letters…).
///
/// - `9` — a digit position that PRINTS its digit, including leading zeros.
/// - `0` — a digit position that SUPPRESSES a leading zero (renders as the fill
///   char until a significant digit has appeared, then prints normally).
///
/// SAS scans the template; the *leftmost* digit selector establishes whether
/// the picture suppresses leading zeros. We implement the common behaviour:
/// digit selectors are filled RIGHT-TO-LEFT from the integer representation of
/// the (scaled, rounded) value; message characters are copied verbatim into
/// their position; a message character that sits to the LEFT of all significant
/// digits is itself blanked when in the leading-zero-suppressed region (SAS
/// drops leading separators), otherwise printed.
///
/// ## Decimal handling & MULT
///
/// SAS pictures do not carry an implicit decimal point. To show decimals you
/// add digit selectors after a literal `.` and scale the value with MULT=. When
/// MULT= is omitted we AUTO-DERIVE it as `10^(number of digit selectors right
/// of the rightmost literal '.')`, matching SAS's documented default so that
/// `picture p low-high='009.99'` on `1.5` yields `1.50`.
///
/// ## Simplifications (documented)
///
/// - Negative numbers: the value is rendered from its magnitude and a leading
///   `-` is prepended (after any PREFIX). SAS's `PREFIX='-'`/picture-2-sign
///   nuances are not modelled; this is the common case.
/// - Rounding is half-away-from-zero on the scaled value.
/// - No `DATATYPE=`, no directives beyond PREFIX/MULT/FILL.
#[derive(Clone, Debug)]
pub struct UserPicture {
    pub ranges: Vec<PictureRange>,
    /// Fallback template (`other='...'`).
    pub other: Option<(String, PictureDirectives)>,
}

impl UserPicture {
    /// Render a numeric value through the matching picture range. Returns `None`
    /// when no range matches and there is no `other` (caller falls back).
    pub fn render(&self, v: &Value) -> Option<String> {
        let n = match v {
            Value::Num(n) => *n,
            _ => return None,
        };
        for range in &self.ranges {
            if Self::range_matches(range, n) {
                return Some(render_template(&range.template, &range.directives, n));
            }
        }
        self.other
            .as_ref()
            .map(|(tpl, dir)| render_template(tpl, dir, n))
    }

    pub(super) fn range_matches(range: &PictureRange, n: f64) -> bool {
        let from_ok = match &range.from {
            Bound::Low => true,
            Bound::High => false,
            Bound::Num(lo) => {
                if range.from_exclusive {
                    n > *lo
                } else {
                    n >= *lo
                }
            }
            Bound::Char(_) => false,
        };
        if !from_ok {
            return false;
        }
        match &range.to {
            Bound::High => true,
            Bound::Low => false,
            Bound::Num(hi) => {
                if range.to_exclusive {
                    n < *hi
                } else {
                    n <= *hi
                }
            }
            Bound::Char(_) => false,
        }
    }
}

/// Count digit selectors (`0`/`9`) to the right of the rightmost literal `.`
/// in the template — used to auto-derive MULT when not given explicitly.
pub(crate) fn decimal_selectors(template: &str) -> u32 {
    match template.rfind('.') {
        Some(pos) => template[pos + 1..]
            .chars()
            .filter(|c| *c == '0' || *c == '9')
            .count() as u32,
        None => 0,
    }
}

/// Core picture renderer.
///
/// Algorithm (faithful v1):
/// 1. Scale `n` by MULT (explicit or auto-derived), round to a non-negative
///    integer magnitude.
/// 2. Count the digit selectors (`0`/`9`) in the template; left-pad the scaled
///    integer's digit string to that many digits (leading zeros).
/// 3. Walk the template LEFT-TO-RIGHT. Each selector consumes the next digit;
///    message characters are copied verbatim. Leading-zero suppression: a `0`
///    selector blanks (fill char) a digit that lies in the leading run of zeros
///    (before the first non-zero digit of the whole padded integer); a `9`
///    selector always prints its digit. A message character that sits before
///    the first printed digit is also blanked (SAS drops leading separators).
/// 4. Apply PREFIX (before the first printed glyph) and a leading `-` for
///    negatives.
pub(crate) fn render_template(template: &str, dir: &PictureDirectives, n: f64) -> String {
    let fill = dir.fill.unwrap_or(' ');

    let n_selectors = template.chars().filter(|c| *c == '0' || *c == '9').count();

    // 1. Scale factor (MULT). Explicit wins; else 10^(decimal selectors).
    let mult = dir
        .mult
        .unwrap_or_else(|| 10f64.powi(decimal_selectors(template) as i32));

    let negative = n.is_sign_negative() && n != 0.0;
    let scaled = (n.abs() * mult).round();

    // 2. Digit string of the scaled magnitude, left-padded to n_selectors.
    let raw_digits: Vec<u8> = format!("{scaled:.0}")
        .bytes()
        .filter(|b| b.is_ascii_digit())
        .map(|b| b - b'0')
        .collect();
    let digits: Vec<u8> = if raw_digits.len() >= n_selectors {
        // Overflow: keep the rightmost n_selectors digits (drop high digits).
        raw_digits[raw_digits.len() - n_selectors..].to_vec()
    } else {
        let mut v = vec![0u8; n_selectors - raw_digits.len()];
        v.extend_from_slice(&raw_digits);
        v
    };

    // Index (into the padded digit string) of the first non-zero digit; every
    // digit before it is a leading zero eligible for suppression.
    let first_significant = digits.iter().position(|&d| d != 0);

    // 3. Walk the template left-to-right.
    let mut out = String::with_capacity(template.len() + 4);
    let mut sel_idx = 0usize; // index into `digits`
    let mut printed_any = false; // have we emitted a real (non-fill) glyph yet?

    for c in template.chars() {
        match c {
            '0' | '9' => {
                let d = digits[sel_idx];
                // Leading-zero suppression applies ONLY to `0` selectors: a `0`
                // selector blanks a digit that lies in the leading run of zeros
                // (before the first non-zero digit). A `9` selector ALWAYS
                // prints its digit, including leading zeros.
                let is_leading_zero = match first_significant {
                    Some(fs) => sel_idx < fs,
                    None => true, // value is all zeros → every position is "leading"
                };
                sel_idx += 1;
                if c == '0' && is_leading_zero {
                    out.push(fill); // suppressed leading zero
                } else {
                    out.push((b'0' + d) as char);
                    printed_any = true;
                }
            }
            other => {
                if printed_any {
                    out.push(other);
                } else {
                    out.push(fill); // drop leading message char
                }
            }
        }
    }

    // 4a. PREFIX — inserted before the first printed glyph, replacing the
    //     leading fill run it visually occupies.
    let mut body = out;
    if let Some(prefix) = &dir.prefix {
        let lead = body.chars().take_while(|c| *c == fill).count();
        let rest: String = body.chars().skip(lead).collect();
        body = format!("{prefix}{rest}");
    }

    // 4b. Negative sign.
    if negative {
        body = format!("-{body}");
    }

    body
}
