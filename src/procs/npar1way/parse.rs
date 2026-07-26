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
                Err(SasError::parse(
                    format!("expected a number after {opt}="),
                    ts.peek().span,
                ))
            }
        }
        _ => Err(SasError::parse(
            format!("expected a number for {opt}="),
            tok.span,
        )),
    }
}

/// Parse PROC NPAR1WAY statement and its options.
///
/// Called AFTER "proc npar1way" was consumed. Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<NparAst> {
    let mut input: Option<DatasetRef> = None;
    let mut output: Option<DatasetRef> = None;
    let mut proc_options = NparProcOptions::default();
    let mut wilcoxon = false;
    let mut kruskal = false;
    let mut median = false;
    let mut savage = false;
    let mut normal = false;
    let mut exact = false;

    // --- PROC NPAR1WAY statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            common::consume_option_eq(ts, "DATA")?;
            input = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("out") {
            common::consume_option_eq(ts, "OUT")?;
            output = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("alpha") {
            common::consume_option_eq(ts, "ALPHA")?;
            proc_options.alpha = parse_num_value(ts, "ALPHA")?;
        } else if ts.peek().is_kw("wilcoxon") {
            ts.next();
            wilcoxon = true;
        } else if ts.peek().is_kw("kruskal") || ts.peek().is_kw("kruskalwallis") {
            ts.next();
            kruskal = true;
        } else if ts.peek().is_kw("median") {
            ts.next();
            median = true;
        } else if ts.peek().is_kw("savage") {
            ts.next();
            savage = true;
        } else if ts.peek().is_kw("normal") || ts.peek().is_kw("vw") {
            ts.next();
            normal = true;
        } else if ts.peek().is_kw("exact") {
            ts.next();
            exact = true;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!("Unknown PROC NPAR1WAY option: {}", name.to_uppercase()),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC NPAR1WAY statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut var_vars: Vec<String> = Vec::new();
    let mut class_var: Option<String> = None;
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
                        "The CLASS statement of PROC NPAR1WAY accepts exactly one variable.",
                    ));
                }
                class_var = Some(names.into_iter().next().unwrap());
                true
            }
            "by" => {
                ts.next();
                by = common::parse_by(ts)?;
                true
            }
            "exact" => {
                // `exact wilcoxon;` — just consume to `;` and enable the flag.
                ts.next();
                exact = true;
                ts.skip_to_semi();
                true
            }
            "output" => {
                // `output out=<ref>;`
                ts.next();
                if ts.peek().is_kw("out") {
                    common::consume_option_eq(ts, "OUT")?;
                    output = Some(ts.parse_dataset_ref()?);
                }
                ts.skip_to_semi();
                true
            }
            _ => false,
        })
    })?;

    let class_var =
        class_var.ok_or_else(|| SasError::runtime("PROC NPAR1WAY requires a CLASS statement."))?;

    // SAS default: with NO test/score option at all, run Wilcoxon (k=2) and
    // Kruskal-Wallis (k≥2). Enabling both flags reproduces that behaviour. A
    // score option (MEDIAN/SAVAGE/NORMAL/VW) or WILCOXON/KRUSKAL suppresses the
    // implicit default; the explicit flags then drive exactly what is shown.
    if !wilcoxon && !kruskal && !median && !savage && !normal {
        wilcoxon = true;
        kruskal = true;
    }

    Ok(NparAst {
        data_options: NparDataOptions { input, output },
        proc_options,
        var_vars,
        class_var,
        test_options: NparTestOptions {
            wilcoxon,
            kruskal,
            median,
            savage,
            normal,
            exact,
            scores: NparScores::Normal,
        },
        by,
    })
}
