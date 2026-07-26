//! PROC DISTANCE — distance/dissimilarity matrix between observations (M27).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc distance data=<ref> out=<ref> [method=euclid|L2|cityblock|L1|Linf|
//!  Chebychev|cosine|corr]; var <name list>; run;`
//!
//! ## Périmètre
//! - `data=`, `out=` (matrice stockée), `method=` (défaut EUCLID).
//! - `var` : variables numériques formant les coordonnées de chaque obs.
//! - `out=` absent : NOTE "No output dataset specified ..." + listing affiché.
//! - Différé : `SHAPE=`, `FREQ`, normalisation, `id=`.
//!
//! ## Sortie
//! - Listing : matrice n×n (n = nombre d'observations), 4 décimales, lignes/
//!   colonnes Row<i>/Col<j>.
//! - `out=` : dataset avec `_TYPE_`="DISTANCE", `_NAME_`=Row<i>, puis Col1..Coln.

// MQ7.2c — `needless_range_loop` assumé dans ce module : l'indice EST le
// langage du domaine (`a[i][j] * b[j][k]`, parcours colonne-major, triangle
// d'une matrice symétrique). La forme itérateur y coûte plus en lisibilité
// qu'elle n'en rend, et la revue a préféré garder les indices explicites.
#![allow(clippy::needless_range_loop)]

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::decode_column;
use crate::procs::common::{char_var_meta, num_var_meta};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::VarType;
use polars::prelude::*;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistMethod {
    Euclid,
    CityBlock,
    Chebychev,
    Cosine,
    Corr,
}

impl DistMethod {
    fn title(self) -> &'static str {
        match self {
            DistMethod::Euclid => "Euclidean",
            DistMethod::CityBlock => "City Block (L1)",
            DistMethod::Chebychev => "Chebychev (Linf)",
            DistMethod::Cosine => "Cosine",
            DistMethod::Corr => "Correlation",
        }
    }
}

pub struct DistanceAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub method: DistMethod,
    pub var: Vec<String>,
}

// ───────────────────────── Parser ─────────────────────────

fn parse_method(ts: &mut StatementStream) -> Result<DistMethod> {
    let span = ts.peek().span;
    let name = ts
        .peek()
        .ident()
        .map(str::to_string)
        .ok_or_else(|| SasError::parse("expected a method name after METHOD=", span))?;
    ts.next();
    match name.to_ascii_lowercase().as_str() {
        "euclid" | "euclidean" | "l2" => Ok(DistMethod::Euclid),
        "cityblock" | "l1" => Ok(DistMethod::CityBlock),
        "linf" | "chebychev" | "chebyshev" => Ok(DistMethod::Chebychev),
        "cosine" => Ok(DistMethod::Cosine),
        "corr" | "correlation" => Ok(DistMethod::Corr),
        other => Err(SasError::parse(
            format!(
                "Unknown METHOD= value '{}' on PROC DISTANCE.",
                other.to_uppercase()
            ),
            span,
        )),
    }
}

/// Parse `proc distance [data=a] [out=b] [method=m]; [var ...;] run;`.
/// Called AFTER "proc distance" has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<DistanceAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut method = DistMethod::Euclid;

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
        } else if ts.peek().is_kw("method") {
            common::consume_option_eq(ts, "METHOD")?;
            method = parse_method(ts)?;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC DISTANCE statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC DISTANCE statement.",
                span,
            ));
        }
    }

    let mut var: Vec<String> = Vec::new();
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next();
                var = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            _ => false,
        })
    })?;

    Ok(DistanceAst {
        data,
        out,
        method,
        var,
    })
}

