//! PROC DATASETS (jalon M7) — run-group proc terminée par QUIT;.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc datasets lib=work [nolist] ; delete a b ; change old=new ;
//! [run ;] quit ;`
//!
//! - Run-group : les statements s'exécutent à chaque `run;` ET à
//!   `quit;` — M7 : exécution à quit; suffit, documenter l'écart.
//!   DEVIATION : sub-statements are accumulated and executed all at once at
//!   `quit;`. SAS would execute them at each `run;` too; we only execute at
//!   `quit;` for simplicity. A `run;` sub-statement is treated as a no-op
//!   separator.
//! - Sans NOLIST : afficher le répertoire de la librairie APRÈS modifications
//!   (table Name / Member Type DATA / nb obs).
//! - `delete` → `LibraryProvider::delete` (inexistant → WARNING comme
//!   SAS, pas ERROR) ; `change old=new` → rename.
//! - Order of operations: `delete`s first, then `change`s (all in declaration
//!   order within each group). Tables deleted first cannot be renamed.

use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;

pub struct DatasetsAst {
    pub lib: String,
    pub nolist: bool,
    pub deletes: Vec<String>,
    pub changes: Vec<(String, String)>,
    /// Ordered M33.8 operations (COPY / EXCHANGE / SAVE / MODIFY), executed in
    /// declaration order AFTER the legacy `deletes` then `changes` groups.
    pub ops: Vec<DsOp>,
}

/// One member/variable-level operation added in M33.8.
#[derive(Debug, Clone, PartialEq)]
pub enum DsOp {
    /// `copy out=<dst> [in=<src>]; [select m1 m2;]` — copy members from the
    /// source library (defaults to the PROC's LIB=) to the destination library.
    /// Empty `select` means "all members of the source library".
    Copy {
        out: String,
        r#in: Option<String>,
        select: Vec<String>,
    },
    /// `exchange a=b;` — swap the two member names (atomic name swap).
    Exchange(String, String),
    /// `save m1 m2;` — delete every member of LIB= except the listed ones.
    Save(Vec<String>),
    /// `modify m; [rename old=new ...;] [label v='..' ...;]` — variable-level
    /// edits on member `m`.
    Modify {
        member: String,
        renames: Vec<(String, String)>,
        labels: Vec<(String, String)>,
    },
}

/// Parse `proc datasets [lib=<ident>] [nolist] ; ... quit ;`
/// Called AFTER "proc datasets" has been consumed. Consumes through `quit;`.
/// Defaults to lib=WORK if the LIB= option is absent.
pub fn parse(ts: &mut StatementStream) -> Result<DatasetsAst> {
    let mut lib = "WORK".to_string();
    let mut nolist = false;
    let mut deletes: Vec<String> = Vec::new();
    let mut changes: Vec<(String, String)> = Vec::new();
    let mut ops: Vec<DsOp> = Vec::new();

    // ── Parse PROC DATASETS header options until `;` ─────────────────────────
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
                return Err(SasError::parse(
                    "expected '=' after LIB",
                    ts.peek().span,
                ));
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
            continue;
        }

        if ts.peek().is_kw("change") {
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
            continue;
        }

        // ── COPY out=<dst> [in=<src>]; [select m1 m2;] ─────────────────────
        if ts.peek().is_kw("copy") {
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
            ops.push(DsOp::Copy { out, r#in: in_lib, select });
            continue;
        }

        // ── EXCHANGE a=b ; ──────────────────────────────────────────────────
        if ts.peek().is_kw("exchange") {
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
            continue;
        }

        // ── SAVE m1 m2 ... ; ────────────────────────────────────────────────
        if ts.peek().is_kw("save") {
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
            ops.push(DsOp::Save(keep));
            continue;
        }

        // ── MODIFY m ; [rename old=new ...;] [label v='..' ...;] ────────────
        if ts.peek().is_kw("modify") {
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
                } else if ts.peek().is_kw("label") {
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
                } else {
                    break;
                }
            }
            ops.push(DsOp::Modify { member: member.to_uppercase(), renames, labels });
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

/// Helper for COPY's `out=`/`in=` options: consume the option keyword, require
/// `=`, then read the libref name (uppercased) into `slot`.
fn in_lib_assign(ts: &mut StatementStream, slot: &mut Option<String>) -> Result<()> {
    ts.next(); // consume option keyword (out/in)
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            "expected '=' after COPY OUT=/IN= option",
            ts.peek().span,
        ));
    }
    ts.next(); // consume `=`
    let tok = ts.peek().clone();
    let Some(name) = tok.ident().map(str::to_string) else {
        return Err(SasError::parse(
            "expected a libref name in COPY OUT=/IN= option",
            tok.span,
        ));
    };
    ts.next();
    *slot = Some(name.to_uppercase());
    Ok(())
}

