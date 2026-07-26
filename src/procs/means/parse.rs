use super::*;

/// Recognized statistic keywords accepted on the PROC MEANS statement.
pub(super) const STAT_KEYWORDS: &[&str] = &[
    "n", "nmiss", "mean", "std", "stddev", "min", "max", "sum", "range", "stderr", "cv", "median",
    "clm", "lclm", "uclm",
    // Percentile keywords (M33.3) — Definition 5, shared with PROC UNIVARIATE.
    "p1", "p5", "p10", "p20", "p25", "p30", "p40", "p50", "p60", "p70", "p75", "p80", "p90", "p95",
    "p99", "q1", "q3", "qrange",
];

pub(super) fn is_stat_keyword(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    STAT_KEYWORDS.iter().any(|k| *k == l)
}

/// Map a percentile keyword to its target fraction `p` (None if not a single
/// percentile keyword). `Q1`=`P25`, `Q3`=`P75`, `P50`=`MEDIAN`. `QRANGE` is
/// handled separately (it is a difference of two percentiles).
pub(super) fn percentile_fraction(stat: &str) -> Option<f64> {
    Some(match stat {
        "p1" => 0.01,
        "p5" => 0.05,
        "p10" => 0.10,
        "p20" => 0.20,
        "p25" | "q1" => 0.25,
        "p30" => 0.30,
        "p40" => 0.40,
        "p50" => 0.50,
        "p60" => 0.60,
        "p70" => 0.70,
        "p75" | "q3" => 0.75,
        "p80" => 0.80,
        "p90" => 0.90,
        "p95" => 0.95,
        "p99" => 0.99,
        _ => return None,
    })
}

/// Parse `proc means [data=a] [noprint] [stat...] ; [class ...;] [var ...;]
/// [output out=b stat(var)=name...;] ... run;`. Called AFTER "proc
/// means"/"proc summary" has been consumed. Consumes through `run;`/`quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<MeansAst> {
    let mut data: Option<DatasetRef> = None;
    let mut noprint = false;
    let mut printalltypes = false;
    let mut stats: Vec<String> = Vec::new();
    // SAS default confidence level. Stays 0.05 unless ALPHA= is given; only
    // the CI statistics read it, so the default path is unaffected.
    let mut alpha: f64 = 0.05;

    // --- PROC MEANS statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next(); // consume `;`
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            crate::procs::common::consume_option_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("noprint") {
            ts.next();
            noprint = true;
        } else if ts.peek().is_kw("print") {
            // explicit PRINT — undo a noprint default (e.g. PROC SUMMARY).
            ts.next();
            noprint = false;
        } else if ts.peek().is_kw("printalltypes") {
            // PRINTALLTYPES (M33.3): print every generated _TYPE_ subtable.
            ts.next();
            printalltypes = true;
        } else if ts.peek().is_kw("alpha") {
            crate::procs::common::consume_option_eq(ts, "ALPHA")?;
            let tok = ts.peek().clone();
            let val = match tok.kind {
                TokenKind::Num(f) => f,
                _ => {
                    return Err(SasError::parse("expected a number after ALPHA=", tok.span));
                }
            };
            ts.next();
            if !(val > 0.0 && val < 1.0) {
                return Err(SasError::runtime(format!(
                    "The ALPHA= value {val} must be between 0 and 1."
                )));
            }
            alpha = val;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            if is_stat_keyword(&name) {
                ts.next();
                stats.push(name.to_ascii_lowercase());
            } else {
                let span = ts.peek().span;
                return Err(SasError::parse(
                    format!(
                        "Unexpected option '{}' on PROC MEANS statement.",
                        name.to_uppercase()
                    ),
                    span,
                ));
            }
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC MEANS statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut class: Vec<String> = Vec::new();
    let mut var: Vec<String> = Vec::new();
    let mut by: Vec<(String, bool)> = Vec::new();
    let mut weight: Option<String> = None;
    let mut ways: Vec<usize> = Vec::new();
    let mut types: Vec<Vec<String>> = Vec::new();
    let mut output: Option<MeansOutput> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    crate::procs::common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "class" => {
                ts.next();
                class = crate::procs::common::parse_class(ts)?;
                true
            }
            "ways" => {
                ts.next();
                ways = parse_ways(ts)?;
                true
            }
            "types" => {
                ts.next();
                types = parse_types(ts)?;
                true
            }
            "var" => {
                ts.next();
                var = crate::procs::common::parse_var_list(ts)?;
                true
            }
            "by" => {
                ts.next();
                by = parse_by_list(ts)?;
                true
            }
            "weight" => {
                ts.next();
                weight = Some(crate::procs::common::parse_weight(ts)?);
                true
            }
            "output" => {
                ts.next();
                output = Some(parse_output(ts)?);
                true
            }
            _ => false,
        })
    })?;

    Ok(MeansAst {
        data,
        summary: false,
        noprint,
        stats,
        class,
        var,
        by,
        weight,
        alpha,
        printalltypes,
        ways,
        types,
        output,
    })
}

