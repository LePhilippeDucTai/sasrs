//! PROC FASTCLUS — k-means clustering (M27).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc fastclus data=<ref> maxclusters=k [out=<ref>] [maxiter=<n>]
//!  [converge=<f>]; var <list>; run;`
//!
//! ## Périmètre
//! - `maxclusters=` (obligatoire), `out=` (colonnes d'entrée + `_CLUSTER_`),
//!   `maxiter=` (défaut 10), `converge=` (défaut 0.02).
//! - `var` : variables numériques (coordonnées).
//! - Non supporté, ERROR (J02-P4) : `SEED=` (table de graines SAS ; un nombre
//!   n'est pas valide), valeur manquante d'une variable VAR (J09-P7),
//!   instructions `ID`, `BY`, `FREQ`, `WEIGHT`.
//!
//! ## Algorithme (approximation documentée, NOTE à chaque exécution)
//! 1. Graines : sélection "farthest-first" — graine 1 = première obs, graine
//!    suivante = obs la plus éloignée des graines déjà choisies.
//! 2. Affecter chaque obs au centroïde le plus proche (euclidien).
//! 3. Recalculer les centroïdes.
//! 4. Répéter jusqu'à maxiter ou convergence (déplacement max < converge×RMS).
//!
//! SAS 9.4 choisit les graines par les règles RADIUS=/REPLACE=, ne fait
//! qu'une itération par défaut (MAXITER=1) et suppose MAXCLUSTERS=100 :
//! les partitions peuvent différer (docs/support-contract.md, « Approximations
//! documentées », roadmap-avancee J09-P7).

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::procs::distance::{DistMethod, distance};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::VarType;
use polars::prelude::*;

mod kmeans;

pub use kmeans::KMeansResult;
pub use kmeans::kmeans;

use kmeans::*;

// ───────────────────────── AST ─────────────────────────

pub struct FastclusAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub maxclusters: usize,
    pub maxiter: usize,
    pub converge: f64,
    pub var: Vec<String>,
}

// ───────────────────────── Parser ─────────────────────────

fn parse_num(ts: &mut StatementStream, opt: &str) -> Result<f64> {
    let span = ts.peek().span;
    match ts.peek().kind {
        TokenKind::Num(v) => {
            ts.next();
            Ok(v)
        }
        _ => Err(SasError::parse(
            format!("expected a number after {opt}="),
            span,
        )),
    }
}

