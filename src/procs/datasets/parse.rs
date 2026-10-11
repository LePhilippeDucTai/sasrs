use super::*;

use crate::token::Span;

/// Parse `proc datasets [lib=<ident>] [nolist] ; ... quit ;`
/// Called AFTER "proc datasets" has been consumed. Consumes through `quit;`.
/// Defaults to lib=WORK if the LIB= option is absent.
///
/// J02-P8 — every statement becomes one [`DsOp`] in SOURCE order (SAS 9.4
/// DATASETS, « Execution of Statements » : « Statements execute in the order
/// in which they are written »). DELETE and CHANGE used to be gathered in two
/// lists executed before every other statement.
pub fn parse(ts: &mut StatementStream) -> Result<DatasetsAst> {
    let mut ops: Vec<DsOp> = Vec::new();

    // ── Parse PROC DATASETS header options until `;` ─────────────────────────
    let (lib, nolist) = parse_header_options(ts)?;

    // ── Parse sub-statements until `quit;` ───────────────────────────────────
    loop {
        if crate::procs::common::parse_proc_inert_or_global(ts)? {
            continue;
        }
        if ts.peek().is_kw("data") || ts.peek().is_kw("proc") {
            break;
        }

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
            // `run;` ends a RUN group; the statements keep their source order
            // (they are all executed at `quit;`, see the module doc).
            ts.next(); // consume "run"
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            continue;
        }

        if ts.peek().is_kw("delete") {
            ops.push(DsOp::Delete(parse_delete_stmt(ts)?));
            continue;
        }

        if ts.peek().is_kw("change") {
            ops.push(DsOp::Change(parse_change_stmt(ts)?));
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
            ops.push(parse_save_stmt(ts)?);
            continue;
        }

        if ts.peek().is_kw("modify") {
            ops.push(parse_modify_stmt(ts)?);
            continue;
        }

        let kw = ts.peek().ident().unwrap_or("").to_ascii_lowercase();
        let span = ts.peek().span;
        if let Some((_, unit)) = UNSUPPORTED_STATEMENTS.iter().find(|(s, _)| *s == kw) {
            return Err(unsupported_statement(&kw, *unit, span));
        }
        if MODIFY_SUBSTATEMENTS.contains(&kw.as_str()) {
            return Err(misplaced_statement(&kw, span));
        }

        crate::procs::common::unhandled_proc_statement(ts, "DATASETS")?;
    }

    Ok(DatasetsAst { lib, nolist, ops })
}

// ─────────────────────── Contract diagnostics (J02-P8) ───────────────────────
//
// Base SAS 9.4 Procedures Guide, DATASETS Procedure, Syntax: the valid
// statements that sasrs does not implement used to be reported as
// « 180-322 … not valid ». CONTRIBUTING §5: each of them can change a data
// set (members, variables, indexes, audit trail…) → ERROR naming the
// roadmap-avancee unit that will implement it.

/// Valid SAS 9.4 PROC DATASETS statements that sasrs does not implement,
/// with the roadmap-avancee unit that will lift the ERROR (None: not planned).
/// ATTRIB, IC, INDEX and XATTR are MODIFY sub-statements.
const UNSUPPORTED_STATEMENTS: &[(&str, Option<&str>)] = &[
    ("age", None),
    ("append", Some("J03-P4")),
    ("attrib", None),
    ("audit", None),
    ("contents", Some("J03-P4")),
    ("exclude", None),
    ("ic", None),
    ("index", None),
    ("rebuild", None),
    ("repair", Some("J11-P6")),
    ("xattr", None),
];

/// MODIFY sub-statements implemented by sasrs. SAS 9.4 (FORMAT, LABEL,
/// RENAME, INFORMAT statements): « Must appear in a MODIFY RUN group ».
const MODIFY_SUBSTATEMENTS: &[&str] = &["format", "informat", "label", "rename"];

/// Suffix naming the roadmap-avancee unit that will lift a provisional ERROR.
fn planned(unit: Option<&str>) -> String {
    match unit {
        Some(u) => format!(" (planned: roadmap-avancee {u})"),
        None => String::new(),
    }
}

