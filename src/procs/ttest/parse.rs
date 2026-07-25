use super::*;

/// Parse a numeric option value (`=<num>`); the `=` must already be consumed.
pub(super) fn parse_num_value(ts: &mut StatementStream, opt: &str) -> Result<f64> {
    let tok = ts.peek().clone();
    match tok.kind {
        TokenKind::Num(n) => {
            ts.next();
            Ok(n)
        }
        TokenKind::Minus => {
            ts.next();
            if let TokenKind::Num(n) = ts.peek().kind {
                ts.next();
                Ok(-n)
            } else {
                Err(SasError::parse(format!("expected a number after {opt}="), ts.peek().span))
            }
        }
        _ => Err(SasError::parse(
            format!("expected a number for {opt}="),
            tok.span,
        )),
    }
}

/// Parse PROC TTEST statement and its options.
///
/// Called AFTER "proc ttest" was consumed. Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<TTestAst> {
    let mut input: Option<DatasetRef> = None;
    let mut output: Option<DatasetRef> = None;
    let mut proc_options = TTestProcOptions::default();

    // --- PROC TTEST statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            common::expect_eq(ts, "DATA")?;
            input = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("out") {
            common::expect_eq(ts, "OUT")?;
            output = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("h0") {
            common::expect_eq(ts, "H0")?;
            proc_options.h0 = parse_num_value(ts, "H0")?;
        } else if ts.peek().is_kw("alpha") {
            common::expect_eq(ts, "ALPHA")?;
            proc_options.alpha = parse_num_value(ts, "ALPHA")?;
        } else if ts.peek().is_kw("ci") {
            common::expect_eq(ts, "CI")?;
            proc_options.ci = parse_num_value(ts, "CI")?;
            proc_options.ci_explicit = true;
        } else if ts.peek().is_kw("equal") {
            common::expect_eq(ts, "EQUAL")?;
            let v = ts
                .peek()
                .ident()
                .map(str::to_string)
                .ok_or_else(|| SasError::parse("expected YES or NO after EQUAL=", ts.peek().span))?;
            ts.next();
            proc_options.equal = !v.eq_ignore_ascii_case("no");
        } else if ts.peek().is_kw("sides") {
            common::expect_eq(ts, "SIDES")?;
            let v = ts
                .peek()
                .ident()
                .map(str::to_string)
                .ok_or_else(|| SasError::parse("expected 2, U or L after SIDES=", ts.peek().span))?;
            ts.next();
            proc_options.sides = match v.to_ascii_uppercase().as_str() {
                "U" => TTestSides::Upper,
                "L" => TTestSides::Lower,
                _ => TTestSides::TwoTailed,
            };
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!("Unknown PROC TTEST option: {}", name.to_uppercase()),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse("Unexpected token on PROC TTEST statement.", span));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut var_vars: Vec<String> = Vec::new();
    let mut class_var: Option<String> = None;
    let mut paired_vars: Vec<(String, String)> = Vec::new();
    let mut by: Vec<(String, bool)> = Vec::new();

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next();
                var_vars = common::parse_var_list(ts)?;
                true
            }
            "class" => {
                ts.next();
                let names = ts.parse_name_list()?;
                ts.expect_semi()?;
                if names.len() != 1 {
                    return Err(SasError::runtime(
                        "The CLASS statement of PROC TTEST accepts exactly one variable.",
                    ));
                }
                class_var = Some(names.into_iter().next().unwrap());
                true
            }
            "paired" => {
                ts.next();
                // `paired x*y z*w;` — each pair is name '*' name.
                loop {
                    if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                        break;
                    }
                    let left = ts
                        .peek()
                        .ident()
                        .map(str::to_string)
                        .ok_or_else(|| SasError::parse("expected a variable name in PAIRED", ts.peek().span))?;
                    ts.next();
                    if ts.peek().kind != TokenKind::Star {
                        return Err(SasError::parse(
                            "expected '*' between paired variables",
                            ts.peek().span,
                        ));
                    }
                    ts.next();
                    let right = ts
                        .peek()
                        .ident()
                        .map(str::to_string)
                        .ok_or_else(|| SasError::parse("expected a variable name after '*' in PAIRED", ts.peek().span))?;
                    ts.next();
                    paired_vars.push((left, right));
                }
                ts.expect_semi()?;
                true
            }
            "by" => {
                ts.next();
                by = common::parse_by(ts)?;
                true
            }
            "output" => {
                // `output out=<ref>;`
                ts.next();
                if ts.peek().is_kw("out") {
                    common::expect_eq(ts, "OUT")?;
                    output = Some(ts.parse_dataset_ref()?);
                }
                ts.skip_to_semi();
                true
            }
            _ => false,
        })
    })?;

    Ok(TTestAst {
        data_options: TTestDataOptions { input, output },
        proc_options,
        var_vars,
        class_var,
        paired_vars,
        by,
    })
}
