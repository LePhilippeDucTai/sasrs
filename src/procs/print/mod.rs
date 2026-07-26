//! PROC PRINT (jalon M1).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! ## Syntaxe M1
//! `proc print data=lib.x [noobs] [label] ; [var v1 v2... ;] run ;`
//!
//! ## Exécution
//! 1. Résoudre le dataset (data= ou _LAST_) ; lire via LibraryProvider ;
//!    forwarder les notes de coercition au log.
//! 2. Colonnes : `var` si présent (ERROR si variable inconnue :
//!    "Variable XXXX not found."), sinon toutes dans l'ordre du dataset.
//! 3. Rendu listing : `page_header()` puis table —
//!    - colonne `Obs` (1..n, alignée droite) sauf NOOBS ;
//!    - numériques : format de la variable si défini (M4 — avant cela
//!      BEST12. trimé via `value::format_best(v, 12)`), missings `.` ou
//!      lettre spéciale ; alignés DROITE ;
//!    - caractères : tels quels, alignés GAUCHE.
//! 4. NOTEs log : "There were N observations read from the data set
//!    WORK.X." (l'appelant procs::execute_proc ajoute la NOTE de timing).

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::num_to_value;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::session::Session;
use crate::value::{Value, format_best};

mod render;

pub(crate) use render::*;

pub struct PrintAst {
    pub data: Option<DatasetRef>,
    pub vars: Option<Vec<String>>,
    pub noobs: bool,
    /// Option LABEL : utilise le libellé de chaque variable (s'il existe)
    /// comme en-tête de colonne au lieu du nom. Défaut = noms (comme SAS).
    pub label: bool,
    /// Option DOUBLE : double-interligne les lignes de données (M33.6).
    pub double: bool,
    /// Option N : imprime une ligne "N = <n>" du nombre d'observations en fin
    /// de section (par groupe BY si BY présent) (M33.6).
    pub n: bool,
    /// Statement BY : sections par groupe BY (entrée triée requise). Liste de
    /// (variable, descending) comme PROC SORT (M33.6).
    pub by: Vec<(String, bool)>,
    /// Statement ID : remplace la colonne `Obs` par les valeurs de ces
    /// variables à gauche de chaque ligne (M33.6).
    pub id: Vec<String>,
    /// Statement SUM : variables numériques à totaliser en bas (sous-totaux par
    /// groupe BY + total général) (M33.6).
    pub sum: Vec<String>,
}

/// Parse `proc print [data=lib.t] [noobs] [label] [double] [n] ;
///        [var ...;] [by ...;] [id ...;] [sum ...;] ... run ;`
/// Called AFTER "proc print" has been consumed. Consumes through `run;`.
pub fn parse(ts: &mut StatementStream) -> Result<PrintAst> {
    let mut data: Option<DatasetRef> = None;
    let mut noobs = false;
    let mut label = false;
    let mut double = false;
    let mut n = false;
    let mut vars: Option<Vec<String>> = None;
    let mut by: Vec<(String, bool)> = Vec::new();
    let mut id: Vec<String> = Vec::new();
    let mut sum: Vec<String> = Vec::new();

    // En-tête PROC PRINT : options jusqu'au `;` (combinateur partagé M31).
    common::parse_proc_options(ts, "PRINT", |ts, kw| {
        Ok(match kw {
            "data" => {
                data = Some(common::parse_dataset_opt(ts, "DATA")?);
                true
            }
            "noobs" => {
                ts.next();
                noobs = true;
                true
            }
            "label" => {
                // LABEL option: utilise les libellés comme en-têtes (M4).
                ts.next();
                label = true;
                true
            }
            "double" => {
                ts.next();
                double = true;
                true
            }
            "n" => {
                ts.next();
                n = true;
                true
            }
            _ => false,
        })
    })?;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next(); // consume "var"
                vars = Some(common::parse_var_list(ts)?);
                true
            }
            "by" => {
                ts.next(); // consume "by"
                by = common::parse_by(ts)?;
                true
            }
            "id" => {
                ts.next(); // consume "id"
                id = common::parse_var_list(ts)?;
                true
            }
            "sum" => {
                ts.next(); // consume "sum"
                sum = common::parse_var_list(ts)?;
                true
            }
            _ => false,
        })
    })?;

    Ok(PrintAst {
        data,
        vars,
        noobs,
        label,
        double,
        n,
        by,
        id,
        sum,
    })
}