/// ERROR for a valid SAS statement that sasrs does not implement.
fn unsupported_statement(kw: &str, unit: Option<&str>, span: Span) -> SasError {
    SasError::parse(
        format!(
            "The {} statement is not supported in PROC DATASETS; it can affect results and \
             cannot be ignored{}.",
            kw.to_ascii_uppercase(),
            planned(unit)
        ),
        span,
    )
}

/// ERROR for a construction of an implemented statement that sasrs cannot
/// honor.
fn unsupported_construct(msg: &str, span: Span) -> SasError {
    SasError::parse(
        format!(
            "{msg} is not supported in PROC DATASETS; it can affect results and cannot be \
             ignored."
        ),
        span,
    )
}

/// A MODIFY sub-statement outside a MODIFY group is used out of proper order
/// (FORMAT and LABEL used to get the shared display WARNING although, here,
/// they change the stored metadata).
fn misplaced_statement(kw: &str, span: Span) -> SasError {
    SasError::parse(
        format!(
            "180-322: Statement '{}' is not valid or it is used out of proper order in PROC \
             DATASETS.",
            kw.to_ascii_uppercase()
        ),
        span,
    )
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
        } else if ts.peek().is_kw("kill") {
            // J02-P4 — KILL deletes every member of the library: honoring it
            // is out of scope here, ignoring it would be destructive-by-omission
            // of intent → ERROR via the shared contract helper.
            return Err(crate::procs::common::unsupported_statement(
                "DATASETS", "KILL",
            ));
        } else {
            // J02-P4 — unknown header option: no silent skip to `;`.
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unexpected option '{bad}' on PROC DATASETS statement."),
                ts.peek().span,
            ));
        }
    }
    Ok((lib, nolist))
}

/// J02-P12 — member list shared by DELETE, SAVE, CHANGE, EXCHANGE and
/// SELECT (Base SAS 9.4 Procedures Guide, DATASETS Procedure: `SAS-file-1
/// <...SAS-file-n> </ ALTER=… GENNUM=… MEMTYPE=mtype>`). Base : tout jeton
/// autre qu'un nom était sauté jusqu'au `;` — `delete m / memtype=catalog;`
/// supprimait la TABLE M (perte de données). Désormais :
/// - `/ MEMTYPE=DATA` (alias `MT=`, `MTYPE=`) est honoré (doc SAS 9.4 DELETE
///   statement : « restricts processing to one member type », « Alias: MT=,
///   MTYPE= », « Default: DATA ») : chaque membre d'une bibliothèque sasrs
///   est une table DATA, le comportement est inchangé ;
/// - toute autre valeur de MEMTYPE= ou toute autre option, une liste entre
///   parenthèses (options par fichier : ALTER=, GENNUM=…) et une plage de
///   noms (`x1-x3`, `x:`) sont une ERROR avant toute exécution du groupe
///   (aucune table supprimée ni renommée) : les ignorer peut changer quelles
///   tables sont traitées.
///
/// Consumes through the terminating `;`.
fn parse_member_list(ts: &mut StatementStream, stmt: &str) -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi | TokenKind::Eof => break,
            TokenKind::Slash => parse_member_options(ts, stmt)?,
            TokenKind::LParen => {
                return Err(unsupported_construct(
                    &format!(
                        "A member list or member options in parentheses in the {stmt} statement"
                    ),
                    tok.span,
                ));
            }
            TokenKind::Ident(name) => {
                if matches!(ts.peek2().kind, TokenKind::Minus | TokenKind::Colon) {
                    return Err(unsupported_construct(
                        &format!(
                            "A member list (range or prefix starting at {}) in the {stmt} \
                             statement",
                            name.to_uppercase()
                        ),
                        tok.span,
                    ));
                }
                names.push(name.to_uppercase());
                ts.next();
            }
            _ => {
                return Err(SasError::parse(
                    format!("expected a member name in the {stmt} statement"),
                    tok.span,
                ));
            }
        }
    }
    // consume trailing `;`
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    Ok(names)
}

