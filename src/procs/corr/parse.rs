use super::*;

/// Parse `proc corr [data=a] [nosimple] [noprob] [nocorr];
/// [var ...;] [with ...;] ... run;`. Called AFTER "proc corr" was consumed.
/// Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<CorrAst> {
    let mut data: Option<DatasetRef> = None;
    let mut nosimple = false;
    let mut noprob = false;
    let mut nocorr = false;
    let mut pearson = false;
    let mut spearman = false;
    let mut kendall = false;
    let mut hoeffding = false;
    let mut outp: Option<DatasetRef> = None;
    let mut outs: Option<DatasetRef> = None;
    let mut outk: Option<DatasetRef> = None;

    // --- PROC CORR statement options, until `;` ---
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
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("nosimple") {
            ts.next();
            nosimple = true;
        } else if ts.peek().is_kw("noprob") {
            ts.next();
            noprob = true;
        } else if ts.peek().is_kw("nocorr") {
            ts.next();
            nocorr = true;
        } else if ts.peek().is_kw("pearson") {
            ts.next();
            pearson = true;
        } else if ts.peek().is_kw("spearman") {
            ts.next();
            spearman = true;
        } else if ts.peek().is_kw("kendall") {
            ts.next();
            kendall = true;
        } else if ts.peek().is_kw("hoeffding") {
            ts.next();
            hoeffding = true;
        } else if ts.peek().is_kw("out") || ts.peek().is_kw("outp") {
            common::consume_option_eq(ts, "OUT")?;
            outp = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("outs") {
            common::consume_option_eq(ts, "OUTS")?;
            outs = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("outk") {
            common::consume_option_eq(ts, "OUTK")?;
            outk = Some(ts.parse_dataset_ref()?);
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC CORR statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC CORR statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut var: Vec<String> = Vec::new();
    let mut with: Vec<String> = Vec::new();
    let mut partial: Vec<String> = Vec::new();
    let mut weight: Option<String> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next();
                var = common::parse_var_list(ts)?;
                true
            }
            "with" => {
                ts.next();
                with = common::parse_var_list(ts)?;
                true
            }
            "partial" => {
                ts.next();
                partial = common::parse_var_list(ts)?;
                true
            }
            "weight" => {
                ts.next();
                let names = ts.parse_name_list()?;
                ts.expect_semi()?;
                // SAS allows a single weight variable.
                if names.len() != 1 {
                    return Err(SasError::runtime(
                        "The WEIGHT statement of PROC CORR accepts exactly one variable.",
                    ));
                }
                weight = Some(names.into_iter().next().unwrap());
                true
            }
            _ => false,
        })
    })?;

    Ok(CorrAst {
        data,
        nosimple,
        noprob,
        nocorr,
        pearson,
        spearman,
        kendall,
        hoeffding,
        var,
        with,
        partial,
        weight,
        outp,
        outs,
        outk,
    })
}
