//! PROC DATASETS (jalon M7) — run-group proc terminée par QUIT;.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc datasets lib=work [nolist] ; delete a b ; change old=new ;
//! [run ;] quit ;`
//!
//! - Ordre d'exécution (J02-P8) : Base SAS 9.4 Procedures Guide, DATASETS
//!   Procedure, Concepts, « Execution of Statements » : « Statements execute
//!   in the order in which they are written. » Chaque statement devient un
//!   [`DsOp`] dans l'ordre du source (DELETE et CHANGE étaient auparavant
//!   regroupés et exécutés avant tout le reste). « RUN-Group Processing » :
//!   « The PROC DATASETS statement always executes immediately » — le
//!   répertoire (sans NOLIST) est donc imprimé AVANT les autres statements,
//!   tel qu'il est à l'ouverture de la procédure.
//! - DEVIATION : les statements d'une étape sont accumulés puis exécutés en
//!   une fois, dans l'ordre du source, à `quit;` (SAS exécute chaque groupe
//!   RUN dès son `run;` ou RUN implicite). Une erreur de parsing rejette
//!   toute l'étape (SAS exécute les groupes RUN précédents) et une erreur
//!   d'exécution arrête les statements suivants. Un `run;` intérieur termine
//!   l'étape : le découpage du programme en segments coupe à chaque `run;`
//!   de niveau supérieur (`src/macros/segmenter.rs`), si bien que les
//!   statements qui le suivent ne sont plus lus comme des statements de
//!   PROC DATASETS (ERROR « … used out of proper order »).
//! - `delete` → `LibraryProvider::delete` (inexistant → WARNING comme
//!   SAS, pas ERROR) ; `change old=new` → rename.
//! - MODIFY : ses sous-statements RENAME, LABEL, FORMAT (J02-P8) et INFORMAT
//!   (J07-P6) s'appliquent eux aussi dans l'ordre du source.
//!
//! ## Contrat J02-P14 — fenêtre OBS=/FIRSTOBS= (lecture de membres)
//!
//! PROC DATASETS est une proc de GESTION de bibliothèque : aucune de ses
//! instructions ne lit les observations d'une table pour les traiter.
//! DELETE, CHANGE, SAVE et EXCHANGE ne consultent que le catalogue
//! (`exists`/`list`) ; COPY recopie le membre ENTIER et MODIFY édite son
//! descripteur — leurs lectures passent par `procs::common::open_resolved_full`
//! (SANS fenêtre OBS=/FIRSTOBS=, comme PROC CONTENTS en J02-P9) : fenêtrer
//! une recopie ou une réécriture de descripteur tronquerait le membre stocké.

use crate::ast::DatasetRef;
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
    /// Statements in source order (J02-P8, SAS 9.4 « Statements execute in
    /// the order in which they are written »).
    pub ops: Vec<DsOp>,
}

/// One PROC DATASETS statement, executed in source order.
#[derive(Debug, Clone, PartialEq)]
pub enum DsOp {
    /// `delete m1 m2 ;` — uppercased member names.
    Delete(Vec<String>),
    /// `change old=new ... ;` — uppercased (old, new) pairs.
    Change(Vec<(String, String)>),
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
    /// `modify m; <sub-statements>` — variable-level edits on member `m`,
    /// applied in source order.
    Modify {
        member: String,
        stmts: Vec<ModifyStmt>,
    },
}

