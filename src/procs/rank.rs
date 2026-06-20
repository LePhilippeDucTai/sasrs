//! PROC RANK — compute ranks (or group numbers) of numeric variables (v1).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc rank data=<ref> [out=<ref>] [descending]
//!  [ties=mean|low|high|dense] [groups=<n>] ;
//!  var <list>; [ranks <list>;] run;`
//!
//! ## Périmètre v1 (fidèle à SAS 9.4 PROC RANK)
//! Options du statement PROC :
//! - `data=`  : dataset d'entrée (défaut = `_LAST_`).
//! - `out=`   : dataset de sortie. ABSENT → SAS réécrit le dataset d'entrée
//!   (on réplique : on écrase l'entrée). Documenté.
//! - `descending` : inverse l'ordre (plus grande valeur → rang 1).
//! - `ties=` : MEAN (défaut), LOW, HIGH, DENSE. Voir « TIES » plus bas.
//! - `groups=<n>` : partitionne les valeurs non-missing en n groupes
//!   numérotés 0..n-1. Voir « GROUPS » plus bas.
//!
//! Sous-statements :
//! - `var <list>` : variables à classer. Si absent, SAS classe TOUTES les
//!   variables numériques (on implémente ce défaut).
//! - `ranks <list>` : noms des variables de rang en sortie, appariés
//!   POSITIONNELLEMENT à VAR. Si RANKS absent → le rang REMPLACE la variable
//!   d'origine. Si présent → les variables d'origine sont conservées et de
//!   nouvelles colonnes de rang sont ajoutées avec les noms RANKS. Si la
//!   longueur de RANKS != longueur de VAR → erreur claire.
//!
//! ## TIES (calcul)
//! Sur les valeurs non-missing triées (ascendant, ou descendant si
//! DESCENDING), un groupe de valeurs égales (au sens `Value::sas_cmp`)
//! occupe les positions ordinales 1-based `lo..=hi` :
//! - MEAN : `(lo + hi) / 2` (moyenne des rangs occupés ; rangs fractionnaires).
//! - LOW  : `lo`.
//! - HIGH : `hi`.
//! - DENSE: indice de groupe d'égalité consécutif (1,2,2,3...), sans trou.
//!
//! ## GROUPS (formule)
//! Avec `groups=n`, la sortie est le NUMÉRO DE GROUPE (0..n-1), pas le rang.
//! On utilise la formule SAS : pour la r-ième valeur en rang ascendant
//! 1-based (r = nombre de valeurs non-missing strictement « avant » + 1,
//! les égalités partageant le même r — on utilise le rang LOW), avec
//! k = nombre de valeurs non-missing :
//!     group = floor(n * r / (k + 1))
//! borné à 0..n-1. Les valeurs égales reçoivent le même groupe (même r).
//! GROUPS= ignore TIES= (les ties partagent toujours le même groupe via r).
//! Documenté comme simplification.
//!
//! ## Missings
//! Le classement porte sur les valeurs NON-missing. Une valeur missing
//! (`.` null OU missing spécial `._`/`.A`..`.Z` — qui sont des NaN, donc
//! `value_to_num` les rend NaN) reçoit un RANG MISSING (`.`) et est exclue
//! du calcul (et de l'affectation de groupe). On suit `Value::sas_cmp` pour
//! l'ordre, donc la collation est identique à PROC SORT.
//!
//! ## Méthodes de rang (M21.5)
//! Par défaut PROC RANK émet le rang ordinaire (avec TIES). Les options de
//! méthode transforment ce rang ordinaire `r` (1-based, ajusté par TIES) sur
//! les `k` valeurs non-missing :
//! - `FRACTION` : `r / k`.
//! - `NPLUS1`   : `r / (k + 1)`.
//! - `PERCENT`  : `100 * r / k`.
//! - `NORMAL=BLOM|TUKEY|VW` : score normal `Φ⁻¹(y)` où
//!     - BLOM  : `y = (r - 3/8) / (k + 1/4)`
//!     - TUKEY : `y = (r - 1/3) / (k + 1/3)`
//!     - VW    : `y = r / (k + 1)`
//! - `SAVAGE` : score exponentiel (Savage). Pour l'ordinal `m` (1..=k),
//!   `s_m = (Σ_{j=k-m+1}^{k} 1/j) − 1`. Les ties reçoivent l'agrégat de leurs
//!   scores ordinaux selon TIES (MEAN → moyenne, LOW → premier, HIGH →
//!   dernier, DENSE → score de l'ordinal LOW du groupe d'égalité).
//! GROUPS= a priorité sur les méthodes (émet le numéro de groupe). Les méthodes
//! sont mutuellement exclusives (deux options → erreur claire).
//!
//! ## BY (M21.5)
//! `by [descending] v1 ... ;` : l'entrée doit être triée par les clés BY
//! (vérifié via `common::by_groups`, `sas_cmp`, sinon erreur « not sorted »).
//! Les rangs/scores sont recalculés INDÉPENDAMMENT dans chaque groupe BY ; la
//! sortie concatène les groupes dans l'ordre d'entrée (groupes contigus).
//!
//! ## Choix / simplifications documentés (pour l'orchestrateur)
//! - Les colonnes de rang/groupe sont numériques (f64) ; rang missing =
//!   `Value::missing()` → null. Les colonnes pass-through sont recopiées
//!   telles quelles (la série Polars d'origine est conservée, donc les
//!   payloads de missings spéciaux sont préservés bit à bit).
//! - Variance nulle / `k = 0` : pas de panic ; toute valeur non calculable
//!   reste missing.

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, by_groups, decode_column, phi_inv, resolve_by_cols};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType};
use polars::prelude::*;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ties {
    Mean,
    Low,
    High,
    Dense,
}

