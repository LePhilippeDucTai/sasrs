//! PROC MEANS / SUMMARY (jalon M5).
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : ÉLEVÉE — _TYPE_)
//!
//! `proc means data=a [noprint] [stats...] ; class c1 c2 ; var v1 v2 ;
//! output out=b [stat(var)=name...] ; run ;`  — SUMMARY = MEANS noprint
//! par défaut.
//!
//! ## Sémantique à répliquer
//! - Stats défaut du rapport : N, Mean, Std Dev, Minimum, Maximum.
//!   Stats demandables : n nmiss mean std min max sum range stderr cv
//!   median.  SAS EXCLUT les missings : chaque stat est calculée sur les
//!   valeurs numériques NON missing du groupe (helper `compute`).
//! - CLASS sans OUTPUT : rapport par combinaison de classes.
//! - OUTPUT OUT= avec CLASS : produit TOUTES les combinaisons de
//!   sous-ensembles de classes — `_TYPE_` = masque binaire (bit le plus
//!   à droite = dernière variable CLASS), `_FREQ_` = effectif. Ordre :
//!   _TYPE_ croissant puis valeurs de classes. Lignes des classes non
//!   actives → missing.
//! - VAR absent : toutes les numériques hors CLASS/BY.
//! - Rapport listing : table par variable x stat, en-tête style SAS
//!   ("The MEANS Procedure").
//!
//! ## Choix de rendu (documenté pour l'orchestrateur)
//! - Titre centré "The MEANS Procedure" via `page_header()` puis une ligne
//!   centrée.
//! - Sans CLASS : une table, colonne `Variable` puis une colonne par stat
//!   demandée (défaut : N, Mean, Std Dev, Minimum, Maximum). Une ligne par
//!   variable analysée.
//! - Avec CLASS : une table COMBINÉE — colonnes de tête = chaque variable
//!   CLASS, puis colonne `Variable`, puis une colonne par stat. Une ligne
//!   par (combinaison de classes × variable). Les combinaisons de classes
//!   sont ordonnées par `sas_cmp`.

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{decode_column, group_by_keys, partition_numeric, sample_std};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};
use polars::prelude::*;
use std::cmp::Ordering;

pub struct MeansAst {
    pub data: Option<DatasetRef>,
    pub summary: bool,
    pub noprint: bool,
    pub stats: Vec<String>,
    pub class: Vec<String>,
    pub var: Vec<String>,
    pub output: Option<MeansOutput>,
}

pub struct MeansOutput {
    pub out: DatasetRef,
    /// (stat, var source, nom de sortie)
    pub specs: Vec<(String, String, String)>,
}

/// Recognized statistic keywords accepted on the PROC MEANS statement.
const STAT_KEYWORDS: &[&str] = &[
    "n", "nmiss", "mean", "std", "stddev", "min", "max", "sum", "range", "stderr", "cv", "median",
];

fn is_stat_keyword(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    STAT_KEYWORDS.iter().any(|k| *k == l)
}

