//! PROC FREQ (jalon M5).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc freq data=a ; tables v1 v2 v1*v2 [/ missing nopercent norow
//! nocol nofreq out=b] ; run ;`
//!
//! - Table 1 voie : valeurs triées (ordre sas_cmp), colonnes Frequency,
//!   Percent, Cumulative Frequency, Cumulative Percent. Par défaut les
//!   missings sont EXCLUS du tableau et comptés sous une ligne
//!   "Frequency Missing = N" ; option MISSING les réintègre (et ils
//!   entrent alors dans les pourcentages).
//! - Table 2 voies (v1*v2) : crosstab avec Frequency / Percent / Row Pct
//!   / Col Pct par cellule + marges. Implémenter par group_by Polars sur
//!   les paires puis mise en forme manuelle (le rendu SAS en blocs de 4
//!   lignes par cellule).
//! - out= (1 voie) : colonnes <var>, COUNT, PERCENT.
//!
//! ## Choix de rendu (documenté pour l'orchestrateur)
//! - Titre centré "The FREQ Procedure" via `page_header()` puis une ligne
//!   centrée.
//! - Une voie : table à 5 colonnes (`<Var>`, Frequency, Percent,
//!   Cumulative Frequency, Cumulative Percent). Sans MISSING et avec des
//!   missings présents, une ligne "Frequency Missing = N" suit la table.
//! - Crosstab v1*v2 : une table dont la colonne de tête liste les valeurs
//!   de `v1` (plus une ligne "Total"), et qui porte une colonne par valeur
//!   de `v2` (plus "Total"). Chaque cellule (croisement) est rendue sur
//!   QUATRE lignes empilées dans la même colonne : Frequency, Percent
//!   (du total général), Row Pct (du total de la ligne), Col Pct (du total
//!   de la colonne). Les cellules de marge "Total" ne portent que
//!   Frequency et Percent (les deux dernières lignes restent vides). On
//!   construit ces lignes empilées à la main puis on les passe à
//!   `write_table` (une "ligne logique" = 4 lignes physiques).

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{chisq_sf, decode_column};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};
use polars::prelude::*;
use std::cmp::Ordering;

pub struct FreqAst {
    pub data: Option<DatasetRef>,
    pub tables: Vec<TableRequest>,
}

pub struct TableRequest {
    /// 1 nom = une voie ; 2 noms = crosstab v1*v2.
    pub vars: Vec<String>,
    pub missing: bool,
    pub out: Option<DatasetRef>,
    /// Display-suppression options (parsed AND honored).
    pub nofreq: bool,
    pub nopercent: bool,
    pub norow: bool,
    pub nocol: bool,
    pub nocum: bool,
    /// CHISQ statistics request (two-way only).
    pub chisq: bool,
}

fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("expected '=' after {opt}"),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

/// Parse `proc freq [data=a] ; [tables ...;]... run;`. Called AFTER
/// "proc freq" has been consumed. Consumes through `run;`/`quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<FreqAst> {
    let mut data: Option<DatasetRef> = None;

    // --- PROC FREQ statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            ts.next();
            expect_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC FREQ statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC FREQ statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut tables: Vec<TableRequest> = Vec::new();

    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }

        if ts.peek().is_kw("tables") || ts.peek().is_kw("table") {
            ts.next();
            let reqs = parse_tables(ts)?;
            tables.extend(reqs);
        } else {
            // Unknown sub-statement: skip it (recovery, like means/sort).
            ts.skip_to_semi();
        }
    }

    Ok(FreqAst { data, tables })
}

