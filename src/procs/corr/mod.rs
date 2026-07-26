//! PROC CORR — Pearson product-moment correlations (v1).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc corr data=<ref> [nosimple] [noprob] [nocorr];
//!  var <name list>; [with <name list>;] run;`
//!
//! ## Périmètre v1 (fidèle à SAS 9.4 PROC CORR, Pearson uniquement)
//! - Options du statement PROC : `data=`, `nosimple`, `noprob`, `nocorr`.
//! - `var`  : variables analysées (défaut = toutes les numériques du dataset,
//!   dans l'ordre du dataset).
//! - `with` : facultatif. Présent → lignes de la matrice = variables WITH,
//!   colonnes = variables VAR. Absent → matrice carrée symétrique sur VAR.
//!
//! ## Sortie listing (titre "The CORR Procedure"), dans l'ordre SAS :
//! 1. Ligne récapitulative des variables analysées. SAS imprime
//!    `N Variables: ...` (sans WITH) ou bien `N With Variables:` / `M Variables:`
//!    (avec WITH). On reproduit ce style.
//! 2. Table « Simple Statistics » (sauf `nosimple`) : une ligne par variable
//!    analysée (union WITH ∪ VAR, dans l'ordre), colonnes Variable, N, Mean,
//!    Std Dev, Sum, Minimum, Maximum. `sample_std` (n-1), missing exclus via
//!    `partition_numeric`.
//! 3. Matrice « Pearson Correlation Coefficients » (sauf `nocorr`) : lignes =
//!    WITH (ou VAR), colonnes = VAR. Chaque cellule : r à 5 décimales ; sous
//!    r, la p-value `Prob > |r|` (test t bilatéral) sauf `noprob` ; et, quand
//!    les N par paire diffèrent, une 3e ligne avec N. Observations
//!    **pairwise-complete** (on retire toute ligne où l'une des deux variables
//!    est missing pour la paire). Variable constante (variance nulle) → r
//!    missing → SAS imprime `.`.
//!
//! ## Choix / simplifications documentés (pour l'orchestrateur)
//! - La p-value diagonale (r==1 exact, même variable) est imprimée par SAS
//!   sans valeur numérique : on suit SAS et imprimons une cellule vide pour
//!   la prob de la diagonale. (SAS laisse la ligne Prob vide sur la diagonale
//!   d'une matrice symétrique VAR×VAR ; pour WITH×VAR où une variable WITH est
//!   aussi VAR la même règle s'applique : r==1 exact, prob vide.)
//! - t-CDF : survie de la loi de Student via la fonction bêta incomplète
//!   régularisée I_x(a,b) (fraction continue de Lentz). Précision ~1e-10 sur
//!   la plage utile ; voir `student_t_sf` et `betai`. Formatage SAS : valeurs
//!   < 0.0001 → `<.0001`, sinon 4 décimales.
//! ## M21.5 — extensions (Spearman / Kendall / OUT= / WEIGHT)
//! - `spearman` : corrélation de rang de Spearman = Pearson sur les **rangs**
//!   (rangs moyens pour les ex æquo) de chaque paire appariée-complète.
//!   `Prob > |r|` via la même approximation t (ddl = n−2). Bloc « Spearman
//!   Correlation Coefficients ».
//! - `kendall` : tau-b de Kendall, τ_b = (n_c − n_d)/√((n0−n1)(n0−n2)),
//!   p-value par approximation normale z = 3·τ·√(n(n−1))/√(2(2n+5)) (sans
//!   correction de ties en v1, documenté). Bloc « Kendall Tau b Coefficients ».
//! - `pearson` : sélectionne explicitement Pearson. Par défaut (aucune option
//!   de méthode), seul Pearson est produit, byte-identique à l'incrément v1.
//! - `weight var` : corrélations **pondérées** (moyennes/(co)variances
//!   pondérées par w ; obs exclue si w manquant ou ≤ 0 — voir
//!   `partition_weighted`). S'applique à Pearson, Spearman et Kendall :
//!   * Pearson pondéré : Pearson sur (co)variances pondérées.
//!   * Spearman pondéré (M34.1) : Pearson pondéré sur les **rangs moyens
//!     pondérés** (`weighted_mean_ranks` — rang d'un bloc d'ex æquo de poids W
//!     démarrant au poids cumulé c = c + (W+1)/2). Avec des poids entiers =
//!     comptes de réplication, équivaut au Spearman ordinaire sur les données
//!     répliquées (testé).
//!   * Kendall pondéré (M34.1) : paires (i,j) pondérées par w_i·w_j dans C, D
//!     et les totaux n0/n1/n2 du dénominateur tau-b. Idem : équivaut au tau-b
//!     ordinaire sur les données répliquées (testé).
//! - `hoeffding` (M34.1) : D de Hoeffding sur les observations
//!   appariées-complètes (n ≥ 5). Bloc « Hoeffding Dependence Coefficients »,
//!   D à 5 décimales, colonne `Prob > D`. D exact (≡ SAS à 5 décimales) ;
//!   `Prob > D` = approximation asymptotique de Blum-Kiefer-Rosenblatt (méthode
//!   d'Imhof), documentée comme approchée pour petit n (SAS tabule la loi
//!   exacte pour n modéré). Voir `hoeffding_d` / `hoeffding_pvalue`.
//! - `out=`/`outp=`/`outs=`/`outk=` : dataset TYPE=CORR. Variables `_TYPE_`
//!   (MEAN/STD/N/CORR), `_NAME_` (nom de variable des lignes CORR), puis une
//!   colonne par variable analysée. OUTP=/OUT= = Pearson, OUTS= = Spearman,
//!   OUTK= = Kendall. NOTE de création, types SAS, `last_dataset` mis à jour.
//!   Le bloc CORR du dataset est carré (analysis × analysis), indépendamment
//!   de WITH (qui ne modifie que la mise en page du listing).
//! - En-têtes de table : on s'appuie sur `ListingWriter::write_table` ; les
//!   cellules multi-lignes (r / prob / N) de la matrice sont rendues avec une
//!   ligne de tableau par composante (r, puis prob, puis N) pour rester dans
//!   le moule monospace existant.

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{
    self, decode_column, partition_numeric, partition_weighted, sample_std,
};
use crate::procs::common::{char_var_meta, num_var_meta};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType, format_best};
use polars::prelude::{Column, DataFrame, NamedFrom, Series};

