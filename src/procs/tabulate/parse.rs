use super::*;

/// Statistic keywords recognized inside a TABLE expression.
pub(super) const STAT_KEYWORDS: &[&str] = &[
    "n", "nmiss", "sum", "mean", "min", "max", "std", "pctn", "pctsum",
];

pub(super) fn is_stat_keyword(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    STAT_KEYWORDS.iter().any(|k| *k == l)
}

// ───────────────────────────── parse ─────────────────────────────

/// Parse `proc tabulate [data=a]; class ...; var ...; table ...; run;`.
/// Called AFTER "proc tabulate" has been consumed. Consumes through
/// `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<TabulateAst> {
    let mut data: Option<DatasetRef> = None;
    let mut format: Option<String> = None;
    let mut out: Option<DatasetRef> = None;

    // --- PROC TABULATE statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            data = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if ts.peek().is_kw("out") {
            out = Some(common::parse_dataset_opt(ts, "OUT")?);
        } else if ts.peek().is_kw("format") {
            // `format=<fmt>` — table-level default cell format (M33.4).
            common::consume_option_eq(ts, "FORMAT")?;
            format = Some(crate::parser::expr::read_format_token(ts)?);
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC TABULATE statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC TABULATE statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut class: Vec<String> = Vec::new();
    let mut var: Vec<String> = Vec::new();
    let mut page: Option<DimExpr> = None;
    let mut row: Option<DimExpr> = None;
    let mut col: Option<DimExpr> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "class" => {
                ts.next();
                class.extend(ts.parse_name_list()?);
                ts.expect_semi()?;
                true
            }
            "var" => {
                ts.next();
                var.extend(ts.parse_name_list()?);
                ts.expect_semi()?;
                true
            }
            "table" | "tables" => {
                ts.next();
                let (p, r, c) = parse_table_statement(ts)?;
                page = p;
                row = r;
                col = Some(c);
                ts.expect_semi()?;
                true
            }
            _ => false,
        })
    })?;

    let col = col.ok_or_else(|| SasError::runtime("PROC TABULATE requires a TABLE statement."))?;

    Ok(TabulateAst {
        data,
        class,
        var,
        page,
        row,
        col,
        format,
        out,
    })
}

/// `dimexpr := term { term }`. Terms are concatenated by blanks; a term ends
/// at a `,`, `)`, `;`, or EOF.
pub(super) fn parse_dimexpr(ts: &mut StatementStream) -> Result<DimExpr> {
    let mut terms = Vec::new();
    loop {
        match ts.peek().kind {
            TokenKind::Comma | TokenKind::RParen | TokenKind::Semi | TokenKind::Eof => break,
            _ => {}
        }
        terms.push(parse_term(ts)?);
    }
    if terms.is_empty() {
        return Err(SasError::parse(
            "PROC TABULATE: empty dimension in TABLE statement",
            ts.peek().span,
        ));
    }
    Ok(DimExpr { terms })
}

/// `term := factor { '*' factor }`. A `*` that introduces an `f=` cell-format
/// suffix on the PRECEDING factor is consumed by `parse_factor`, not here, so
/// it is never mistaken for a crossing.
pub(super) fn parse_term(ts: &mut StatementStream) -> Result<Term> {
    let mut factors = vec![parse_factor(ts)?];
    while ts.peek().kind == TokenKind::Star && !next_is_format_suffix(ts) {
        ts.next();
        factors.push(parse_factor(ts)?);
    }
    Ok(Term { factors })
}

/// True when the current `*` introduces an `f=` cell-format suffix
/// (`* f =`), i.e. the token after `*` is the identifier `f` (or `format`)
/// and the one after that is `=`. Such a `*` belongs to the preceding factor.
pub(super) fn next_is_format_suffix(ts: &StatementStream) -> bool {
    // peek() is `*`; peek2() is the next token. We only have two-token
    // lookahead, so confirm peek2 is `f`/`format`; the `=` is re-checked when
    // the factor parser actually consumes it.
    matches!(ts.peek2().ident(), Some(id) if id.eq_ignore_ascii_case("f") || id.eq_ignore_ascii_case("format"))
}

/// `factor := atom | '(' dimexpr ')'` where
/// `atom := (NAME | STATKW) [ '=' STRLIT ] [ '*' 'F' '=' FORMAT ]`.
pub(super) fn parse_factor(ts: &mut StatementStream) -> Result<Factor> {
    if ts.peek().kind == TokenKind::LParen {
        ts.next();
        let inner = parse_dimexpr(ts)?;
        if ts.peek().kind != TokenKind::RParen {
            return Err(SasError::parse(
                "PROC TABULATE: expected ')' in TABLE expression",
                ts.peek().span,
            ));
        }
        ts.next();
        return Ok(Factor::Group(inner));
    }
    if let Some(name) = ts.peek().ident().map(str::to_string) {
        ts.next();
        // Optional `='label'` header override (M33.4).
        let mut label: Option<String> = None;
        if ts.peek().kind == TokenKind::Eq {
            ts.next();
            match &ts.peek().kind {
                TokenKind::Str { value, .. } => {
                    label = Some(value.clone());
                    ts.next();
                }
                _ => {
                    return Err(SasError::parse(
                        "PROC TABULATE: expected a quoted label after '=' in TABLE expression",
                        ts.peek().span,
                    ));
                }
            }
        }
        // Optional `*f=<fmt>` cell format (M33.4). Only consume the `*` when it
        // truly introduces an `f=` suffix (checked via two-token lookahead).
        let mut format: Option<String> = None;
        if ts.peek().kind == TokenKind::Star && next_is_format_suffix(ts) {
            ts.next(); // '*'
            ts.next(); // 'f' / 'format'
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "PROC TABULATE: expected '=' after '*F' in TABLE expression",
                    ts.peek().span,
                ));
            }
            ts.next(); // '='
            format = Some(crate::parser::expr::read_format_token(ts)?);
        }
        return Ok(Factor::Name {
            name,
            label,
            format,
        });
    }
    Err(SasError::parse(
        "PROC TABULATE: expected a variable name, statistic, or '(' in TABLE expression",
        ts.peek().span,
    ))
}