/// Parse `proc means [data=a] [noprint] [stat...] ; [class ...;] [var ...;]
/// [output out=b stat(var)=name...;] ... run;`. Called AFTER "proc
/// means"/"proc summary" has been consumed. Consumes through `run;`/`quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<MeansAst> {
    let mut data: Option<DatasetRef> = None;
    let mut noprint = false;
    let mut stats: Vec<String> = Vec::new();

    // --- PROC MEANS statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next(); // consume `;`
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            ts.next();
            expect_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("noprint") {
            ts.next();
            noprint = true;
        } else if ts.peek().is_kw("print") {
            // explicit PRINT — undo a noprint default (e.g. PROC SUMMARY).
            ts.next();
            noprint = false;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            if is_stat_keyword(&name) {
                ts.next();
                stats.push(name.to_ascii_lowercase());
            } else {
                let span = ts.peek().span;
                return Err(SasError::parse(
                    format!(
                        "Unexpected option '{}' on PROC MEANS statement.",
                        name.to_uppercase()
                    ),
                    span,
                ));
            }
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC MEANS statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut class: Vec<String> = Vec::new();
    let mut var: Vec<String> = Vec::new();
    let mut output: Option<MeansOutput> = None;

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

        if ts.peek().is_kw("class") {
            ts.next();
            class = ts.parse_name_list()?;
            ts.expect_semi()?;
        } else if ts.peek().is_kw("var") {
            ts.next();
            var = ts.parse_name_list()?;
            ts.expect_semi()?;
        } else if ts.peek().is_kw("output") {
            ts.next();
            output = Some(parse_output(ts)?);
        } else {
            // Unknown sub-statement: skip it (recovery, like sort/print).
            ts.skip_to_semi();
        }
    }

    Ok(MeansAst {
        data,
        summary: false,
        noprint,
        stats,
        class,
        var,
        output,
    })
}

/// Parse the OUTPUT statement body (after "output" was consumed), through
/// its terminating `;`. `output out=lib.t [stat(var)=name ...] ;`
fn parse_output(ts: &mut StatementStream) -> Result<MeansOutput> {
    let mut out: Option<DatasetRef> = None;
    let mut specs: Vec<(String, String, String)> = Vec::new();

    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("out") {
            ts.next();
            expect_eq(ts, "OUT")?;
            out = Some(ts.parse_dataset_ref()?);
        } else if let Some(stat) = ts.peek().ident().map(str::to_string) {
            // Expect `stat(var)=name`.
            ts.next(); // stat
            if ts.peek().kind != TokenKind::LParen {
                return Err(SasError::parse(
                    format!("expected '(' after statistic '{}' in OUTPUT", stat),
                    ts.peek().span,
                ));
            }
            ts.next(); // '('
            let var = match ts.peek().ident().map(str::to_string) {
                Some(v) => {
                    ts.next();
                    v
                }
                None => {
                    return Err(SasError::parse(
                        "expected a variable name inside OUTPUT statistic spec",
                        ts.peek().span,
                    ));
                }
            };
            if ts.peek().kind != TokenKind::RParen {
                return Err(SasError::parse(
                    "expected ')' in OUTPUT statistic spec",
                    ts.peek().span,
                ));
            }
            ts.next(); // ')'
            expect_eq(ts, "OUTPUT statistic")?;
            let name = match ts.peek().ident().map(str::to_string) {
                Some(n) => {
                    ts.next();
                    n
                }
                None => {
                    return Err(SasError::parse(
                        "expected an output variable name in OUTPUT statistic spec",
                        ts.peek().span,
                    ));
                }
            };
            specs.push((stat.to_ascii_lowercase(), var, name));
        } else {
            return Err(SasError::parse(
                "unexpected token in OUTPUT statement",
                ts.peek().span,
            ));
        }
    }

    let out = out.ok_or_else(|| {
        SasError::runtime("The OUTPUT statement requires the OUT= option in PROC MEANS.")
    })?;
    Ok(MeansOutput { out, specs })
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