/// Find a member name not currently used in `provider`, for the EXCHANGE swap.
fn unique_temp_name(provider: &dyn crate::library::LibraryProvider) -> String {
    let mut i = 0u32;
    loop {
        let candidate = format!("__SASRS_XCHG_{i}__");
        if !provider.exists(&candidate) {
            return candidate;
        }
        i += 1;
    }
}

/// Execute PROC DATASETS.
pub fn execute(ast: &DatasetsAst, session: &mut Session) -> Result<()> {
    let lib = ast.lib.to_uppercase();
    let provider = session.libs.get(&lib).map_err(|_| {
        SasError::runtime(format!("Libref {} is not assigned.", lib))
    })?;

    // ── Apply deletes ─────────────────────────────────────────────────────────
    for name in &ast.deletes {
        let name_upper = name.to_uppercase();
        if provider.exists(&name_upper) {
            provider.delete(&name_upper)?;
            session.log.note(&format!(
                "Deleting {}.{} (memtype=DATA).",
                lib, name_upper
            ));
        } else {
            session.log.warning(&format!(
                "Table {}.{} does not exist and was not deleted.",
                lib, name_upper
            ));
        }
    }

    // ── Apply renames (CHANGE old=new) ────────────────────────────────────────
    for (old, new) in &ast.changes {
        let old_upper = old.to_uppercase();
        let new_upper = new.to_uppercase();
        provider.rename(&old_upper, &new_upper)?;
        session.log.note(&format!(
            "Changing the name {}.{} to {}.{} (memtype=DATA).",
            lib, old_upper, lib, new_upper
        ));
    }

    // ── M33.8 operations (COPY / EXCHANGE / SAVE / MODIFY) ────────────────────
    for op in &ast.ops {
        match op {
            DsOp::Copy { out, r#in, select } => {
                let src_lib = r#in.clone().unwrap_or_else(|| lib.clone());
                let src = session.libs.get(&src_lib).map_err(|_| {
                    SasError::runtime(format!("Libref {} is not assigned.", src_lib))
                })?;
                let dst = session.libs.get(out).map_err(|_| {
                    SasError::runtime(format!("Libref {} is not assigned.", out))
                })?;
                // Members to copy: SELECT list, or every member of the source.
                let members: Vec<String> = if select.is_empty() {
                    let mut m = src.list()?;
                    m.sort();
                    m
                } else {
                    select.clone()
                };
                for m in &members {
                    let mu = m.to_uppercase();
                    if !src.exists(&mu) {
                        session.log.warning(&format!(
                            "Member {}.{} not found; not copied.",
                            src_lib, mu
                        ));
                        continue;
                    }
                    let (ds, notes) = src.read(&mu)?;
                    for note in notes {
                        session.log.forward(&note);
                    }
                    dst.write(&mu, &ds)?;
                    session.log.note(&format!(
                        "Copying {}.{} to {}.{} (memtype=DATA).",
                        src_lib, mu, out, mu
                    ));
                }
            }
            DsOp::Exchange(a, b) => {
                let a = a.to_uppercase();
                let b = b.to_uppercase();
                // Swap the two members via a temporary name in the same lib.
                if !provider.exists(&a) || !provider.exists(&b) {
                    session.log.warning(&format!(
                        "Cannot exchange {lib}.{a} and {lib}.{b}: one or both do not exist."
                    ));
                } else {
                    // Pick a temp name that does not already exist.
                    let tmp = unique_temp_name(provider.as_ref());
                    provider.rename(&a, &tmp)?;
                    provider.rename(&b, &a)?;
                    provider.rename(&tmp, &b)?;
                    session.log.note(&format!(
                        "Exchanging the names {lib}.{a} and {lib}.{b} (memtype=DATA)."
                    ));
                }
            }
            DsOp::Save(keep) => {
                let keep_upper: Vec<String> = keep.iter().map(|s| s.to_uppercase()).collect();
                let mut tables = provider.list()?;
                tables.sort();
                for t in &tables {
                    let tu = t.to_uppercase();
                    if !keep_upper.contains(&tu) {
                        provider.delete(&tu)?;
                        session.log.note(&format!(
                            "Deleting {lib}.{tu} (memtype=DATA)."
                        ));
                    }
                }
            }
            DsOp::Modify { member, renames, labels } => {
                let mu = member.to_uppercase();
                if !provider.exists(&mu) {
                    session.log.warning(&format!(
                        "Member {lib}.{mu} not found; MODIFY skipped."
                    ));
                    continue;
                }
                let (mut ds, notes) = provider.read(&mu)?;
                for note in notes {
                    session.log.forward(&note);
                }
                // RENAME variables (rename both VarMeta and the DataFrame column).
                for (old, new) in renames {
                    match ds.vars.iter().position(|v| v.name.eq_ignore_ascii_case(old)) {
                        Some(idx) => {
                            let phys_old = ds.vars[idx].name.clone();
                            ds.df.rename(&phys_old, new.as_str().into())?;
                            ds.vars[idx].name = new.clone();
                            session.log.note(&format!(
                                "Variable {} renamed to {} in {lib}.{mu}.",
                                old.to_uppercase(),
                                new.to_uppercase()
                            ));
                        }
                        None => {
                            session.log.warning(&format!(
                                "Variable {} not found in {lib}.{mu}; not renamed.",
                                old.to_uppercase()
                            ));
                        }
                    }
                }
                // LABEL variables.
                for (var, text) in labels {
                    match ds.vars.iter().position(|v| v.name.eq_ignore_ascii_case(var)) {
                        Some(idx) => {
                            ds.vars[idx].label = Some(text.clone());
                        }
                        None => {
                            session.log.warning(&format!(
                                "Variable {} not found in {lib}.{mu}; not labelled.",
                                var.to_uppercase()
                            ));
                        }
                    }
                }
                provider.write(&mu, &ds)?;
            }
        }
    }

    // ── Directory listing (unless NOLIST) ─────────────────────────────────────
    if !ast.nolist {
        let mut tables = provider.list()?;
        tables.sort();

        session.listing.page_header();

        let headers = vec![
            "#".to_string(),
            "Name".to_string(),
            "Member Type".to_string(),
        ];
        let aligns = vec![Align::Right, Align::Left, Align::Left];

        let rows: Vec<Vec<String>> = tables
            .iter()
            .enumerate()
            .map(|(i, t)| {
                vec![
                    (i + 1).to_string(),
                    t.to_uppercase(),
                    "DATA".to_string(),
                ]
            })
            .collect();

        session.listing.write_table(&headers, &aligns, &rows);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::prelude::*;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_datasets_src(src: &str) -> Result<DatasetsAst> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "datasets"
        parse(&mut ts)
    }

    /// Write a simple numeric dataset into WORK.
    fn write_simple_dataset(session: &mut Session, name: &str) {
        let df = df!["x" => [1.0_f64, 2.0]].unwrap();
        let vars = vec![VarMeta {
            name: "x".to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }];
        let ds = SasDataset { df, vars };
        session
            .libs
            .get("WORK")
            .unwrap()
            .write(name, &ds)
            .unwrap();
    }

    /// Write a dataset with a format and label so a sidecar file is created.
    fn write_dataset_with_meta(session: &mut Session, name: &str) {
        let df = df!["age" => [30.0_f64, 25.0]].unwrap();
        let vars = vec![VarMeta {
            name: "age".to_string(),
            ty: VarType::Num,
            length: 8,
            format: Some("best12.".to_string()),
            label: Some("Age".to_string()),
        }];
        let ds = SasDataset { df, vars };
        session
            .libs
            .get("WORK")
            .unwrap()
            .write(name, &ds)
            .unwrap();
    }

    // ── Parse tests ───────────────────────────────────────────────────────────

    #[test]
    fn parse_full_example() {
        let src = "proc datasets lib=work nolist; delete a b; change c=d; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(ast.lib, "WORK");
        assert!(ast.nolist);
        assert_eq!(ast.deletes, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(
            ast.changes,
            vec![("C".to_string(), "D".to_string())]
        );
    }

    #[test]
    fn parse_defaults_to_work() {
        let src = "proc datasets nolist; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(ast.lib, "WORK");
        assert!(ast.nolist);
        assert!(ast.deletes.is_empty());
        assert!(ast.changes.is_empty());
    }

    #[test]
    fn parse_library_alias() {
        let src = "proc datasets library=mylib nolist; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(ast.lib, "MYLIB");
        assert!(ast.nolist);
    }

    #[test]
    fn parse_no_nolist_defaults_false() {
        let src = "proc datasets lib=work; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert!(!ast.nolist);
    }

    #[test]
    fn parse_multiple_changes() {
        let src = "proc datasets lib=work nolist; change a=b c=d; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(
            ast.changes,
            vec![
                ("A".to_string(), "B".to_string()),
                ("C".to_string(), "D".to_string()),
            ]
        );
    }

    #[test]
    fn parse_run_is_noop_separator() {
        // run; between statements should not stop accumulation
        let src = "proc datasets lib=work nolist; delete a; run; change b=c; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(ast.deletes, vec!["A".to_string()]);
        assert_eq!(ast.changes, vec![("B".to_string(), "C".to_string())]);
    }

    // ── Execute tests ─────────────────────────────────────────────────────────

    #[test]
    fn execute_delete_removes_table() {
        let mut session = make_session();
        write_simple_dataset(&mut session, "ALPHA");

        assert!(session.libs.get("WORK").unwrap().exists("ALPHA"));

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: true,
            deletes: vec!["ALPHA".to_string()],
            changes: vec![],
            ops: vec![],
        };
        execute(&ast, &mut session).unwrap();

        assert!(!session.libs.get("WORK").unwrap().exists("ALPHA"));
        let log = session.log.into_string();
        assert!(log.contains("Deleting"), "log: {log}");
        assert!(log.contains("ALPHA"), "log: {log}");
    }

    #[test]
    fn execute_delete_missing_is_warning_not_error() {
        let mut session = make_session();

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: true,
            deletes: vec!["NONEXISTENT".to_string()],
            changes: vec![],
            ops: vec![],
        };
        // Must not return Err
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(
            log.contains("WARNING") || log.contains("does not exist"),
            "log: {log}"
        );
    }

    #[test]
    fn execute_change_renames_table() {
        let mut session = make_session();
        write_simple_dataset(&mut session, "OLDNAME");

        assert!(session.libs.get("WORK").unwrap().exists("OLDNAME"));
        assert!(!session.libs.get("WORK").unwrap().exists("NEWNAME"));

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: true,
            deletes: vec![],
            changes: vec![("OLDNAME".to_string(), "NEWNAME".to_string())],
            ops: vec![],
        };
        execute(&ast, &mut session).unwrap();

        assert!(!session.libs.get("WORK").unwrap().exists("OLDNAME"));
        assert!(session.libs.get("WORK").unwrap().exists("NEWNAME"));

        // Verify data is intact
        let (ds, _) = session
            .libs
            .get("WORK")
            .unwrap()
            .read("NEWNAME")
            .unwrap();
        assert_eq!(ds.n_obs(), 2);

        let log = session.log.into_string();
        assert!(log.contains("Changing"), "log: {log}");
        assert!(log.contains("OLDNAME"), "log: {log}");
        assert!(log.contains("NEWNAME"), "log: {log}");
    }

    #[test]
    fn execute_nolist_suppresses_listing() {
        let mut session = make_session();
        write_simple_dataset(&mut session, "T1");

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: true,
            deletes: vec![],
            changes: vec![],
            ops: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.is_empty(), "listing should be empty with nolist: {listing}");
    }

    #[test]
    fn execute_without_nolist_emits_directory() {
        let mut session = make_session();
        write_simple_dataset(&mut session, "T1");
        write_simple_dataset(&mut session, "T2");

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: false,
            deletes: vec![],
            changes: vec![],
            ops: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("T1"), "listing: {listing}");
        assert!(listing.contains("T2"), "listing: {listing}");
        assert!(listing.contains("DATA"), "Member Type column: {listing}");
        assert!(listing.contains("Name"), "Name header: {listing}");
    }

    #[test]
    fn execute_rename_moves_sidecar() {
        let mut session = make_session();
        write_dataset_with_meta(&mut session, "WITHFORMAT");

        let ast = DatasetsAst {
            lib: "WORK".to_string(),
            nolist: true,
            deletes: vec![],
            changes: vec![("WITHFORMAT".to_string(), "RENAMED".to_string())],
            ops: vec![],
        };
        execute(&ast, &mut session).unwrap();

        assert!(!session.libs.get("WORK").unwrap().exists("WITHFORMAT"));
        assert!(session.libs.get("WORK").unwrap().exists("RENAMED"));

        // Read back and verify format survived the rename (sidecar was moved)
        let (ds, _) = session
            .libs
            .get("WORK")
            .unwrap()
            .read("RENAMED")
            .unwrap();
        let age_var = ds.vars.iter().find(|v| v.name.eq_ignore_ascii_case("age")).unwrap();
        assert_eq!(
            age_var.format.as_deref(),
            Some("best12."),
            "format should survive rename via sidecar move"
        );
        assert_eq!(
            age_var.label.as_deref(),
            Some("Age"),
            "label should survive rename via sidecar move"
        );
    }

    // ── M33.8 : COPY / EXCHANGE / SAVE / MODIFY ───────────────────────────────

    fn base_ast(lib: &str) -> DatasetsAst {
        DatasetsAst {
            lib: lib.to_string(),
            nolist: true,
            deletes: vec![],
            changes: vec![],
            ops: vec![],
        }
    }

    #[test]
    fn parse_copy_with_select() {
        let src = "proc datasets lib=work nolist; copy out=tgt in=src; select a b; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(
            ast.ops,
            vec![DsOp::Copy {
                out: "TGT".into(),
                r#in: Some("SRC".into()),
                select: vec!["A".into(), "B".into()],
            }]
        );
    }

    #[test]
    fn parse_exchange_save_modify() {
        let src = "proc datasets lib=work nolist; \
                   exchange a=b; save keep1 keep2; \
                   modify m; rename old=new; label v='hi'; quit;";
        let ast = parse_datasets_src(src).unwrap();
        assert_eq!(
            ast.ops,
            vec![
                DsOp::Exchange("A".into(), "B".into()),
                DsOp::Save(vec!["KEEP1".into(), "KEEP2".into()]),
                DsOp::Modify {
                    member: "M".into(),
                    renames: vec![("old".into(), "new".into())],
                    labels: vec![("v".into(), "hi".into())],
                },
            ]
        );
    }

    #[test]
    fn execute_copy_moves_member_to_other_lib() {
        let mut session = make_session();
        // Source lib = WORK; destination lib = a fresh assigned dir.
        write_simple_dataset(&mut session, "SRCTAB");
        let tmp = tempfile::tempdir().unwrap();
        session.libs.assign("TGT", tmp.path().to_path_buf()).unwrap();

        let ast = DatasetsAst {
            ops: vec![DsOp::Copy {
                out: "TGT".into(),
                r#in: None, // defaults to WORK (the PROC's lib)
                select: vec!["SRCTAB".into()],
            }],
            ..base_ast("WORK")
        };
        execute(&ast, &mut session).unwrap();

        // Original still in WORK, copy present in TGT.
        assert!(session.libs.get("WORK").unwrap().exists("SRCTAB"));
        assert!(session.libs.get("TGT").unwrap().exists("SRCTAB"));
        let (ds, _) = session.libs.get("TGT").unwrap().read("SRCTAB").unwrap();
        assert_eq!(ds.n_obs(), 2);
    }

    #[test]
    fn execute_exchange_swaps_names() {
        let mut session = make_session();
        // ALPHA holds x=[1,2]; BETA holds a single different row so we can tell
        // them apart after the swap.
        write_simple_dataset(&mut session, "ALPHA"); // x = [1, 2]
        let df = df!["x" => [9.0_f64]].unwrap();
        let vars = vec![VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None }];
        session.libs.get("WORK").unwrap().write("BETA", &SasDataset { df, vars }).unwrap();

        let ast = DatasetsAst {
            ops: vec![DsOp::Exchange("ALPHA".into(), "BETA".into())],
            ..base_ast("WORK")
        };
        execute(&ast, &mut session).unwrap();

        // After exchange: ALPHA now has BETA's content (1 row), BETA has 2 rows.
        let (a, _) = session.libs.get("WORK").unwrap().read("ALPHA").unwrap();
        let (b, _) = session.libs.get("WORK").unwrap().read("BETA").unwrap();
        assert_eq!(a.n_obs(), 1, "ALPHA should now be old BETA");
        assert_eq!(b.n_obs(), 2, "BETA should now be old ALPHA");
    }

    #[test]
    fn execute_save_deletes_all_but_listed() {
        let mut session = make_session();
        write_simple_dataset(&mut session, "KEEPME");
        write_simple_dataset(&mut session, "DROP1");
        write_simple_dataset(&mut session, "DROP2");

        let ast = DatasetsAst {
            ops: vec![DsOp::Save(vec!["KEEPME".into()])],
            ..base_ast("WORK")
        };
        execute(&ast, &mut session).unwrap();

        assert!(session.libs.get("WORK").unwrap().exists("KEEPME"));
        assert!(!session.libs.get("WORK").unwrap().exists("DROP1"));
        assert!(!session.libs.get("WORK").unwrap().exists("DROP2"));
    }

    #[test]
    fn execute_modify_renames_variable_and_sets_label() {
        let mut session = make_session();
        write_dataset_with_meta(&mut session, "MTAB"); // var "age"

        let ast = DatasetsAst {
            ops: vec![DsOp::Modify {
                member: "MTAB".into(),
                renames: vec![("age".into(), "years".into())],
                labels: vec![("years".into(), "Years old".into())],
            }],
            ..base_ast("WORK")
        };
        execute(&ast, &mut session).unwrap();

        let (ds, _) = session.libs.get("WORK").unwrap().read("MTAB").unwrap();
        // Variable renamed (no "age", has "years"), and label updated.
        assert!(ds.vars.iter().all(|v| !v.name.eq_ignore_ascii_case("age")));
        let years = ds.vars.iter().find(|v| v.name.eq_ignore_ascii_case("years")).unwrap();
        assert_eq!(years.label.as_deref(), Some("Years old"));
        // DataFrame column was renamed too.
        assert!(ds.df.column("years").is_ok(), "df column renamed");
    }
}
