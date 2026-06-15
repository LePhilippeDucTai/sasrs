//! PROC FORMAT (jalon M4).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc format ; value sexfmt 1='Male' 2='Female' other='?' ;
//! value $cityfmt 'PAR'='Paris' ; run ;`
//!
//! - Parser chaque statement VALUE en `formats::userdef::UserFormat`
//!   (plages : valeur, `a-b`, `low-<b`, `a<-high`, listes virgule).
//! - Enregistrer dans `session.format_catalog` (nom upcase, `$` inclus
//!   pour les formats char). NOTE par format : "Format SEXFMT has been
//!   output." — en session seulement, pas de catalogue persistant
//!   (limitation documentée dans README).
//! - INVALUE (informats utilisateur) : M4+, ERROR propre d'ici là.
//!
//! ## Naming convention
//! Format names are registered WITHOUT a leading `$` transformation beyond
//! what the user writes. The `$` prefix is kept as part of the name, e.g.
//! `$CITYFMT`. `FormatCatalog::define` upcases the whole string, so the
//! stored key is `$CITYFMT`. When `FormatSpec::parse` sees `$CITYFMT.` it
//! produces `name="$CITYFMT"`, which matches the catalog key exactly.

use crate::error::{Result, SasError};
use crate::formats::userdef::{
    Bound, InformatRange, InformatValue, PictureDirectives, PictureRange, Range, UserFormat,
    UserInformat, UserPicture,
};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;

pub struct FormatAst {
    /// (nom, définition brute à parser en UserFormat)
    pub values: Vec<(String, UserFormat)>,
    /// (nom, définition brute à parser en UserInformat) — M18.2
    pub invalues: Vec<(String, UserInformat)>,
    /// (nom, définition brute à parser en UserPicture) — M18.3
    pub pictures: Vec<(String, UserPicture)>,
}

/// Parse `proc format; value ... ; [value ... ;] run;`
/// Called AFTER "proc format" has been consumed. Consumes through `run;`/`quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<FormatAst> {
    // Consume the trailing `;` of the `proc format` statement header.
    // There may be options between `proc format` and `;` (none supported yet).
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        // Skip any unrecognised proc-header options (none for FORMAT).
        ts.next();
    }

    let mut values: Vec<(String, UserFormat)> = Vec::new();
    let mut invalues: Vec<(String, UserInformat)> = Vec::new();
    let mut pictures: Vec<(String, UserPicture)> = Vec::new();

    loop {
        // Skip stray semicolons.
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }

        if ts.peek().kind == TokenKind::Eof {
            break;
        }

        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }

        if ts.peek().is_kw("value") {
            ts.next(); // consume "value"
            let (name, uf) = parse_value_stmt(ts)?;
            values.push((name, uf));
        } else if ts.peek().is_kw("invalue") {
            ts.next(); // consume "invalue"
            let (name, ui) = parse_invalue_stmt(ts)?;
            invalues.push((name, ui));
        } else if ts.peek().is_kw("picture") {
            ts.next(); // consume "picture"
            let (name, up) = parse_picture_stmt(ts)?;
            pictures.push((name, up));
        } else {
            // Unknown sub-statement: skip it.
            ts.skip_to_semi();
        }
    }

    Ok(FormatAst { values, invalues, pictures })
}

/// Parse one VALUE statement (after the "value" keyword has been consumed):
///   [$]<fmtname>  <range>='label'  [<range>='label' ...] [other='label'] ;
fn parse_value_stmt(ts: &mut StatementStream) -> Result<(String, UserFormat)> {
    // --- format name: optional `$` then identifier ---
    let is_char = ts.peek().kind == TokenKind::Dollar;
    let dollar_span = ts.peek().span;
    if is_char {
        ts.next(); // consume `$`
    }

    let name_tok = ts.peek().clone();
    let base_name = match name_tok.ident() {
        Some(n) => n.to_string(),
        None => {
            return Err(SasError::parse(
                "expected a format name after VALUE",
                name_tok.span,
            ));
        }
    };
    ts.next();

    // Build the stored name: include the `$` prefix for char formats.
    let name = if is_char {
        let _ = dollar_span; // used above for is_char detection
        format!("${}", base_name)
    } else {
        base_name
    };

    // --- parse range='label' pairs until `;` ---
    let mut ranges: Vec<Range> = Vec::new();
    let mut other: Option<String> = None;

    loop {
        // Skip stray semicolons within the statement (there should not be any,
        // but be defensive). A real `;` ends the statement.
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        // `run` / `quit` as step terminator (in case `;` was already consumed).
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            break;
        }

        // OTHER keyword.
        if ts.peek().is_kw("other") {
            ts.next(); // consume "other"
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after OTHER",
                    ts.peek().span,
                ));
            }
            ts.next(); // consume `=`
            let lbl = parse_string_literal(ts)?;
            other = Some(lbl);
            continue;
        }

        // Parse one or more bounds that share a label (comma list or range).
        // Collect all ranges for this label-group.
        let group_ranges = parse_range_group(ts, is_char)?;

        // Now expect `=` then label.
        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' after range specification",
                ts.peek().span,
            ));
        }
        ts.next(); // consume `=`

        let label = parse_string_literal(ts)?;

        // Assign label to every range in the group.
        for mut r in group_ranges {
            r.label = label.clone();
            ranges.push(r);
        }
    }

    let uf = UserFormat { is_char, ranges, other };
    Ok((name, uf))
}