/// Resolve `data=` or `_LAST_` into a concrete DatasetRef.
fn resolve_input(ast: &MeansAst, session: &Session) -> Result<DatasetRef> {
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

/// Compute one statistic over the NON-MISSING numeric values `xs` of a
/// group. `n`/`nmiss` are passed the group's non-missing/missing counts
/// separately because they depend on the missing tally, not on `xs`.
/// Returns a `Value` (`Value::missing()` when undefined for the group).
pub fn compute(stat: &str, xs: &[f64], n_missing: usize) -> Value {
    let n = xs.len();
    match stat {
        "n" => Value::Num(n as f64),
        "nmiss" => Value::Num(n_missing as f64),
        "min" => {
            if n == 0 {
                Value::missing()
            } else {
                Value::Num(xs.iter().cloned().fold(f64::INFINITY, f64::min))
            }
        }
        "max" => {
            if n == 0 {
                Value::missing()
            } else {
                Value::Num(xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
            }
        }
        "range" => {
            if n == 0 {
                Value::missing()
            } else {
                let mn = xs.iter().cloned().fold(f64::INFINITY, f64::min);
                let mx = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                Value::Num(mx - mn)
            }
        }
        "sum" => Value::Num(xs.iter().sum()),
        "mean" => {
            if n == 0 {
                Value::missing()
            } else {
                Value::Num(xs.iter().sum::<f64>() / n as f64)
            }
        }
        "std" | "stddev" => match sample_std(xs) {
            Some(s) => Value::Num(s),
            None => Value::missing(),
        },
        "stderr" => match sample_std(xs) {
            Some(s) if n >= 1 => Value::Num(s / (n as f64).sqrt()),
            _ => Value::missing(),
        },
        "cv" => {
            let mean = if n == 0 {
                return Value::missing();
            } else {
                xs.iter().sum::<f64>() / n as f64
            };
            match sample_std(xs) {
                Some(s) if mean != 0.0 => Value::Num(100.0 * s / mean),
                _ => Value::missing(),
            }
        }
        "median" => match median(xs) {
            Some(m) => Value::Num(m),
            None => Value::missing(),
        },
        _ => Value::missing(),
    }
}

/// Median of the non-missing values (None when empty).
fn median(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    let mut v: Vec<f64> = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        Some(v[n / 2])
    } else {
        Some((v[n / 2 - 1] + v[n / 2]) / 2.0)
    }
}

/// Header text used both as report header and a stat label in the listing.
pub fn stat_header(stat: &str) -> &'static str {
    match stat {
        "n" => "N",
        "nmiss" => "NMiss",
        "mean" => "Mean",
        "std" | "stddev" => "Std Dev",
        "min" => "Minimum",
        "max" => "Maximum",
        "sum" => "Sum",
        "range" => "Range",
        "stderr" => "Std Error",
        "cv" => "CV",
        "median" => "Median",
        _ => "Stat",
    }
}

/// Render a single computed stat value into a listing cell.
fn fmt_stat_cell(stat: &str, v: &Value) -> String {
    match v {
        Value::Num(f) => {
            if stat == "n" || stat == "nmiss" {
                format!("{}", *f as i64)
            } else {
                format_best(*f, 12)
            }
        }
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.clone(),
    }
}