/// NORMAL= score formula variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalScore {
    Blom,
    Tukey,
    Vw,
}

/// Ranking method (transformation applied to the TIES-adjusted ordinary rank).
/// `GROUPS=` is handled separately and takes priority over any method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Ordinary rank (default).
    Rank,
    /// `r / k`.
    Fraction,
    /// `r / (k + 1)`.
    NPlus1,
    /// `100 * r / k`.
    Percent,
    /// Savage (exponential) scores.
    Savage,
    /// Normal scores `Φ⁻¹(y)`.
    Normal(NormalScore),
}

pub struct RankAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub descending: bool,
    pub ties: Ties,
    pub groups: Option<usize>,
    /// Ranking method (default = ordinary rank). Ignored when `groups` is set.
    pub method: Method,
    /// BY variables (var, descending). Empty = no BY grouping.
    pub by: Vec<(String, bool)>,
    /// Explicit VAR list (empty = default to all numeric variables).
    pub var: Vec<String>,
    /// Optional RANKS list (empty = none → ranks replace the originals).
    pub ranks: Vec<String>,
}

/// Parse `proc rank [data=a] [out=b] [descending] [ties=...] [groups=n];
/// [var ...;] [ranks ...;] ... run;`. Called AFTER "proc rank" was consumed.
/// Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<RankAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut descending = false;
    let mut ties = Ties::Mean;
    let mut groups: Option<usize> = None;
    let mut method = Method::Rank;

    // Set the method, rejecting a second mutually-exclusive method option.
    let set_method = |m: Method, cur: &mut Method| -> Result<()> {
        if *cur != Method::Rank {
            return Err(SasError::runtime(
                "Only one ranking-method option (FRACTION, PERCENT, NORMAL=, \
                 SAVAGE or NPLUS1) may be specified on PROC RANK.",
            ));
        }
        *cur = m;
        Ok(())
    };

    // --- PROC RANK statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            common::expect_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("out") {
            common::expect_eq(ts, "OUT")?;
            out = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("descending") {
            ts.next();
            descending = true;
        } else if ts.peek().is_kw("ties") {
            common::expect_eq(ts, "TIES")?;
            let tok = ts.peek().clone();
            let name = tok.ident().ok_or_else(|| {
                SasError::parse("expected a TIES= method (MEAN|LOW|HIGH|DENSE)", tok.span)
            })?;
            ties = match name.to_ascii_lowercase().as_str() {
                "mean" => Ties::Mean,
                "low" => Ties::Low,
                "high" => Ties::High,
                "dense" => Ties::Dense,
                other => {
                    return Err(SasError::parse(
                        format!(
                            "Unknown TIES= method '{}' (expected MEAN, LOW, HIGH or DENSE).",
                            other.to_uppercase()
                        ),
                        tok.span,
                    ));
                }
            };
            ts.next();
        } else if ts.peek().is_kw("groups") {
            common::expect_eq(ts, "GROUPS")?;
            let tok = ts.peek().clone();
            let n = match &tok.kind {
                TokenKind::Num(v) => *v,
                _ => {
                    return Err(SasError::parse(
                        "expected a number after GROUPS=",
                        tok.span,
                    ))
                }
            };
            if n < 1.0 || n.fract() != 0.0 {
                return Err(SasError::runtime(
                    "The GROUPS= value must be a positive integer.",
                ));
            }
            groups = Some(n as usize);
            ts.next();
        } else if ts.peek().is_kw("fraction") {
            ts.next();
            set_method(Method::Fraction, &mut method)?;
        } else if ts.peek().is_kw("nplus1") {
            ts.next();
            set_method(Method::NPlus1, &mut method)?;
        } else if ts.peek().is_kw("percent") {
            ts.next();
            set_method(Method::Percent, &mut method)?;
        } else if ts.peek().is_kw("savage") {
            ts.next();
            set_method(Method::Savage, &mut method)?;
        } else if ts.peek().is_kw("normal") {
            common::expect_eq(ts, "NORMAL")?;
            let tok = ts.peek().clone();
            let name = tok.ident().ok_or_else(|| {
                SasError::parse("expected a NORMAL= method (BLOM|TUKEY|VW)", tok.span)
            })?;
            let score = match name.to_ascii_lowercase().as_str() {
                "blom" => NormalScore::Blom,
                "tukey" => NormalScore::Tukey,
                "vw" => NormalScore::Vw,
                other => {
                    return Err(SasError::parse(
                        format!(
                            "Unknown NORMAL= method '{}' (expected BLOM, TUKEY or VW).",
                            other.to_uppercase()
                        ),
                        tok.span,
                    ));
                }
            };
            ts.next();
            set_method(Method::Normal(score), &mut method)?;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC RANK statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC RANK statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut var: Vec<String> = Vec::new();
    let mut ranks: Vec<String> = Vec::new();
    let mut by: Vec<(String, bool)> = Vec::new();

    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next();
                var = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            "ranks" => {
                ts.next();
                ranks = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            "by" => {
                ts.next();
                by = crate::procs::means::parse_by_list(ts)?;
                true
            }
            _ => false,
        })
    })?;

    Ok(RankAst {
        data,
        out,
        descending,
        ties,
        groups,
        method,
        by,
        var,
        ranks,
    })
}