// ── PICTURE parsing (M18.3) ──────────────────────────────────────────────────

/// Parse one PICTURE statement (after the "picture" keyword has been consumed):
///   <picname>  <range>='template' [(dirs)]  [<range>='template' [(dirs)]] ... ;
///
/// PICTURE formats are always numeric (no `$`). Each range maps to a picture
/// template string, optionally followed by parenthesised directives
/// (`PREFIX=` / `MULT=` / `FILL=`).
fn parse_picture_stmt(ts: &mut StatementStream) -> Result<(String, UserPicture)> {
    let name_tok = ts.peek().clone();
    let name = match name_tok.ident() {
        Some(n) => n.to_string(),
        None => {
            return Err(SasError::parse(
                "expected a picture name after PICTURE",
                name_tok.span,
            ));
        }
    };
    ts.next();

    let mut ranges: Vec<PictureRange> = Vec::new();
    let mut other: Option<(String, PictureDirectives)> = None;

    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            break;
        }

        // OTHER keyword → fallback template.
        if ts.peek().is_kw("other") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after OTHER in PICTURE",
                    ts.peek().span,
                ));
            }
            ts.next(); // `=`
            let template = parse_string_literal(ts)?;
            let directives = parse_picture_directives(ts)?;
            other = Some((template, directives));
            continue;
        }

        // A group of numeric ranges sharing one template (comma list).
        let group = parse_picture_range_group(ts)?;

        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' after range specification in PICTURE",
                ts.peek().span,
            ));
        }
        ts.next(); // `=`

        let template = parse_string_literal(ts)?;
        let directives = parse_picture_directives(ts)?;

        for (from, to, from_excl, to_excl) in group {
            ranges.push(PictureRange {
                from,
                to,
                from_exclusive: from_excl,
                to_exclusive: to_excl,
                template: template.clone(),
                directives: directives.clone(),
            });
        }
    }

    Ok((name, UserPicture { ranges, other }))
}

/// Parse a comma-separated group of numeric picture ranges (no template yet).
/// Returns tuples `(from, to, from_exclusive, to_exclusive)`.
fn parse_picture_range_group(
    ts: &mut StatementStream,
) -> Result<Vec<(Bound, Bound, bool, bool)>> {
    let mut out = Vec::new();
    loop {
        // Reuse the numeric VALUE range parser (is_char = false).
        let r = parse_single_range(ts, false)?;
        out.push((r.from, r.to, r.from_exclusive, r.to_exclusive));
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
        } else {
            break;
        }
    }
    Ok(out)
}

/// Parse the optional `(PREFIX='...' MULT=n FILL='c' ...)` directive list that
/// follows a picture template. Returns defaults when no `(` is present.
/// Directives are space-separated `KEY=VALUE` pairs.
fn parse_picture_directives(ts: &mut StatementStream) -> Result<PictureDirectives> {
    let mut dir = PictureDirectives::default();
    if ts.peek().kind != TokenKind::LParen {
        return Ok(dir);
    }
    ts.next(); // `(`

    loop {
        if ts.peek().kind == TokenKind::RParen {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            return Err(SasError::parse(
                "unterminated directive list in PICTURE (missing ')')",
                ts.peek().span,
            ));
        }

        let key_tok = ts.peek().clone();
        let key = match key_tok.ident() {
            Some(k) => k.to_lowercase(),
            None => {
                return Err(SasError::parse(
                    "expected a directive name (PREFIX/MULT/FILL) in PICTURE",
                    key_tok.span,
                ));
            }
        };
        ts.next();

        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' after directive name in PICTURE",
                ts.peek().span,
            ));
        }
        ts.next(); // `=`

        match key.as_str() {
            "prefix" => {
                dir.prefix = Some(parse_string_literal(ts)?);
            }
            "fill" => {
                let s = parse_string_literal(ts)?;
                dir.fill = s.chars().next();
            }
            "mult" | "multiplier" => {
                // MULT=n — a (possibly negative, possibly decimal) number.
                let negative = if ts.peek().kind == TokenKind::Minus
                    && matches!(ts.peek2().kind, TokenKind::Num(_))
                {
                    ts.next();
                    true
                } else {
                    false
                };
                match ts.peek().kind.clone() {
                    TokenKind::Num(n) => {
                        ts.next();
                        dir.mult = Some(if negative { -n } else { n });
                    }
                    _ => {
                        return Err(SasError::parse(
                            "expected a number after MULT= in PICTURE",
                            ts.peek().span,
                        ));
                    }
                }
            }
            other => {
                return Err(SasError::parse(
                    format!("unsupported PICTURE directive '{other}'"),
                    key_tok.span,
                ));
            }
        }
    }

    Ok(dir)
}

/// Parse a comma-separated list of range specs that share a single label.
/// Each element is a bound or a bound-range (`a-b`, `low-<b`, etc.).
/// Returns a Vec of Range with empty labels (caller fills them in).
fn parse_range_group(ts: &mut StatementStream, is_char: bool) -> Result<Vec<Range>> {
    let mut out: Vec<Range> = Vec::new();

    loop {
        let r = parse_single_range(ts, is_char)?;
        out.push(r);

        // If next token is a comma, consume it and continue with another range.
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
            // After comma there must be another range before `=`.
        } else {
            break;
        }
    }

    Ok(out)
}