/// Execute PROC MEANS / SUMMARY. Called by `procs::execute_proc`.
pub fn execute(ast: &MeansAst, session: &mut Session) -> Result<()> {
    let in_ref = resolve_input(ast, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_obs = ds.n_obs();

    // Resolve CLASS column indices (validate existence).
    let mut class_cols: Vec<usize> = Vec::with_capacity(ast.class.len());
    for cname in &ast.class {
        match ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(cname))
        {
            Some(i) => class_cols.push(i),
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    cname.to_uppercase()
                )));
            }
        }
    }

    // Determine the VAR list: explicit `var`, else all NUMERIC variables not
    // in CLASS.
    let var_cols: Vec<usize> = if !ast.var.is_empty() {
        let mut v = Vec::with_capacity(ast.var.len());
        for vname in &ast.var {
            match ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(vname))
            {
                Some(i) => v.push(i),
                None => {
                    return Err(SasError::runtime(format!(
                        "Variable {} not found.",
                        vname.to_uppercase()
                    )));
                }
            }
        }
        v
    } else {
        (0..ds.vars.len())
            .filter(|&i| ds.vars[i].ty == VarType::Num && !class_cols.contains(&i))
            .collect()
    };

    // Decode CLASS columns and VAR columns once each.
    let class_values: Vec<Vec<Value>> = class_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;
    let var_values: Vec<Vec<Value>> = var_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;

    // Default report stats when none requested.
    let report_stats: Vec<String> = if ast.stats.is_empty() {
        vec![
            "n".into(),
            "mean".into(),
            "std".into(),
            "min".into(),
            "max".into(),
        ]
    } else {
        ast.stats.clone()
    };

    // --- Report ---
    if !ast.noprint {
        emit_report(
            session,
            &ds,
            &class_cols,
            &class_values,
            &var_cols,
            &var_values,
            &report_stats,
            n_obs,
        );
    }

    // --- OUTPUT OUT= ---
    if let Some(out) = &ast.output {
        write_output(
            session,
            &ds,
            &class_cols,
            &class_values,
            &var_values,
            &var_cols,
            out,
            n_obs,
        )?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_report(
    session: &mut Session,
    ds: &SasDataset,
    class_cols: &[usize],
    class_values: &[Vec<Value>],
    var_cols: &[usize],
    var_values: &[Vec<Value>],
    report_stats: &[String],
    n_obs: usize,
) {
    session.listing.page_header();
    // Centered procedure title line.
    let title = "The MEANS Procedure";
    let ls = session.listing.ls;
    let pad = ls.saturating_sub(title.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), title));
    session.listing.blank();

    let mut headers: Vec<String> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();

    // Leading CLASS columns (only when CLASS present).
    for &ci in class_cols {
        headers.push(ds.vars[ci].name.clone());
        aligns.push(match ds.vars[ci].ty {
            VarType::Num => Align::Right,
            VarType::Char => Align::Left,
        });
    }
    headers.push("Variable".to_string());
    aligns.push(Align::Left);
    for s in report_stats {
        headers.push(stat_header(s).to_string());
        aligns.push(Align::Right);
    }

    let mut rows: Vec<Vec<String>> = Vec::new();

    if class_cols.is_empty() {
        // One section over all rows: one row per analysis variable.
        let all_rows: Vec<usize> = (0..n_obs).collect();
        for (vi, vname_idx) in var_cols.iter().enumerate() {
            let (xs, nmiss) = partition_numeric(&var_values[vi], &all_rows);
            let mut row = vec![ds.vars[*vname_idx].name.clone()];
            for s in report_stats {
                let v = compute(s, &xs, nmiss);
                row.push(fmt_stat_cell(s, &v));
            }
            rows.push(row);
        }
    } else {
        let cv_refs: Vec<&Vec<Value>> = class_values.iter().collect();
        let groups = group_by_keys(&cv_refs, n_obs);
        for (key, grp_rows) in &groups {
            for (vi, vname_idx) in var_cols.iter().enumerate() {
                let (xs, nmiss) = partition_numeric(&var_values[vi], grp_rows);
                let mut row: Vec<String> = Vec::new();
                for kv in key {
                    row.push(class_cell(kv));
                }
                row.push(ds.vars[*vname_idx].name.clone());
                for s in report_stats {
                    let v = compute(s, &xs, nmiss);
                    row.push(fmt_stat_cell(s, &v));
                }
                rows.push(row);
            }
        }
    }

    session.listing.write_table(&headers, &aligns, &rows);
}

