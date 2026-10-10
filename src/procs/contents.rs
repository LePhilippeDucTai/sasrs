//! PROC CONTENTS (jalon M4).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc contents data=lib.x [varnum] ; run ;` — affiche les métadonnées :
//! en-tête (nom, observations, variables), puis table des variables
//! (# / Variable / Type Num|Char / Len / Format / Label), triée par nom
//! (défaut) ou par position (VARNUM). Lit uniquement les métadonnées
//! (VarMeta + hauteur) — pas besoin de matérialiser les données :
//! prévoir plus tard un `LibraryProvider::read_meta` si les fichiers
//! deviennent gros.
//! `data=lib._all_` : liste les tables de la librairie (via `list()`).
//!
//! ## Header block layout
//!
//! Two-column layout: left column (~25 chars) holds the label, right column
//! holds the value.  Three lines:
//!
//!   Data Set Name: WORK.CLASS       Observations:  10
//!   Member Type:   DATA             Variables:      3
//!   Engine:        PARQUET
//!
//! Variable table columns (in order): #, Variable, Type, Len, Format, Label.
//! `#` and `Len` are right-aligned; all others left-aligned.

use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::{char_var_meta, num_var_meta};
use crate::session::Session;
use crate::value::VarType;
use polars::prelude::*;

pub struct ContentsAst {
    pub data: Option<DatasetRef>,
    pub varnum: bool,
    /// data=lib._all_
    pub all: bool,
    /// OUT=<ds> : écrit un dataset (une ligne par variable) au lieu du listing
    /// normal de la table des variables (M33.7). J07-P6 : les options de
    /// dataset `out=ds(keep=...)` sont honorées (keep= sélectionne ET ordonne
    /// les colonnes du dataset de sortie, comme l'oracle CONTENTS OUT=).
    pub out: Option<DatasetRef>,
    /// keep= de OUT= (liste en ordre de déclaration = ordre des colonnes).
    pub out_keep: Option<Vec<String>>,
    /// drop= de OUT=.
    pub out_drop: Option<Vec<String>>,
    /// SHORT : n'imprime qu'une liste à plat des noms de variables (M33.7).
    pub short: bool,
    /// DETAILS : ajoute des infos d'observations/taille au bloc d'en-tête
    /// (M33.7).
    pub details: bool,
    /// NOPRINT (J01-P3) : supprime l'impression du listing (en-tête, table
    /// des variables) ; OUT= reste toujours écrit (doc SAS 9.4, chap. 14
    /// CONTENTS Procedure).
    pub noprint: bool,
}

/// Parse `proc contents [data=lib.x] [varnum] [out=ds] [short] [details]
///        [nodetails] [noprint] ; run ;`
/// Called AFTER "proc contents" has been consumed. Consumes through `run;`.
pub fn parse(ts: &mut StatementStream) -> Result<ContentsAst> {
    let mut data: Option<DatasetRef> = None;
    let mut varnum = false;
    let mut all = false;
    let mut out: Option<DatasetRef> = None;
    let mut out_keep: Option<Vec<String>> = None;
    let mut out_drop: Option<Vec<String>> = None;
    let mut short = false;
    let mut details = false;
    let mut noprint = false;

    // Parse PROC CONTENTS header options until `;` (combinateur partagé M31).
    common::parse_proc_options(ts, "CONTENTS", |ts, kw| {
        Ok(match kw {
            "data" => {
                let ds_ref = common::parse_dataset_opt(ts, "DATA")?;
                // Detect data=lib._all_ or data=_all_
                if ds_ref.name.to_uppercase() == "_ALL_" {
                    all = true;
                }
                data = Some(ds_ref);
                true
            }
            "varnum" => {
                ts.next();
                varnum = true;
                true
            }
            "out" => {
                common::consume_option_eq(ts, "OUT")?;
                let spec = ts.parse_dataset_spec()?;
                let o = &spec.options;
                if !o.rename.is_empty() {
                    return Err(SasError::parse(
                        "The RENAME= dataset option is not supported on CONTENTS OUT=.",
                        ts.peek().span,
                    ));
                }
                if o.where_.is_some() || o.in_.is_some() || o.updatemode.is_some() {
                    return Err(SasError::parse(
                        "Only KEEP= and DROP= are supported on CONTENTS OUT=.",
                        ts.peek().span,
                    ));
                }
                out_keep = o.keep.clone();
                out_drop = o.drop.clone();
                out = Some(spec.dref);
                true
            }
            "short" => {
                ts.next();
                short = true;
                true
            }
            "details" => {
                ts.next();
                details = true;
                true
            }
            "nodetails" => {
                ts.next();
                details = false;
                true
            }
            "noprint" => {
                ts.next();
                noprint = true;
                true
            }
            _ => false,
        })
    })?;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, "CONTENTS", |_ts, _kw| Ok(false))?;

    Ok(ContentsAst {
        data,
        varnum,
        all,
        out,
        out_keep,
        out_drop,
        short,
        details,
        noprint,
    })
}