/// Parse a single bound or range:
///   single:  `1`  or  `'PAR'`
///   range:   `1-3`  |  `low-<5`  |  `5<-high`  |  `1<-<100`
///            `low-5`  |  `1-high`  etc.
///
/// Exclusivity encoding (`<` next to `-`):
///   `a-<b`  → from_exclusive=false, to_exclusive=true
///   `a<-b`  → from_exclusive=true,  to_exclusive=false
///   `a<-<b` → from_exclusive=true,  to_exclusive=true
fn parse_single_range(ts: &mut StatementStream, is_char: bool) -> Result<Range> {
    // Parse the "from" bound.
    let (from_bound, from_lt_before_minus) = parse_bound_with_lt(ts, is_char)?;

    // Check if there is a `-` or `<-` token sequence starting a range.
    // After the from bound:
    //   Case A: Next is `-`       → simple `-`, to_exclusive stays false by default.
    //   Case B: Next is `<` then `-` (already consumed `<` as from_lt_before_minus)
    //      → from_exclusive=true, to_exclusive depends on `<` after `-`.
    //
    // from_lt_before_minus == true means we already consumed a `<` between the
    // from value and the `-` (i.e. `5 < - high`).

    // Now decide if there's a range at all.
    // If the next token is `=` or `,` or `;` or EOF or step-boundary → single value.
    let has_range = match ts.peek().kind {
        TokenKind::Minus => true,
        _ => false,
    };

    if !has_range {
        // Single value: from == to, no exclusivity.
        // from_lt_before_minus being true here would be a parse error, but
        // we'll just ignore it and treat as single value.
        let to = from_bound.clone();
        return Ok(Range {
            from: from_bound,
            to,
            from_exclusive: false,
            to_exclusive: false,
            label: String::new(),
        });
    }

    // Consume the `-`.
    ts.next(); // `-`

    // Check for `<` immediately after `-` → to_exclusive = true.
    let to_exclusive = if ts.peek().kind == TokenKind::Lt {
        ts.next(); // consume `<`
        true
    } else {
        false
    };

    // Parse the "to" bound.
    let (to_bound, _) = parse_bound_with_lt(ts, is_char)?;

    Ok(Range {
        from: from_bound,
        to: to_bound,
        from_exclusive: from_lt_before_minus,
        to_exclusive,
        label: String::new(),
    })
}


/// Parse a bound token, also detecting a leading `<` before the `-` dash
/// (which would indicate from_exclusive=true for a range like `5<-high`).
///
/// Returns `(Bound, had_lt_before_dash)`.
/// The `had_lt_before_dash` is true when we see `<` and the *next* token is
/// `-` (so `5<-high`). We consume the `<` in that case.
fn parse_bound_with_lt(
    ts: &mut StatementStream,
    is_char: bool,
) -> Result<(Bound, bool)> {
    // Check for a leading `<` that precedes `-` (from_exclusive pattern).
    // We need lookahead: peek is `<` and peek2 is `-`.
    // Actually the `<` comes AFTER the bound value in `5<-high`:
    //   tokens: Num(5), Lt, Minus, Ident("high")
    // So we parse the bound normally, then check for `<` before `-`.

    let bound = parse_bound(ts, is_char)?;

    // After the bound, check for `<` immediately followed by `-` → from_exclusive.
    let had_lt = if ts.peek().kind == TokenKind::Lt && ts.peek2().kind == TokenKind::Minus {
        ts.next(); // consume `<`
        true
    } else {
        false
    };

    Ok((bound, had_lt))
}

/// Parse a single bound value (LOW, HIGH, a number, or a quoted string).
fn parse_bound(ts: &mut StatementStream, is_char: bool) -> Result<Bound> {
    if ts.peek().is_kw("low") {
        ts.next();
        return Ok(Bound::Low);
    }
    if ts.peek().is_kw("high") {
        ts.next();
        return Ok(Bound::High);
    }

    if is_char {
        // Character bound: must be a string literal.
        let s = parse_string_literal(ts)?;
        return Ok(Bound::Char(s));
    }

    // Numeric bound: a number literal.
    // Handle optional leading minus sign (negative numbers).
    let negative = if ts.peek().kind == TokenKind::Minus {
        // But only if the next-next is a number (not another operator).
        // Peek2 check: is the token after `-` a Num?
        if matches!(ts.peek2().kind, TokenKind::Num(_)) {
            ts.next(); // consume `-`
            true
        } else {
            false
        }
    } else {
        false
    };

    match ts.peek().kind.clone() {
        TokenKind::Num(n) => {
            ts.next();
            let v = if negative { -n } else { n };
            Ok(Bound::Num(v))
        }
        _ => Err(SasError::parse(
            "expected a numeric bound (number, LOW, or HIGH)",
            ts.peek().span,
        )),
    }
}

/// Parse a quoted string literal and return its content.
fn parse_string_literal(ts: &mut StatementStream) -> Result<String> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Str { value, .. } => {
            let s = value.clone();
            ts.next();
            Ok(s)
        }
        _ => Err(SasError::parse(
            "expected a quoted string literal",
            tok.span,
        )),
    }
}

// ── INVALUE parsing (M18.2) ──────────────────────────────────────────────────