// ───────────────────────── ranking core ─────────────────────────

/// Compute the rank/group/score output for one decoded column.
///
/// Returns a vector of `Value` aligned to the input rows: `Value::Num(..)`
/// for non-missing input cells, `Value::missing()` for missing cells.
///
/// `groups` (GROUPS=) takes priority over `method`; otherwise the TIES-adjusted
/// ordinary rank is transformed per `method`.
fn rank_column(
    col: &[Value],
    descending: bool,
    ties: Ties,
    groups: Option<usize>,
    method: Method,
) -> Vec<Value> {
    let n = col.len();

    // Indices of non-missing cells (special missings are NaN via value_to_num).
    let mut idx: Vec<usize> = Vec::with_capacity(n);
    for (i, v) in col.iter().enumerate() {
        match value_to_num(v) {
            Some(f) if !f.is_nan() => idx.push(i),
            _ => {}
        }
    }
    let k = idx.len();

    // Stable sort the non-missing indices via sas_cmp (DESCENDING reverses).
    idx.sort_by(|&a, &b| {
        let c = col[a].sas_cmp(&col[b]);
        if descending {
            c.reverse()
        } else {
            c
        }
    });

    // Output buffer; missing cells stay missing.
    let mut out = vec![Value::missing(); n];
    if k == 0 {
        return out;
    }

    // SAVAGE needs the cumulative reverse-harmonic per ordinal. Precompute
    // s_m = (sum_{j=k-m+1}^{k} 1/j) - 1 for m = 1..=k.
    let savage = matches!(method, Method::Savage);
    let savage_scores: Vec<f64> = if savage {
        let mut acc = 0.0;
        let mut v = Vec::with_capacity(k);
        // m=1 adds 1/k, m=2 adds 1/(k-1), ... m=k adds 1/1.
        for m in 1..=k {
            acc += 1.0 / (k - m + 1) as f64;
            v.push(acc - 1.0);
        }
        v
    } else {
        Vec::new()
    };

    // Walk the sorted order, grouping consecutive equal values (sas_cmp).
    // For each tie group occupying ordinal positions lo..=hi (1-based), assign
    // a group number (GROUPS=) or the (possibly transformed) rank/score.
    let mut pos = 0usize; // 0-based offset into `idx`
    let mut dense_rank = 0usize; // consecutive distinct-value counter (DENSE)
    while pos < k {
        let mut end = pos + 1;
        while end < k && col[idx[end]].sas_cmp(&col[idx[pos]]) == Ordering::Equal {
            end += 1;
        }
        let lo = pos + 1; // 1-based first ordinal of the tie group
        let hi = end; // 1-based last ordinal of the tie group
        dense_rank += 1;

        let value = match groups {
            Some(ng) => {
                // SAS group formula on the LOW ordinal rank (ties share it):
                // group = floor(n_groups * r / (k + 1)), clamped to 0..n-1.
                let r = lo;
                let g = (ng * r) / (k + 1);
                let g = g.min(ng - 1);
                g as f64
            }
            None if savage => {
                // Savage scores: aggregate the per-ordinal scores over the tie
                // group according to TIES (MEAN → average, LOW/HIGH → endpoint
                // ordinal's score, DENSE → LOW ordinal's score).
                match ties {
                    Ties::Mean => {
                        let sum: f64 = savage_scores[pos..end].iter().sum();
                        sum / (end - pos) as f64
                    }
                    Ties::Low | Ties::Dense => savage_scores[lo - 1],
                    Ties::High => savage_scores[hi - 1],
                }
            }
            None => {
                // TIES-adjusted ordinary rank, then the method transform.
                let r = match ties {
                    Ties::Mean => (lo + hi) as f64 / 2.0,
                    Ties::Low => lo as f64,
                    Ties::High => hi as f64,
                    Ties::Dense => dense_rank as f64,
                };
                transform_rank(r, k, method)
            }
        };

        for &orig in &idx[pos..end] {
            out[orig] = Value::Num(value);
        }
        pos = end;
    }

    out
}

