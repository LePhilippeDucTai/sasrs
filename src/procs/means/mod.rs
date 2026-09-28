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
//!
//! ## WEIGHT statement (jalon WEIGHT)
//! `weight <var>;` — une seule variable numérique. Quand elle est présente,
//! toutes les stats passent par `compute_weighted` (analogue pondéré de
//! `compute`). Le chemin non-pondéré reste BYTE-IDENTIQUE : `compute_weighted`
//! n'est appelé que si `ast.weight.is_some()`. Fonctionne avec CLASS et BY
//! (poids partitionnés par groupe), et OUTPUT OUT= utilise les stats pondérées.
//!
//! Formules pondérées (J03-P2) — w_i poids, x_i valeurs, W = Σw_i :
//!   SumWgt = Σw_i ; Sum = Σw_i x_i ; Mean = Σw_i x_i / Σw_i ;
//!   CSS_w = Σw_i(x_i−x̄_w)² ;
//!   Variance = CSS_w/(W−1) (VARDEF=DF, défaut) ou CSS_w/(W−Σw_i²/W)
//!   (VARDEF=WEIGHT) — voir `common::weighted_variance` ;
//!   Std = √Variance (n ≥ 2 requis, sinon missing) ;
//!   StdErr = Std/√(Σw_i) (SAS pondère l'erreur-type par √ΣW) ;
//!   CV = 100·Std/x̄_w ; Min/Max = min/max NON pondérés de x_i ; N = n ;
//!   NMiss = x manquants À POIDS VALIDE (un poids manquant ou ≤ 0 exclut
//!   l'observation de l'analyse, N et NMiss compris) ;
//!   MEDIAN / Pxx / QRANGE = quantiles pondérés Définition 5
//!   (`common::weighted_quantile_def5`, partagé avec UNIVARIATE).
//! Exclusions : voir `common::partition_weighted_strict`.
//! Références (doc SAS 9.4) :
//!   - WEIGHT/VARDEF/SUMWGT : proc/p1ays1la1f3e2tn1m8owq8h9n1df.htm
//!   - formules : proc/n1y2f6nudl7zfjn1joclatu2h3zh.htm
//!   - Définition 5 : a002473616.htm (support.sas.com)
//!
//! ## Quantiles pondérés (J03-P2)
//! - MEDIAN / percentiles / QRANGE sous WEIGHT sont calculés via la
//!   Définition 5 pondérée dans les QUATRE chemins (listing,
//!   PRINTALLTYPES/WAYS/TYPES, `OUTPUT OUT=`, ODS « Summary ») — plus de
//!   divergence « non pondéré ».

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::num_var_meta;
use crate::procs::common::{
    VarDef, by_groups, decode_column, partition_numeric, partition_weighted_strict,
    resolve_by_cols, sample_std, t_quantile, weighted_quantile_def5, weighted_variance,
};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType, format_best};
use polars::prelude::*;
use std::cmp::Ordering;

mod output;
mod parse;
mod report;
mod stats;
mod types;

pub use parse::parse;
pub(crate) use parse::parse_named;

// `parse_single_var` et `parse_by_list` ont été déplacés vers `procs::common`
// (M31.2). Ré-export `pub(crate)` pour les appelants existants
// (`means` lui-même, `univariate`, `rank` via `means::parse_by_list`).
pub(crate) use crate::procs::common::{parse_by as parse_by_list, parse_single_var};
pub use report::stat_header;
pub use stats::compute;
pub use stats::compute_weighted;

use output::*;
use parse::*;
use report::*;
use types::*;