/// Parse one INVALUE statement (after the "invalue" keyword has been consumed):
///   [$]<inforname>  'key'=value  ['key'=value ...] [other=value] ;
///
/// Keys are always character strings (quoted); the result (`value`) is:
///   - a numeric literal   → `InformatValue::Num(f64)`
///   - a quoted string     → `InformatValue::Char(String)`
///   - `_SAME_` keyword    → `InformatValue::Same`
///   - `.`/`._`/`.A`..`.Z`→ `InformatValue::Missing(kind_str)`
fn parse_invalue_stmt(ts: &mut StatementStream) -> Result<(String, UserInformat)> {
    // --- informat name: optional `$` then identifier ---
    let is_char_result = ts.peek().kind == TokenKind::Dollar;
    if is_char_result {
        ts.next(); // consume `$`
    }

    let name_tok = ts.peek().clone();
    let base_name = match name_tok.ident() {
        Some(n) => n.to_string(),
        None => {
            return Err(SasError::parse(
                "expected an informat name after INVALUE",
                name_tok.span,
            ));
        }
    };
    ts.next();

    // Build stored name (include `$` prefix for char informats).
    let name = if is_char_result {
        format!("${}", base_name)
    } else {
        base_name
    };

    // --- parse 'key'=result pairs until `;` ---
    let mut ranges: Vec<InformatRange> = Vec::new();
    let mut other: Option<InformatValue> = None;

    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            break;
        }

        // OTHER keyword.
        if ts.peek().is_kw("other") {
            ts.next(); // consume "other"
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after OTHER in INVALUE",
                    ts.peek().span,
                ));
            }
            ts.next(); // consume `=`
            let iv = parse_informat_value(ts)?;
            other = Some(iv);
            continue;
        }

        // Parse a group of key ranges sharing a single result value.
        // Keys are always character strings (quoted strings or LOW/HIGH).
        let group_ranges = parse_invalue_range_group(ts)?;

        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' after key range specification in INVALUE",
                ts.peek().span,
            ));
        }
        ts.next(); // consume `=`

        let result = parse_informat_value(ts)?;

        for mut r in group_ranges {
            r.result = result.clone();
            ranges.push(r);
        }
    }

    let ui = UserInformat { is_char_result, ranges, other };
    Ok((name, ui))
}

/// Parse a comma-separated group of character key ranges for INVALUE.
/// Returns Vec<InformatRange> with placeholder `result` values (caller fills in).
fn parse_invalue_range_group(ts: &mut StatementStream) -> Result<Vec<InformatRange>> {
    let mut out: Vec<InformatRange> = Vec::new();
    loop {
        let r = parse_invalue_single_range(ts)?;
        out.push(r);
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
        } else {
            break;
        }
    }
    Ok(out)
}

/// Parse a single key range for INVALUE: `'A'`, `'A'-'Z'`, `low-'C'`, etc.
fn parse_invalue_single_range(ts: &mut StatementStream) -> Result<InformatRange> {
    // INVALUE keys are always char-mode bounds.
    let (from_bound, from_lt_before_minus) = parse_invalue_bound_with_lt(ts)?;

    let has_range = ts.peek().kind == TokenKind::Minus;

    if !has_range {
        let to = from_bound.clone();
        return Ok(InformatRange {
            from: from_bound,
            to,
            from_exclusive: false,
            to_exclusive: false,
            result: InformatValue::Same, // placeholder, caller replaces
        });
    }

    ts.next(); // consume `-`

    let to_exclusive = if ts.peek().kind == TokenKind::Lt {
        ts.next(); // consume `<`
        true
    } else {
        false
    };

    let (to_bound, _) = parse_invalue_bound_with_lt(ts)?;

    Ok(InformatRange {
        from: from_bound,
        to: to_bound,
        from_exclusive: from_lt_before_minus,
        to_exclusive,
        result: InformatValue::Same, // placeholder, caller replaces
    })
}

/// Parse an INVALUE key bound with optional leading `<` (from_exclusive marker).
fn parse_invalue_bound_with_lt(ts: &mut StatementStream) -> Result<(Bound, bool)> {
    let bound = parse_invalue_bound(ts)?;
    // Check for `<` immediately followed by `-` (from_exclusive pattern).
    let had_lt = if ts.peek().kind == TokenKind::Lt && ts.peek2().kind == TokenKind::Minus {
        ts.next(); // consume `<`
        true
    } else {
        false
    };
    Ok((bound, had_lt))
}

/// Parse one INVALUE key bound: LOW, HIGH, or a quoted string.
fn parse_invalue_bound(ts: &mut StatementStream) -> Result<Bound> {
    if ts.peek().is_kw("low") {
        ts.next();
        return Ok(Bound::Low);
    }
    if ts.peek().is_kw("high") {
        ts.next();
        return Ok(Bound::High);
    }
    // Must be a quoted string.
    let s = parse_string_literal(ts)?;
    Ok(Bound::Char(s))
}