/// Distance between two coordinate vectors under the given method.
pub fn distance(method: DistMethod, a: &[f64], b: &[f64]) -> f64 {
    match method {
        DistMethod::Euclid => a
            .iter()
            .zip(b)
            .map(|(x, y)| (x - y) * (x - y))
            .sum::<f64>()
            .sqrt(),
        DistMethod::CityBlock => a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum(),
        DistMethod::Chebychev => a
            .iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0_f64, f64::max),
        DistMethod::Cosine => {
            let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            let na: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
            let nb: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
            if na == 0.0 || nb == 0.0 {
                0.0
            } else {
                1.0 - dot / (na * nb)
            }
        }
        DistMethod::Corr => {
            let p = a.len() as f64;
            if p < 2.0 {
                return 0.0;
            }
            let ma = a.iter().sum::<f64>() / p;
            let mb = b.iter().sum::<f64>() / p;
            let mut sab = 0.0;
            let mut saa = 0.0;
            let mut sbb = 0.0;
            for (x, y) in a.iter().zip(b) {
                let dx = x - ma;
                let dy = y - mb;
                sab += dx * dy;
                saa += dx * dx;
                sbb += dy * dy;
            }
            if saa == 0.0 || sbb == 0.0 {
                0.0
            } else {
                1.0 - sab / (saa.sqrt() * sbb.sqrt())
            }
        }
    }
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &DistanceAst, session: &mut Session) -> Result<()> {
    if ast.var.is_empty() {
        return Err(SasError::runtime("PROC DISTANCE requires a VAR statement."));
    }

    let (ds, display) = common::open_input_display(&ast.data, session)?;
    let n_read = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_read, display
    ));

    // Resolve VAR columns (numeric only).
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

    // One coordinate vector per observation.
    let coords: Vec<Vec<f64>> = (0..n_read)
        .map(|r| decoded.iter().map(|col| col[r]).collect())
        .collect();
    let n = coords.len();

    // Symmetric n×n distance matrix.
    let mut dist = vec![vec![0.0_f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let d = distance(ast.method, &coords[i], &coords[j]);
            dist[i][j] = d;
            dist[j][i] = d;
        }
    }

    // ───────────────────────── listing ─────────────────────────
    session.listing.page_header();
    centered(session, "The DISTANCE Procedure");
    session.listing.blank();
    centered(session, &format!("Distance Method: {}", ast.method.title()));
    session.listing.blank();
    centered(session, &format!("N = {}    Variables = {}", n, p));
    session.listing.blank();
    centered(session, "Distance Matrix");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for j in 0..n {
            headers.push(format!("Col{}", j + 1));
            aligns.push(Align::Right);
        }
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(n);
        for (i, dist_row) in dist.iter().enumerate().take(n) {
            let mut row = vec![format!("Row{}", i + 1)];
            for d in dist_row.iter().take(n) {
                row.push(format!("{d:.4}"));
            }
            rows.push(row);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // ───────────────────────── out= ─────────────────────────
    match &ast.out {
        None => {
            session
                .log
                .note("No output dataset specified for PROC DISTANCE, results not stored.");
        }
        Some(out_ref) => {
            let mut columns: Vec<Column> = Vec::with_capacity(n + 2);
            let mut vars: Vec<crate::dataset::VarMeta> = Vec::with_capacity(n + 2);

            // _TYPE_ : "DISTANCE" for every row.
            let type_vals: Vec<&str> = vec!["DISTANCE"; n];
            columns.push(Series::new("_TYPE_".into(), type_vals).into());
            vars.push(char_var_meta("_TYPE_", "DISTANCE".len()));

            // _NAME_ : Row<i>.
            let name_vals: Vec<String> = (0..n).map(|i| format!("Row{}", i + 1)).collect();
            // `.max(1)` : une longueur char SAS vaut au moins 1, même si toutes
            // les valeurs sont vides (garde portée par l'ancien helper local).
            let name_len = name_vals.iter().map(|s| s.len()).max().unwrap_or(1).max(1);
            columns.push(
                Series::new(
                    "_NAME_".into(),
                    name_vals.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                )
                .into(),
            );
            vars.push(char_var_meta("_NAME_", name_len));

            // Col1..Coln : distances.
            for j in 0..n {
                let col_name = format!("Col{}", j + 1);
                let vals: Vec<f64> = (0..n).map(|i| dist[i][j]).collect();
                columns.push(Series::new(col_name.as_str().into(), vals).into());
                vars.push(num_var_meta(&col_name));
            }

            let df = DataFrame::new(columns)?;
            let out_ds = SasDataset { df, vars };

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
    }

    Ok(())
}

use crate::dataset::SasDataset;

use crate::procs::common::centered;

#[cfg(test)]
mod tests;