pub struct MeansAst {
    pub data: Option<DatasetRef>,
    pub summary: bool,
    pub noprint: bool,
    pub stats: Vec<String>,
    pub class: Vec<String>,
    pub var: Vec<String>,
    /// BY variables (var, descending). Outer grouping; input must be sorted.
    pub by: Vec<(String, bool)>,
    /// WEIGHT variable (single numeric var). When `Some`, all statistics are
    /// computed through the weighted code path (see `compute_weighted`).
    pub weight: Option<String>,
    /// FREQ variable (J07-P2): each observation counts `w` times — N, NMiss
    /// and _FREQ_ are multiplied by w, the variance divisor becomes Σw−1.
    pub freq: Option<String>,
    /// ID variables (J07-P2): copied into each OUT= dataset (largest level
    /// observed per group).
    pub id: Vec<String>,
    /// VARDEF= divisor for the weighted variance (J03-P2). Default DF
    /// (divisor Σw − 1); WEIGHT divides by Σw − Σw²/Σw.
    pub vardef: VarDef,
    /// Confidence level alpha for CLM/LCLM/UCLM (SAS default 0.05). Only the
    /// CI statistics consult it; it never affects the default output.
    pub alpha: f64,
    /// PRINTALLTYPES PROC option (M33.3). When false (default), the printed
    /// table shows only the all-CLASS-combined `_TYPE_`; when true, every
    /// generated `_TYPE_` subtable is printed.
    pub printalltypes: bool,
    /// NWAY (J07-P2): the OUT= dataset and the listing keep only the highest
    /// `_TYPE_` (every CLASS variable crossed).
    pub nway: bool,
    /// MISSING (J07-P2, PROC option or CLASS option): observations with a
    /// missing CLASS value form their own class level. Default (false):
    /// those observations are excluded from the analysis — class levels AND
    /// the `_TYPE_=0` row alike.
    pub missing: bool,
    /// ORDER= (J07-P2): CLASS level ordering (DATA/FORMATTED/FREQ/INTERNAL).
    pub order: ClassOrder,
    /// MAXDEC= (J07-P2): decimals of the PRINTED report only (0..9). None =
    /// BEST. formatting (no effect on OUT= datasets).
    pub maxdec: Option<usize>,
    /// DESCENDTYPES (J07-P2): `_TYPE_` values in descending order.
    pub descendtypes: bool,
    /// COMPLETETYPES (J07-P2): OUT= contains every combination of the
    /// observed CLASS levels, even unobserved ones (freq 0, missing stats).
    pub completetypes: bool,
    /// CHARTYPE (J07-P2): `_TYPE_` as a character mask ('101') in OUT=.
    pub chartype: bool,
    /// EXCLNPWGT (J07-P2): observations with a nonpositive WEIGHT/FREQ value
    /// are excluded (the strict partitions already do; the option is the
    /// documented explicit form).
    pub exclnpwgt: bool,
    /// WAYS values (M33.3): each requests the `_TYPE_` rows whose number of
    /// active CLASS variables equals the value. Empty → no WAYS restriction.
    pub ways: Vec<usize>,
    /// TYPES specifications (M33.3): each entry is a set of CLASS variable
    /// names (a specific crossing, e.g. `(a*b)`). Empty → no TYPES restriction.
    pub types: Vec<Vec<String>>,
    /// OUTPUT statements (J07-P2: several allowed, one OUT= each).
    pub output: Vec<MeansOutput>,
}

/// One OUTPUT statement: OUT= target, statistic specs, `/ AUTONAME` flag.
pub struct MeansOutput {
    pub out: DatasetRef,
    pub specs: Vec<OutSpec>,
    /// `/ AUTONAME` (J07-P2): unnamed specs get `<var>_<STAT>` names.
    pub autoname: bool,
}

/// One statistic request of an OUTPUT statement. `vars` empty = every VAR
/// variable; `names` may be empty (AUTONAME, or the stat keyword itself when
/// a single analysis variable is in scope).
pub struct OutSpec {
    pub stat: String,
    pub vars: Vec<String>,
    pub names: Vec<String>,
}

/// Shared per-invocation execution context (J07-P2): everything the listing
/// and the OUT= writers need beyond their row scopes.
pub(super) struct ExecCtx<'a> {
    pub ds: &'a SasDataset,
    pub class_cols: &'a [usize],
    pub class_values: &'a [Vec<Value>],
    pub var_cols: &'a [usize],
    pub var_values: &'a [Vec<Value>],
    pub weight_values: Option<&'a [Value]>,
    pub freq_values: Option<&'a [Value]>,
    pub report_stats: &'a [String],
    pub alpha: f64,
    pub vardef: VarDef,
    pub order: ClassOrder,
    pub maxdec: Option<usize>,
    /// Per-class-variable levels in display order (ORDER=), rank 0 first.
    /// Empty under ORDER=INTERNAL.
    pub ranks: Vec<Vec<Value>>,
}

impl ExecCtx<'_> {
    /// Compute `stat` over `col` restricted to `rows`, choosing the FREQ /
    /// WEIGHT / plain path.
    pub(super) fn stat_over(&self, col: &[Value], rows: &[usize], stat: &str) -> Value {
        if let Some(wv) = self.freq_values {
            let (pairs, nmiss) = stats::partition_freq(col, wv, rows);
            return stats::compute_freq(stat, &pairs, nmiss, self.vardef, self.alpha);
        }
        if let Some(wv) = self.weight_values {
            let (pairs, nmiss) = partition_weighted_strict(col, wv, rows);
            return compute_weighted(stat, &pairs, nmiss, self.vardef, self.alpha);
        }
        let (xs, nmiss) = partition_numeric(col, rows);
        compute(stat, &xs, nmiss, self.alpha)
    }

    /// Group `rows` by the CLASS variables at `active` positions, ordered
    /// per ORDER= (Internal → plain `sas_cmp` ascending).
    pub(super) fn ordered_groups(
        &self,
        active: &[usize],
        rows: &[usize],
    ) -> Vec<(Vec<Value>, Vec<usize>)> {
        let active_refs: Vec<&Vec<Value>> = active.iter().map(|&i| &self.class_values[i]).collect();
        let mut groups = group_by_keys_subset(&active_refs, rows);
        if self.order != ClassOrder::Internal && !active.is_empty() {
            sort_groups_by_ranks(&mut groups, &self.ranks, active);
        }
        groups
    }
}