/// Parse one TABLES statement body (after "tables" consumed), through its
/// terminating `;`. Returns one TableRequest per spec.
fn parse_tables(ts: &mut StatementStream) -> Result<Vec<TableRequest>> {
    let mut specs: Vec<Vec<String>> = Vec::new();

    // Specs until `/` (options) or `;`.
    loop {
        match &ts.peek().kind {
            TokenKind::Semi | TokenKind::Slash | TokenKind::Eof => break,
            _ => {}
        }
        // One spec: v or v1*v2.
        let first_tok = ts.peek().clone();
        let Some(first) = first_tok.ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected a variable name in the TABLES statement",
                first_tok.span,
            ));
        };
        ts.next();
        let mut vars = vec![first];
        if ts.peek().kind == TokenKind::Star {
            ts.next();
            let snd_tok = ts.peek().clone();
            let Some(snd) = snd_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a variable name after '*' in the TABLES statement",
                    snd_tok.span,
                ));
            };
            ts.next();
            vars.push(snd);
        }
        specs.push(vars);
    }

    // Options after `/`.
    let mut missing = false;
    let mut out: Option<DatasetRef> = None;
    let mut nofreq = false;
    let mut nopercent = false;
    let mut norow = false;
    let mut nocol = false;
    let mut nocum = false;
    let mut chisq = false;
    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        loop {
            match &ts.peek().kind {
                TokenKind::Semi | TokenKind::Eof => break,
                _ => {}
            }
            if ts.peek().is_kw("missing") {
                ts.next();
                missing = true;
            } else if ts.peek().is_kw("out") {
                ts.next();
                expect_eq(ts, "OUT")?;
                out = Some(ts.parse_dataset_ref()?);
            } else if ts.peek().is_kw("nopercent") {
                ts.next();
                nopercent = true;
            } else if ts.peek().is_kw("norow") {
                ts.next();
                norow = true;
            } else if ts.peek().is_kw("nocol") {
                ts.next();
                nocol = true;
            } else if ts.peek().is_kw("nofreq") {
                ts.next();
                nofreq = true;
            } else if ts.peek().is_kw("nocum") {
                ts.next();
                nocum = true;
            } else if ts.peek().is_kw("chisq") {
                ts.next();
                chisq = true;
            } else if let Some(name) = ts.peek().ident().map(str::to_string) {
                // Unknown option: ignore leniently (skip the token, and any
                // `=value` that follows).
                ts.next();
                if ts.peek().kind == TokenKind::Eq {
                    ts.next();
                    // skip a single value token (ident/num)
                    if !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
                        ts.next();
                    }
                }
            } else {
                // Unexpected token among options: stop (let expect_semi catch
                // the terminator).
                break;
            }
        }
    }

    ts.expect_semi()?;

    // OUT= requires exactly one table spec on the TABLES statement (SAS rule).
    if out.is_some() && specs.len() != 1 {
        return Err(SasError::runtime(
            "The OUT= option in PROC FREQ requires a single table request on the TABLES statement.",
        ));
    }

    let n = specs.len();
    Ok(specs
        .into_iter()
        .enumerate()
        .map(|(i, vars)| TableRequest {
            vars,
            missing,
            // OUT= only applies (and is only valid) for a single spec.
            out: if i == 0 && n == 1 { out.clone() } else { None },
            nofreq,
            nopercent,
            norow,
            nocol,
            nocum,
            chisq,
        })
        .collect())
}

/// Resolve `data=` or `_LAST_` into a concrete DatasetRef.
fn resolve_input(ast: &FreqAst, session: &Session) -> Result<DatasetRef> {
    match &ast.data {
        Some(r) => Ok(r.clone()),
        None => {
            let last = session.last_dataset.clone().ok_or_else(|| {
                SasError::runtime("There is no default input data set (_LAST_ is undefined).")
            })?;
            let parts: Vec<&str> = last.splitn(2, '.').collect();
            if parts.len() == 2 {
                Ok(DatasetRef {
                    libref: Some(parts[0].to_string()),
                    name: parts[1].to_string(),
                })
            } else {
                Ok(DatasetRef {
                    libref: None,
                    name: last,
                })
            }
        }
    }
}

/// Find a variable column index by name (case-insensitive), or error.
fn find_var(ds: &SasDataset, name: &str) -> Result<usize> {
    ds.vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", name.to_uppercase())))
}

/// Render a category value for the listing (numeric via format_best, char as
/// the string, missing via its MissingKind display).
fn category_label(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Char(s) => s.trim_end().to_string(),
        Value::Missing(k) => k.display(),
    }
}

/// Format a percentage to two decimals.
fn fmt_pct(p: f64) -> String {
    format!("{p:.2}")
}

/// A distinct category with its observed frequency, in sas_cmp order.
struct Category {
    value: Value,
    freq: usize,
}

/// Tally the distinct values of `col` into categories ordered by sas_cmp.
/// When `include_missing` is false, missing values are excluded (their count
/// is returned separately as `n_missing`).
fn tally(col: &[Value], include_missing: bool) -> (Vec<Category>, usize) {
    let mut cats: Vec<Category> = Vec::new();
    let mut n_missing = 0usize;
    for v in col {
        if v.is_missing() {
            n_missing += 1;
            if !include_missing {
                continue;
            }
        }
        match cats
            .iter_mut()
            .find(|c| c.value.sas_cmp(v) == Ordering::Equal)
        {
            Some(c) => c.freq += 1,
            None => cats.push(Category {
                value: v.clone(),
                freq: 1,
            }),
        }
    }
    cats.sort_by(|a, b| a.value.sas_cmp(&b.value));
    (cats, n_missing)
}

