use super::*;

/// Parse `proc datasets [lib=<ident>] [nolist] ; ... quit ;`
/// Called AFTER "proc datasets" has been consumed. Consumes through `quit;`.
/// Defaults to lib=WORK if the LIB= option is absent.
pub fn parse(ts: &mut StatementStream) -> Result<DatasetsAst> {
    let mut deletes: Vec<String> = Vec::new();
    let mut changes: Vec<(String, String)> = Vec::new();
    let mut ops: Vec<DsOp> = Vec::new();

    // ── Parse PROC DATASETS header options until `;` ─────────────────────────
    let (lib, nolist) = parse_header_options(ts)?;

    // ── Parse sub-statements until `quit;` ───────────────────────────────────
    loop {
        // Skip stray semicolons
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }

        if ts.peek().kind == TokenKind::Eof {
            break;
        }

        if ts.peek().is_kw("quit") {
            ts.next(); // consume "quit"
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }

        if ts.peek().is_kw("run") {
            // `run;` is a no-op separator in M7 (run-group deviation documented above)
            ts.next(); // consume "run"
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            continue;
        }

        if ts.peek().is_kw("delete") {
            parse_delete_stmt(ts, &mut deletes);
            continue;
        }

        if ts.peek().is_kw("change") {
            parse_change_stmt(ts, &mut changes)?;
            continue;
        }

        if ts.peek().is_kw("copy") {
            ops.push(parse_copy_stmt(ts)?);
            continue;
        }

        if ts.peek().is_kw("exchange") {
            parse_exchange_stmt(ts, &mut ops)?;
            continue;
        }

        if ts.peek().is_kw("save") {
            ops.push(parse_save_stmt(ts));
            continue;
        }

        if ts.peek().is_kw("modify") {
            ops.push(parse_modify_stmt(ts)?);
            continue;
        }

        // Unknown sub-statement: skip to `;`
        ts.skip_to_semi();
    }

    Ok(DatasetsAst {
        lib,
        nolist,
        deletes,
        changes,
        ops,
    })
}

/// Header options `[lib=<ident>] [nolist]` until `;`. Defaults to lib=WORK.
pub(super) fn parse_header_options(ts: &mut StatementStream) -> Result<(String, bool)> {
    let mut lib = "WORK".to_string();
    let mut nolist = false;
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next(); // consume `;`
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("lib") || ts.peek().is_kw("library") {
            ts.next(); // consume "lib" / "library"
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse("expected '=' after LIB", ts.peek().span));
            }
            ts.next(); // consume `=`
            let ident_tok = ts.peek().clone();
            let Some(name) = ident_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a libref name after LIB=",
                    ident_tok.span,
                ));
            };
            ts.next();
            lib = name.to_uppercase();
        } else if ts.peek().is_kw("nolist") {
            ts.next();
            nolist = true;
        } else {
            // Unknown header option: skip to `;`
            ts.skip_to_semi();
            break;
        }
    }
    Ok((lib, nolist))
}

/// `delete m1 m2 ... ;` — append uppercased member names to `deletes`.
pub(super) fn parse_delete_stmt(ts: &mut StatementStream, deletes: &mut Vec<String>) {
    ts.next(); // consume "delete"
    // Read one or more names until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }
        let name_tok = ts.peek().clone();
        let Some(name) = name_tok.ident().map(str::to_string) else {
            // non-ident token: skip to `;`
            ts.skip_to_semi();
            break;
        };
        ts.next();
        deletes.push(name.to_uppercase());
    }
    // consume trailing `;`
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
}

/// `change old=new ... ;` — append uppercased pairs to `changes`.
pub(super) fn parse_change_stmt(
    ts: &mut StatementStream,
    changes: &mut Vec<(String, String)>,
) -> Result<()> {
    ts.next(); // consume "change"
    // Read one or more `old=new` pairs until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }
        let old_tok = ts.peek().clone();
        let Some(old_name) = old_tok.ident().map(str::to_string) else {
            ts.skip_to_semi();
            break;
        };
        ts.next(); // consume old name
        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' in CHANGE statement old=new pair",
                ts.peek().span,
            ));
        }
        ts.next(); // consume `=`
        let new_tok = ts.peek().clone();
        let Some(new_name) = new_tok.ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected a new name after '=' in CHANGE statement",
                new_tok.span,
            ));
        };
        ts.next(); // consume new name
        changes.push((old_name.to_uppercase(), new_name.to_uppercase()));
    }
    // consume trailing `;`
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    Ok(())
}