/// Execute PROC MEANS / SUMMARY. Called by `procs::execute_proc`.
pub fn execute(ast: &MeansAst, session: &mut Session) -> Result<()> {
    let (ds, in_libref, in_table) = crate::procs::common::open_input(&ast.data, session)?;

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

    // Resolve & decode the WEIGHT column once (None → unweighted path,
    // byte-identical to before).
    let weight_values: Option<Vec<Value>> = match &ast.weight {
        Some(wname) => {
            let wi = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(wname))
                .ok_or_else(|| {
                    SasError::runtime(format!("Variable {} not found.", wname.to_uppercase()))
                })?;
            Some(decode_column(&ds, wi)?)
        }
        None => None,
    };

    // J07-P2 — FREQ column: like WEIGHT but N/NMiss/_FREQ_ count Σw and the
    // variance divisor is Σw−1 (see `stats::compute_freq`).
    let freq_values: Option<Vec<Value>> = match &ast.freq {
        Some(fname) => {
            let fi = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(fname))
                .ok_or_else(|| {
                    SasError::runtime(format!("Variable {} not found.", fname.to_uppercase()))
                })?;
            Some(decode_column(&ds, fi)?)
        }
        None => None,
    };

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

    // --- BY processing: resolve, verify sortedness, partition into groups. ---
    // No BY → a single group spanning all rows (output byte-identical).
    let by_cols = resolve_by_cols(&ds, &ast.by)?;
    let by_values: Vec<Vec<Value>> = by_cols
        .iter()
        .map(|c| decode_column(&ds, c.col_idx))
        .collect::<Result<_>>()?;
    let by_groups_list: Vec<(Vec<Value>, Vec<usize>)> = if by_cols.is_empty() {
        vec![(Vec::new(), (0..n_obs).collect())]
    } else {
        let descending: Vec<bool> = by_cols.iter().map(|c| c.descending).collect();
        let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();
        let in_display = format!("{in_libref}.{in_table}");
        by_groups(&by_values, &descending, n_obs, &by_names, &in_display)?
    };
    let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();

    // --- J07-P2 — MISSING : sans l'option, les observations dont une valeur
    // CLASS est manquante sont exclues de l'analyse (niveaux de classe ET
    // ligne _TYPE_=0 comprises). Avec MISSING (PROC ou CLASS), elles forment
    // un niveau de classe à part entière. ---
    let class_missing_row =
        |r: usize| !ast.missing && class_values.iter().any(|c| is_missing_class(&c[r]));
    let by_analysis: Vec<Vec<usize>> = by_groups_list
        .iter()
        .map(|(_, rows)| {
            rows.iter()
                .copied()
                .filter(|&r| !class_missing_row(r))
                .collect()
        })
        .collect();

    // --- ORDER= (J07-P2) : rangs des niveaux de chaque variable CLASS,
    // calculés sur l'ensemble des lignes d'analyse (tous groupes BY). ---
    let k = class_cols.len();
    let ranks: Vec<Vec<Value>> = if ast.order == ClassOrder::Internal || k == 0 {
        Vec::new()
    } else {
        (0..k)
            .map(|i| {
                let mut levels: Vec<LevelInfo> = Vec::new();
                for group in &by_analysis {
                    for &r in group {
                        let v = &class_values[i][r];
                        match levels
                            .iter()
                            .position(|l| l.value.sas_cmp(v) == Ordering::Equal)
                        {
                            Some(p) => levels[p].count += 1,
                            None => levels.push(LevelInfo {
                                value: v.clone(),
                                count: 1,
                                first_seen: r,
                            }),
                        }
                    }
                }
                rank_levels(ast.order, &levels)
            })
            .collect()
    };

    // --- WAYS / TYPES restriction (M33.3) + NWAY (J07-P2): the set of
    // _TYPE_ values to keep, or None for "no restriction" (default path).
    // NWAY ne retient que le _TYPE_ maximal (croisement de toutes les
    // CLASS), intersecté avec WAYS/TYPES si les deux sont présents. ---
    let mut allowed = allowed_types(ast, &ast.class, k)?;
    if ast.nway {
        let full = type_mask(&(0..k).collect::<Vec<_>>(), k);
        match &mut allowed {
            Some(set) => set.retain(|t| *t == full),
            None => allowed = Some(std::iter::once(full).collect()),
        }
    }

    // Which _TYPE_ subtables the listing prints. SAS default: ONLY the highest
    // _TYPE_ (all CLASS crossed). PRINTALLTYPES (or any WAYS/TYPES request)
    // prints each selected _TYPE_ as its own subtable. Without CLASS there is a
    // single _TYPE_=0 table either way (byte-identical default).
    let mut print_types: Vec<u64> = if k == 0 {
        vec![0]
    } else if ast.printalltypes || allowed.is_some() {
        // Every selected _TYPE_, ascending. With no WAYS/TYPES but
        // PRINTALLTYPES → all 2^k types.
        let mut v: Vec<u64> = match &allowed {
            Some(set) => set.iter().copied().collect(),
            None => (0u32..(1u32 << k))
                .map(|mask| {
                    let active: Vec<usize> = (0..k).filter(|&i| (mask >> i) & 1 == 1).collect();
                    type_mask(&active, k)
                })
                .collect(),
        };
        v.sort_unstable();
        v.dedup();
        v
    } else {
        // Default: only the highest _TYPE_ (all CLASS active) → byte-identical.
        vec![type_mask(&(0..k).collect::<Vec<_>>(), k)]
    };
    // J07-P2 — DESCENDTYPES : _TYPE_ décroissants (listing et OUT=).
    if ast.descendtypes {
        print_types.reverse();
    }

    let ctx = ExecCtx {
        ds: &ds,
        class_cols: &class_cols,
        class_values: &class_values,
        var_cols: &var_cols,
        var_values: &var_values,
        weight_values: weight_values.as_deref(),
        freq_values: freq_values.as_deref(),
        report_stats: &report_stats,
        alpha: ast.alpha,
        vardef: ast.vardef,
        order: ast.order,
        maxdec: ast.maxdec,
        ranks,
    };

    // --- Report ---
    // M38.4 — le rapport de MEANS porte le nom d'objet ODS « Summary » : une
    // liste ODS SELECT/EXCLUDE qui l'écarte supprime tout le bloc listing
    // (en-tête compris — c'est le seul objet du proc, SAS ne produit alors
    // aucune page), exactement comme NOPRINT. Les datasets OUTPUT OUT= et la
    // capture ODS OUTPUT Summary= restent produits.
    if !ast.noprint && session.ods_displays("Summary") {
        // Title printed once per proc invocation.
        session.listing.page_header();
        let title = "The MEANS Procedure";
        let ls = session.listing.ls();
        let pad = ls.saturating_sub(title.len()) / 2;
        session
            .listing
            .write_line(&format!("{}{}", " ".repeat(pad), title));
        session.listing.blank();

        for ((by_key, _grp_rows), analysis_rows) in by_groups_list.iter().zip(&by_analysis) {
            if !by_names.is_empty() {
                emit_by_heading(session, &by_names, by_key);
            }
            // Default single-type path stays byte-identical: when k==0 or the
            // only printed type is the full crossing AND neither PRINTALLTYPES
            // nor WAYS/TYPES is active, use the original combined-table emitter.
            let full_type = type_mask(&(0..k).collect::<Vec<_>>(), k);
            if !ast.printalltypes && allowed.is_none() && print_types == [full_type] {
                emit_report_group(session, &ctx, analysis_rows);
            } else {
                for &ty in &print_types {
                    emit_report_type(session, &ctx, analysis_rows, ty);
                }
            }
        }
    }

    // --- OUTPUT OUT= (J07-P2 : plusieurs statements OUTPUT, un OUT= chacun) ---
    for out in &ast.output {
        write_output(
            session,
            &ctx,
            out,
            &ast.id,
            &by_cols,
            &by_groups_list,
            &by_analysis,
            ast.descendtypes,
            ast.completetypes,
            ast.chartype,
            allowed.as_ref(),
        )?;
    }

    // --- ODS OUTPUT Summary= (M22.3, J07-P2) ---
    // Capture la table ODS "Summary" comme dataset si `ODS OUTPUT Summary=...`
    // est actif. Inactif par défaut (registre vide) → aucun effet, listing
    // byte-identique. J07-P2 : la table porte les colonnes BY/CLASS — une
    // ligne par (groupe BY × combinaison de classes affichée × variable VAR).
    if let Some(target) = session.ods_output_target("Summary") {
        write_ods_summary(
            session,
            &ctx,
            &by_cols,
            &by_groups_list,
            &by_analysis,
            &print_types,
            &target,
        )?;
        // M38.3 — capture immédiate : signaler la production pour que la
        // frontière de step n'émette pas « Output 'Summary' was not created ».
        session.mark_ods_output_created("Summary");
    }

    Ok(())
}

#[cfg(test)]
mod tests;
