use super::*;

pub struct CompareAst {
    pub base: DatasetRef,
    pub compare: DatasetRef,
    pub out: Option<DatasetRef>,
    pub novalues: bool,
    pub briefsummary: bool,
}

/// Parse `proc compare base=... compare=... [out=...] [novalues] [briefsummary]; run;`
/// Called AFTER "proc compare" has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<CompareAst> {
    let mut base: Option<DatasetRef> = None;
    let mut compare: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut novalues = false;
    let mut briefsummary = false;

    // Parse header options until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }

        if ts.peek().is_kw("base") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse("expected '=' after BASE", ts.peek().span));
            }
            ts.next();
            base = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("compare") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after COMPARE",
                    ts.peek().span,
                ));
            }
            ts.next();
            compare = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("out") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse("expected '=' after OUT", ts.peek().span));
            }
            ts.next();
            out = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("novalues") {
            ts.next();
            novalues = true;
        } else if ts.peek().is_kw("briefsummary") {
            ts.next();
            briefsummary = true;
        } else if ts.peek().is_kw("criterion")
            || ts.peek().is_kw("method")
            || ts.peek().is_kw("brief")
            || ts.peek().is_kw("listall")
            || ts.peek().is_kw("outbase")
            || ts.peek().is_kw("outcomp")
            || ts.peek().is_kw("outdif")
            || ts.peek().is_kw("outnoequal")
            || ts.peek().is_kw("outpercent")
            || ts.peek().is_kw("maxprint")
        {
            // J02-P4 — options SAS reconnues mais non honorées : elles peuvent
            // changer la comparaison effectuée (tolérance, sortie OUT=, volume
            // affiché) ; l'ignorer silencieusement produirait un faux « all
            // values equal ». ERROR via le helper du contrat J02-P3.
            let opt = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(crate::procs::common::unsupported_statement(
                "COMPARE",
                &format!("{opt}="),
            ));
        } else {
            // Unknown option: no silent skip (contrat J02-P3) — SAS would
            // reject it too.
            return Err(crate::procs::common::unknown_option_error(ts, "COMPARE"));
        }
    }

    crate::procs::common::parse_proc_body(ts, "COMPARE", |_ts, _kw| Ok(false))?;

    let base = base.ok_or_else(|| {
        SasError::parse(
            "BASE= is required for PROC COMPARE",
            crate::token::Span::default(),
        )
    })?;
    let compare = compare.ok_or_else(|| {
        SasError::parse(
            "COMPARE= is required for PROC COMPARE",
            crate::token::Span::default(),
        )
    })?;

    Ok(CompareAst {
        base,
        compare,
        out,
        novalues,
        briefsummary,
    })
}