/// Options after the `/` of a member list (J02-P12). Only `MEMTYPE=DATA`
/// (aliases `MT=`, `MTYPE=`) is honored; every other value or option is an
/// ERROR. Does not consume the terminating `;`.
fn parse_member_options(ts: &mut StatementStream, stmt: &str) -> Result<()> {
    ts.next(); // consume `/`
    loop {
        if matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
            return Ok(());
        }
        let tok = ts.peek().clone();
        let Some(opt) = tok.ident().map(|s| s.to_ascii_uppercase()) else {
            return Err(SasError::parse(
                format!("expected an option after / in the {stmt} statement"),
                tok.span,
            ));
        };
        if matches!(opt.as_str(), "MEMTYPE" | "MT" | "MTYPE") && ts.peek2().kind == TokenKind::Eq {
            ts.next(); // option keyword
            ts.next(); // `=`
            let val = ts.peek().clone();
            if val
                .ident()
                .map(|v| v.eq_ignore_ascii_case("DATA"))
                .unwrap_or(false)
            {
                ts.next();
                continue;
            }
            let bad = val.ident().unwrap_or("").to_ascii_uppercase();
            return Err(unsupported_construct(
                &format!("The {opt}={bad} option of the {stmt} statement"),
                tok.span,
            ));
        }
        let has_value = ts.peek2().kind == TokenKind::Eq;
        return Err(unsupported_construct(
            &format!(
                "The {opt}{} option of the {stmt} statement",
                if has_value { "=" } else { "" }
            ),
            tok.span,
        ));
    }
}

/// `delete m1 m2 ... ;` — uppercased member names.
pub(super) fn parse_delete_stmt(ts: &mut StatementStream) -> Result<Vec<String>> {
    ts.next(); // consume "delete"
    parse_member_list(ts, "DELETE")
}

/// `change old=new ... ;` — uppercased (old, new) pairs.
pub(super) fn parse_change_stmt(ts: &mut StatementStream) -> Result<Vec<(String, String)>> {
    ts.next(); // consume "change"
    let mut changes: Vec<(String, String)> = Vec::new();
    // Read one or more `old=new` pairs until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().kind == TokenKind::Slash {
            // J02-P12 — options after `/` (MEMTYPE=DATA honored).
            parse_member_options(ts, "CHANGE")?;
            continue;
        }
        let old_tok = ts.peek().clone();
        let Some(old_name) = old_tok.ident().map(str::to_string) else {
            // J02-P12 — no silent skip to `;`: a parenthesized list or a
            // name range can change which tables are renamed.
            if ts.peek().kind == TokenKind::LParen {
                return Err(unsupported_construct(
                    "A member list or member options in parentheses in the CHANGE statement",
                    old_tok.span,
                ));
            }
            return Err(SasError::parse(
                "expected a member name in the CHANGE statement",
                old_tok.span,
            ));
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
    Ok(changes)
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
            // J02-P4 — unknown COPY option: skipping the rest of the statement
            // silently could drop a requested transformation → ERROR.
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unexpected option '{bad}' on PROC DATASETS COPY statement."),
                ts.peek().span,
            ));
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
        select = parse_member_list(ts, "SELECT")?;
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
        if ts.peek().kind == TokenKind::Slash {
            // J02-P12 — options after `/` (MEMTYPE=DATA honored).
            parse_member_options(ts, "EXCHANGE")?;
            continue;
        }
        let a_tok = ts.peek().clone();
        let Some(a) = a_tok.ident().map(str::to_string) else {
            // J02-P12 — no silent skip to `;`.
            if ts.peek().kind == TokenKind::LParen {
                return Err(unsupported_construct(
                    "A member list or member options in parentheses in the EXCHANGE statement",
                    a_tok.span,
                ));
            }
            return Err(SasError::parse(
                "expected a member name in the EXCHANGE statement",
                a_tok.span,
            ));
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
pub(super) fn parse_save_stmt(ts: &mut StatementStream) -> Result<DsOp> {
    ts.next(); // consume "save"
    Ok(DsOp::Save(parse_member_list(ts, "SAVE")?))
}

/// `modify m ; [rename old=new ...;] [label v='..' ...;] [format v fmt. ...;]
/// [informat v token ...;]` — the sub-statements form the MODIFY group, in
/// source order.
///
/// J02-P8 — FORMAT is a MODIFY sub-statement (SAS 9.4 DATASETS FORMAT
/// statement, « Must appear in a MODIFY RUN group ») : it used to end the
/// group, so a following RENAME/INFORMAT was reported « 180-322 … not
/// valid » and a following LABEL was dropped with a display WARNING.
/// Comments, empty statements and global statements stay inside the group.
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
    match ts.peek().kind {
        TokenKind::LParen => {
            return Err(unsupported_construct(
                "A data set option list on the MODIFY statement",
                ts.peek().span,
            ));
        }
        TokenKind::Slash => {
            return Err(unsupported_construct(
                "An option after / on the MODIFY statement",
                ts.peek().span,
            ));
        }
        _ => ts.expect_semi()?,
    }
    let mut stmts: Vec<ModifyStmt> = Vec::new();
    loop {
        if crate::procs::common::parse_proc_inert_or_global(ts)? {
            continue;
        }
        let kw = ts.peek().ident().unwrap_or("").to_ascii_lowercase();
        match kw.as_str() {
            "rename" => {
                let mut renames = Vec::new();
                parse_modify_renames(ts, &mut renames)?;
                stmts.push(ModifyStmt::Rename(renames));
            }
            "label" => {
                let mut labels = Vec::new();
                parse_modify_labels(ts, &mut labels)?;
                stmts.push(ModifyStmt::Label(labels));
            }
            "format" => stmts.push(ModifyStmt::Format(parse_modify_formats(ts)?)),
            "informat" => stmts.push(ModifyStmt::Informat(parse_modify_informats(ts)?)),
            _ => break,
        }
    }
    Ok(DsOp::Modify {
        member: member.to_uppercase(),
        stmts,
    })
}

