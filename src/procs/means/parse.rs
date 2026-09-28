use super::*;

/// Recognized statistic keywords accepted on the PROC MEANS statement.
pub(super) const STAT_KEYWORDS: &[&str] = &[
    "n", "nmiss", "mean", "std", "stddev", "min", "max", "sum", "sumwgt", "range", "stderr", "cv",
    "median", "clm", "lclm", "uclm",
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
    parse_named(ts, "MEANS")
}

pub(crate) fn parse_named(ts: &mut StatementStream, proc_name: &str) -> Result<MeansAst> {
    let mut data: Option<DatasetRef> = None;
    let mut noprint = false;
    let mut printalltypes = false;
    let mut stats: Vec<String> = Vec::new();
    // SAS default confidence level. Stays 0.05 unless ALPHA= is given; only
    // the CI statistics read it, so the default path is unaffected.
    let mut alpha: f64 = 0.05;
    // J03-P2 — VARDEF= (DF par défaut) pour la variance pondérée.
    let mut vardef = VarDef::Df;
    // J07-P2 — options de production.
    let mut nway = false;
    let mut missing = false;
    let mut order = ClassOrder::Internal;
    let mut maxdec: Option<usize> = None;
    let mut descendtypes = false;
    let mut completetypes = false;
    let mut chartype = false;
    let mut exclnpwgt = false;

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
        } else if ts.peek().is_kw("nway") {
            // J07-P2 — NWAY: OUT= keeps only the observations with the
            // highest _TYPE_ value (all CLASS variables crossed).
            ts.next();
            nway = true;
        } else if ts.peek().is_kw("missing") {
            // J07-P2 — MISSING: observations with missing CLASS values form
            // their own class level instead of being excluded.
            ts.next();
            missing = true;
        } else if ts.peek().is_kw("descendtypes") {
            // J07-P2 — DESCENDTYPES: _TYPE_ values in descending order.
            ts.next();
            descendtypes = true;
        } else if ts.peek().is_kw("completetypes") {
            // J07-P2 — COMPLETETYPES: all level combinations in OUT=, even
            // unobserved ones (freq 0, missing statistics).
            ts.next();
            completetypes = true;
        } else if ts.peek().is_kw("chartype") {
            // J07-P2 — CHARTYPE: _TYPE_ as a character mask ('101').
            ts.next();
            chartype = true;
        } else if ts.peek().is_kw("exclnpwgt") {
            // J07-P2 — EXCLNPWGT: exclude observations with a nonpositive
            // WEIGHT/FREQ value from the analysis (the strict partition
            // already excludes them; the option makes it explicit).
            ts.next();
            exclnpwgt = true;
        } else if ts.peek().is_kw("order") {
            // J07-P2 — ORDER= DATA|FORMATTED|FREQ|INTERNAL (défaut INTERNAL).
            crate::procs::common::consume_option_eq(ts, "ORDER")?;
            let tok = ts.peek().clone();
            let val = tok.ident().map(|s| s.to_ascii_lowercase());
            match val.as_deref().and_then(ClassOrder::parse) {
                Some(o) => {
                    ts.next();
                    order = o;
                }
                _ => {
                    return Err(SasError::parse(
                        format!(
                            "Unexpected option 'ORDER={}' on PROC {} statement.",
                            tok.ident().unwrap_or("?").to_uppercase(),
                            proc_name
                        ),
                        tok.span,
                    ));
                }
            }
        } else if ts.peek().is_kw("maxdec") {
            // J07-P2 — MAXDEC=n (0..9): decimals of the PRINTED report only,
            // no effect on the OUTPUT OUT= dataset.
            crate::procs::common::consume_option_eq(ts, "MAXDEC")?;
            let tok = ts.peek().clone();
            match tok.kind {
                TokenKind::Num(d) if d >= 0.0 && d.fract() == 0.0 && d <= 9.0 => {
                    ts.next();
                    maxdec = Some(d as usize);
                }
                _ => {
                    return Err(SasError::runtime(
                        "The MAXDEC= value must be an integer between 0 and 9.",
                    ));
                }
            }
        } else if ts.peek().is_kw("vardef") {
            // J03-P2 — VARDEF= divise la variance pondérée : DF (défaut,
            // Σw−1) et WEIGHT/WGT (Σw−Σw²/Σw) sont honorés ; toute autre
            // valeur change la variance → ERROR explicite.
            crate::procs::common::consume_option_eq(ts, "VARDEF")?;
            let tok = ts.peek().clone();
            match tok.ident().map(|s| s.to_ascii_lowercase()).as_deref() {
                Some("df") => {
                    ts.next();
                    vardef = VarDef::Df;
                }
                Some("weight" | "wgt") => {
                    ts.next();
                    vardef = VarDef::Weight;
                }
                _ => {
                    return Err(SasError::parse(
                        format!(
                            "Unexpected option 'VARDEF={}' on PROC {} statement.",
                            tok.ident().unwrap_or("?").to_uppercase(),
                            proc_name
                        ),
                        tok.span,
                    ));
                }
            }
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
    let mut freq: Option<String> = None;
    let mut id: Vec<String> = Vec::new();
    let mut ways: Vec<usize> = Vec::new();
    let mut types: Vec<Vec<String>> = Vec::new();
    let mut output: Vec<MeansOutput> = Vec::new();

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    // J07-P2 — CLASS / VAR / WAYS / TYPES / ID / OUTPUT s'accumulent : SAS
    // accepte plusieurs occurrences de chaque statement.
    crate::procs::common::parse_proc_body(ts, proc_name, |ts, kw| {
        Ok(match kw {
            "class" => {
                ts.next();
                let (names, class_missing, class_order) = parse_class_opts(ts, proc_name)?;
                class.extend(names);
                missing = missing || class_missing;
                if let Some(o) = class_order {
                    order = o;
                }
                true
            }
            "ways" => {
                ts.next();
                ways.extend(parse_ways(ts)?);
                true
            }
            "types" => {
                ts.next();
                types.extend(parse_types(ts)?);
                true
            }
            "var" => {
                ts.next();
                var.extend(crate::procs::common::parse_var_list(ts)?);
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
            "freq" => {
                // J07-P2 — FREQ var ; : chaque observation compte pour w
                // (N, NMISS et _FREQ_ multipliés par w, variance divisée par
                // Σw−1).
                ts.next();
                freq = Some(crate::procs::common::parse_weight(ts)?);
                true
            }
            "id" => {
                // J07-P2 — ID v1 v2 ... ; copiées dans l'OUT= (plus grand
                // niveau observé par groupe).
                ts.next();
                id.extend(crate::procs::common::parse_var_list(ts)?);
                true
            }
            "output" => {
                ts.next();
                output.push(parse_output(ts, proc_name)?);
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
        freq,
        id,
        vardef,
        alpha,
        printalltypes,
        nway,
        missing,
        order,
        maxdec,
        descendtypes,
        completetypes,
        chartype,
        exclnpwgt,
        ways,
        types,
        output,
    })
}

/// Parse a CLASS statement body (after `class` was consumed) with its J07-P2
/// options: `class v1 v2 [/ missing] [/ order=...] ;`. Returns the names,
/// whether `/ missing` was given, and an ORDER= override (None = unchanged).
fn parse_class_opts(
    ts: &mut StatementStream,
    proc_name: &str,
) -> Result<(Vec<String>, bool, Option<ClassOrder>)> {
    // Fast path: no `/ options` on the statement → the shared CLASS parser
    // (M31.2). Detection scans ahead to the `;` WITHOUT consuming anything.
    let mut look = 0usize;
    let mut has_options = false;
    loop {
        match ts.peek_nth(look).kind {
            TokenKind::Semi | TokenKind::Eof => break,
            TokenKind::Slash => {
                has_options = true;
                break;
            }
            _ => look += 1,
        }
    }
    if !has_options {
        let names = crate::procs::common::parse_class(ts)?;
        return Ok((names, false, None));
    }

    let mut names: Vec<String> = Vec::new();
    let mut missing = false;
    let mut order: Option<ClassOrder> = None;
    loop {
        match ts.peek().kind {
            TokenKind::Semi => {
                ts.next();
                break;
            }
            TokenKind::Eof => break,
            TokenKind::Slash => {
                ts.next();
                loop {
                    if ts.peek().kind == TokenKind::Semi {
                        ts.next();
                        return Ok((names, missing, order));
                    }
                    if ts.peek().kind == TokenKind::Eof {
                        return Ok((names, missing, order));
                    }
                    if ts.peek().is_kw("missing") {
                        ts.next();
                        missing = true;
                    } else if ts.peek().is_kw("order") {
                        crate::procs::common::consume_option_eq(ts, "ORDER")?;
                        let tok = ts.peek().clone();
                        let val = tok.ident().map(|s| s.to_ascii_lowercase());
                        match val.as_deref().and_then(ClassOrder::parse) {
                            Some(o) => {
                                ts.next();
                                order = Some(o);
                            }
                            _ => {
                                return Err(SasError::parse(
                                    format!(
                                        "Unexpected option 'ORDER={}' on the CLASS statement of PROC {}.",
                                        tok.ident().unwrap_or("?").to_uppercase(),
                                        proc_name
                                    ),
                                    tok.span,
                                ));
                            }
                        }
                    } else {
                        return Err(SasError::parse(
                            format!(
                                "Unexpected option '{}' on the CLASS statement of PROC {}.",
                                ts.peek().ident().unwrap_or("?").to_uppercase(),
                                proc_name
                            ),
                            ts.peek().span,
                        ));
                    }
                }
            }
            _ => {
                let tok = ts.peek().clone();
                match tok.ident() {
                    Some(n) => {
                        ts.next();
                        names.push(n.to_string());
                    }
                    None => {
                        return Err(SasError::parse(
                            "expected a variable name in the CLASS statement",
                            tok.span,
                        ));
                    }
                }
            }
        }
    }
    Ok((names, missing, order))
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
/// its terminating `;`. J07-P2 production grammar:
///
/// `output out=lib.t [stat[(varlist)][= namelist]]... [/ autoname] ;`
///
/// - `stat(var)=name` — one analysis variable, one output name (forme M5) ;
/// - `mean(x y)=m1 m2` — a list of variables and as many names ;
/// - `mean=` / `mean(x y)=` — sans noms : AUTONAME nomme `<var>_<STAT>` ;
///   sans AUTONAME, un nom explicite est exigé (stat en majuscules si une
///   seule variable d'analyse).
/// - `stat=` sans parenthèses s'applique à toutes les variables VAR.
///
/// J02-P4 — each statistic keyword is validated: an unknown keyword, or one
/// that has no single computable value in a dataset context (`clm(x)=` names
/// ONE variable but CLM is a pair of bounds), is an ERROR — the previous
/// behaviour wrote a silent missing column.
pub(super) fn parse_output(ts: &mut StatementStream, proc_name: &str) -> Result<MeansOutput> {
    let mut out: Option<DatasetRef> = None;
    let mut specs: Vec<OutSpec> = Vec::new();
    let mut autoname = false;

    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().kind == TokenKind::Slash {
            // Statement options: `/ autoname` (J07-P2).
            ts.next();
            loop {
                if ts.peek().kind == TokenKind::Semi {
                    ts.next();
                    return finish_output(out, specs, autoname);
                }
                if ts.peek().kind == TokenKind::Eof {
                    return finish_output(out, specs, autoname);
                }
                if ts.peek().is_kw("autoname") {
                    ts.next();
                    autoname = true;
                } else {
                    return Err(SasError::parse(
                        format!(
                            "Unexpected option '{}' in the OUTPUT statement of PROC {}.",
                            ts.peek().ident().unwrap_or("?").to_uppercase(),
                            proc_name
                        ),
                        ts.peek().span,
                    ));
                }
            }
        }
        if ts.peek().is_kw("out") {
            crate::procs::common::consume_option_eq(ts, "OUT")?;
            out = Some(ts.parse_dataset_ref()?);
            continue;
        }
        let Some(stat) = ts.peek().ident().map(str::to_string) else {
            return Err(SasError::parse(
                "unexpected token in OUTPUT statement",
                ts.peek().span,
            ));
        };
        let stat_l = stat.to_ascii_lowercase();
        if !is_stat_keyword(&stat_l) || stat_l == "clm" {
            return Err(crate::procs::common::unsupported_statement(
                proc_name,
                &format!("OUTPUT statistic {}", stat.to_uppercase()),
            ));
        }
        ts.next(); // stat keyword

        // Optional `(varlist)` — empty list means "every VAR variable".
        let mut vars: Vec<String> = Vec::new();
        if ts.peek().kind == TokenKind::LParen {
            ts.next(); // '('
            loop {
                if ts.peek().kind == TokenKind::RParen {
                    ts.next();
                    break;
                }
                if ts.peek().kind == TokenKind::Eof {
                    return Err(SasError::parse(
                        "expected ')' in OUTPUT statistic spec",
                        ts.peek().span,
                    ));
                }
                let Some(v) = ts.peek().ident().map(str::to_string) else {
                    return Err(SasError::parse(
                        "expected a variable name inside OUTPUT statistic spec",
                        ts.peek().span,
                    ));
                };
                ts.next();
                vars.push(v);
            }
        }

        // Optional `= namelist`. A following identifier is a NAME unless the
        // token after it is `=` (then it opens the next `stat=` spec).
        let mut names: Vec<String> = Vec::new();
        if ts.peek().kind == TokenKind::Eq {
            ts.next(); // '='
            loop {
                // A following identifier is a NAME unless the token after it
                // is `=` (next `stat=` spec) or `(` (next `stat(var)` spec).
                let is_name = match ts.peek().kind {
                    TokenKind::Ident(_) => {
                        !matches!(ts.peek2().kind, TokenKind::Eq | TokenKind::LParen)
                    }
                    _ => false,
                };
                if is_name {
                    names.push(ts.peek().ident().unwrap().to_string());
                    ts.next();
                } else {
                    break;
                }
            }
        }

        specs.push(OutSpec {
            stat: stat_l,
            vars,
            names,
        });
    }
    finish_output(out, specs, autoname)
}

/// Validate and assemble a parsed OUTPUT statement (OUT= is mandatory).
fn finish_output(
    out: Option<DatasetRef>,
    specs: Vec<OutSpec>,
    autoname: bool,
) -> Result<MeansOutput> {
    let out = out.ok_or_else(|| {
        SasError::runtime("The OUTPUT statement requires the OUT= option in PROC MEANS.")
    })?;
    // Name resolution: with AUTONAME every spec takes <var>_<STAT> names;
    // without it, an empty name list needs exactly one analysis variable.
    for sp in &specs {
        if sp.names.len() > 1 && sp.vars.len() > 1 && sp.names.len() != sp.vars.len() {
            return Err(SasError::runtime(format!(
                "The OUTPUT statistic {} names {} variables but {} output names were given.",
                sp.stat.to_uppercase(),
                sp.vars.len(),
                sp.names.len()
            )));
        }
    }
    Ok(MeansOutput {
        out,
        specs,
        autoname,
    })
}