/// Parse the PROC FASTCLUS block. Called AFTER "proc fastclus" consumed.
pub fn parse(ts: &mut StatementStream) -> Result<FastclusAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut maxclusters: Option<usize> = None;
    let mut maxiter: usize = 10;
    let mut converge: f64 = 0.02;

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
        } else if ts.peek().is_kw("out") {
            out = Some(common::parse_out_opt(ts)?);
        } else if ts.peek().is_kw("maxclusters") || ts.peek().is_kw("maxc") {
            common::consume_option_eq(ts, "MAXCLUSTERS")?;
            maxclusters = Some(parse_num(ts, "MAXCLUSTERS")? as usize);
        } else if ts.peek().is_kw("maxiter") {
            common::consume_option_eq(ts, "MAXITER")?;
            maxiter = parse_num(ts, "MAXITER")? as usize;
        } else if ts.peek().is_kw("converge") {
            common::consume_option_eq(ts, "CONVERGE")?;
            converge = parse_num(ts, "CONVERGE")?;
        } else if ts.peek().is_kw("seed") {
            // J02-P4 — SEED=<number> used to be accepted and ignored (the
            // seeds stayed farthest-first). SAS/STAT 9.4, PROC FASTCLUS
            // statement: SEED= names a data set of initial cluster seeds.
            common::consume_option_eq(ts, "SEED")?;
            let span = ts.peek().span;
            if matches!(ts.peek().kind, TokenKind::Num(_)) {
                return Err(SasError::parse(
                    "SEED= names a SAS data set of initial cluster seeds in PROC FASTCLUS, \
                     not a number.",
                    span,
                ));
            }
            return Err(SasError::parse(
                "The SEED= option (data set of initial cluster seeds) is not supported in \
                 PROC FASTCLUS; it can affect results and cannot be ignored (planned: \
                 roadmap-avancee J09-P7).",
                span,
            ));
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC FASTCLUS statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC FASTCLUS statement.",
                span,
            ));
        }
    }

    let maxclusters = maxclusters.ok_or_else(|| {
        SasError::runtime("The MAXCLUSTERS= option is required for PROC FASTCLUS.")
    })?;

    let mut var: Vec<String> = Vec::new();
    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31). BY,
    // FREQ and WEIGHT fall back to the shared contract ERROR, like ID.
    common::parse_proc_body(ts, "FASTCLUS", |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next();
                var = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            "id" => return Err(common::unsupported_statement("FASTCLUS", "ID")),
            _ => false,
        })
    })?;

    Ok(FastclusAst {
        data,
        out,
        maxclusters,
        maxiter,
        converge,
        var,
    })
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &FastclusAst, session: &mut Session) -> Result<()> {
    if ast.var.is_empty() {
        return Err(SasError::runtime("PROC FASTCLUS requires a VAR statement."));
    }

    let (ds, display) = common::open_input_display(&ast.data, session)?;
    let n_read = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_read, display
    ));

    let mut cols: Vec<usize> = Vec::with_capacity(ast.var.len());
    for nm in &ast.var {
        match ds.vars.iter().position(|m| m.name.eq_ignore_ascii_case(nm)) {
            Some(i) if ds.vars[i].ty == VarType::Num => cols.push(i),
            _ => {
                return Err(SasError::runtime(format!(
                    "Variable '{}' not found in dataset '{}'.",
                    nm, display
                )));
            }
        }
    }
    let p = cols.len();
    let names: Vec<String> = cols.iter().map(|&c| ds.vars[c].name.clone()).collect();

    let decoded: Vec<Vec<f64>> = cols
        .iter()
        .map(|&c| {
            decode_column(&ds, c).map(|vals| {
                vals.iter()
                    .map(|v| value_to_num(v).unwrap_or(f64::NAN))
                    .collect::<Vec<f64>>()
            })
        })
        .collect::<Result<Vec<_>>>()?;

    // J02-P4 — a missing VAR value used to enter seeds, distances and
    // centroids as NaN silently. ERROR until roadmap-avancee J09-P7 applies
    // the SAS rules for missing values.
    for (col, name) in decoded.iter().zip(&names) {
        if let Some(row) = col.iter().position(|x| x.is_nan()) {
            return Err(SasError::runtime(format!(
                "A missing value in a VAR variable ({}, observation {}) is not supported in \
                 PROC FASTCLUS; it can affect results and cannot be ignored (planned: \
                 roadmap-avancee J09-P7).",
                name.to_uppercase(),
                row + 1
            )));
        }
    }

    let coords: Vec<Vec<f64>> = (0..n_read)
        .map(|r| decoded.iter().map(|col| col[r]).collect())
        .collect();
    let n = coords.len();
    let k = ast.maxclusters.min(n).max(1);

    // J02-P4 — the k-means below diverges from the SAS 9.4 algorithm without
    // saying so: documented approximation, NOTE at each execution.
    session.log.note(
        "PROC FASTCLUS approximates the SAS algorithm: seeds are selected farthest-first, \
         MAXITER= defaults to 10 and MAXCLUSTERS= is required (SAS: RADIUS=/REPLACE= seed \
         rules, MAXITER=1, MAXCLUSTERS=100); cluster assignments can differ from SAS \
         (planned: roadmap-avancee J09-P7).",
    );

    let res = kmeans(&coords, k, ast.maxiter, ast.converge);

    // Per-cluster stats.
    let mut freq = vec![0usize; k];
    let mut max_seed_dist = vec![0.0_f64; k];
    for (assign, coord) in res.assign.iter().zip(coords.iter()).take(n) {
        let c = assign - 1;
        freq[c] += 1;
        let d = distance(DistMethod::Euclid, coord, &res.centroids[c]);
        max_seed_dist[c] = max_seed_dist[c].max(d);
    }

    // Nearest cluster (by centroid distance) + that distance.
    let mut nearest = vec![0usize; k];
    let mut nearest_dist = vec![0.0_f64; k];
    for c in 0..k {
        let mut best = c;
        let mut bestd = f64::INFINITY;
        for o in 0..k {
            if o == c {
                continue;
            }
            let d = distance(DistMethod::Euclid, &res.centroids[c], &res.centroids[o]);
            if d < bestd {
                bestd = d;
                best = o;
            }
        }
        nearest[c] = best;
        nearest_dist[c] = if bestd.is_finite() { bestd } else { 0.0 };
    }

    // Per-cluster RMS std (pooled across variables, denominator n).
    let cluster_rms: Vec<f64> = (0..k)
        .map(|c| cluster_rms_std(&coords, &res.assign, c + 1, &res.centroids[c]))
        .collect();

    // R-Square per variable and overall: 1 - within-SS / total-SS.
    let (var_rsq, overall_rsq, total_std, within_std) =
        variable_rsquare(&coords, &res.assign, &res.centroids, p);

    // ───────────────────────── listing ─────────────────────────
    session.listing.page_header();
    centered(session, "The FASTCLUS Procedure");
    session.listing.blank();
    centered(
        session,
        &format!(
            "Replace=FULL   Radius=0   Maxclusters={}   Maxiter={}",
            k, ast.maxiter
        ),
    );
    session.listing.blank();
    centered(session, "Cluster Summary");
    session.listing.blank();
    {
        let headers: Vec<String> = vec![
            "Cluster".into(),
            "Frequency".into(),
            "RMS Std".into(),
            "Maximum Distance from Seed to Obs".into(),
            "Nearest Cluster".into(),
            "Distance Between Cluster Centroids".into(),
        ];
        let aligns = vec![
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let rows: Vec<Vec<String>> = (0..k)
            .map(|c| {
                vec![
                    (c + 1).to_string(),
                    freq[c].to_string(),
                    format!("{:.4}", cluster_rms[c]),
                    format!("{:.4}", max_seed_dist[c]),
                    (nearest[c] + 1).to_string(),
                    format!("{:.4}", nearest_dist[c]),
                ]
            })
            .collect();
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    centered(session, "Statistics for Variables");
    session.listing.blank();
    {
        let headers: Vec<String> = vec![
            "Variable".into(),
            "Total STD".into(),
            "Within STD".into(),
            "R-Square".into(),
            "RSQ/(1-RSQ)".into(),
        ];
        let aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(p + 1);
        for j in 0..p {
            let rsq = var_rsq[j];
            rows.push(vec![
                names[j].clone(),
                format!("{:.4}", total_std[j]),
                format!("{:.4}", within_std[j]),
                format!("{:.4}", rsq),
                format!("{:.4}", ratio(rsq)),
            ]);
        }
        // Overall row.
        let tot_all = (total_std.iter().map(|s| s * s).sum::<f64>() / p as f64).sqrt();
        let wit_all = (within_std.iter().map(|s| s * s).sum::<f64>() / p as f64).sqrt();
        rows.push(vec![
            "OVER-ALL".into(),
            format!("{:.4}", tot_all),
            format!("{:.4}", wit_all),
            format!("{:.4}", overall_rsq),
            format!("{:.4}", ratio(overall_rsq)),
        ]);
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // ───────────────────────── out= ─────────────────────────
    if let Some(out_ref) = &ast.out {
        // Copy input columns and append _CLUSTER_.
        let mut out_ds_df = ds.df.clone();
        let cluster_col: Vec<f64> = res.assign.iter().map(|&c| c as f64).collect();
        out_ds_df
            .with_column(Series::new("_CLUSTER_".into(), cluster_col))
            .map_err(|e| SasError::runtime(format!("FASTCLUS OUT= build failed: {e}")))?;
        let mut vars = ds.vars.clone();
        vars.push(crate::dataset::VarMeta {
            name: "_CLUSTER_".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
            informat: None,
        });
        let out_ds = SasDataset {
            df: out_ds_df,
            vars,
        };
        let out_libref = out_ref.libref_or_work();
        let out_table = out_ref.name.to_uppercase();
        let out_display = format!("{out_libref}.{out_table}");
        let n_rows = out_ds.n_obs();
        let n_vars = out_ds.vars.len();
        session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
        session.last_dataset = Some(out_display.clone());
        session.log.note(&format!(
            "The data set {} has {} observations and {} variables.",
            out_display, n_rows, n_vars
        ));
    }

    Ok(())
}

use crate::dataset::SasDataset;

use crate::procs::common::centered;

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod tests;