/// MODIFY sub-statement `format v1 v2 fmt. v3 ... ;` (J02-P8). Base SAS 9.4
/// Procedures Guide, DATASETS FORMAT statement : `FORMAT variable-1 <...
/// variable-n> <format-1> <...variable-n <format-n>>;` — « specifies a format
/// to apply to the variable or variables listed before it. If you do not
/// specify a format, the FORMAT statement removes any format associated with
/// the variables in variable-list. » The format token is read like in the
/// DATA step FORMAT statement (`read_format_token`, `date9` + `.` adjacent)
/// and validated here, before any statement of the step runs.
pub(super) fn parse_modify_formats(
    ts: &mut StatementStream,
) -> Result<Vec<(Vec<String>, Option<String>)>> {
    ts.next(); // consume "format"
    let mut groups: Vec<(Vec<String>, Option<String>)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi | TokenKind::Eof => {
                if !names.is_empty() {
                    groups.push((std::mem::take(&mut names), None));
                }
                if groups.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the FORMAT statement",
                        tok.span,
                    ));
                }
                if tok.kind == TokenKind::Semi {
                    ts.next();
                }
                return Ok(groups);
            }
            TokenKind::Ident(name) if !ident_begins_format(ts) => {
                let upper = name.to_ascii_uppercase();
                if matches!(
                    upper.as_str(),
                    "_ALL_" | "_NUMERIC_" | "_CHARACTER_" | "_CHAR_"
                ) {
                    return Err(unsupported_construct(
                        &format!("The variable list {upper} in the FORMAT statement"),
                        tok.span,
                    ));
                }
                if matches!(ts.peek2().kind, TokenKind::Minus | TokenKind::Colon) {
                    return Err(unsupported_construct(
                        &format!(
                            "A variable list (range or prefix starting at {upper}) in the \
                             FORMAT statement"
                        ),
                        tok.span,
                    ));
                }
                names.push(name.clone());
                ts.next();
            }
            TokenKind::Ident(_) | TokenKind::Dollar | TokenKind::Num(_) | TokenKind::Dot => {
                if names.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name before the format in the FORMAT statement",
                        tok.span,
                    ));
                }
                let token = crate::parser::expr::read_format_token(ts)?;
                if crate::formats::FormatSpec::parse(&token).is_none() {
                    return Err(SasError::parse(
                        format!("The format {token} is not valid."),
                        tok.span,
                    ));
                }
                groups.push((std::mem::take(&mut names), Some(token)));
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name or a format in the FORMAT statement",
                    tok.span,
                ));
            }
        }
    }
}

