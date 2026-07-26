//! PROC CLUSTER — agglomerative hierarchical clustering (M27).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc cluster data=<ref> [method=ward|average|single|complete]
//!  [outtree=<ref>] [print=<n>] [noeigen]; var <list>; [id <var>;] run;`
//!
//! ## Périmètre
//! - `data=`, `method=` (défaut WARD), `outtree=` (parse-accepté, NOTE),
//!   `print=` (parse-accepté), `noeigen` (parse-accepté, ignoré).
//! - `var` : variables numériques (coordonnées). `id <var>` : étiquette (parse).
//! - Sortie : "Cluster History" (NCl, Clusters Joined, Freq, SPRSQ, RSQ).
//! - OUTTREE= : dataset dendrogramme (_NAME_/_PARENT_/_NCL_/_FREQ_/_HEIGHT_
//!   + valeurs VAR pour les feuilles).
//! - Différé : section Eigenvalues.
//!
//! ## Algorithme
//! - n clusters singletons ; matrice de dissimilarité euclidienne initiale.
//! - À chaque étape : paire (i,j) minimisant le critère de la méthode
//!   (Ward = (ni*nj)/(ni+nj) * d², single/complete/average sur les distances).
//! - Mise à jour Lance-Williams. Tie-break : indices (i<j) croissants, on ne
//!   remplace le meilleur que sur STRICTEMENT inférieur (plus petits indices).
//!
//! ## SPRSQ / RSQ (TOUJOURS basés sur la somme des carrés intra, indépendant
//! de la méthode de liaison) :
//! - SS_total = Σ sur toutes les obs et variables des carrés des écarts à la
//!   moyenne globale.
//! - À chaque fusion, ΔSS = (ni*nj)/(ni+nj) * d²(centroïde_i, centroïde_j)
//!   (formule de Ward = augmentation exacte de la SS intra).
//! - SPRSQ = ΔSS / SS_total ; RSQ = 1 - (SS_intra_cumulée / SS_total).
//! - Nommage : un cluster formé à la ligne où il reste NCl clusters → "CL<NCl>".

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::VarType;

mod agglomerate;
mod output;
mod parse;

pub use agglomerate::MergeStep;
pub use agglomerate::agglomerate;
pub use parse::ClusterAst;
pub use parse::LinkMethod;
pub use parse::parse;

use output::*;

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &ClusterAst, session: &mut Session) -> Result<()> {
    if ast.var.is_empty() {
        return Err(SasError::runtime("PROC CLUSTER requires a VAR statement."));
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

    let coords: Vec<Vec<f64>> = (0..n_read)
        .map(|r| decoded.iter().map(|col| col[r]).collect())
        .collect();
    let n = coords.len();
    if n < 2 {
        return Err(SasError::runtime(
            "PROC CLUSTER requires at least 2 observations.",
        ));
    }

    // Singleton labels: ID value if an ID variable is present, else OB<i>.
    let labels: Vec<String> = match &ast.id {
        Some(idname) => {
            let idx = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(idname))
                .ok_or_else(|| {
                    SasError::runtime(format!(
                        "Variable '{}' not found in dataset '{}'.",
                        idname, display
                    ))
                })?;
            let vals = decode_column(&ds, idx)?;
            vals.iter().map(label_of_value).collect()
        }
        None => (0..n).map(|i| format!("OB{}", i + 1)).collect(),
    };

    let history = agglomerate(&coords, ast.method, &labels);

    // ───────────────────────── listing ─────────────────────────
    session.listing.page_header();
    centered(session, "The CLUSTER Procedure");
    centered(session, ast.method.title());
    session.listing.blank();
    centered(session, "Cluster History");
    session.listing.blank();
    {
        let headers: Vec<String> = vec![
            "NCl".into(),
            "Clusters Joined".into(),
            String::new(),
            "Freq".into(),
            "SPRSQ".into(),
            "RSQ".into(),
        ];
        let aligns = vec![
            Align::Right,
            Align::Right,
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let rows: Vec<Vec<String>> = history
            .iter()
            .map(|s| {
                vec![
                    s.ncl.to_string(),
                    s.joined_a.clone(),
                    s.joined_b.clone(),
                    s.freq.to_string(),
                    format!("{:.4}", s.sprsq),
                    format!("{:.4}", s.rsq.max(0.0)),
                ]
            })
            .collect();
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    if ast.noeigen {
        // Eigenvalues section is deferred; NOEIGEN simply confirms we skip it.
        session
            .log
            .note("PROC CLUSTER NOEIGEN: eigenvalue section is not produced.");
    }
    if let Some(k) = ast.print {
        session.log.note(&format!(
            "PROC CLUSTER PRINT={} is accepted; full history is shown.",
            k
        ));
    }
    if let Some(out_ref) = &ast.outtree {
        write_outtree(out_ref, &labels, &decoded, &ast.var, &history, session)?;
    }

    Ok(())
}

use crate::procs::common::centered;

#[cfg(test)]
mod tests;