/// Execute PROC CONTENTS. Called by `procs::execute_proc`.
pub fn execute(ast: &ContentsAst, session: &mut Session) -> Result<()> {
    if !ast.noprint {
        session.listing.page_header();
    }

    if ast.all {
        // data=lib._all_  — list all tables in the library
        let libref = match &ast.data {
            Some(r) => r.libref_or_work(),
            None => "WORK".to_string(),
        };
        let provider = session.libs.get(&libref)?;
        let mut tables = provider.list()?;
        tables.sort();

        if !ast.noprint {
            let headers = vec!["Member Name".to_string()];
            let aligns = vec![Align::Left];
            let rows: Vec<Vec<String>> =
                tables.into_iter().map(|t| vec![t.to_uppercase()]).collect();
            session.listing.write_table(&headers, &aligns, &rows);
        }
        return Ok(());
    }

    // Resolve the dataset reference (data= or _LAST_) and read it (MQ6.1).
    let (ds, display_name) = common::open_input_display(&ast.data, session)?;

    let n_obs = ds.n_obs();
    let n_vars = ds.vars.len();

    // ── Header block ─────────────────────────────────────────────────────────
    //
    // Two-column layout.  Each line: left label (fixed 25 chars wide),
    // left value.  Two items per line where a right-hand item exists.
    //
    //   Data Set Name: WORK.CLASS       Observations:  10
    //   Member Type:   DATA             Variables:       3
    //   Engine:        PARQUET
    //
    // Label column width = 16 chars (enough for "Data Set Name: ").
    // We use simple string formatting; no table renderer needed.
    //
    // NOPRINT (J01-P3) suppresses this whole listing (doc SAS 9.4, chap. 14
    // CONTENTS Procedure) ; OUT= below is always written regardless.
    if !ast.noprint {
        let left_label_width = 16usize;
        let left_value_width = 20usize; // pad left value to this width for alignment

        // Line 1: Data Set Name / Observations
        session.listing.write_line(&format!(
            "{:<lw$}{:<vw$}  {:<lw$}{}",
            "Data Set Name:",
            display_name,
            "Observations:",
            n_obs,
            lw = left_label_width,
            vw = left_value_width,
        ));
        // Line 2: Member Type / Variables
        session.listing.write_line(&format!(
            "{:<lw$}{:<vw$}  {:<lw$}{}",
            "Member Type:",
            "DATA",
            "Variables:",
            n_vars,
            lw = left_label_width,
            vw = left_value_width,
        ));
        // Line 3: Engine (no right-hand item)
        session.listing.write_line(&format!(
            "{:<lw$}{}",
            "Engine:",
            "PARQUET",
            lw = left_label_width,
        ));
        // DETAILS (M33.7) : extra observation/size info. We report the
        // observation count again as "# Observations" plus a derived "Obs in
        // Buffer" proxy (number of observations, the only size figure
        // available without reading the parquet page layout). Documented
        // simplification: SAS reports physical file size / page size, which
        // the parquet engine does not surface here.
        if ast.details {
            session.listing.write_line(&format!(
                "{:<lw$}{}",
                "# Observations:",
                n_obs,
                lw = left_label_width,
            ));
            session.listing.write_line(&format!(
                "{:<lw$}{}",
                "# Variables:",
                n_vars,
                lw = left_label_width,
            ));
        }
        session.listing.blank();
    }

    // ── OUT= dataset (M33.7) ──────────────────────────────────────────────────
    //
    // One row per variable. Column set (documented subset of SAS's CONTENTS
    // OUT= dataset, in creation order):
    //   NAME    (char)  variable name
    //   TYPE    (num)   SAS convention: 1 = numeric, 2 = character
    //   LENGTH  (num)   storage length in bytes
    //   VARNUM  (num)   1-based creation-order position
    //   LABEL   (char)  variable label ("" if none)
    //   FORMAT  (char)  format name ("" if none)
    // Rows are ordered by VARNUM (creation order), matching SAS's default
    // OUT= ordering (the VARNUM column makes any later re-sort lossless).
    if let Some(out_ref) = &ast.out {
        write_out_dataset(
            &ds,
            out_ref,
            ast.out_keep.as_deref(),
            ast.out_drop.as_deref(),
            session,
        )?;
    }

    // SHORT (M33.7) : just a space-separated list of variable names (in display
    // order: alphabetical by default, creation order under VARNUM). No header
    // table, no per-variable detail. Suppressed entirely by NOPRINT.
    if ast.short {
        if !ast.noprint {
            let mut idxs: Vec<usize> = (0..n_vars).collect();
            if !ast.varnum {
                idxs.sort_by(|&a, &b| {
                    ds.vars[a]
                        .name
                        .to_ascii_lowercase()
                        .cmp(&ds.vars[b].name.to_ascii_lowercase())
                });
            }
            let names: Vec<String> = idxs.iter().map(|&i| ds.vars[i].name.clone()).collect();
            session.listing.write_line(&names.join(" "));
        }
        return Ok(());
    }

    if ast.noprint {
        return Ok(());
    }

    // ── Variable table ────────────────────────────────────────────────────────
    //
    // Columns: #, Variable, Type, Len, Format, Label
    // `#` and `Len` are right-aligned; others left-aligned.
    //
    // Sort order:
    //   - default: alphabetical by variable name (case-insensitive)
    //   - varnum:  creation order (original position in ds.vars)
    //
    // In all cases `#` shows the CREATION-ORDER position (1-based).

    let headers: Vec<String> = vec![
        "#".to_string(),
        "Variable".to_string(),
        "Type".to_string(),
        "Len".to_string(),
        "Format".to_string(),
        "Informat".to_string(),
        "Label".to_string(),
    ];
    let aligns: Vec<Align> = vec![
        Align::Right, // #
        Align::Left,  // Variable
        Align::Left,  // Type
        Align::Right, // Len
        Align::Left,  // Format
        Align::Left,  // Informat (J07-P6)
        Align::Left,  // Label
    ];

    // Build index array, then sort it
    let mut indices: Vec<usize> = (0..n_vars).collect();
    if !ast.varnum {
        // Sort alphabetically by name, case-insensitive
        indices.sort_by(|&a, &b| {
            ds.vars[a]
                .name
                .to_ascii_lowercase()
                .cmp(&ds.vars[b].name.to_ascii_lowercase())
        });
    }
    // If varnum=true, leave in creation order (already 0..n_vars)

    let rows: Vec<Vec<String>> = indices
        .into_iter()
        .map(|i| {
            let v = &ds.vars[i];
            let type_str = match v.ty {
                VarType::Num => "Num",
                VarType::Char => "Char",
            };
            vec![
                (i + 1).to_string(), // creation-order #
                v.name.clone(),
                type_str.to_string(),
                v.length.to_string(),
                v.format.as_deref().unwrap_or("").to_string(),
                informat_display(v),
                v.label.as_deref().unwrap_or("").to_string(),
            ]
        })
        .collect();

    session.listing.write_table(&headers, &aligns, &rows);

    Ok(())
}