/// One sub-statement of a MODIFY group, applied in source order.
#[derive(Debug, Clone, PartialEq)]
pub enum ModifyStmt {
    /// `rename old=new ... ;`
    Rename(Vec<(String, String)>),
    /// `label v='text' ... ;`
    Label(Vec<(String, String)>),
    /// J02-P8 — `format v1 v2 fmt. v3 ;` : each list of variables takes the
    /// format that follows it ; a list without a format has its format
    /// removed (SAS 9.4 DATASETS FORMAT statement). The format is stored in
    /// the data set metadata (sidecar).
    Format(Vec<(Vec<String>, Option<String>)>),
    /// J07-P6 (regroupé en J02-P12) — `informat v1 v2 fmt. v3 ;` : each list
    /// of variables takes the informat that follows it ; a list without an
    /// informat has its informat removed (SAS 9.4 DATASETS INFORMAT
    /// statement). Persisted via the sidecar.
    Informat(Vec<(Vec<String>, Option<String>)>),
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
/// J04-P2 : « utilisé » couvre aussi un sidecar orphelin (`<name>.parquet.
/// sasmeta.json` sans parquet) — un nom temporaire ne doit entrer en collision
/// avec AUCUN artefact résiduel, sinon le sidecar déplacé de la vraie table
/// écraserait un résidu (ou l'inverse) au lieu de simplement occuper un nom
/// libre.
fn unique_temp_name(provider: &dyn crate::library::LibraryProvider) -> String {
    let mut i = 0u32;
    loop {
        let candidate = format!("__SASRS_XCHG_{i}__");
        if !provider.exists(&candidate) && !provider.sidecar_exists(&candidate) {
            return candidate;
        }
        i += 1;
    }
}

/// Execute PROC DATASETS.
///
/// J02-P8 — the PROC DATASETS statement is the first RUN group and executes
/// immediately (SAS 9.4 « RUN-Group Processing ») : the directory is listed
/// first, then every statement runs in source order.
pub fn execute(ast: &DatasetsAst, session: &mut Session) -> Result<()> {
    let lib = ast.lib.to_uppercase();
    let provider = session
        .libs
        .get(&lib)
        .map_err(|_| SasError::runtime(format!("Libref {} is not assigned.", lib)))?;

    // ── RUN group 1: the PROC DATASETS statement (directory, unless NOLIST) ──
    if !ast.nolist {
        print_directory(session, provider.as_ref())?;
    }

    for op in &ast.ops {
        match op {
            DsOp::Delete(names) => {
                for name in names {
                    let name_upper = name.to_uppercase();
                    if provider.exists(&name_upper) {
                        provider.delete(&name_upper)?;
                        session
                            .log
                            .note(&format!("Deleting {}.{} (memtype=DATA).", lib, name_upper));
                    } else {
                        session.log.warning(&format!(
                            "Table {}.{} does not exist and was not deleted.",
                            lib, name_upper
                        ));
                    }
                }
            }
            DsOp::Change(pairs) => {
                for (old, new) in pairs {
                    let old_upper = old.to_uppercase();
                    let new_upper = new.to_uppercase();
                    provider.rename(&old_upper, &new_upper)?;
                    session.log.note(&format!(
                        "Changing the name {}.{} to {}.{} (memtype=DATA).",
                        lib, old_upper, lib, new_upper
                    ));
                }
            }
            DsOp::Copy { out, r#in, select } => {
                let src_lib = r#in.clone().unwrap_or_else(|| lib.clone());
                let src = session.libs.get(&src_lib).map_err(|_| {
                    SasError::runtime(format!("Libref {} is not assigned.", src_lib))
                })?;
                let dst = session
                    .libs
                    .get(out)
                    .map_err(|_| SasError::runtime(format!("Libref {} is not assigned.", out)))?;
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
                        session
                            .log
                            .warning(&format!("Member {}.{} not found; not copied.", src_lib, mu));
                        continue;
                    }
                    // J02-P14 — lecture du membre via le helper commun
                    // `open_resolved_full` (SANS fenêtre OBS=/FIRSTOBS=) :
                    // COPY recopie le membre ENTIER, c'est une opération de
                    // gestion de bibliothèque (comme PROC CONTENTS, J02-P9),
                    // pas une lecture de données au sens des options système
                    // OBS=/FIRSTOBS= (doc SAS 9.4, System Options).
                    let ds = crate::procs::common::open_resolved_full(
                        &DatasetRef {
                            libref: Some(src_lib.clone()),
                            name: mu.clone(),
                        },
                        session,
                    )?;
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
                    // Pick a temp name free of ANY residual artifact
                    // (parquet or orphan sidecar).
                    let tmp = unique_temp_name(provider.as_ref());
                    // Swap in three renames, each without orphans (J04-P2) :
                    // a→tmp, b→a, tmp→b. Si une étape intermédiaire échoue,
                    // on annule les renommages déjà faits — l'échange est
                    // tout ou rien, jamais de bibliothèque à moitié échangée.
                    provider.rename(&a, &tmp)?;
                    if let Err(e) = provider.rename(&b, &a) {
                        let _ = provider.rename(&tmp, &a); // rollback
                        return Err(e);
                    }
                    if let Err(e) = provider.rename(&tmp, &b) {
                        let _ = provider.rename(&a, &b); // rollback
                        let _ = provider.rename(&tmp, &a); // rollback
                        return Err(e);
                    }
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
                        session
                            .log
                            .note(&format!("Deleting {lib}.{tu} (memtype=DATA)."));
                    }
                }
            }
            DsOp::Modify { member, stmts } => {
                modify_member(session, provider.as_ref(), &lib, member, stmts)?;
            }
        }
    }

    Ok(())
}