/// Parse the result value on the right-hand side of `=` in an INVALUE mapping:
///   numeric literal  → `InformatValue::Num`
///   quoted string    → `InformatValue::Char`
///   `_SAME_`         → `InformatValue::Same`
///   `.` / `._` / `.A`..`.Z` → `InformatValue::Missing`
fn parse_informat_value(ts: &mut StatementStream) -> Result<InformatValue> {
    // `_SAME_` keyword (identifier).
    if let Some(id) = ts.peek().ident() {
        if id.eq_ignore_ascii_case("_same_") {
            ts.next();
            return Ok(InformatValue::Same);
        }
    }

    // Missing value: starts with `.`
    if ts.peek().kind == TokenKind::Dot {
        ts.next(); // consume `.`
        // Check for special suffix: `_` or letter.
        if let Some(id) = ts.peek().ident() {
            let s = id.to_uppercase();
            if s == "_" || (s.len() == 1 && s.chars().next().unwrap().is_ascii_uppercase()) {
                ts.next();
                return Ok(InformatValue::Missing(s));
            }
        }
        return Ok(InformatValue::Missing(".".to_string()));
    }

    // Quoted string → character result.
    if let TokenKind::Str { value, .. } = &ts.peek().kind.clone() {
        let s = value.clone();
        ts.next();
        return Ok(InformatValue::Char(s));
    }

    // Numeric literal (possibly negative).
    let negative = if ts.peek().kind == TokenKind::Minus {
        if matches!(ts.peek2().kind, TokenKind::Num(_)) {
            ts.next(); // consume `-`
            true
        } else {
            false
        }
    } else {
        false
    };

    match ts.peek().kind.clone() {
        TokenKind::Num(n) => {
            ts.next();
            let v = if negative { -n } else { n };
            Ok(InformatValue::Num(v))
        }
        _ => Err(SasError::parse(
            "expected a result value (number, quoted string, _SAME_, or missing) in INVALUE",
            ts.peek().span,
        )),
    }
}