/// True when the current Ident starts a format token rather than naming a
/// variable : the NEXT token touches it and is a format piece (`$`, a number
/// or `.`). `date9.` is a format, `weight 8.2` keeps `weight` as a name. Same
/// rule as the DATA step FORMAT statement (`parser::datastep`).
fn ident_begins_format(ts: &StatementStream) -> bool {
    let cur = ts.peek();
    let next = ts.peek2();
    next.span.start == cur.span.end
        && matches!(
            next.kind,
            TokenKind::Dollar | TokenKind::Num(_) | TokenKind::Dot
        )
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

/// MODIFY sub-statement `informat v1 v2 fmt. v3 ... ;` (J07-P6, regroupé en
/// J02-P12) — Base SAS 9.4 Procedures Guide, DATASETS Procedure, INFORMAT
/// statement : `INFORMAT variable-1 <informat-1> <…variable-n <informat-n>>;`
/// — l'informat « specifies an informat for the variables immediately
/// preceding it in the statement. If you do not specify an informat, the
/// INFORMAT statement removes any existing informats for the variables in
/// variable-list » (exemple de la doc : `informat a b 2. x1-x3 4.1 c;`).
///
/// J02-P12 — base : une variable était lue par informat (`informat a b
/// date9.;` associait l'informat « b » à A puis échouait). Désormais chaque
/// liste de variables reçoit l'informat qui la suit, comme le statement
/// FORMAT ; une liste finale sans informat retire l'informat. Le token est
/// lu par `read_format_token` (robuste au découpage du lexer) et validé
/// ici, avant toute exécution de l'étape.
pub(super) fn parse_modify_informats(
    ts: &mut StatementStream,
) -> Result<Vec<(Vec<String>, Option<String>)>> {
    ts.next(); // consume "informat"
    let mut groups: Vec<(Vec<String>, Option<String>)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi | TokenKind::Eof => {
                if !names.is_empty() {
                    groups.push((std::mem::take(&mut names), None));
                }
                if groups.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the INFORMAT statement",
                        tok.span,
                    ));
                }
                if tok.kind == TokenKind::Semi {
                    ts.next();
                }
                return Ok(groups);
            }
            TokenKind::Ident(name) if !ident_begins_format(ts) => {
                let upper = name.to_ascii_uppercase();
                if matches!(
                    upper.as_str(),
                    "_ALL_" | "_NUMERIC_" | "_CHARACTER_" | "_CHAR_"
                ) {
                    return Err(unsupported_construct(
                        &format!("The variable list {upper} in the INFORMAT statement"),
                        tok.span,
                    ));
                }
                if matches!(ts.peek2().kind, TokenKind::Minus | TokenKind::Colon) {
                    return Err(unsupported_construct(
                        &format!(
                            "A variable list (range or prefix starting at {upper}) in the \
                             INFORMAT statement"
                        ),
                        tok.span,
                    ));
                }
                names.push(name.clone());
                ts.next();
            }
            TokenKind::Ident(_) | TokenKind::Dollar | TokenKind::Num(_) | TokenKind::Dot => {
                if names.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name before the informat in the INFORMAT statement",
                        tok.span,
                    ));
                }
                let token = crate::parser::expr::read_format_token(ts)?;
                if crate::formats::FormatSpec::parse(&token).is_none() {
                    return Err(SasError::parse(
                        format!("The informat {token} is not valid."),
                        tok.span,
                    ));
                }
                groups.push((std::mem::take(&mut names), Some(token)));
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name or an informat in the INFORMAT statement",
                    tok.span,
                ));
            }
        }
    }
}