mod matrix;
mod output;
mod parse;
mod pearson;
mod rank_stats;
mod report;
mod special;

pub use parse::parse;

use matrix::*;
use output::*;
use pearson::*;
use rank_stats::*;
use report::*;
use special::*;

pub struct CorrAst {
    pub data: Option<DatasetRef>,
    pub nosimple: bool,
    pub noprob: bool,
    pub nocorr: bool,
    /// Request Pearson coefficients explicitly. When any of pearson/spearman/
    /// kendall is set, only the requested methods are produced; otherwise
    /// Pearson is the default.
    pub pearson: bool,
    /// Request Spearman rank correlation coefficients.
    pub spearman: bool,
    /// Request Kendall tau-b coefficients.
    pub kendall: bool,
    /// Request Hoeffding's D measure of dependence.
    pub hoeffding: bool,
    /// Explicit VAR list (empty = default to all numeric variables).
    pub var: Vec<String>,
    /// Optional WITH list (empty = none).
    pub with: Vec<String>,
    /// Optional PARTIAL list (empty = none) — variables partialled out
    /// (controlled for) before computing the (Pearson) correlations.
    pub partial: Vec<String>,
    /// Optional WEIGHT variable (Pearson only). None = unweighted.
    pub weight: Option<String>,
    /// OUTP= / OUT= : Pearson output dataset (TYPE=CORR).
    pub outp: Option<DatasetRef>,
    /// OUTS= : Spearman output dataset (TYPE=CORR).
    pub outs: Option<DatasetRef>,
    /// OUTK= : Kendall output dataset (TYPE=CORR).
    pub outk: Option<DatasetRef>,
}