/// Transform a TIES-adjusted ordinary rank `r` (over `k` non-missing values)
/// per the ranking `method`. SAVAGE is handled in `rank_column` (it needs the
/// ordinal, not just `r`); this covers RANK/FRACTION/NPLUS1/PERCENT/NORMAL.
fn transform_rank(r: f64, k: usize, method: Method) -> f64 {
    let kf = k as f64;
    match method {
        Method::Rank => r,
        Method::Fraction => r / kf,
        Method::NPlus1 => r / (kf + 1.0),
        Method::Percent => 100.0 * r / kf,
        Method::Normal(score) => {
            let y = match score {
                NormalScore::Blom => (r - 0.375) / (kf + 0.25),
                NormalScore::Tukey => (r - 1.0 / 3.0) / (kf + 1.0 / 3.0),
                NormalScore::Vw => r / (kf + 1.0),
            };
            phi_inv(y)
        }
        // SAVAGE never reaches here (handled in rank_column).
        Method::Savage => r,
    }
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &RankAst, session: &mut Session) -> Result<()> {
    let in_ref = common::resolve_last_dataset(&ast.data, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    // Resolve VAR list: explicit, else all numeric vars in dataset order.
    let var_cols: Vec<usize> = if !ast.var.is_empty() {
        let mut out = Vec::with_capacity(ast.var.len());
        for nm in &ast.var {
            match ds.vars.iter().position(|m| m.name.eq_ignore_ascii_case(nm)) {
                Some(i) => {
                    if ds.vars[i].ty != VarType::Num {
                        return Err(SasError::runtime(format!(
                            "Variable {} in the VAR list is not numeric.",
                            nm.to_uppercase()
                        )));
                    }
                    out.push(i);
                }
                None => {
                    return Err(SasError::runtime(format!(
                        "Variable {} not found.",
                        nm.to_uppercase()
                    )));
                }
            }
        }
        out
    } else {
        (0..ds.vars.len())
            .filter(|&i| ds.vars[i].ty == VarType::Num)
            .collect()
    };

    if var_cols.is_empty() {
        return Err(SasError::runtime(
            "No numeric variables found for PROC RANK.",
        ));
    }

    // RANKS list: if present, must pair 1:1 with VAR.
    let use_ranks = !ast.ranks.is_empty();
    if use_ranks && ast.ranks.len() != var_cols.len() {
        return Err(SasError::runtime(format!(
            "The RANKS list has {} names but the VAR list has {} variables.",
            ast.ranks.len(),
            var_cols.len()
        )));
    }

    // Resolve BY columns and partition rows into contiguous BY groups (the
    // input must be sorted by the BY key). Without BY → a single group of all
    // rows in input order.
    let n_obs = ds.n_obs();
    let in_display = format!("{in_libref}.{in_table}");
    let by_cols = resolve_by_cols(&ds, &ast.by)?;
    let groups_rows: Vec<Vec<usize>> = if by_cols.is_empty() {
        vec![(0..n_obs).collect()]
    } else {
        let by_values: Vec<Vec<Value>> = by_cols
            .iter()
            .map(|c| decode_column(&ds, c.col_idx))
            .collect::<Result<_>>()?;
        let descending: Vec<bool> = by_cols.iter().map(|c| c.descending).collect();
        let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();
        by_groups(&by_values, &descending, n_obs, &by_names, &in_display)?
            .into_iter()
            .map(|(_key, rows)| rows)
            .collect()
    };

    // Compute the rank output for each VAR column (decode each ONCE), ranking
    // INDEPENDENTLY within each BY group and scattering back into row order.
    let mut rank_values: Vec<Vec<Value>> = Vec::with_capacity(var_cols.len());
    for &ci in &var_cols {
        let col = decode_column(&ds, ci)?;
        let mut out = vec![Value::missing(); n_obs];
        for rows in &groups_rows {
            let sub: Vec<Value> = rows.iter().map(|&r| col[r].clone()).collect();
            let ranked = rank_column(&sub, ast.descending, ast.ties, ast.groups, ast.method);
            for (j, &r) in rows.iter().enumerate() {
                out[r] = ranked[j].clone();
            }
        }
        rank_values.push(out);
    }

    // Build the output dataset. Preserve every input column and its order; the
    // only changes are: ranked columns replaced in place (no RANKS), or new
    // rank columns appended (RANKS). Pass-through columns keep their original
    // Polars series verbatim (special-missing payloads preserved).
    let mut columns: Vec<Column> = Vec::with_capacity(ds.vars.len() + ast.ranks.len());
    let mut vars: Vec<VarMeta> = Vec::with_capacity(ds.vars.len() + ast.ranks.len());

    // Map each input column index → its position in var_cols (if ranked).
    let ranked_pos = |ci: usize| -> Option<usize> { var_cols.iter().position(|&c| c == ci) };

    for ci in 0..ds.vars.len() {
        match ranked_pos(ci) {
            Some(vp) if !use_ranks => {
                // Replace this column's data with the computed ranks; keep the
                // original name and VarMeta (numeric).
                let name = ds.vars[ci].name.clone();
                let series = rank_series(&name, &rank_values[vp], n_obs);
                columns.push(series.into());
                vars.push(num_var_meta(&name));
            }
            _ => {
                // Pass-through: keep the original column verbatim.
                columns.push(ds.df.get_columns()[ci].clone());
                vars.push(ds.vars[ci].clone());
            }
        }
    }

    // Append new rank columns when RANKS= was given.
    if use_ranks {
        for (vp, rname) in ast.ranks.iter().enumerate() {
            let series = rank_series(rname, &rank_values[vp], n_obs);
            columns.push(series.into());
            vars.push(num_var_meta(rname));
        }
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    // OUT= destination; absent → overwrite the input dataset (SAS behavior).
    let out_ref = ast.out.clone().unwrap_or_else(|| in_ref.clone());
    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
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

/// Build an f64 Polars series from rank `Value`s (missing → null/NaN-payload
/// via `value_to_num`).
fn rank_series(name: &str, values: &[Value], n_obs: usize) -> Series {
    debug_assert_eq!(values.len(), n_obs);
    let data: Vec<Option<f64>> = values.iter().map(value_to_num).collect();
    Series::new(name.into(), data)
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

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    fn read_num_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
        let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
        let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
        decode_column(&ds, idx).unwrap()
    }

    fn parse_rank(src: &str) -> Result<RankAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "rank"
        parse(&mut ts)
    }

    fn dref(table: &str) -> DatasetRef {
        DatasetRef {
            libref: Some("WORK".into()),
            name: table.into(),
        }
    }

    // ───────────── parse tests ─────────────

    #[test]
    fn parse_minimal() {
        let ast = parse_rank("proc rank data=a; var x; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert!(ast.out.is_none());
        assert!(!ast.descending);
        assert_eq!(ast.ties, Ties::Mean);
        assert!(ast.groups.is_none());
        assert_eq!(ast.var, vec!["x"]);
        assert!(ast.ranks.is_empty());
    }

    #[test]
    fn parse_all_options() {
        let ast = parse_rank(
            "proc rank data=lib.a out=work.b descending ties=high groups=4; var x y; ranks rx ry; run;",
        )
        .unwrap();
        assert_eq!(ast.data.as_ref().unwrap().libref.as_deref(), Some("lib"));
        assert_eq!(ast.out.as_ref().unwrap().name, "b");
        assert!(ast.descending);
        assert_eq!(ast.ties, Ties::High);
        assert_eq!(ast.groups, Some(4));
        assert_eq!(ast.var, vec!["x", "y"]);
        assert_eq!(ast.ranks, vec!["rx", "ry"]);
    }

    #[test]
    fn parse_ties_variants() {
        assert_eq!(parse_rank("proc rank ties=mean; var x; run;").unwrap().ties, Ties::Mean);
        assert_eq!(parse_rank("proc rank ties=low; var x; run;").unwrap().ties, Ties::Low);
        assert_eq!(parse_rank("proc rank ties=high; var x; run;").unwrap().ties, Ties::High);
        assert_eq!(parse_rank("proc rank ties=dense; var x; run;").unwrap().ties, Ties::Dense);
    }

    #[test]
    fn parse_unknown_ties_errors() {
        let r = parse_rank("proc rank ties=bogus; var x; run;");
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("BOGUS"));
    }

    // NB : les méthodes FRACTION/PERCENT/NORMAL/SAVAGE et le statement BY sont
    // désormais implémentés (M21.5) — voir `parse_method_options`,
    // `parse_by_now_supported`, `method_*` et `execute_by_*`. Les anciens tests
    // « not yet implemented » ont été retirés en conséquence.

    #[test]
    fn parse_unknown_option_errors() {
        let r = parse_rank("proc rank data=a bogus; var x; run;");
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("BOGUS"));
    }

    // ───────────── ranking core tests ─────────────

    fn nums(xs: &[f64]) -> Vec<Value> {
        xs.iter().map(|v| Value::Num(*v)).collect()
    }

    #[test]
    fn rank_basic_ascending() {
        let out = rank_column(&nums(&[30.0, 10.0, 20.0]), false, Ties::Mean, None, Method::Rank);
        assert_eq!(out, nums(&[3.0, 1.0, 2.0]));
    }

    #[test]
    fn rank_descending() {
        let out = rank_column(&nums(&[30.0, 10.0, 20.0]), true, Ties::Mean, None, Method::Rank);
        // 30 largest → rank 1.
        assert_eq!(out, nums(&[1.0, 3.0, 2.0]));
    }

    #[test]
    fn rank_ties_all_variants() {
        let data = nums(&[10.0, 20.0, 20.0, 40.0]);
        assert_eq!(
            rank_column(&data, false, Ties::Mean, None, Method::Rank),
            nums(&[1.0, 2.5, 2.5, 4.0])
        );
        assert_eq!(
            rank_column(&data, false, Ties::Low, None, Method::Rank),
            nums(&[1.0, 2.0, 2.0, 4.0])
        );
        assert_eq!(
            rank_column(&data, false, Ties::High, None, Method::Rank),
            nums(&[1.0, 3.0, 3.0, 4.0])
        );
        assert_eq!(
            rank_column(&data, false, Ties::Dense, None, Method::Rank),
            nums(&[1.0, 2.0, 2.0, 3.0])
        );
    }

    #[test]
    fn rank_missing_excluded() {
        let data = vec![
            Value::Num(10.0),
            Value::missing(),
            Value::Num(30.0),
            Value::Num(20.0),
        ];
        let out = rank_column(&data, false, Ties::Mean, None, Method::Rank);
        assert_eq!(out[0], Value::Num(1.0));
        assert!(out[1].is_missing());
        assert_eq!(out[2], Value::Num(3.0));
        assert_eq!(out[3], Value::Num(2.0));
    }

    #[test]
    fn rank_groups_partition() {
        // 10 distinct values, groups=4 → group = floor(4*r/11), r=1..10.
        let data = nums(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
        let out = rank_column(&data, false, Ties::Mean, Some(4), Method::Rank);
        let expected: Vec<f64> = (1..=10).map(|r| ((4 * r) / 11).min(3) as f64).collect();
        assert_eq!(out, nums(&expected));
        // sanity: groups are within 0..3.
        for v in &out {
            if let Value::Num(g) = v {
                assert!((0.0..=3.0).contains(g));
            }
        }
    }

    #[test]
    fn rank_groups_ties_share_group() {
        // Tied values must land in the same group (same LOW ordinal r).
        let data = nums(&[10.0, 20.0, 20.0, 40.0]);
        let out = rank_column(&data, false, Ties::Mean, Some(2), Method::Rank);
        // r for the two 20s is 2 (LOW), so both share the same group.
        assert_eq!(out[1], out[2]);
    }

    // ───────────── execute tests ─────────────

    #[test]
    fn execute_replace_no_ranks() {
        let mut session = make_session();
        let df = df!["x" => [30.0_f64, 10.0, 20.0], "g" => ["a", "b", "c"]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), char_meta("g")] };
        write_dataset(&mut session, "T", ds);

        let ast = RankAst {
            data: Some(dref("T")),
            out: None,
            descending: false,
            ties: Ties::Mean,
            groups: None,
            method: Method::Rank,
            by: vec![],
            var: vec!["x".into()],
            ranks: vec![],
        };
        execute(&ast, &mut session).unwrap();

        // x replaced with ranks; g unchanged; no new column.
        let (out, _) = session.libs.get("WORK").unwrap().read("T").unwrap();
        assert_eq!(out.vars.len(), 2);
        let x = read_num_col(&session, "T", "x");
        assert_eq!(x, nums(&[3.0, 1.0, 2.0]));
        let g: Vec<String> = out
            .df
            .column("g")
            .unwrap()
            .str()
            .unwrap()
            .iter()
            .map(|o| o.unwrap().to_string())
            .collect();
        assert_eq!(g, vec!["a", "b", "c"]);
    }

    #[test]
    fn execute_ranks_appends_new_columns() {
        let mut session = make_session();
        let df = df!["x" => [30.0_f64, 10.0, 20.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = RankAst {
            data: Some(dref("T")),
            out: Some(dref("O")),
            descending: false,
            ties: Ties::Mean,
            groups: None,
            method: Method::Rank,
            by: vec![],
            var: vec!["x".into()],
            ranks: vec!["rx".into()],
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        // Original x preserved + new rx appended.
        assert_eq!(out.vars.len(), 2);
        let x = read_num_col(&session, "O", "x");
        assert_eq!(x, nums(&[30.0, 10.0, 20.0]));
        let rx = read_num_col(&session, "O", "rx");
        assert_eq!(rx, nums(&[3.0, 1.0, 2.0]));
    }

    #[test]
    fn execute_ranks_length_mismatch_errors() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0], "y" => [3.0_f64, 4.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
        write_dataset(&mut session, "T", ds);

        let ast = RankAst {
            data: Some(dref("T")),
            out: None,
            descending: false,
            ties: Ties::Mean,
            groups: None,
            method: Method::Rank,
            by: vec![],
            var: vec!["x".into(), "y".into()],
            ranks: vec!["rx".into()], // only one name for two vars
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("RANKS"));
    }

    #[test]
    fn execute_default_var_all_numeric() {
        let mut session = make_session();
        let df = df![
            "x" => [30.0_f64, 10.0, 20.0],
            "g" => ["a", "b", "c"],
            "y" => [1.0_f64, 3.0, 2.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), char_meta("g"), num_meta("y")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = RankAst {
            data: Some(dref("T")),
            out: None,
            descending: false,
            ties: Ties::Mean,
            groups: None,
            method: Method::Rank,
            by: vec![],
            var: vec![], // default: all numerics (x, y), not g
            ranks: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let x = read_num_col(&session, "T", "x");
        assert_eq!(x, nums(&[3.0, 1.0, 2.0]));
        let y = read_num_col(&session, "T", "y");
        assert_eq!(y, nums(&[1.0, 3.0, 2.0]));
    }

    #[test]
    fn execute_missing_rank_and_note() {
        let mut session = make_session();
        let df = df!["x" => [Some(10.0_f64), None, Some(30.0), Some(20.0)]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = RankAst {
            data: Some(dref("T")),
            out: None,
            descending: false,
            ties: Ties::Mean,
            groups: None,
            method: Method::Rank,
            by: vec![],
            var: vec!["x".into()],
            ranks: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let x = read_num_col(&session, "T", "x");
        assert_eq!(x[0], Value::Num(1.0));
        assert!(x[1].is_missing());
        assert_eq!(x[2], Value::Num(3.0));
        assert_eq!(x[3], Value::Num(2.0));

        let log = session.log.into_string();
        assert!(
            log.contains("The data set WORK.T has 4 observations and 1 variables."),
            "log: {log}"
        );
    }

    #[test]
    fn execute_groups_output() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = RankAst {
            data: Some(dref("T")),
            out: Some(dref("O")),
            descending: false,
            ties: Ties::Mean,
            groups: Some(4),
            method: Method::Rank,
            by: vec![],
            var: vec!["x".into()],
            ranks: vec!["grp".into()],
        };
        execute(&ast, &mut session).unwrap();

        let grp = read_num_col(&session, "O", "grp");
        let expected: Vec<f64> = (1..=10).map(|r| ((4 * r) / 11).min(3) as f64).collect();
        assert_eq!(grp, nums(&expected));
    }

    #[test]
    fn execute_out_omitted_overwrites_input() {
        let mut session = make_session();
        let df = df!["x" => [30.0_f64, 10.0, 20.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = RankAst {
            data: Some(dref("T")),
            out: None,
            descending: false,
            ties: Ties::Mean,
            groups: None,
            method: Method::Rank,
            by: vec![],
            var: vec!["x".into()],
            ranks: vec![],
        };
        execute(&ast, &mut session).unwrap();

        // Input WORK.T overwritten in place; last_dataset points at it.
        let x = read_num_col(&session, "T", "x");
        assert_eq!(x, nums(&[3.0, 1.0, 2.0]));
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.T"));
    }

    // ───────────── method parse tests ─────────────

    #[test]
    fn parse_method_options() {
        assert_eq!(parse_rank("proc rank fraction; var x; run;").unwrap().method, Method::Fraction);
        assert_eq!(parse_rank("proc rank nplus1; var x; run;").unwrap().method, Method::NPlus1);
        assert_eq!(parse_rank("proc rank percent; var x; run;").unwrap().method, Method::Percent);
        assert_eq!(parse_rank("proc rank savage; var x; run;").unwrap().method, Method::Savage);
        assert_eq!(
            parse_rank("proc rank normal=blom; var x; run;").unwrap().method,
            Method::Normal(NormalScore::Blom)
        );
        assert_eq!(
            parse_rank("proc rank normal=tukey; var x; run;").unwrap().method,
            Method::Normal(NormalScore::Tukey)
        );
        assert_eq!(
            parse_rank("proc rank normal=vw; var x; run;").unwrap().method,
            Method::Normal(NormalScore::Vw)
        );
    }

    #[test]
    fn parse_normal_requires_method() {
        assert!(parse_rank("proc rank normal=bogus; var x; run;").is_err());
        assert!(parse_rank("proc rank normal; var x; run;").is_err());
    }

    #[test]
    fn parse_two_methods_errors() {
        let r = parse_rank("proc rank fraction percent; var x; run;");
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("Only one ranking-method"));
    }

    #[test]
    fn parse_by_now_supported() {
        let ast = parse_rank("proc rank data=a; by g; var x; run;").unwrap();
        assert_eq!(ast.by, vec![("g".to_string(), false)]);
        let ast2 = parse_rank("proc rank data=a; by descending g h; var x; run;").unwrap();
        assert_eq!(ast2.by, vec![("g".to_string(), true), ("h".to_string(), false)]);
    }

    // ───────────── method core tests (hand-verified) ─────────────

    fn approx_eq(out: &[Value], exp: &[f64]) {
        assert_eq!(out.len(), exp.len());
        for (o, e) in out.iter().zip(exp) {
            match o {
                Value::Num(v) => assert!((v - e).abs() < 1e-9, "got {v}, want {e}"),
                _ => panic!("missing where {e} expected"),
            }
        }
    }

    #[test]
    fn method_fraction() {
        let data = nums(&[10.0, 20.0, 30.0, 40.0]);
        let out = rank_column(&data, false, Ties::Mean, None, Method::Fraction);
        approx_eq(&out, &[0.25, 0.50, 0.75, 1.00]);
    }

    #[test]
    fn method_nplus1() {
        let data = nums(&[10.0, 20.0, 30.0, 40.0]);
        let out = rank_column(&data, false, Ties::Mean, None, Method::NPlus1);
        approx_eq(&out, &[0.2, 0.4, 0.6, 0.8]);
    }

    #[test]
    fn method_percent() {
        let data = nums(&[10.0, 20.0, 30.0, 40.0]);
        let out = rank_column(&data, false, Ties::Mean, None, Method::Percent);
        approx_eq(&out, &[25.0, 50.0, 75.0, 100.0]);
    }

    #[test]
    fn method_normal_blom() {
        // y = (r - 0.375)/4.25 for r=1..4, then Phi^-1.
        let data = nums(&[10.0, 20.0, 30.0, 40.0]);
        let out = rank_column(&data, false, Ties::Mean, None, Method::Normal(NormalScore::Blom));
        let exp: Vec<f64> = (1..=4)
            .map(|r| phi_inv((r as f64 - 0.375) / 4.25))
            .collect();
        approx_eq(&out, &exp);
        // BLOM scores are antisymmetric for symmetric ranks: s1 = -s4, s2 = -s3.
        if let (Value::Num(a), Value::Num(d)) = (&out[0], &out[3]) {
            assert!((a + d).abs() < 1e-9);
        }
    }

    #[test]
    fn method_normal_vw() {
        // van der Waerden: y = r/(n+1) = r/5.
        let data = nums(&[10.0, 20.0, 30.0, 40.0]);
        let out = rank_column(&data, false, Ties::Mean, None, Method::Normal(NormalScore::Vw));
        let exp: Vec<f64> = (1..=4).map(|r| phi_inv(r as f64 / 5.0)).collect();
        approx_eq(&out, &exp);
    }

    #[test]
    fn method_savage_no_ties() {
        // n=4: s_m = (sum_{j=n-m+1}^{n} 1/j) - 1.
        let data = nums(&[10.0, 20.0, 30.0, 40.0]);
        let out = rank_column(&data, false, Ties::Mean, None, Method::Savage);
        let s1 = 1.0 / 4.0 - 1.0;
        let s2 = 1.0 / 4.0 + 1.0 / 3.0 - 1.0;
        let s3 = 1.0 / 4.0 + 1.0 / 3.0 + 1.0 / 2.0 - 1.0;
        let s4 = 1.0 / 4.0 + 1.0 / 3.0 + 1.0 / 2.0 + 1.0 - 1.0;
        approx_eq(&out, &[s1, s2, s3, s4]);
        // Savage scores sum to ~0.
        let sum: f64 = out.iter().map(|v| if let Value::Num(x) = v { *x } else { 0.0 }).sum();
        assert!(sum.abs() < 1e-9);
    }

    #[test]
    fn method_savage_ties_mean() {
        // Two tied 20s occupy ordinals 2 and 3; MEAN → average of s2 and s3.
        let data = nums(&[10.0, 20.0, 20.0, 40.0]);
        let out = rank_column(&data, false, Ties::Mean, None, Method::Savage);
        let s2 = 1.0 / 4.0 + 1.0 / 3.0 - 1.0;
        let s3 = 1.0 / 4.0 + 1.0 / 3.0 + 1.0 / 2.0 - 1.0;
        let mid = (s2 + s3) / 2.0;
        if let (Value::Num(a), Value::Num(b)) = (&out[1], &out[2]) {
            assert!((a - mid).abs() < 1e-9 && (b - mid).abs() < 1e-9);
        } else {
            panic!("ties not numeric");
        }
    }

    #[test]
    fn method_fraction_with_ties() {
        // Ties::Mean rank of two 20s is 2.5 → fraction 2.5/4 = 0.625.
        let data = nums(&[10.0, 20.0, 20.0, 40.0]);
        let out = rank_column(&data, false, Ties::Mean, None, Method::Fraction);
        approx_eq(&out, &[0.25, 0.625, 0.625, 1.0]);
    }

    #[test]
    fn method_empty_column_no_panic() {
        let data = vec![Value::missing(), Value::missing()];
        let out = rank_column(&data, false, Ties::Mean, None, Method::Fraction);
        assert!(out.iter().all(|v| v.is_missing()));
        let out2 = rank_column(&data, false, Ties::Mean, None, Method::Savage);
        assert!(out2.iter().all(|v| v.is_missing()));
    }

    // ───────────── BY execute tests (hand-verified) ─────────────

    #[test]
    fn execute_by_independent_groups() {
        let mut session = make_session();
        // Two BY groups (g=a: 10,30,20 ; g=b: 5,15). Ranks recomputed per group.
        let df = df![
            "g" => ["a", "a", "a", "b", "b"],
            "x" => [10.0_f64, 30.0, 20.0, 5.0, 15.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = RankAst {
            data: Some(dref("T")),
            out: Some(dref("O")),
            descending: false,
            ties: Ties::Mean,
            groups: None,
            method: Method::Rank,
            by: vec![("g".into(), false)],
            var: vec!["x".into()],
            ranks: vec!["rx".into()],
        };
        execute(&ast, &mut session).unwrap();

        let rx = read_num_col(&session, "O", "rx");
        // Group a: 10→1, 30→3, 20→2. Group b: 5→1, 15→2.
        assert_eq!(rx, nums(&[1.0, 3.0, 2.0, 1.0, 2.0]));
    }

    #[test]
    fn execute_by_fraction_per_group() {
        let mut session = make_session();
        // Group a has 2 obs, group b has 3 obs → different denominators.
        let df = df![
            "g" => ["a", "a", "b", "b", "b"],
            "x" => [10.0_f64, 20.0, 10.0, 20.0, 30.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = RankAst {
            data: Some(dref("T")),
            out: Some(dref("O")),
            descending: false,
            ties: Ties::Mean,
            groups: None,
            method: Method::Fraction,
            by: vec![("g".into(), false)],
            var: vec!["x".into()],
            ranks: vec!["fx".into()],
        };
        execute(&ast, &mut session).unwrap();

        let fx = read_num_col(&session, "O", "fx");
        // a: 1/2, 2/2 ; b: 1/3, 2/3, 3/3.
        approx_eq(&fx, &[0.5, 1.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]);
    }

    #[test]
    fn execute_by_not_sorted_errors() {
        let mut session = make_session();
        // BY key not sorted (a, b, a) → by_groups must error.
        let df = df![
            "g" => ["a", "b", "a"],
            "x" => [10.0_f64, 20.0, 30.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = RankAst {
            data: Some(dref("T")),
            out: None,
            descending: false,
            ties: Ties::Mean,
            groups: None,
            method: Method::Rank,
            by: vec![("g".into(), false)],
            var: vec!["x".into()],
            ranks: vec![],
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("not sorted"));
    }
}