/// Execute PROC FREQ. Called by `procs::execute_proc`.
pub fn execute(ast: &FreqAst, session: &mut Session) -> Result<()> {
    let in_ref = resolve_input(ast, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_obs = ds.n_obs();

    session.listing.page_header();
    // Centered procedure title line.
    let title = "The FREQ Procedure";
    let ls = session.listing.ls;
    let pad = ls.saturating_sub(title.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), title));
    session.listing.blank();

    for req in &ast.tables {
        match req.vars.len() {
            1 => one_way(session, &ds, req, n_obs)?,
            2 => two_way(session, &ds, req, n_obs)?,
            _ => {
                return Err(SasError::runtime(
                    "A TABLES request must name one or two variables.",
                ));
            }
        }
    }

    Ok(())
}

/// One-way frequency table for a single variable.
fn one_way(
    session: &mut Session,
    ds: &SasDataset,
    req: &TableRequest,
    n_obs: usize,
) -> Result<()> {
    let col_idx = find_var(ds, &req.vars[0])?;
    let col = decode_column(ds, col_idx)?;
    let var_name = ds.vars[col_idx].name.clone();

    let (cats, n_missing) = tally(&col, req.missing);

    // Denominator: the sum of the category frequencies already gives the
    // right value — with MISSING the missing categories are included in
    // `cats`, otherwise they are excluded (so denom = non-missing count).
    let denom: usize = cats.iter().map(|c| c.freq).sum();

    // Listing table. Display options suppress whole columns:
    //   NOFREQ    -> drop Frequency
    //   NOPERCENT -> drop Percent (and Cumulative Percent)
    //   NOCUM     -> drop Cumulative Frequency and Cumulative Percent
    // The default (no options) keeps all five columns exactly as before.
    let show_freq = !req.nofreq;
    let show_pct = !req.nopercent;
    let show_cum_freq = !req.nocum;
    let show_cum_pct = !req.nocum && !req.nopercent;

    let mut headers = vec![var_name.clone()];
    let mut aligns = vec![if ds.vars[col_idx].ty == VarType::Num {
        Align::Right
    } else {
        Align::Left
    }];
    if show_freq {
        headers.push("Frequency".to_string());
        aligns.push(Align::Right);
    }
    if show_pct {
        headers.push("Percent".to_string());
        aligns.push(Align::Right);
    }
    if show_cum_freq {
        headers.push("Cumulative Frequency".to_string());
        aligns.push(Align::Right);
    }
    if show_cum_pct {
        headers.push("Cumulative Percent".to_string());
        aligns.push(Align::Right);
    }

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(cats.len());
    let mut cum_freq = 0usize;
    for c in &cats {
        cum_freq += c.freq;
        let pct = if denom > 0 {
            100.0 * c.freq as f64 / denom as f64
        } else {
            0.0
        };
        let cum_pct = if denom > 0 {
            100.0 * cum_freq as f64 / denom as f64
        } else {
            0.0
        };
        let mut row = vec![category_label(&c.value)];
        if show_freq {
            row.push(format!("{}", c.freq));
        }
        if show_pct {
            row.push(fmt_pct(pct));
        }
        if show_cum_freq {
            row.push(format!("{cum_freq}"));
        }
        if show_cum_pct {
            row.push(fmt_pct(cum_pct));
        }
        rows.push(row);
    }

    session.listing.write_table(&headers, &aligns, &rows);

    // Frequency Missing line (only when missings are excluded).
    if !req.missing && n_missing > 0 {
        session.listing.blank();
        session
            .listing
            .write_line(&format!("Frequency Missing = {n_missing}"));
    }

    // OUT= dataset (one-way only).
    if let Some(out) = &req.out {
        write_one_way_out(session, ds, col_idx, &cats, denom, out)?;
    }

    Ok(())
}

/// Build and write the OUT= dataset for a one-way table: columns <var>,
/// COUNT, PERCENT.
fn write_one_way_out(
    session: &mut Session,
    ds: &SasDataset,
    col_idx: usize,
    cats: &[Category],
    denom: usize,
    out: &DatasetRef,
) -> Result<()> {
    let meta = &ds.vars[col_idx];
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // Category column (same type/meta as the input variable).
    let cat_series = match meta.ty {
        VarType::Num => {
            let vals: Vec<Option<f64>> =
                cats.iter().map(|c| value_to_num(&c.value)).collect();
            Series::new(meta.name.as_str().into(), vals)
        }
        VarType::Char => {
            let vals: Vec<Option<String>> = cats
                .iter()
                .map(|c| match &c.value {
                    Value::Char(s) if s.trim_end().is_empty() => None,
                    Value::Char(s) => Some(s.trim_end().to_string()),
                    _ => None,
                })
                .collect();
            Series::new(meta.name.as_str().into(), vals)
        }
    };
    columns.push(cat_series.into());
    vars.push(meta.clone());

    // COUNT.
    let count_vals: Vec<Option<f64>> = cats.iter().map(|c| Some(c.freq as f64)).collect();
    columns.push(Series::new("COUNT".into(), count_vals).into());
    vars.push(num_var_meta("COUNT"));

    // PERCENT.
    let pct_vals: Vec<Option<f64>> = cats
        .iter()
        .map(|c| {
            Some(if denom > 0 {
                100.0 * c.freq as f64 / denom as f64
            } else {
                0.0
            })
        })
        .collect();
    columns.push(Series::new("PERCENT".into(), pct_vals).into());
    vars.push(num_var_meta("PERCENT"));

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = out.libref_or_work();
    let out_table = out.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());

    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));

    Ok(())
}