impl CorrAst {
    /// Whether the Pearson method should be computed/displayed. Pearson is the
    /// default when no method option was given.
    fn want_pearson(&self) -> bool {
        self.pearson || !(self.spearman || self.kendall || self.hoeffding)
    }
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &CorrAst, session: &mut Session) -> Result<()> {
    let (ds, _, _) = common::open_input(&ast.data, session)?;

    let n_obs = ds.n_obs();

    // Resolve VAR list: explicit, else all numeric vars in dataset order.
    let resolve_names = |names: &[String]| -> Result<Vec<usize>> {
        let mut out = Vec::with_capacity(names.len());
        for nm in names {
            match ds.vars.iter().position(|m| m.name.eq_ignore_ascii_case(nm)) {
                Some(i) => {
                    if ds.vars[i].ty != VarType::Num {
                        return Err(SasError::runtime(format!(
                            "Variable {} in the VAR or WITH list is not numeric.",
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
        Ok(out)
    };

    let var_cols: Vec<usize> = if !ast.var.is_empty() {
        resolve_names(&ast.var)?
    } else {
        (0..ds.vars.len())
            .filter(|&i| ds.vars[i].ty == VarType::Num)
            .collect()
    };

    if var_cols.is_empty() {
        return Err(SasError::runtime(
            "No numeric variables found for PROC CORR analysis.",
        ));
    }

    let with_cols: Vec<usize> = if !ast.with.is_empty() {
        resolve_names(&ast.with)?
    } else {
        Vec::new()
    };

    // Resolve the PARTIAL variables (controlled-for set). Numeric only.
    let partial_cols: Vec<usize> = if !ast.partial.is_empty() {
        resolve_names(&ast.partial)?
    } else {
        Vec::new()
    };

    // Resolve the WEIGHT variable (single numeric column). Applies to Pearson.
    let weight_col: Option<usize> = match &ast.weight {
        Some(nm) => Some(resolve_names(std::slice::from_ref(nm))?[0]),
        None => None,
    };

    // Matrix rows = WITH (or VAR if no WITH); columns = VAR.
    let row_cols: Vec<usize> = if with_cols.is_empty() {
        var_cols.clone()
    } else {
        with_cols.clone()
    };
    let col_cols: Vec<usize> = var_cols.clone();

    // Analysis variables for Simple Statistics = union(rows, cols) in order:
    // WITH first (if any), then VAR, skipping duplicates by column index.
    let mut analysis_cols: Vec<usize> = Vec::new();
    for &c in row_cols.iter().chain(col_cols.iter()) {
        if !analysis_cols.contains(&c) {
            analysis_cols.push(c);
        }
    }

    // Decode each needed column exactly once (analysis vars + weight).
    let mut decoded: std::collections::HashMap<usize, Vec<Value>> =
        std::collections::HashMap::new();
    for &c in &analysis_cols {
        decoded.insert(c, decode_column(&ds, c)?);
    }
    for &c in &partial_cols {
        decoded
            .entry(c)
            .or_insert_with(|| decode_column(&ds, c).unwrap_or_default());
    }
    if let Some(wc) = weight_col {
        decoded
            .entry(wc)
            .or_insert_with(|| decode_column(&ds, wc).unwrap_or_default());
    }
    let weight_vals: Option<&[Value]> = weight_col.map(|wc| decoded[&wc].as_slice());

    // --- listing ---
    session.listing.page_header();
    centered(session, "The CORR Procedure");
    session.listing.blank();

    // Variable summary line(s), SAS style.
    if with_cols.is_empty() {
        let names: Vec<String> = var_cols.iter().map(|&c| ds.vars[c].name.clone()).collect();
        session.listing.write_line(&format!(
            "{} Variables:  {}",
            var_cols.len(),
            names.join(" ")
        ));
    } else {
        let wnames: Vec<String> = with_cols.iter().map(|&c| ds.vars[c].name.clone()).collect();
        let vnames: Vec<String> = var_cols.iter().map(|&c| ds.vars[c].name.clone()).collect();
        session.listing.write_line(&format!(
            "{} With Variables:  {}",
            with_cols.len(),
            wnames.join(" ")
        ));
        session.listing.write_line(&format!(
            "{} Variables:  {}",
            var_cols.len(),
            vnames.join(" ")
        ));
    }
    session.listing.blank();

    // --- Simple Statistics ---
    if !ast.nosimple {
        emit_simple_statistics(session, &ds, &analysis_cols, &decoded, n_obs);
    }

    // Which methods are requested (Pearson default when none specified).
    let methods: Vec<Method> = {
        let mut m = Vec::new();
        if ast.want_pearson() {
            m.push(Method::Pearson);
        }
        if ast.spearman {
            m.push(Method::Spearman);
        }
        if ast.kendall {
            m.push(Method::Kendall);
        }
        if ast.hoeffding {
            m.push(Method::Hoeffding);
        }
        m
    };

    // --- Correlation Coefficients (one block per requested method) ---
    if !ast.nocorr {
        if !partial_cols.is_empty() {
            // PARTIAL correlation: Pearson only in this build. If the user also
            // asked for Spearman/Kendall, note that those are not partialled.
            if ast.spearman || ast.kendall {
                session.log.note(
                    "PROC CORR: partial Spearman/Kendall correlations are not yet \
                     implemented; only Pearson partial correlations are produced.",
                );
            }
            let cells = partial_pearson_matrix(&row_cols, &col_cols, &partial_cols, &decoded);
            let k = partial_cols.len();
            let controlling: Vec<String> = partial_cols
                .iter()
                .map(|&c| ds.vars[c].name.clone())
                .collect();
            let heading = format!(
                "Pearson Partial Correlation Coefficients, Controlled for: {}",
                controlling.join(" ")
            );
            let _ = k; // df = n − k − 2 is applied inside partial_pvalue
            let prob_line = "Prob > |r| under H0: Partial Rho=0".to_string();
            emit_correlations(
                session, &ds, &row_cols, &col_cols, &heading, &prob_line, &cells, ast.noprob,
            );
        } else {
            for &method in &methods {
                let cells = compute_matrix(method, &row_cols, &col_cols, &decoded, weight_vals);
                let prob_line = match method {
                    Method::Kendall => "Prob > |tau| under H0: Tau=0",
                    Method::Hoeffding => "Prob > D under H0: D=0",
                    _ => "Prob > |r| under H0: Rho=0",
                };
                emit_correlations(
                    session,
                    &ds,
                    &row_cols,
                    &col_cols,
                    method.heading(),
                    prob_line,
                    &cells,
                    ast.noprob,
                );
            }
        }
    }

    // --- OUT= / OUTP= / OUTS= / OUTK= : TYPE=CORR datasets ---
    // The CORR block of the output dataset is square (analysis × analysis),
    // independent of WITH.
    let out_targets: [(Method, &Option<DatasetRef>); 3] = [
        (Method::Pearson, &ast.outp),
        (Method::Spearman, &ast.outs),
        (Method::Kendall, &ast.outk),
    ];
    for (method, target) in out_targets {
        if let Some(target) = target {
            let out_ds =
                build_out_dataset(method, &ds, &analysis_cols, &decoded, weight_vals, n_obs)?;
            write_out_dataset(session, target, out_ds)?;
        }
    }

    Ok(())
}

use crate::procs::common::centered;

#[cfg(test)]
mod tests;