/// Build and write the CONTENTS OUT= dataset (one row per variable). See the
/// column documentation at the OUT= call site. Emits the standard
/// "The data set X has N observations and M variables." NOTE and updates
/// `_LAST_`.
fn write_out_dataset(
    ds: &SasDataset,
    out_ref: &DatasetRef,
    keep: Option<&[String]>,
    drop: Option<&[String]>,
    session: &mut Session,
) -> Result<()> {
    // J01-P7 — NAME de l'OUT= conserve la casse déclarée (exemple SAS 9.4
    // Procedures Guide, CONTENTS statement : `length aa 7 bb 6 ...; proc
    // contents data=a out=a1;` → PROC PRINT montre NAME = aa, bb, cc...).
    let names: Vec<Option<String>> = ds.vars.iter().map(|v| Some(v.name.clone())).collect();
    let types: Vec<Option<f64>> = ds
        .vars
        .iter()
        .map(|v| {
            Some(match v.ty {
                VarType::Num => 1.0,
                VarType::Char => 2.0,
            })
        })
        .collect();
    let lengths: Vec<Option<f64>> = ds.vars.iter().map(|v| Some(v.length as f64)).collect();
    let varnums: Vec<Option<f64>> = (0..ds.vars.len()).map(|i| Some((i + 1) as f64)).collect();
    let labels: Vec<Option<String>> = ds
        .vars
        .iter()
        .map(|v| Some(v.label.clone().unwrap_or_default()))
        .collect();
    let formats: Vec<Option<String>> = ds
        .vars
        .iter()
        .map(|v| Some(v.format.clone().unwrap_or_default()))
        .collect();

    // J07-P6 — INFORMAT (nom, "" si aucun), INFORML (largeur, 0 si aucun),
    // INFORMD (décimales, 0 si aucune) : cf. SAS 9.4, chap. 14, « OUT= Data
    // Set ». `date9.` → DATE/9/0 ; `$8.` → $/8/0 ; `comma12.2` → COMMA/12/2.
    let informats: Vec<Option<String>> =
        ds.vars.iter().map(|v| Some(informat_display(v))).collect();
    let informls: Vec<Option<f64>> = ds
        .vars
        .iter()
        .map(|v| Some(informat_width(v) as f64))
        .collect();
    let informds: Vec<Option<f64>> = ds
        .vars
        .iter()
        .map(|v| Some(informat_decimals(v) as f64))
        .collect();

    let columns: Vec<Column> = vec![
        Series::new("NAME".into(), names).into(),
        Series::new("TYPE".into(), types).into(),
        Series::new("LENGTH".into(), lengths).into(),
        Series::new("VARNUM".into(), varnums).into(),
        Series::new("LABEL".into(), labels).into(),
        Series::new("FORMAT".into(), formats).into(),
        Series::new("INFORMAT".into(), informats).into(),
        Series::new("INFORMD".into(), informds).into(),
        Series::new("INFORML".into(), informls).into(),
    ];
    let out_vars = vec![
        char_var_meta("NAME", 32),
        num_var_meta("TYPE"),
        num_var_meta("LENGTH"),
        num_var_meta("VARNUM"),
        char_var_meta("LABEL", 256),
        char_var_meta("FORMAT", 49),
        char_var_meta("INFORMAT", 32),
        num_var_meta("INFORMD"),
        num_var_meta("INFORML"),
    ];
    let df = DataFrame::new(columns)?;
    // keep= sélectionne ET ORDONNE les colonnes selon la liste (J07-P6) ;
    // drop= retire les colonnes listées. Appliqués conjointement sur le
    // dataset complet avant l'écriture (SAS : keep= first, then drop=).
    let mut df = df;
    let mut out_vars = out_vars;
    if let Some(keep) = keep {
        apply_keep(&mut df, &mut out_vars, keep);
    }
    if let Some(drop) = drop {
        apply_drop(&mut df, &mut out_vars, drop);
    }
    let out_ds = SasDataset { df, vars: out_vars };

    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = ds.vars.len();
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));
    Ok(())
}

