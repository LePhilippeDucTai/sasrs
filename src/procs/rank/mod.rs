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
//!
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

mod compute;

use compute::*;

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
                _ => return Err(SasError::parse("expected a number after GROUPS=", tok.span)),
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

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &RankAst, session: &mut Session) -> Result<()> {
    let in_ref = common::resolve_last_dataset(&ast.data, session)?;
    let ds = common::open_resolved(&in_ref, session)?;

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
    let in_display = in_ref.display();
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

#[cfg(test)]
mod tests;