/// Two-way crosstab for v1*v2.
fn two_way(
    session: &mut Session,
    ds: &SasDataset,
    req: &TableRequest,
    n_obs: usize,
) -> Result<()> {
    let row_idx = find_var(ds, &req.vars[0])?;
    let col_idx = find_var(ds, &req.vars[1])?;
    let row_col = decode_column(ds, row_idx)?;
    let col_col = decode_column(ds, col_idx)?;
    let row_name = ds.vars[row_idx].name.clone();
    let col_name = ds.vars[col_idx].name.clone();

    // Distinct row and column values (ordered by sas_cmp), filtering missings
    // unless MISSING is set. A cell counts an obs only when BOTH its row and
    // column values are kept.
    let keep = |v: &Value| req.missing || !v.is_missing();

    let mut row_vals: Vec<Value> = Vec::new();
    let mut col_vals: Vec<Value> = Vec::new();
    for v in &row_col {
        if keep(v) && !row_vals.iter().any(|x| x.sas_cmp(v) == Ordering::Equal) {
            row_vals.push(v.clone());
        }
    }
    for v in &col_col {
        if keep(v) && !col_vals.iter().any(|x| x.sas_cmp(v) == Ordering::Equal) {
            col_vals.push(v.clone());
        }
    }
    row_vals.sort_by(|a, b| a.sas_cmp(b));
    col_vals.sort_by(|a, b| a.sas_cmp(b));

    let nr = row_vals.len();
    let nc = col_vals.len();

    // freq[r][c]
    let mut freq = vec![vec![0usize; nc]; nr];
    for i in 0..n_obs {
        let rv = &row_col[i];
        let cv = &col_col[i];
        if !keep(rv) || !keep(cv) {
            continue;
        }
        let r = row_vals.iter().position(|x| x.sas_cmp(rv) == Ordering::Equal);
        let c = col_vals.iter().position(|x| x.sas_cmp(cv) == Ordering::Equal);
        if let (Some(r), Some(c)) = (r, c) {
            freq[r][c] += 1;
        }
    }

    let row_tot: Vec<usize> = (0..nr).map(|r| freq[r].iter().sum()).collect();
    let col_tot: Vec<usize> = (0..nc)
        .map(|c| (0..nr).map(|r| freq[r][c]).sum())
        .collect();
    let grand: usize = row_tot.iter().sum();

    // Which stacked per-cell lines to show. Display options drop a line:
    //   NOFREQ    -> Frequency, NOPERCENT -> Percent,
    //   NOROW     -> Row Pct,   NOCOL     -> Col Pct.
    // Default (no options) keeps all four, exactly as before.
    let show_freq = !req.nofreq;
    let show_pct = !req.nopercent;
    let show_rowp = !req.norow;
    let show_colp = !req.nocol;

    // Legend reflecting the lines actually shown.
    let mut legend_parts: Vec<&str> = Vec::new();
    if show_freq {
        legend_parts.push("Frequency");
    }
    if show_pct {
        legend_parts.push("Percent");
    }
    if show_rowp {
        legend_parts.push("Row Pct");
    }
    if show_colp {
        legend_parts.push("Col Pct");
    }

    session
        .listing
        .write_line(&format!("Table of {row_name} by {col_name}"));
    session.listing.blank();
    if !legend_parts.is_empty() {
        session
            .listing
            .write_line(&format!("Cell contents: {}", legend_parts.join(" / ")));
        session.listing.blank();
    }

    // Header: row-var name, one column per col value, then Total.
    let mut headers = vec![row_name.clone()];
    for cv in &col_vals {
        headers.push(category_label(cv));
    }
    headers.push("Total".to_string());
    let mut aligns = vec![Align::Left];
    for _ in 0..nc {
        aligns.push(Align::Right);
    }
    aligns.push(Align::Right);

    // Each logical row -> 4 physical rows (Frequency, Percent, Row Pct,
    // Col Pct). The first physical row carries the row-value label.
    let mut rows: Vec<Vec<String>> = Vec::new();
    let pct_of = |num: usize, den: usize| -> String {
        if den > 0 {
            fmt_pct(100.0 * num as f64 / den as f64)
        } else {
            fmt_pct(0.0)
        }
    };

    // The row-value label rides on the first physical line that is shown.
    let label_on_freq = show_freq;
    let label_on_pct = !show_freq && show_pct;
    let label_on_rowp = !show_freq && !show_pct && show_rowp;
    let label_on_colp = !show_freq && !show_pct && !show_rowp && show_colp;

    for r in 0..nr {
        let mut line_freq = vec![if label_on_freq {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        let mut line_pct = vec![if label_on_pct {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        let mut line_rowp = vec![if label_on_rowp {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        let mut line_colp = vec![if label_on_colp {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        for c in 0..nc {
            let f = freq[r][c];
            line_freq.push(format!("{f}"));
            line_pct.push(pct_of(f, grand));
            line_rowp.push(pct_of(f, row_tot[r]));
            line_colp.push(pct_of(f, col_tot[c]));
        }
        // Row total margin: Frequency + Percent only.
        line_freq.push(format!("{}", row_tot[r]));
        line_pct.push(pct_of(row_tot[r], grand));
        line_rowp.push(String::new());
        line_colp.push(String::new());
        if show_freq {
            rows.push(line_freq);
        }
        if show_pct {
            rows.push(line_pct);
        }
        if show_rowp {
            rows.push(line_rowp);
        }
        if show_colp {
            rows.push(line_colp);
        }
    }

    // Total row (column totals + grand total): Frequency + Percent only.
    let mut tot_freq = vec!["Total".to_string()];
    let mut tot_pct = vec![String::new()];
    for c in 0..nc {
        tot_freq.push(format!("{}", col_tot[c]));
        tot_pct.push(pct_of(col_tot[c], grand));
    }
    tot_freq.push(format!("{grand}"));
    tot_pct.push(pct_of(grand, grand));
    if show_freq {
        rows.push(tot_freq);
    }
    if show_pct {
        // When the Frequency line is suppressed the "Total" label needs to
        // land on the percent line so the margin row stays identifiable.
        if !show_freq {
            tot_pct[0] = "Total".to_string();
        }
        rows.push(tot_pct);
    }

    session.listing.write_table(&headers, &aligns, &rows);

    // CHISQ statistics (two-way only), printed after the crosstab.
    if req.chisq {
        chisq_block(session, &row_name, &col_name, &freq, &row_tot, &col_tot, grand);
    }

    Ok(())
}

/// Format a p-value SAS-style: `<.0001`, else 4 decimals (mirrors corr.rs).
fn fmt_chisq_p(p: f64) -> String {
    if p < 0.0001 {
        "<.0001".to_string()
    } else {
        format!("{p:.4}")
    }
}

/// Print the "Statistics for Table of <row> by <col>" CHISQ block for a
/// two-way table: Pearson chi-square and the likelihood-ratio chi-square,
/// each with DF and an upper-tail p-value. Degenerate tables (grand total 0,
/// any zero margin, or DF <= 0) are skipped gracefully with a note.
fn chisq_block(
    session: &mut Session,
    row_name: &str,
    col_name: &str,
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
) {
    session.listing.blank();
    session
        .listing
        .write_line(&format!("Statistics for Table of {row_name} by {col_name}"));
    session.listing.blank();

    let nr = row_tot.len();
    let nc = col_tot.len();
    let df = (nr.saturating_sub(1)) * (nc.saturating_sub(1));

    // Guard against degenerate tables: no expected counts are defined.
    if grand == 0
        || df == 0
        || row_tot.contains(&0)
        || col_tot.contains(&0)
    {
        session
            .listing
            .write_line("Chi-Square statistics are not computable for this table.");
        return;
    }

    let g = grand as f64;
    let mut pearson = 0.0_f64;
    let mut lratio = 0.0_f64;
    for r in 0..nr {
        for c in 0..nc {
            let e = (row_tot[r] as f64) * (col_tot[c] as f64) / g;
            let n = freq[r][c] as f64;
            if e > 0.0 {
                let d = n - e;
                pearson += d * d / e;
            }
            if n > 0.0 && e > 0.0 {
                lratio += n * (n / e).ln();
            }
        }
    }
    lratio *= 2.0;

    let df_f = df as f64;
    let p_pearson = chisq_sf(pearson, df_f);
    let p_lratio = chisq_sf(lratio, df_f);

    let headers = vec![
        "Statistic".to_string(),
        "DF".to_string(),
        "Value".to_string(),
        "Prob".to_string(),
    ];
    let aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
    let rows = vec![
        vec![
            "Chi-Square".to_string(),
            format!("{df}"),
            format!("{pearson:.4}"),
            fmt_chisq_p(p_pearson),
        ],
        vec![
            "Likelihood Ratio Chi-Square".to_string(),
            format!("{df}"),
            format!("{lratio:.4}"),
            fmt_chisq_p(p_lratio),
        ],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

fn num_var_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::df;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_freq(src: &str) -> Result<FreqAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "freq"
        parse(&mut ts)
    }

    fn num_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }
    }

    fn char_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Char,
            length: 4,
            format: None,
            label: None,
        }
    }

    /// Build a TableRequest with all display/chisq options off (defaults).
    fn tr(vars: &[&str], missing: bool, out: Option<DatasetRef>) -> TableRequest {
        TableRequest {
            vars: vars.iter().map(|s| s.to_string()).collect(),
            missing,
            out,
            nofreq: false,
            nopercent: false,
            norow: false,
            nocol: false,
            nocum: false,
            chisq: false,
        }
    }

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    fn read_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
        let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
        let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
        decode_column(&ds, idx).unwrap()
    }

    // ───────────────────────────── parse tests ─────────────────────────────

    #[test]
    fn parse_one_way() {
        let ast = parse_freq("proc freq data=a; tables x; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert_eq!(ast.tables.len(), 1);
        assert_eq!(ast.tables[0].vars, vec!["x"]);
        assert!(!ast.tables[0].missing);
        assert!(ast.tables[0].out.is_none());
    }

    #[test]
    fn parse_multiple_specs_and_crosstab() {
        let ast = parse_freq("proc freq data=a; tables a b a*b; run;").unwrap();
        assert_eq!(ast.tables.len(), 3);
        assert_eq!(ast.tables[0].vars, vec!["a"]);
        assert_eq!(ast.tables[1].vars, vec!["b"]);
        assert_eq!(ast.tables[2].vars, vec!["a", "b"]);
    }

    #[test]
    fn parse_missing_and_out() {
        let ast = parse_freq("proc freq data=a; tables a / missing out=work.o; run;").unwrap();
        assert_eq!(ast.tables.len(), 1);
        assert!(ast.tables[0].missing);
        let out = ast.tables[0].out.as_ref().unwrap();
        assert_eq!(out.libref.as_deref(), Some("work"));
        assert_eq!(out.name, "o");
    }

    #[test]
    fn parse_out_requires_single_spec() {
        let r = parse_freq("proc freq data=a; tables a b / out=work.o; run;");
        assert!(r.is_err());
        let msg = r.err().unwrap().to_string();
        assert!(msg.contains("OUT="), "msg: {msg}");
    }

    #[test]
    fn parse_ignores_display_options() {
        let ast =
            parse_freq("proc freq data=a; tables x / nopercent norow nocol nofreq nocum; run;")
                .unwrap();
        assert_eq!(ast.tables.len(), 1);
        assert_eq!(ast.tables[0].vars, vec!["x"]);
    }

    #[test]
    fn parse_accepts_table_spelling() {
        let ast = parse_freq("proc freq data=a; table x; run;").unwrap();
        assert_eq!(ast.tables.len(), 1);
        assert_eq!(ast.tables[0].vars, vec!["x"]);
    }

    #[test]
    fn parse_multiple_tables_statements_accumulate() {
        let ast = parse_freq("proc freq data=a; tables x; tables y*z; run;").unwrap();
        assert_eq!(ast.tables.len(), 2);
        assert_eq!(ast.tables[0].vars, vec!["x"]);
        assert_eq!(ast.tables[1].vars, vec!["y", "z"]);
    }

    // ───────────────────────────── tally tests ─────────────────────────────

    #[test]
    fn tally_excludes_missing_by_default() {
        let col = vec![
            Value::Num(2.0),
            Value::Num(1.0),
            Value::Num(2.0),
            Value::missing(),
        ];
        let (cats, nm) = tally(&col, false);
        assert_eq!(nm, 1);
        // sas_cmp order: 1 then 2.
        assert_eq!(cats.len(), 2);
        assert_eq!(cats[0].value, Value::Num(1.0));
        assert_eq!(cats[0].freq, 1);
        assert_eq!(cats[1].value, Value::Num(2.0));
        assert_eq!(cats[1].freq, 2);
    }

    #[test]
    fn tally_includes_missing_when_requested() {
        let col = vec![Value::Num(2.0), Value::missing(), Value::Num(2.0)];
        let (cats, nm) = tally(&col, true);
        assert_eq!(nm, 1);
        // Missing sorts before numbers.
        assert_eq!(cats.len(), 2);
        assert!(cats[0].value.is_missing());
        assert_eq!(cats[0].freq, 1);
        assert_eq!(cats[1].value, Value::Num(2.0));
        assert_eq!(cats[1].freq, 2);
    }

    // ───────────────────────────── execute tests ───────────────────────────

    #[test]
    fn execute_one_way_default_excludes_missing() {
        let mut session = make_session();
        // x = 1,1,2,. -> non-missing denom = 3.
        let df = df!["x" => [Some(1.0_f64), Some(1.0), Some(2.0), None]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![tr(&["x"], false, None)],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("The FREQ Procedure"), "{listing}");
        assert!(listing.contains("Frequency Missing = 1"), "{listing}");
        // 1: freq 2, percent 2/3 = 66.67; 2: freq 1, 33.33; cumulative 100.00.
        assert!(listing.contains("66.67"), "{listing}");
        assert!(listing.contains("33.33"), "{listing}");
        assert!(listing.contains("100.00"), "{listing}");
        // cumulative frequency 3 present.
        assert!(listing.contains('3'), "{listing}");
    }

    #[test]
    fn execute_one_way_missing_option_includes_it() {
        let mut session = make_session();
        let df = df!["x" => [Some(1.0_f64), Some(1.0), Some(2.0), None]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![tr(&["x"], true, None)],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // With MISSING the denom is 4: 1 -> 2/4 = 50.00; missing -> 1/4 = 25.00.
        assert!(listing.contains("50.00"), "{listing}");
        assert!(listing.contains("25.00"), "{listing}");
        // No "Frequency Missing" footnote when MISSING is set.
        assert!(!listing.contains("Frequency Missing"), "{listing}");
    }

    #[test]
    fn execute_out_dataset() {
        let mut session = make_session();
        // x = a,a,b -> a:2 (66.67), b:1 (33.33).
        let df = df!["x" => ["a", "a", "b"]].unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![tr(
                &["x"],
                false,
                Some(DatasetRef { libref: Some("WORK".into()), name: "O".into() }),
            )],
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 2);
        let cat = read_col(&session, "O", "x");
        let count = read_col(&session, "O", "COUNT");
        let pct = read_col(&session, "O", "PERCENT");
        // sas_cmp order: a then b.
        assert_eq!(cat, vec![Value::Char("a".into()), Value::Char("b".into())]);
        assert_eq!(count, vec![Value::Num(2.0), Value::Num(1.0)]);
        if let (Value::Num(pa), Value::Num(pb)) = (&pct[0], &pct[1]) {
            assert!((pa - 200.0 / 3.0).abs() < 1e-9, "pa={pa}");
            assert!((pb - 100.0 / 3.0).abs() < 1e-9, "pb={pb}");
        } else {
            panic!("percent must be numeric: {pct:?}");
        }

        let log = session.log.into_string();
        assert!(
            log.contains("The data set WORK.O has 2 observations and 3 variables."),
            "log: {log}"
        );
    }

    #[test]
    fn execute_crosstab_counts_and_total() {
        let mut session = make_session();
        // 2x2: (a,1),(a,2),(b,1),(b,1)
        // rows a: 1->1, 2->1 ; b: 1->2, 2->0. grand=4.
        let df = df![
            "r" => ["a", "a", "b", "b"],
            "c" => [1.0_f64, 2.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("r"), num_meta("c")] };
        write_dataset(&mut session, "T", ds);

        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![tr(&["r", "c"], false, None)],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Table of r by c"), "{listing}");
        // Grand total 4 and column total for c=1 is 3.
        assert!(listing.contains("Total"), "{listing}");
        // The 4 stacked-cell legend.
        assert!(listing.contains("Row Pct"), "{listing}");
        // Grand-total percent 100.00 must appear.
        assert!(listing.contains("100.00"), "{listing}");
    }

    // ───────────────────────── chisq_sf tests ─────────────────────────

    #[test]
    fn chisq_sf_known_values() {
        // 95th percentile of chi-square(1) is 3.841 -> upper tail ~ 0.05.
        assert!((chisq_sf(3.841, 1.0) - 0.05).abs() < 1e-3, "{}", chisq_sf(3.841, 1.0));
        // At 0 the survival function is 1.
        assert!((chisq_sf(0.0, 1.0) - 1.0).abs() < 1e-12);
        // Far in the tail -> ~0.
        assert!(chisq_sf(100.0, 1.0) < 1e-3);
    }

    // ───────────────────────── display-option tests ─────────────────────────

    fn one_way_listing(opts: impl Fn(&mut TableRequest)) -> String {
        let mut session = make_session();
        let df = df!["x" => [Some(1.0_f64), Some(1.0), Some(2.0)]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);
        let mut req = tr(&["x"], false, None);
        opts(&mut req);
        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![req],
        };
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    }

    #[test]
    fn one_way_nofreq_drops_frequency_column() {
        let l = one_way_listing(|r| r.nofreq = true);
        // The standalone "Frequency" header is gone, but "Cumulative
        // Frequency" (NOCUM not set) remains.
        let default = one_way_listing(|_| {});
        let default_freq = default.matches("Frequency").count();
        assert_eq!(l.matches("Frequency").count(), default_freq - 1, "{l}");
        assert!(l.contains("Percent"), "{l}");
        assert!(l.contains("Cumulative Frequency"), "{l}");
    }

    #[test]
    fn one_way_nopercent_drops_percent_columns() {
        let l = one_way_listing(|r| r.nopercent = true);
        assert!(!l.contains("Percent"), "{l}");
        assert!(l.contains("Frequency"), "{l}");
        assert!(l.contains("Cumulative Frequency"), "{l}");
    }

    #[test]
    fn one_way_nocum_drops_cumulative_columns() {
        let l = one_way_listing(|r| r.nocum = true);
        assert!(!l.contains("Cumulative"), "{l}");
        assert!(l.contains("Frequency"), "{l}");
        assert!(l.contains("Percent"), "{l}");
    }

    fn crosstab_listing(opts: impl Fn(&mut TableRequest)) -> String {
        let mut session = make_session();
        let df = df![
            "r" => ["a", "a", "b", "b"],
            "c" => [1.0_f64, 2.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("r"), num_meta("c")] };
        write_dataset(&mut session, "T", ds);
        let mut req = tr(&["r", "c"], false, None);
        opts(&mut req);
        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![req],
        };
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    }

    #[test]
    fn crosstab_norow_drops_row_pct() {
        let l = crosstab_listing(|r| r.norow = true);
        assert!(!l.contains("Row Pct"), "{l}");
        assert!(l.contains("Col Pct"), "{l}");
    }

    #[test]
    fn crosstab_nocol_drops_col_pct() {
        let l = crosstab_listing(|r| r.nocol = true);
        assert!(!l.contains("Col Pct"), "{l}");
        assert!(l.contains("Row Pct"), "{l}");
    }

    #[test]
    fn crosstab_nofreq_keeps_others() {
        // NOFREQ drops the per-cell Frequency line; Percent/Row/Col remain.
        let l = crosstab_listing(|r| r.nofreq = true);
        assert!(l.contains("Percent"), "{l}");
        assert!(l.contains("Row Pct"), "{l}");
        assert!(l.contains("Col Pct"), "{l}");
        // The label row must still identify the row categories and Total.
        assert!(l.contains("Total"), "{l}");
    }

    // ───────────────────────── chisq block test ─────────────────────────

    #[test]
    fn crosstab_chisq_2x2_hand_computed() {
        // 2x2 table:
        //          c=1  c=2  | tot
        //   r=a :   10    20 |  30
        //   r=b :   30    40 |  70
        //   col :   40    60 | 100
        // Expected: e_a1=30*40/100=12, e_a2=18, e_b1=28, e_b2=42.
        // Pearson = (10-12)^2/12 + (20-18)^2/18 + (30-28)^2/28 + (40-42)^2/42
        //         = 4/12 + 4/18 + 4/28 + 4/42
        //         = 0.333333 + 0.222222 + 0.142857 + 0.095238 = 0.793651
        // DF = 1; p = chisq_sf(0.793651, 1) ~ 0.3730.
        let mut session = make_session();
        // Build column vectors that reproduce the table counts.
        let mut r: Vec<&str> = Vec::new();
        let mut c: Vec<f64> = Vec::new();
        for _ in 0..10 { r.push("a"); c.push(1.0); }
        for _ in 0..20 { r.push("a"); c.push(2.0); }
        for _ in 0..30 { r.push("b"); c.push(1.0); }
        for _ in 0..40 { r.push("b"); c.push(2.0); }
        let df = df!["r" => r, "c" => c].unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("r"), num_meta("c")] };
        write_dataset(&mut session, "T", ds);

        let mut req = tr(&["r", "c"], false, None);
        req.chisq = true;
        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![req],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Statistics for Table of r by c"), "{listing}");
        assert!(listing.contains("Chi-Square"), "{listing}");
        assert!(listing.contains("Likelihood Ratio Chi-Square"), "{listing}");
        // Pearson value formatted to 4 decimals.
        assert!(listing.contains("0.7937"), "{listing}");

        // Cross-check the numeric pieces directly.
        let pearson: f64 = 4.0 / 12.0 + 4.0 / 18.0 + 4.0 / 28.0 + 4.0 / 42.0;
        assert!((pearson - 0.793651).abs() < 1e-4, "{pearson}");
        let p = chisq_sf(pearson, 1.0);
        assert!((p - 0.3730).abs() < 1e-3, "p={p}");
    }
}
