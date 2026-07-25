use super::*;

/// Parse a DEFINE statement body, after `define` was consumed, through `;`.
/// `define <var> / <usage> [order=asc|desc] ['label'] ;`
pub(crate) fn parse_define(ts: &mut StatementStream) -> Result<Define> {
    // Variable name.
    let var = match ts.peek().ident().map(str::to_string) {
        Some(v) => {
            ts.next();
            v
        }
        None => {
            return Err(SasError::parse(
                "expected a variable name after DEFINE",
                ts.peek().span,
            ));
        }
    };

    // Optional `/` introducing attributes. If the statement ends right away
    // (just `define var;`), SAS uses the column's default usage; we mirror
    // that by leaving usage unset here and resolving the default at execute.
    let mut usage: Option<Usage> = None;
    let mut order = OrderDir::Ascending;
    let mut label: Option<String> = None;
    let mut format: Option<String> = None;
    let mut width: Option<usize> = None;
    let mut spacing: Option<usize> = None;

    if ts.peek().kind == TokenKind::Slash {
        ts.next(); // consume '/'
        loop {
            match &ts.peek().kind {
                TokenKind::Semi | TokenKind::Eof => break,
                TokenKind::Str { value, .. } => {
                    label = Some(value.clone());
                    ts.next();
                }
                TokenKind::Ident(raw) => {
                    let kw = raw.to_ascii_lowercase();
                    match kw.as_str() {
                        "display" => {
                            usage = Some(Usage::Display);
                            ts.next();
                        }
                        "order" => {
                            // `order` is BOTH a usage AND an option `order=`.
                            // Disambiguate via the following token.
                            ts.next();
                            if ts.peek().kind == TokenKind::Eq {
                                ts.next(); // '='
                                order = parse_order_dir(ts)?;
                                // `order=` does not by itself set the usage;
                                // it only applies to ORDER/GROUP. If usage is
                                // still unset, treat the column as ORDER.
                                if usage.is_none() {
                                    usage = Some(Usage::Order);
                                }
                            } else {
                                usage = Some(Usage::Order);
                            }
                        }
                        "group" => {
                            usage = Some(Usage::Group);
                            ts.next();
                        }
                        "analysis" => {
                            ts.next();
                            // Optional statistic keyword follows.
                            let stat = if let Some(s) = ts.peek().ident() {
                                if is_analysis_stat(s) {
                                    let st = s.to_ascii_lowercase();
                                    ts.next();
                                    st
                                } else {
                                    "sum".to_string()
                                }
                            } else {
                                "sum".to_string()
                            };
                            usage = Some(Usage::Analysis(stat));
                        }
                        // A bare statistic keyword (e.g. `define x / sum;`) is
                        // shorthand for ANALYSIS <stat> in SAS.
                        s if is_analysis_stat(s) => {
                            usage = Some(Usage::Analysis(s.to_string()));
                            ts.next();
                        }
                        "across" => {
                            usage = Some(Usage::Across);
                            ts.next();
                        }
                        "computed" => {
                            usage = Some(Usage::Computed);
                            ts.next();
                        }
                        // `format=<fmt>` — SAS format / `w.d` for displayed
                        // values (M33.5). Read the raw format token verbatim.
                        "format" => {
                            ts.next();
                            if ts.peek().kind != TokenKind::Eq {
                                return Err(SasError::parse(
                                    "expected '=' after FORMAT in DEFINE statement",
                                    ts.peek().span,
                                ));
                            }
                            ts.next(); // '='
                            format = Some(crate::parser::expr::read_format_token(ts)?);
                        }
                        // `width=<n>` — column display width (M33.5).
                        "width" => {
                            ts.next();
                            if ts.peek().kind != TokenKind::Eq {
                                return Err(SasError::parse(
                                    "expected '=' after WIDTH in DEFINE statement",
                                    ts.peek().span,
                                ));
                            }
                            ts.next(); // '='
                            width = Some(parse_usize_opt(ts, "WIDTH")?);
                        }
                        // `spacing=<n>` — blank spaces before the column (M33.5).
                        "spacing" => {
                            ts.next();
                            if ts.peek().kind != TokenKind::Eq {
                                return Err(SasError::parse(
                                    "expected '=' after SPACING in DEFINE statement",
                                    ts.peek().span,
                                ));
                            }
                            ts.next(); // '='
                            spacing = Some(parse_usize_opt(ts, "SPACING")?);
                        }
                        other => {
                            return Err(SasError::runtime(format!(
                                "PROC REPORT v1 does not support the DEFINE option '{}'.",
                                other.to_uppercase()
                            )));
                        }
                    }
                }
                _ => {
                    return Err(SasError::parse(
                        "unexpected token in DEFINE statement",
                        ts.peek().span,
                    ));
                }
            }
        }
    }

    ts.expect_semi()?;

    // If no explicit usage was given, leave a placeholder that resolves to the
    // SAS type-based default at execute time. We encode "unset" as Display
    // here only when we KNOW the column; but since type is unknown at parse,
    // signal "unset" via a sentinel. Simplest: store usage=None semantics by
    // defaulting to Display and recording whether it was explicit.
    let usage = usage.unwrap_or(Usage::Display);

    Ok(Define {
        var,
        usage,
        order,
        label,
        format,
        width,
        spacing,
    })
}

/// Parse a non-negative integer option value (e.g. `width=8`, `spacing=4`).
pub(crate) fn parse_usize_opt(ts: &mut StatementStream, opt: &str) -> Result<usize> {
    match ts.peek().kind {
        TokenKind::Num(n) if n >= 0.0 && n.fract() == 0.0 => {
            ts.next();
            Ok(n as usize)
        }
        _ => Err(SasError::parse(
            format!("expected a non-negative integer after {opt}= in DEFINE statement"),
            ts.peek().span,
        )),
    }
}

pub(crate) fn parse_order_dir(ts: &mut StatementStream) -> Result<OrderDir> {
    match ts.peek().ident() {
        Some(s) if s.eq_ignore_ascii_case("descending") || s.eq_ignore_ascii_case("desc") => {
            ts.next();
            Ok(OrderDir::Descending)
        }
        Some(s) if s.eq_ignore_ascii_case("ascending") || s.eq_ignore_ascii_case("asc") => {
            ts.next();
            Ok(OrderDir::Ascending)
        }
        _ => Err(SasError::parse(
            "expected ASCENDING or DESCENDING after ORDER=",
            ts.peek().span,
        )),
    }
}