/// Spécification de l'informat déclaré d'une variable (None si absent ou
/// invalide — traité comme « pas d'informat »).
fn informat_spec(v: &crate::dataset::VarMeta) -> Option<crate::formats::FormatSpec> {
    v.informat
        .as_deref()
        .and_then(crate::formats::FormatSpec::parse)
}

/// Nom de l'informat pour l'affichage / la colonne INFORMAT de OUT=
/// (« DATE », « $ », « COMMA » ; vide si aucun).
fn informat_display(v: &crate::dataset::VarMeta) -> String {
    informat_spec(v).map(|s| s.name.clone()).unwrap_or_default()
}

/// Largeur de l'informat (colonne INFORML de OUT=) ; 0 si aucun informat.
fn informat_width(v: &crate::dataset::VarMeta) -> u16 {
    informat_spec(v).and_then(|s| s.w).unwrap_or(0)
}

/// Décimales de l'informat (colonne INFORMD de OUT=) ; 0 si aucune.
fn informat_decimals(v: &crate::dataset::VarMeta) -> u16 {
    informat_spec(v).and_then(|s| s.d).unwrap_or(0)
}

/// keep= de OUT= : ne garde, DANS L'ORDRE DE LA LISTE, que les colonnes
/// nommées (une colonne absente du dataset est ignorée).
fn apply_keep(df: &mut DataFrame, out_vars: &mut Vec<crate::dataset::VarMeta>, keep: &[String]) {
    let mut columns = Vec::with_capacity(keep.len());
    let mut vars = Vec::with_capacity(keep.len());
    for name in keep {
        if let Some(idx) = out_vars
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case(name))
        {
            columns.push(df.get_columns()[idx].clone());
            vars.push(out_vars[idx].clone());
        }
    }
    if let Ok(new) = DataFrame::new(columns) {
        *df = new;
        *out_vars = vars;
    }
}

/// drop= de OUT= : retire les colonnes listées (l'ordre des autres est
/// préservé).
fn apply_drop(df: &mut DataFrame, out_vars: &mut Vec<crate::dataset::VarMeta>, drop: &[String]) {
    let mut columns = Vec::new();
    let mut vars = Vec::new();
    for (idx, v) in out_vars.iter().enumerate() {
        if !drop.iter().any(|d| v.name.eq_ignore_ascii_case(d)) {
            columns.push(df.get_columns()[idx].clone());
            vars.push(v.clone());
        }
    }
    if let Ok(new) = DataFrame::new(columns) {
        *df = new;
        *out_vars = vars;
    }
}

#[cfg(test)]
mod tests;