/// Parse a WAYS statement body (after `ways` was consumed), through its `;`.
/// `ways 0 1 2;` — a list of non-negative integers (the desired numbers of
/// active CLASS variables). Errors on a non-integer token.
pub(super) fn parse_ways(ts: &mut StatementStream) -> Result<Vec<usize>> {
    let mut out: Vec<usize> = Vec::new();
    loop {
        match ts.peek().kind {
            TokenKind::Semi => {
                ts.next();
                break;
            }
            TokenKind::Eof => break,
            TokenKind::Num(f) if f >= 0.0 && f.fract() == 0.0 => {
                ts.next();
                out.push(f as usize);
            }
            _ => {
                return Err(SasError::parse(
                    "expected a non-negative integer in the WAYS statement",
                    ts.peek().span,
                ));
            }
        }
    }
    Ok(out)
}

/// Parse a TYPES statement body (after `types` was consumed), through its `;`.
/// `types () (a) (a*b) a*b;` — a space-separated list of CLASS crossings; each
/// crossing is a `*`-joined set of CLASS names, optionally parenthesized. `()`
/// denotes the empty crossing (overall, `_TYPE_`=0). Returns one `Vec<String>`
/// per crossing (the empty crossing → an empty inner vector).
pub(super) fn parse_types(ts: &mut StatementStream) -> Result<Vec<Vec<String>>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    loop {
        match ts.peek().kind {
            TokenKind::Semi => {
                ts.next();
                break;
            }
            TokenKind::Eof => break,
            TokenKind::LParen => {
                ts.next(); // '('
                let mut crossing: Vec<String> = Vec::new();
                loop {
                    if ts.peek().kind == TokenKind::RParen {
                        ts.next();
                        break;
                    }
                    let name = ts.peek().ident().map(str::to_string).ok_or_else(|| {
                        SasError::parse("expected a CLASS name in TYPES", ts.peek().span)
                    })?;
                    ts.next();
                    crossing.push(name);
                    if ts.peek().kind == TokenKind::Star {
                        ts.next();
                    }
                }
                out.push(crossing);
            }
            _ => {
                // Un-parenthesized crossing: name [* name]*.
                let mut crossing: Vec<String> = Vec::new();
                let name = ts.peek().ident().map(str::to_string).ok_or_else(|| {
                    SasError::parse("expected a CLASS name in TYPES", ts.peek().span)
                })?;
                ts.next();
                crossing.push(name);
                while ts.peek().kind == TokenKind::Star {
                    ts.next();
                    let name = ts.peek().ident().map(str::to_string).ok_or_else(|| {
                        SasError::parse("expected a CLASS name after '*' in TYPES", ts.peek().span)
                    })?;
                    ts.next();
                    crossing.push(name);
                }
                out.push(crossing);
            }
        }
    }
    Ok(out)
}

/// Parse the OUTPUT statement body (after "output" was consumed), through
/// its terminating `;`. `output out=lib.t [stat(var)=name ...] ;`
pub(super) fn parse_output(ts: &mut StatementStream) -> Result<MeansOutput> {
    let mut out: Option<DatasetRef> = None;
    let mut specs: Vec<(String, String, String)> = Vec::new();

    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("out") {
            crate::procs::common::consume_option_eq(ts, "OUT")?;
            out = Some(ts.parse_dataset_ref()?);
        } else if let Some(stat) = ts.peek().ident().map(str::to_string) {
            // Expect `stat(var)=name`.
            ts.next(); // stat
            if ts.peek().kind != TokenKind::LParen {
                return Err(SasError::parse(
                    format!("expected '(' after statistic '{}' in OUTPUT", stat),
                    ts.peek().span,
                ));
            }
            ts.next(); // '('
            let var = match ts.peek().ident().map(str::to_string) {
                Some(v) => {
                    ts.next();
                    v
                }
                None => {
                    return Err(SasError::parse(
                        "expected a variable name inside OUTPUT statistic spec",
                        ts.peek().span,
                    ));
                }
            };
            if ts.peek().kind != TokenKind::RParen {
                return Err(SasError::parse(
                    "expected ')' in OUTPUT statistic spec",
                    ts.peek().span,
                ));
            }
            ts.next(); // ')'
            expect_eq(ts, "OUTPUT statistic")?;
            let name = match ts.peek().ident().map(str::to_string) {
                Some(n) => {
                    ts.next();
                    n
                }
                None => {
                    return Err(SasError::parse(
                        "expected an output variable name in OUTPUT statistic spec",
                        ts.peek().span,
                    ));
                }
            };
            specs.push((stat.to_ascii_lowercase(), var, name));
        } else {
            return Err(SasError::parse(
                "unexpected token in OUTPUT statement",
                ts.peek().span,
            ));
        }
    }

    let out = out.ok_or_else(|| {
        SasError::runtime("The OUTPUT statement requires the OUT= option in PROC MEANS.")
    })?;
    Ok(MeansOutput { out, specs })
}