/// Directory listing of the PROC DATASETS statement (table Name / Member
/// Type), printed when the procedure starts.
fn print_directory(
    session: &mut Session,
    provider: &dyn crate::library::LibraryProvider,
) -> Result<()> {
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
        .map(|(i, t)| vec![(i + 1).to_string(), t.to_uppercase(), "DATA".to_string()])
        .collect();

    session.listing.write_table(&headers, &aligns, &rows);
    Ok(())
}

/// MODIFY group: read the member once, apply its sub-statements in source
/// order, write it back (data and metadata sidecar).
///
/// J02-P14 — la lecture du membre passe par le helper commun
/// `open_resolved_full` (SANS fenêtre OBS=/FIRSTOBS=) : MODIFY édite le
/// DESCRIPTEUR du membre (noms, formats, libellés) — une lecture de
/// métadonnées au sens de J02-P9 (doc SAS 9.4, DATASETS Procedure) ; l'y
/// fenêtrer réécrirait le membre TRONQUÉ (perte de données).
fn modify_member(
    session: &mut Session,
    provider: &dyn crate::library::LibraryProvider,
    lib: &str,
    member: &str,
    stmts: &[ModifyStmt],
) -> Result<()> {
    let mu = member.to_uppercase();
    if !provider.exists(&mu) {
        session
            .log
            .warning(&format!("Member {lib}.{mu} not found; MODIFY skipped."));
        return Ok(());
    }
    let mut ds = crate::procs::common::open_resolved_full(
        &DatasetRef {
            libref: Some(lib.to_uppercase()),
            name: mu.clone(),
        },
        session,
    )?;
    let position = |ds: &crate::dataset::SasDataset, var: &str| {
        ds.vars
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case(var))
    };
    for stmt in stmts {
        match stmt {
            // RENAME variables (rename both VarMeta and the DataFrame column).
            ModifyStmt::Rename(renames) => {
                for (old, new) in renames {
                    match position(&ds, old) {
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
            }
            // LABEL variables.
            ModifyStmt::Label(labels) => {
                for (var, text) in labels {
                    match position(&ds, var) {
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
            }
            // FORMAT variables (J02-P8) : the token was validated at parse
            // time ; `None` removes the format. Unknown variable → WARNING,
            // like RENAME/LABEL.
            ModifyStmt::Format(groups) => {
                for (vars, format) in groups {
                    for var in vars {
                        match position(&ds, var) {
                            Some(idx) => {
                                ds.vars[idx].format = format.clone();
                            }
                            None => {
                                let what = if format.is_some() {
                                    "format not assigned"
                                } else {
                                    "format not removed"
                                };
                                session.log.warning(&format!(
                                    "Variable {} not found in {lib}.{mu}; {what}.",
                                    var.to_uppercase()
                                ));
                            }
                        }
                    }
                }
            }
            // INFORMAT variables (J07-P6, regroupé en J02-P12 comme le
            // statement FORMAT) : toutes les variables listées reçoivent
            // l'informat qui suit (doc SAS 9.4 DATASETS INFORMAT statement) ;
            // une liste finale sans informat retire l'informat. Variable
            // inconnue → WARNING, comme RENAME/LABEL.
            ModifyStmt::Informat(groups) => {
                for (vars, token) in groups {
                    for var in vars {
                        match position(&ds, var) {
                            Some(idx) => match token {
                                Some(token) => {
                                    ds.vars[idx].informat = Some(token.clone());
                                    session.log.note(&format!(
                                        "Informat {} was assigned to variable {} in {lib}.{mu}.",
                                        token,
                                        var.to_uppercase()
                                    ));
                                }
                                None => {
                                    ds.vars[idx].informat = None;
                                }
                            },
                            None => {
                                let what = if token.is_some() {
                                    "informat not assigned"
                                } else {
                                    "informat not removed"
                                };
                                session.log.warning(&format!(
                                    "Variable {} not found in {lib}.{mu}; {what}.",
                                    var.to_uppercase()
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    provider.write(&mu, &ds)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod contract_j02_p12;

#[cfg(test)]
mod contract_j02_p14;