/// Execute PROC PRINT. Called by `procs::execute_proc` which wraps with timing.
pub fn execute(ast: &PrintAst, session: &mut Session) -> Result<()> {
    // Resolve dataset reference (data= or _LAST_) and read it (MQ6.1).
    let (ds, display_name) = common::open_input_display(&ast.data, session)?;

    let n_obs = ds.n_obs();

    // Determine columns to print
    let col_indices = resolve_print_columns(&ds, ast)?;

    // ── M33.6 : résolution des statements ID / SUM / BY ──────────────────────
    // ID variables (gauche, remplacent Obs). Validés contre le dataset.
    let id_indices = resolve_names(&ds, &ast.id)?;
    // SUM variables (numériques totalisées en bas). Validées.
    let sum_indices = resolve_names(&ds, &ast.sum)?;
    // BY columns (entrée triée requise).
    let by_cols = common::resolve_by_cols(&ds, &ast.by)?;

    let use_id = !id_indices.is_empty();

    // Build headers and alignments.
    let (headers, aligns) = build_headers(&ds, ast, &id_indices, &col_indices, use_id);

    // Décode chaque colonne UNE seule fois (downcast par colonne, jamais
    // par cellule — checklist PLAN.md point 3). On formate à la fois les
    // colonnes ID et les colonnes de données. All cell formatting is done
    // here, before any &mut session use below.
    let mut id_cells: Vec<Vec<String>> = Vec::with_capacity(id_indices.len());
    for &col_i in &id_indices {
        id_cells.push(format_column(&ds, session, col_i)?);
    }
    let mut col_cells: Vec<Vec<String>> = Vec::with_capacity(col_indices.len());
    for &col_i in &col_indices {
        col_cells.push(format_column(&ds, session, col_i)?);
    }

    // For SUM: decode the raw numeric values of each sum variable (once).
    let mut sum_values: Vec<Vec<Value>> = Vec::with_capacity(sum_indices.len());
    for &col_i in &sum_indices {
        sum_values.push(common::decode_column(&ds, col_i)?);
    }
    // Column position (within the rendered row) of each SUM variable, so the
    // totals line places each total under its column. A sum var must be a
    // displayed data column (in col_indices); if not displayed it is ignored
    // for placement (SAS would still total it, but it has no column to sit in).
    let sum_render_pos: Vec<Option<usize>> = sum_indices
        .iter()
        .map(|&si| {
            col_indices.iter().position(|&ci| ci == si).map(|p| {
                // offset by ID columns + optional Obs column on the left
                id_indices.len() + usize::from(!ast.noobs && !use_id) + p
            })
        })
        .collect();
    let ctx = RenderCtx {
        headers,
        aligns,
        id_cells,
        col_cells,
        sum_indices: &sum_indices,
        sum_values,
        sum_render_pos,
        obs_col: !ast.noobs && !use_id,
        double: ast.double,
        n_flag: ast.n,
    };

    // ── Rendu ────────────────────────────────────────────────────────────────
    session.listing.page_header();

    if by_cols.is_empty() {
        render_plain(session, &ctx, n_obs);
    } else {
        render_by_groups(session, &ctx, &ds, &by_cols, n_obs, &display_name)?;
    }

    // Log NOTE — "There were N observations read from the data set WORK.X."
    // PLAN.md checklist item 7: pluriel invariable ("1 observations." — fidèle à SAS)
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_obs, display_name
    ));

    Ok(())
}

/// Columns to print: the VAR list (validated) or every dataset column.
fn resolve_print_columns(ds: &crate::dataset::SasDataset, ast: &PrintAst) -> Result<Vec<usize>> {
    if let Some(ref var_names) = ast.vars {
        // Validate each name
        let mut idxs = Vec::with_capacity(var_names.len());
        for vname in var_names {
            let idx = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(vname));
            match idx {
                Some(i) => idxs.push(i),
                None => {
                    return Err(SasError::runtime(format!(
                        "Variable {} not found.",
                        vname.to_uppercase()
                    )));
                }
            }
        }
        Ok(idxs)
    } else {
        Ok((0..ds.vars.len()).collect())
    }
}

/// Headers + alignments.
///
/// Layout: [ID cols | Obs | VAR/data cols]. With ID, the `Obs` column is
/// suppressed (ID values replace it as the row identifier on the left).
fn build_headers(
    ds: &crate::dataset::SasDataset,
    ast: &PrintAst,
    id_indices: &[usize],
    col_indices: &[usize],
    use_id: bool,
) -> (Vec<String>, Vec<Align>) {
    let mut headers: Vec<String> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();

    let header_of = |idx: usize| -> String {
        match (ast.label, &ds.vars[idx].label) {
            (true, Some(lbl)) if !lbl.is_empty() => lbl.clone(),
            _ => ds.vars[idx].name.clone(),
        }
    };
    let align_of = |idx: usize| -> Align {
        match ds.vars[idx].ty {
            crate::value::VarType::Num => Align::Right,
            crate::value::VarType::Char => Align::Left,
        }
    };

    for &idx in id_indices {
        headers.push(header_of(idx));
        aligns.push(align_of(idx));
    }
    if !ast.noobs && !use_id {
        headers.push("Obs".to_string());
        aligns.push(Align::Right);
    }
    for &idx in col_indices {
        headers.push(header_of(idx));
        aligns.push(align_of(idx));
    }
    (headers, aligns)
}

/// Format one column into display cells (one downcast per column, never per
/// cell). Uses the session's format catalog and MISSING= character.
fn format_column(
    ds: &crate::dataset::SasDataset,
    session: &Session,
    col_i: usize,
) -> Result<Vec<String>> {
    let cat = &session.format_catalog;
    // M38.2 — MISSING= : character for ordinary numeric missing ('.').
    let missing_char = session.options.missing_char;

    let series = ds.df.get_columns()[col_i].as_materialized_series();
    let spec = ds.vars[col_i]
        .format
        .as_deref()
        .and_then(crate::formats::FormatSpec::parse);
    let cells: Vec<String> = match ds.vars[col_i].ty {
        crate::value::VarType::Num => series
            .f64()?
            .iter()
            .map(|o| {
                let v = num_to_value(o);
                match &spec {
                    Some(spec) => cat.format(&v, spec),
                    None => match v {
                        // Ordinary missing `.` uses the session MISSING= char.
                        // Special missings keep their SAS suffix (._/.A..Z).
                        Value::Missing(crate::value::MissingKind::Dot) => missing_char.to_string(),
                        Value::Missing(kind) => kind.display(),
                        Value::Num(f) => format_best(f, 12),
                        Value::Char(_) => unreachable!("num column decoded to char"),
                    },
                }
            })
            .collect(),
        crate::value::VarType::Char => series
            .str()?
            .iter()
            .map(|o| {
                let raw = o.unwrap_or("");
                match &spec {
                    Some(spec) => cat.format(&Value::Char(raw.to_string()), spec),
                    None => raw.to_string(),
                }
            })
            .collect(),
    };
    Ok(cells)
}

#[cfg(test)]
mod tests;