pub fn execute(ast: &FormatAst, session: &mut Session) -> Result<()> {
    for (name, uf) in &ast.values {
        let uname = name.to_uppercase();
        session.log.note(&format!("Format {} has been output.", uname));
        session.format_catalog.define(&uname, uf.clone());
    }
    for (name, ui) in &ast.invalues {
        let uname = name.to_uppercase();
        session.log.note(&format!("Informat {} has been output.", uname));
        session.format_catalog.define_informat(&uname, ui.clone());
    }
    for (name, up) in &ast.pictures {
        let uname = name.to_uppercase();
        session.log.note(&format!("Format {} has been output.", uname));
        session.format_catalog.define_picture(&uname, up.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use crate::source::SourceFile;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_format_src(src: &str) -> Result<FormatAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "format"
        parse(&mut ts)
    }

    // ── parse tests ───────────────────────────────────────────────────────────

    #[test]
    fn parse_minimal_empty() {
        let ast = parse_format_src("proc format; run;").unwrap();
        assert!(ast.values.is_empty());
    }

    #[test]
    fn parse_single_value_numeric() {
        let ast = parse_format_src(
            "proc format; value sexfmt 1='Male' 2='Female'; run;",
        )
        .unwrap();
        assert_eq!(ast.values.len(), 1);
        let (name, uf) = &ast.values[0];
        assert_eq!(name, "sexfmt");
        assert!(!uf.is_char);
        assert_eq!(uf.ranges.len(), 2);
        assert_eq!(uf.ranges[0].label, "Male");
        assert_eq!(uf.ranges[1].label, "Female");
    }

    #[test]
    fn parse_char_format_with_dollar() {
        let ast = parse_format_src(
            "proc format; value $cityfmt 'PAR'='Paris' 'NYC'='New York'; run;",
        )
        .unwrap();
        assert_eq!(ast.values.len(), 1);
        let (name, uf) = &ast.values[0];
        assert_eq!(name, "$cityfmt");
        assert!(uf.is_char);
        assert_eq!(uf.ranges.len(), 2);
        assert_eq!(uf.ranges[0].label, "Paris");
        assert_eq!(uf.ranges[1].label, "New York");
    }

    #[test]
    fn parse_inclusive_range() {
        let ast = parse_format_src(
            "proc format; value agefmt 0-17='Child' 18-64='Adult' 65-high='Senior'; run;",
        )
        .unwrap();
        let (_, uf) = &ast.values[0];
        assert_eq!(uf.ranges.len(), 3);
        // 0-17: from=Num(0), to=Num(17), both inclusive
        assert!(matches!(uf.ranges[0].from, Bound::Num(n) if n == 0.0));
        assert!(matches!(uf.ranges[0].to, Bound::Num(n) if n == 17.0));
        assert!(!uf.ranges[0].from_exclusive);
        assert!(!uf.ranges[0].to_exclusive);
        // 65-high
        assert!(matches!(uf.ranges[2].from, Bound::Num(n) if n == 65.0));
        assert!(matches!(uf.ranges[2].to, Bound::High));
    }

    #[test]
    fn parse_low_exclusive_upper() {
        // low-<5='Below5'
        let ast = parse_format_src(
            "proc format; value f low-<5='Below5' 5-high='AtLeast5'; run;",
        )
        .unwrap();
        let (_, uf) = &ast.values[0];
        assert!(matches!(uf.ranges[0].from, Bound::Low));
        assert!(matches!(uf.ranges[0].to, Bound::Num(n) if n == 5.0));
        assert!(!uf.ranges[0].from_exclusive);
        assert!(uf.ranges[0].to_exclusive);
    }

    #[test]
    fn parse_exclusive_lower_to_high() {
        // 5<-high='Above5'
        let ast = parse_format_src(
            "proc format; value f low-5='AtMost5' 5<-high='Above5'; run;",
        )
        .unwrap();
        let (_, uf) = &ast.values[0];
        // Second range: 5<-high
        assert!(matches!(uf.ranges[1].from, Bound::Num(n) if n == 5.0));
        assert!(uf.ranges[1].from_exclusive);
        assert!(!uf.ranges[1].to_exclusive);
        assert!(matches!(uf.ranges[1].to, Bound::High));
    }

    #[test]
    fn parse_both_exclusive() {
        // 1<-<10='Middle'
        let ast = parse_format_src(
            "proc format; value f 1<-<10='Middle'; run;",
        )
        .unwrap();
        let (_, uf) = &ast.values[0];
        assert!(uf.ranges[0].from_exclusive);
        assert!(uf.ranges[0].to_exclusive);
    }

    #[test]
    fn parse_comma_list() {
        // 1,2,3='Group'  → 3 ranges with same label
        let ast = parse_format_src(
            "proc format; value f 1,2,3='Group'; run;",
        )
        .unwrap();
        let (_, uf) = &ast.values[0];
        assert_eq!(uf.ranges.len(), 3);
        for r in &uf.ranges {
            assert_eq!(r.label, "Group");
        }
    }

    #[test]
    fn parse_other() {
        let ast = parse_format_src(
            "proc format; value f 1='One' other='Unknown'; run;",
        )
        .unwrap();
        let (_, uf) = &ast.values[0];
        assert_eq!(uf.other, Some("Unknown".to_string()));
        assert_eq!(uf.ranges.len(), 1);
    }

    #[test]
    fn parse_multiple_value_stmts() {
        let ast = parse_format_src(
            "proc format; value a 1='x'; value b 2='y'; run;",
        )
        .unwrap();
        assert_eq!(ast.values.len(), 2);
        assert_eq!(ast.values[0].0, "a");
        assert_eq!(ast.values[1].0, "b");
    }

    #[test]
    fn parse_invalue_numeric_basic() {
        // INVALUE without $ → numeric result.
        let ast = parse_format_src(
            "proc format; invalue grade 'A'=4 'B'=3 'C'=2 'D'=1 'F'=0; run;",
        )
        .unwrap();
        assert_eq!(ast.invalues.len(), 1);
        let (name, ui) = &ast.invalues[0];
        assert_eq!(name, "grade");
        assert!(!ui.is_char_result);
        assert_eq!(ui.ranges.len(), 5);
        // First range: 'A'=4
        assert!(matches!(ui.ranges[0].from, Bound::Char(ref s) if s == "A"));
        assert!(matches!(ui.ranges[0].result, InformatValue::Num(n) if n == 4.0));
        // Last range: 'F'=0
        assert!(matches!(ui.ranges[4].result, InformatValue::Num(n) if n == 0.0));
    }

    #[test]
    fn parse_invalue_char_with_dollar() {
        // INVALUE with $ → character result.
        let ast = parse_format_src(
            "proc format; invalue $size 'S'='Small' 'M'='Medium' 'L'='Large'; run;",
        )
        .unwrap();
        assert_eq!(ast.invalues.len(), 1);
        let (name, ui) = &ast.invalues[0];
        assert_eq!(name, "$size");
        assert!(ui.is_char_result);
        assert_eq!(ui.ranges.len(), 3);
        assert!(matches!(&ui.ranges[0].result, InformatValue::Char(s) if s == "Small"));
        assert!(matches!(&ui.ranges[2].result, InformatValue::Char(s) if s == "Large"));
    }

    #[test]
    fn parse_invalue_other_and_same() {
        // `_same_` → Same variant; `other=.` (unquoted dot) → Missing.
        let ast = parse_format_src(
            "proc format; invalue $code low-'Z'=_same_ other=.; run;",
        )
        .unwrap();
        let (_, ui) = &ast.invalues[0];
        assert!(matches!(ui.ranges[0].result, InformatValue::Same));
        assert!(matches!(ui.other, Some(InformatValue::Missing(_))));
    }

    #[test]
    fn parse_invalue_quoted_string_other() {
        // `other='?'` → Char variant (quoted string result).
        let ast = parse_format_src(
            "proc format; invalue $code 'A'='Alpha' other='?'; run;",
        )
        .unwrap();
        let (_, ui) = &ast.invalues[0];
        assert!(matches!(&ui.other, Some(InformatValue::Char(s)) if s == "?"));
    }

    #[test]
    fn parse_invalue_range_with_exclusion() {
        let ast = parse_format_src(
            "proc format; invalue f 'A'-<'Z'=1; run;",
        )
        .unwrap();
        let (_, ui) = &ast.invalues[0];
        assert!(!ui.ranges[0].from_exclusive);
        assert!(ui.ranges[0].to_exclusive);
    }

    #[test]
    fn parse_invalue_mixed_with_value() {
        // Can have both VALUE and INVALUE in same PROC FORMAT.
        let ast = parse_format_src(
            "proc format; value sexfmt 1='Male'; invalue grade 'A'=4; run;",
        )
        .unwrap();
        assert_eq!(ast.values.len(), 1);
        assert_eq!(ast.invalues.len(), 1);
    }

    // ── execute tests ─────────────────────────────────────────────────────────

    #[test]
    fn execute_registers_format_in_catalog() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let mut session = make_session();
        let ast = FormatAst {
            values: vec![(
                "SEXFMT".to_string(),
                UserFormat {
                    is_char: false,
                    ranges: vec![
                        crate::formats::userdef::Range {
                            from: Bound::Num(1.0),
                            to: Bound::Num(1.0),
                            from_exclusive: false,
                            to_exclusive: false,
                            label: "Male".to_string(),
                        },
                        crate::formats::userdef::Range {
                            from: Bound::Num(2.0),
                            to: Bound::Num(2.0),
                            from_exclusive: false,
                            to_exclusive: false,
                            label: "Female".to_string(),
                        },
                    ],
                    other: Some("Unknown".to_string()),
                },
            )],
            invalues: vec![],
            pictures: vec![],
        };

        execute(&ast, &mut session).unwrap();

        // Verify it's in the catalog.
        let spec = FormatSpec::parse("SEXFMT.").unwrap();
        let result = session.format_catalog.format(&Value::Num(1.0), &spec);
        // Right-justified to w=0 (no width in spec) → label as-is.
        assert!(result.contains("Male"), "result: {result}");

        // NOTE logged.
        let log = session.log.into_string();
        assert!(log.contains("Format SEXFMT has been output."), "log: {log}");
    }

    #[test]
    fn execute_round_trip_parse_and_execute() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let mut session = make_session();
        let source = SourceFile::new(
            "proc format; value sexfmt 1='Male' 2='Female' other='?'; run;",
        );
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // format
        let ast = parse(&mut ts).unwrap();
        execute(&ast, &mut session).unwrap();

        let spec = FormatSpec::parse("SEXFMT.").unwrap();
        assert_eq!(
            session.format_catalog.format(&Value::Num(1.0), &spec),
            "Male"
        );
        assert_eq!(
            session.format_catalog.format(&Value::Num(2.0), &spec),
            "Female"
        );
        assert_eq!(
            session.format_catalog.format(&Value::Num(99.0), &spec),
            "?"
        );
    }

    // ── INVALUE execute tests (M18.2) ─────────────────────────────────────────

    fn run_format_src(src: &str) -> crate::session::Session {
        let mut session = make_session();
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // format
        let ast = parse(&mut ts).unwrap();
        execute(&ast, &mut session).unwrap();
        session
    }

    #[test]
    fn execute_invalue_numeric_registered_in_catalog() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let session = run_format_src(
            "proc format; invalue grade 'A'=4 'B'=3 'C'=2 'D'=1 'F'=0; run;",
        );

        let spec = FormatSpec::parse("GRADE.").unwrap();
        assert_eq!(session.format_catalog.informat("A", &spec), Value::Num(4.0));
        assert_eq!(session.format_catalog.informat("B", &spec), Value::Num(3.0));
        assert_eq!(session.format_catalog.informat("F", &spec), Value::Num(0.0));

        // NOTE logged for informat.
        let log = session.log.into_string();
        assert!(log.contains("Informat GRADE has been output."), "log: {log}");
    }

    #[test]
    fn execute_invalue_char_dollar_registered() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let session = run_format_src(
            "proc format; invalue $size 'S'='Small' 'M'='Medium' 'L'='Large'; run;",
        );

        let spec = FormatSpec::parse("$SIZE.").unwrap();
        assert_eq!(session.format_catalog.informat("S", &spec), Value::Char("Small".to_string()));
        assert_eq!(session.format_catalog.informat("M", &spec), Value::Char("Medium".to_string()));
        assert_eq!(session.format_catalog.informat("L", &spec), Value::Char("Large".to_string()));
    }

    #[test]
    fn execute_invalue_unmatched_returns_missing() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let session = run_format_src(
            "proc format; invalue grade 'A'=4 'B'=3; run;",
        );

        let spec = FormatSpec::parse("GRADE.").unwrap();
        // "X" not matched, no other → missing.
        assert_eq!(session.format_catalog.informat("X", &spec), Value::missing());
    }

    #[test]
    fn execute_invalue_other_fallback() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let session = run_format_src(
            "proc format; invalue grade 'A'=4 'B'=3 other=.; run;",
        );

        let spec = FormatSpec::parse("GRADE.").unwrap();
        assert_eq!(session.format_catalog.informat("A", &spec), Value::Num(4.0));
        assert_eq!(session.format_catalog.informat("Z", &spec), Value::missing());
    }

    #[test]
    fn execute_invalue_and_value_coexist() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let session = run_format_src(
            "proc format; \
             value sexfmt 1='Male' 2='Female'; \
             invalue grade 'A'=4 'B'=3; \
             run;",
        );

        // VALUE format still works.
        let fspec = FormatSpec::parse("SEXFMT.").unwrap();
        assert_eq!(session.format_catalog.format(&Value::Num(1.0), &fspec), "Male");

        // INVALUE informat also works.
        let ispec = FormatSpec::parse("GRADE.").unwrap();
        assert_eq!(session.format_catalog.informat("A", &ispec), Value::Num(4.0));
    }

    // ── PICTURE parse tests (M18.3) ───────────────────────────────────────────

    #[test]
    fn parse_picture_string_bounds_rejected() {
        // PICTURE is numeric-only: quoted (string) bounds are a parse error.
        let ast = parse_format_src(
            "proc format; picture mmddyy '01'-'12' = '99/99/9999'; run;",
        );
        assert!(ast.is_err());
    }

    #[test]
    fn parse_picture_numeric_range_template() {
        let ast = parse_format_src(
            "proc format; picture mmddyy low-high = '99/99/9999'; run;",
        )
        .unwrap();
        assert_eq!(ast.pictures.len(), 1);
        let (name, up) = &ast.pictures[0];
        assert_eq!(name, "mmddyy");
        assert_eq!(up.ranges.len(), 1);
        assert_eq!(up.ranges[0].template, "99/99/9999");
        assert!(matches!(up.ranges[0].from, Bound::Low));
        assert!(matches!(up.ranges[0].to, Bound::High));
    }

    #[test]
    fn parse_picture_with_prefix_directive() {
        let ast = parse_format_src(
            "proc format; picture dollarpic low-high = '000,000,009.99' (prefix='$'); run;",
        )
        .unwrap();
        let (_, up) = &ast.pictures[0];
        assert_eq!(up.ranges[0].directives.prefix.as_deref(), Some("$"));
        assert_eq!(up.ranges[0].directives.mult, None);
        assert_eq!(up.ranges[0].directives.fill, None);
    }

    #[test]
    fn parse_picture_with_mult_and_fill() {
        let ast = parse_format_src(
            "proc format; picture pct other = '009.9%' (mult=100 fill='*'); run;",
        )
        .unwrap();
        let (_, up) = &ast.pictures[0];
        assert!(up.ranges.is_empty());
        let (tpl, dir) = up.other.as_ref().unwrap();
        assert_eq!(tpl, "009.9%");
        assert_eq!(dir.mult, Some(100.0));
        assert_eq!(dir.fill, Some('*'));
    }

    #[test]
    fn parse_picture_multiple_ranges() {
        let ast = parse_format_src(
            "proc format; picture p 0-9='9' 10-high='999'; run;",
        )
        .unwrap();
        let (_, up) = &ast.pictures[0];
        assert_eq!(up.ranges.len(), 2);
        assert_eq!(up.ranges[0].template, "9");
        assert_eq!(up.ranges[1].template, "999");
    }

    #[test]
    fn parse_picture_coexists_with_value() {
        let ast = parse_format_src(
            "proc format; value sexfmt 1='Male'; picture p low-high='009'; run;",
        )
        .unwrap();
        assert_eq!(ast.values.len(), 1);
        assert_eq!(ast.pictures.len(), 1);
    }

    // ── PICTURE execute tests (M18.3) ─────────────────────────────────────────

    #[test]
    fn execute_picture_registered_and_applies() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let session = run_format_src(
            "proc format; picture dollarpic low-high = '000,000,009.99' (prefix='$'); run;",
        );
        let spec = FormatSpec::parse("DOLLARPIC.").unwrap();
        // No width → rendered as-is.
        assert_eq!(
            session.format_catalog.format(&Value::Num(1234.5), &spec),
            "$1,234.50"
        );
        // NOTE logged.
        let log = session.log.into_string();
        assert!(log.contains("Format DOLLARPIC has been output."), "log: {log}");
    }

    #[test]
    fn execute_picture_mult_directive() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let session = run_format_src(
            "proc format; picture pct low-high = '009.9%' (mult=100); run;",
        );
        let spec = FormatSpec::parse("PCT.").unwrap();
        assert_eq!(session.format_catalog.format(&Value::Num(0.125), &spec), "  1.3%");
    }

    #[test]
    fn execute_picture_with_width_right_justifies() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let session = run_format_src(
            "proc format; picture p low-high = '009'; run;",
        );
        let spec = FormatSpec::parse("P10.").unwrap();
        // Rendered "  5" then right-justified to width 10.
        let out = session.format_catalog.format(&Value::Num(5.0), &spec);
        assert_eq!(out.len(), 10);
        assert!(out.ends_with("5"));
    }

    #[test]
    fn execute_picture_missing_value() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let session = run_format_src(
            "proc format; picture p low-high = '009.99'; run;",
        );
        let spec = FormatSpec::parse("P5.").unwrap();
        // Numeric missing intercepted before picture → missing char, right-justified.
        assert_eq!(session.format_catalog.format(&Value::missing(), &spec), "    .");
    }

    #[test]
    fn execute_picture_shadows_builtin_name() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        // Define a picture named COMMA (a builtin format name) — user picture wins.
        let session = run_format_src(
            "proc format; picture comma low-high = '009'; run;",
        );
        let spec = FormatSpec::parse("COMMA.").unwrap();
        // Builtin COMMA on 5 would give "5"; our picture '009' gives "  5".
        assert_eq!(session.format_catalog.format(&Value::Num(5.0), &spec), "  5");
    }

    fn run_det(src: &str) -> crate::RunOutcome {
        crate::run(
            src,
            crate::RunOptions {
                work_dir: None,
                base_dir: None,
                deterministic: true,
                vectorize: false,
            },
        )
    }

    #[test]
    fn execute_picture_via_put_function() {
        // PUT(value, picture.) through the data step function path.
        let out = run_det(
            "proc format; picture dp low-high='000,009.99' (prefix='$'); run;\n\
             data _null_; x = 1234.5; y = put(x, dp.); put y=; run;",
        );
        assert_eq!(out.exit_code, 0, "log: {}", out.log);
        // PUT y= renders "y=$1,234.50" (PUT() result trimmed of leading blanks).
        assert!(out.log.contains("$1,234.50"), "log: {}", out.log);
    }

    #[test]
    fn execute_picture_via_format_statement() {
        // FORMAT statement + PROC PRINT path.
        let out = run_det(
            "proc format; picture dp low-high='009.99'; run;\n\
             data t; x = 12.34; format x dp.; output; run;\n\
             proc print data=t; run;",
        );
        assert_eq!(out.exit_code, 0, "log: {}", out.log);
        assert!(out.listing.contains("12.34"), "listing: {}", out.listing);
    }
}
