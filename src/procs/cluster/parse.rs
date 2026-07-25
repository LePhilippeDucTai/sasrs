use super::*;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkMethod {
    Ward,
    Average,
    Single,
    Complete,
}

impl LinkMethod {
    pub(super) fn title(self) -> &'static str {
        match self {
            LinkMethod::Ward => "Ward's Minimum Variance Cluster Analysis",
            LinkMethod::Average => "Average Linkage Cluster Analysis",
            LinkMethod::Single => "Single Linkage Cluster Analysis",
            LinkMethod::Complete => "Complete Linkage Cluster Analysis",
        }
    }
}

pub struct ClusterAst {
    pub data: Option<DatasetRef>,
    pub method: LinkMethod,
    pub outtree: Option<DatasetRef>,
    pub print: Option<usize>,
    pub noeigen: bool,
    pub var: Vec<String>,
    pub id: Option<String>,
}

// ───────────────────────── Parser ─────────────────────────

pub(super) fn parse_method(ts: &mut StatementStream) -> Result<LinkMethod> {
    let span = ts.peek().span;
    let name = ts
        .peek()
        .ident()
        .map(str::to_string)
        .ok_or_else(|| SasError::parse("expected a method name after METHOD=", span))?;
    ts.next();
    match name.to_ascii_lowercase().as_str() {
        "ward" => Ok(LinkMethod::Ward),
        "average" | "ave" => Ok(LinkMethod::Average),
        "single" => Ok(LinkMethod::Single),
        "complete" | "com" => Ok(LinkMethod::Complete),
        other => Err(SasError::parse(
            format!("Unknown METHOD= value '{}' on PROC CLUSTER.", other.to_uppercase()),
            span,
        )),
    }
}

/// Parse the PROC CLUSTER block. Called AFTER "proc cluster" has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<ClusterAst> {
    let mut data: Option<DatasetRef> = None;
    let mut method = LinkMethod::Ward;
    let mut outtree: Option<DatasetRef> = None;
    let mut print: Option<usize> = None;
    let mut noeigen = false;

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
        } else if ts.peek().is_kw("method") {
            common::expect_eq(ts, "METHOD")?;
            method = parse_method(ts)?;
        } else if ts.peek().is_kw("outtree") {
            outtree = Some(common::parse_dataset_opt(ts, "OUTTREE")?);
        } else if ts.peek().is_kw("print") {
            common::expect_eq(ts, "PRINT")?;
            let span = ts.peek().span;
            let k = match ts.peek().kind {
                TokenKind::Num(v) => v,
                _ => return Err(SasError::parse("expected a number after PRINT=", span)),
            };
            ts.next();
            print = Some(k as usize);
        } else if ts.peek().is_kw("noeigen") {
            ts.next();
            noeigen = true;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC CLUSTER statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC CLUSTER statement.",
                span,
            ));
        }
    }

    let mut var: Vec<String> = Vec::new();
    let mut id: Option<String> = None;
    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next();
                var = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            "id" => {
                ts.next();
                let names = ts.parse_name_list()?;
                id = names.into_iter().next();
                ts.expect_semi()?;
                true
            }
            _ => false,
        })
    })?;

    Ok(ClusterAst {
        data,
        method,
        outtree,
        print,
        noeigen,
        var,
        id,
    })
}