/// Render a class-value cell in the listing.
fn class_cell(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn write_output(
    session: &mut Session,
    ds: &SasDataset,
    class_cols: &[usize],
    class_values: &[Vec<Value>],
    var_values: &[Vec<Value>],
    var_cols: &[usize],
    out: &MeansOutput,
    n_obs: usize,
) -> Result<()> {
    let k = class_cols.len();

    // Resolve each output spec's source VAR to an index into var_cols /
    // var_values (the source column must be a VAR — decode it on demand if
    // not already in the VAR list).
    // Build a name->decoded-column map for the spec sources.
    struct Spec {
        stat: String,
        outname: String,
        col: Vec<Value>,
    }
    let mut specs: Vec<Spec> = Vec::with_capacity(out.specs.len());
    for (stat, srcvar, outname) in &out.specs {
        // Find the source column in the dataset.
        let col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(srcvar))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", srcvar.to_uppercase()))
            })?;
        // Reuse already-decoded VAR column if available, else decode.
        let col = match var_cols.iter().position(|&c| c == col_idx) {
            Some(p) => var_values[p].clone(),
            None => decode_column(ds, col_idx)?,
        };
        specs.push(Spec {
            stat: stat.clone(),
            outname: outname.clone(),
            col,
        });
    }

    // Output rows accumulate as: (type, key_tuple_for_active_classes,
    // per-class-cell-values, freq, stat-values).
    struct OutRow {
        ty: f64,
        // class cell value per class var (active = group key value;
        // inactive = missing of right type).
        class_cells: Vec<Value>,
        // sort key: only the active classes' values (in class order).
        sort_key: Vec<Value>,
        freq: f64,
        stats: Vec<Value>,
    }
    let mut out_rows: Vec<OutRow> = Vec::new();

    let class_refs: Vec<&Vec<Value>> = class_values.iter().collect();

    // Enumerate all 2^k subsets.
    for mask in 0u32..(1u32 << k) {
        // Active class indices for this subset.
        let active: Vec<usize> = (0..k).filter(|&i| (mask >> i) & 1 == 1).collect();

        // _TYPE_ : LSB corresponds to the LAST class variable. For class var
        // i (0-based), bit position (k-1 - i) represents it.
        let mut ty: u64 = 0;
        for &i in &active {
            ty |= 1u64 << (k - 1 - i);
        }

        // Group rows by the active class variables.
        let active_refs: Vec<&Vec<Value>> = active.iter().map(|&i| class_refs[i]).collect();
        let groups = group_by_keys(&active_refs, n_obs);

        for (active_key, grp_rows) in &groups {
            // Build the per-class-var cell values: active vars use the group
            // key value; inactive vars store a missing of the right type.
            let mut class_cells: Vec<Value> = Vec::with_capacity(k);
            let mut ai = 0usize;
            for (i, &col_idx) in class_cols.iter().enumerate() {
                if active.contains(&i) {
                    class_cells.push(active_key[ai].clone());
                    ai += 1;
                } else {
                    // Missing of the right type.
                    match ds.vars[col_idx].ty {
                        VarType::Num => class_cells.push(Value::missing()),
                        VarType::Char => class_cells.push(Value::Char(String::new())),
                    }
                }
            }

            let freq = grp_rows.len() as f64;

            let mut stat_vals: Vec<Value> = Vec::with_capacity(specs.len());
            for sp in &specs {
                let (xs, nmiss) = partition_numeric(&sp.col, grp_rows);
                stat_vals.push(compute(&sp.stat, &xs, nmiss));
            }

            out_rows.push(OutRow {
                ty: ty as f64,
                class_cells,
                sort_key: active_key.clone(),
                freq,
                stats: stat_vals,
            });
        }
    }

    // Order rows: _TYPE_ ascending, then active class-value tuple sas_cmp.
    out_rows.sort_by(|a, b| {
        match a.ty.partial_cmp(&b.ty).unwrap_or(Ordering::Equal) {
            Ordering::Equal => {}
            other => return other,
        }
        for (x, y) in a.sort_key.iter().zip(&b.sort_key) {
            let c = x.sas_cmp(y);
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    });

    // Build the output DataFrame column-by-column.
    let n_rows = out_rows.len();
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // CLASS columns (copy input VarMeta; encode per-row values).
    for (ci, &col_idx) in class_cols.iter().enumerate() {
        let meta = &ds.vars[col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = out_rows
                    .iter()
                    .map(|r| value_to_num(&r.class_cells[ci]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = out_rows
                    .iter()
                    .map(|r| match &r.class_cells[ci] {
                        Value::Char(s) if s.is_empty() => None,
                        Value::Char(s) => Some(s.clone()),
                        Value::Missing(_) => None,
                        Value::Num(_) => None,
                    })
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
        vars.push(meta.clone());
    }

    // _TYPE_
    let type_vals: Vec<Option<f64>> = out_rows.iter().map(|r| Some(r.ty)).collect();
    columns.push(Series::new("_TYPE_".into(), type_vals).into());
    vars.push(num_var_meta("_TYPE_"));

    // _FREQ_
    let freq_vals: Vec<Option<f64>> = out_rows.iter().map(|r| Some(r.freq)).collect();
    columns.push(Series::new("_FREQ_".into(), freq_vals).into());
    vars.push(num_var_meta("_FREQ_"));

    // One column per output spec.
    for (si, sp) in specs.iter().enumerate() {
        let vals: Vec<Option<f64>> = out_rows
            .iter()
            .map(|r| value_to_num(&r.stats[si]))
            .collect();
        columns.push(Series::new(sp.outname.as_str().into(), vals).into());
        vars.push(num_var_meta(&sp.outname));
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = out.out.libref_or_work();
    let out_table = out.out.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());

    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));

    Ok(())
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

    fn parse_means(src: &str) -> Result<MeansAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "means"
        parse(&mut ts)
    }

    // ───────────────────────────── parse tests ─────────────────────────────

    #[test]
    fn parse_header_stats() {
        let ast = parse_means("proc means data=a n mean std min max; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert!(!ast.noprint);
        assert_eq!(ast.stats, vec!["n", "mean", "std", "min", "max"]);
    }

    #[test]
    fn parse_noprint() {
        let ast = parse_means("proc means data=a noprint; run;").unwrap();
        assert!(ast.noprint);
    }

    #[test]
    fn parse_class_and_var() {
        let ast =
            parse_means("proc means data=a; class g h; var x y; run;").unwrap();
        assert_eq!(ast.class, vec!["g", "h"]);
        assert_eq!(ast.var, vec!["x", "y"]);
    }

    #[test]
    fn parse_output_specs() {
        let ast = parse_means(
            "proc means data=a; var height; output out=b mean(height)=avg_h n(height)=n_h; run;",
        )
        .unwrap();
        let out = ast.output.as_ref().unwrap();
        assert_eq!(out.out.name, "b");
        assert_eq!(
            out.specs,
            vec![
                ("mean".to_string(), "height".to_string(), "avg_h".to_string()),
                ("n".to_string(), "height".to_string(), "n_h".to_string()),
            ]
        );
    }

    #[test]
    fn parse_unknown_option_errors() {
        let r = parse_means("proc means data=a bogus; run;");
        assert!(r.is_err());
        let msg = r.err().unwrap().to_string();
        assert!(msg.contains("BOGUS"), "msg: {msg}");
    }

    // ───────────────────────────── compute tests ───────────────────────────

    #[test]
    fn compute_basic_stats_with_a_missing() {
        // values: 2, 4, 6, missing -> non-missing [2,4,6], nmiss=1
        let xs = vec![2.0, 4.0, 6.0];
        assert_eq!(compute("n", &xs, 1), Value::Num(3.0));
        assert_eq!(compute("nmiss", &xs, 1), Value::Num(1.0));
        assert_eq!(compute("mean", &xs, 1), Value::Num(4.0));
        assert_eq!(compute("min", &xs, 1), Value::Num(2.0));
        assert_eq!(compute("max", &xs, 1), Value::Num(6.0));
        assert_eq!(compute("sum", &xs, 1), Value::Num(12.0));
        assert_eq!(compute("range", &xs, 1), Value::Num(4.0));
        assert_eq!(compute("median", &xs, 1), Value::Num(4.0));
        // std of [2,4,6]: variance = ((2-4)^2+(4-4)^2+(6-4)^2)/2 = 8/2 = 4 -> std 2
        assert_eq!(compute("std", &xs, 1), Value::Num(2.0));
    }

    #[test]
    fn compute_median_even() {
        let xs = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(compute("median", &xs, 0), Value::Num(2.5));
    }

    #[test]
    fn compute_edge_n0_and_n1() {
        let empty: Vec<f64> = vec![];
        assert_eq!(compute("n", &empty, 0), Value::Num(0.0));
        assert!(compute("mean", &empty, 0).is_missing());
        assert!(compute("std", &empty, 0).is_missing());
        assert!(compute("min", &empty, 0).is_missing());
        assert!(compute("range", &empty, 0).is_missing());
        assert_eq!(compute("sum", &empty, 0), Value::Num(0.0));

        let one = vec![5.0];
        assert_eq!(compute("n", &one, 0), Value::Num(1.0));
        assert_eq!(compute("mean", &one, 0), Value::Num(5.0));
        // std needs n>=2.
        assert!(compute("std", &one, 0).is_missing());
        assert!(compute("stderr", &one, 0).is_missing());
        assert_eq!(compute("min", &one, 0), Value::Num(5.0));
    }

    #[test]
    fn compute_cv_and_stderr() {
        // [2,4,6]: mean 4, std 2 -> cv = 100*2/4 = 50; stderr = 2/sqrt(3)
        let xs = vec![2.0, 4.0, 6.0];
        assert_eq!(compute("cv", &xs, 0), Value::Num(50.0));
        if let Value::Num(se) = compute("stderr", &xs, 0) {
            assert!((se - 2.0 / 3.0_f64.sqrt()).abs() < 1e-12);
        } else {
            panic!("stderr should be numeric");
        }
    }

    // ───────────────────────────── execute tests ───────────────────────────

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    fn read_num_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
        let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
        let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
        decode_column(&ds, idx).unwrap()
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

    #[test]
    fn execute_output_k0_no_class() {
        let mut session = make_session();
        let df = df!["x" => [Some(2.0_f64), Some(4.0), Some(6.0), None]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: true,
            stats: vec![],
            class: vec![],
            var: vec!["x".into()],
            output: Some(MeansOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![
                    ("mean".into(), "x".into(), "m".into()),
                    ("n".into(), "x".into(), "cnt".into()),
                    ("nmiss".into(), "x".into(), "nm".into()),
                ],
            }),
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 1);
        let ty = read_num_col(&session, "O", "_TYPE_");
        let freq = read_num_col(&session, "O", "_FREQ_");
        let m = read_num_col(&session, "O", "m");
        let cnt = read_num_col(&session, "O", "cnt");
        let nm = read_num_col(&session, "O", "nm");
        assert_eq!(ty, vec![Value::Num(0.0)]);
        assert_eq!(freq, vec![Value::Num(4.0)]); // all rows incl. missing
        assert_eq!(m, vec![Value::Num(4.0)]);
        assert_eq!(cnt, vec![Value::Num(3.0)]);
        assert_eq!(nm, vec![Value::Num(1.0)]);
    }

    #[test]
    fn execute_output_k1() {
        let mut session = make_session();
        // group g: a(1,3) b(10)  -> means a=2, b=10
        let df = df![
            "g" => ["a", "a", "b"],
            "x" => [1.0_f64, 3.0, 10.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: true,
            stats: vec![],
            class: vec!["g".into()],
            var: vec!["x".into()],
            output: Some(MeansOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![
                    ("mean".into(), "x".into(), "mx".into()),
                ],
            }),
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        // _TYPE_ 0 (overall) + 2 levels = 3 rows.
        assert_eq!(out.n_obs(), 3);

        let ty = read_num_col(&session, "O", "_TYPE_");
        let freq = read_num_col(&session, "O", "_FREQ_");
        let mx = read_num_col(&session, "O", "mx");
        let g = read_num_col(&session, "O", "g"); // char col decoded as Value::Char

        // Row 0 = overall (_TYPE_=0): freq 3, mean (1+3+10)/3.
        assert_eq!(ty[0], Value::Num(0.0));
        assert_eq!(freq[0], Value::Num(3.0));
        assert_eq!(mx[0], Value::Num((1.0 + 3.0 + 10.0) / 3.0));
        // The overall class cell is blank (inactive char -> empty/null).
        assert_eq!(g[0], Value::Char(String::new()));

        // Rows 1,2 = per-level (_TYPE_=1), ordered a then b.
        assert_eq!(ty[1], Value::Num(1.0));
        assert_eq!(ty[2], Value::Num(1.0));
        assert_eq!(g[1], Value::Char("a".into()));
        assert_eq!(g[2], Value::Char("b".into()));
        assert_eq!(freq[1], Value::Num(2.0));
        assert_eq!(freq[2], Value::Num(1.0));
        assert_eq!(mx[1], Value::Num(2.0));
        assert_eq!(mx[2], Value::Num(10.0));
    }

    #[test]
    fn execute_output_k2_type_set_and_rowcount() {
        let mut session = make_session();
        // c0 (g) has 2 levels {a,b}; c1 (h) has 2 levels {1,2}.
        // combos present: (a,1),(a,2),(b,1) -> 3 combos.
        let df = df![
            "g" => ["a", "a", "b"],
            "h" => [1.0_f64, 2.0, 1.0],
            "x" => [5.0_f64, 7.0, 9.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("g"), num_meta("h"), num_meta("x")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: true,
            stats: vec![],
            class: vec!["g".into(), "h".into()],
            var: vec!["x".into()],
            output: Some(MeansOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![("sum".into(), "x".into(), "sx".into())],
            }),
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        // Expected rows:
        //   _TYPE_=0 : 1 (overall)
        //   _TYPE_=1 : levels of LAST class (h) = {1,2} -> 2
        //   _TYPE_=2 : levels of FIRST class (g) = {a,b} -> 2
        //   _TYPE_=3 : combos = 3
        // total = 1 + 2 + 2 + 3 = 8
        assert_eq!(out.n_obs(), 8);

        let ty = read_num_col(&session, "O", "_TYPE_");
        let type_set: std::collections::BTreeSet<i64> = ty
            .iter()
            .map(|v| match v {
                Value::Num(f) => *f as i64,
                _ => panic!("type must be numeric"),
            })
            .collect();
        assert_eq!(
            type_set,
            [0i64, 1, 2, 3].iter().cloned().collect()
        );

        // _TYPE_ is ascending.
        let tys: Vec<f64> = ty.iter().map(|v| match v {
            Value::Num(f) => *f,
            _ => unreachable!(),
        }).collect();
        let mut sorted = tys.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(tys, sorted);

        // _TYPE_=3 sum check: combo (a,1) sum=5, (a,2)=7, (b,1)=9; overall freq.
        let freq = read_num_col(&session, "O", "_FREQ_");
        assert_eq!(freq[0], Value::Num(3.0)); // overall _TYPE_=0
    }

    #[test]
    fn execute_report_contains_title_and_var() {
        let mut session = make_session();
        let df = df!["height" => [60.0_f64, 62.0, 64.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("height")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: false,
            stats: vec![],
            class: vec![],
            var: vec![],
            output: None,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("The MEANS Procedure"), "listing: {listing}");
        assert!(listing.contains("height"), "listing: {listing}");
        // default stats headers
        assert!(listing.contains("Mean"), "listing: {listing}");
        assert!(listing.contains("Minimum"), "listing: {listing}");
    }

    #[test]
    fn execute_noprint_writes_nothing_to_listing() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: true,
            noprint: true,
            stats: vec![],
            class: vec![],
            var: vec![],
            output: None,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(
            !listing.contains("The MEANS Procedure"),
            "noprint should not emit a report: {listing}"
        );
    }
}
