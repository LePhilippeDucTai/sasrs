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


mod parse;

pub use parse::parse;


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
mod tests;