/// `copy out=<dst> [in=<src>]; [select m1 m2;]`
pub(super) fn parse_copy_stmt(ts: &mut StatementStream) -> Result<DsOp> {
    ts.next(); // consume "copy"
    let mut out: Option<String> = None;
    let mut in_lib: Option<String> = None;
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("out") {
            in_lib_assign(ts, &mut out)?;
        } else if ts.peek().is_kw("in") || ts.peek().kind == TokenKind::In {
            in_lib_assign(ts, &mut in_lib)?;
        } else {
            // Unknown COPY option: skip rest of statement.
            ts.skip_to_semi();
            break;
        }
    }
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    let Some(out) = out else {
        return Err(SasError::parse(
            "The COPY statement requires the OUT= option in PROC DATASETS.",
            ts.peek().span,
        ));
    };
    // Optional immediately-following SELECT statement.
    let mut select: Vec<String> = Vec::new();
    while ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    if ts.peek().is_kw("select") {
        ts.next(); // consume "select"
        loop {
            if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                break;
            }
            let tok = ts.peek().clone();
            let Some(name) = tok.ident().map(str::to_string) else {
                ts.skip_to_semi();
                break;
            };
            ts.next();
            select.push(name.to_uppercase());
        }
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
    }
    Ok(DsOp::Copy {
        out,
        r#in: in_lib,
        select,
    })
}

/// `exchange a=b ... ;` — one `DsOp::Exchange` per pair, appended to `ops`.
pub(super) fn parse_exchange_stmt(ts: &mut StatementStream, ops: &mut Vec<DsOp>) -> Result<()> {
    ts.next(); // consume "exchange"
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }
        let a_tok = ts.peek().clone();
        let Some(a) = a_tok.ident().map(str::to_string) else {
            ts.skip_to_semi();
            break;
        };
        ts.next();
        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' in EXCHANGE statement a=b pair",
                ts.peek().span,
            ));
        }
        ts.next(); // consume `=`
        let b_tok = ts.peek().clone();
        let Some(b) = b_tok.ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected a member name after '=' in EXCHANGE statement",
                b_tok.span,
            ));
        };
        ts.next();
        ops.push(DsOp::Exchange(a.to_uppercase(), b.to_uppercase()));
    }
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    Ok(())
}

/// `save m1 m2 ... ;`
pub(super) fn parse_save_stmt(ts: &mut StatementStream) -> DsOp {
    ts.next(); // consume "save"
    let mut keep: Vec<String> = Vec::new();
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }
        let tok = ts.peek().clone();
        let Some(name) = tok.ident().map(str::to_string) else {
            ts.skip_to_semi();
            break;
        };
        ts.next();
        keep.push(name.to_uppercase());
    }
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    DsOp::Save(keep)
}

/// `modify m ; [rename old=new ...;] [label v='..' ...;]`
pub(super) fn parse_modify_stmt(ts: &mut StatementStream) -> Result<DsOp> {
    ts.next(); // consume "modify"
    let m_tok = ts.peek().clone();
    let Some(member) = m_tok.ident().map(str::to_string) else {
        return Err(SasError::parse(
            "expected a member name after MODIFY",
            m_tok.span,
        ));
    };
    ts.next();
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    let mut renames: Vec<(String, String)> = Vec::new();
    let mut labels: Vec<(String, String)> = Vec::new();
    // Consume RENAME / LABEL sub-statements that belong to this MODIFY.
    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().is_kw("rename") {
            parse_modify_renames(ts, &mut renames)?;
        } else if ts.peek().is_kw("label") {
            parse_modify_labels(ts, &mut labels)?;
        } else {
            break;
        }
    }
    Ok(DsOp::Modify {
        member: member.to_uppercase(),
        renames,
        labels,
    })
}

/// MODIFY sub-statement `rename old=new ... ;`.
pub(super) fn parse_modify_renames(
    ts: &mut StatementStream,
    renames: &mut Vec<(String, String)>,
) -> Result<()> {
    ts.next(); // consume "rename"
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }
        let old_tok = ts.peek().clone();
        let Some(old) = old_tok.ident().map(str::to_string) else {
            ts.skip_to_semi();
            break;
        };
        ts.next();
        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' in RENAME old=new pair",
                ts.peek().span,
            ));
        }
        ts.next(); // consume `=`
        let new_tok = ts.peek().clone();
        let Some(new) = new_tok.ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected a new variable name after '=' in RENAME",
                new_tok.span,
            ));
        };
        ts.next();
        renames.push((old, new));
    }
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    Ok(())
}

/// MODIFY sub-statement `label v='text' ... ;`.
pub(super) fn parse_modify_labels(
    ts: &mut StatementStream,
    labels: &mut Vec<(String, String)>,
) -> Result<()> {
    ts.next(); // consume "label"
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }
        let v_tok = ts.peek().clone();
        let Some(var) = v_tok.ident().map(str::to_string) else {
            ts.skip_to_semi();
            break;
        };
        ts.next();
        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' in LABEL var='text' pair",
                ts.peek().span,
            ));
        }
        ts.next(); // consume `=`
        let txt_tok = ts.peek().clone();
        let text = match &txt_tok.kind {
            crate::token::TokenKind::Str { value, .. } => value.clone(),
            _ => {
                return Err(SasError::parse(
                    "expected a quoted label after '=' in LABEL statement",
                    txt_tok.span,
                ));
            }
        };
        ts.next();
        labels.push((var, text));
    }
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    Ok(())
}
