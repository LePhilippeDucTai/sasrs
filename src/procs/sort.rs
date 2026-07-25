//! PROC SORT (jalon M3 + options M33.9).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc sort data=a [out=b] [nodupkey] [noduprecs]
//!           [tagsort] [sortseq=ASCII|LINGUISTIC] ;
//! by [descending] v1 [descending] v2... ;
//! key=var [/ descending] ;   (une ou plusieurs)
//! run ;`
//!
//! ## Piège central : la collation SAS des missings
//! Ordre numérique SAS : `._ < . < .A < ... < .Z < nombres`. Les flags
//! nulls_first/last de Polars ne connaissent pas les missings spéciaux
//! (NaN-payload). L'en-tête historique proposait une colonne de rang i8
//! compagnon. Or `Value::sas_cmp` ENCODE DÉJÀ cette collation (cf.
//! `value.rs` : `._ < . < .A.. < nombres`, et `. = .` est vrai). On
//! choisit donc la voie recommandée par PLAN.md : décoder chaque colonne
//! clé UNE fois en `Vec<Value>` (downcast par colonne, jamais par cellule
//! — checklist point 3), construire un vecteur d'indices `0..n` et le
//! trier de façon STABLE avec un comparateur qui applique `sas_cmp` clé
//! par clé (inversé pour DESCENDING). La colonne de rang devient inutile.
//! Le réordonnancement final se fait en UN SEUL `df.take` sur toutes les
//! colonnes.
//!
//! ## Collation caractère
//! `sas_cmp` compare les chaînes via `trim_end()` (les blancs de fin sont
//! ignorés, comme SAS qui padde l'opérande le plus court). C'est fidèle à
//! SAS et indépendant du padding de stockage. DESCENDING inverse l'ordre
//! de la clé concernée.
//!
//! ## Options M33.9
//! - TAGSORT : hint de performance SAS (tri par tag+clé en deux passes).
//!   N'a AUCUN EFFET sur la sortie. Accepté et ignoré — la sortie est
//!   octet-identique à un tri sans TAGSORT.
//! - SORTSEQ=ASCII : collation ASCII — comportement courant. No-op.
//! - SORTSEQ=LINGUISTIC : collation linguistique (locale). SIMPLIFICATION :
//!   on applique la collation `sas_cmp` existante (ordre binaire UTF-8).
//!   La différence n'est visible que pour des caractères accentués/CJK ;
//!   documenté dans le log par une NOTE de simplification.
//! - KEY=var [/ DESCENDING] : alternative moderne à BY. Chaque KEY=
//!   s'ajoute à la liste des clés de tri (équivalent exact à un BY
//!   variable). Si BY et KEY sont présents simultanément, KEY prend le
//!   dessus (SAS 9.4 : les KEY= remplacent BY lorsque les deux coexistent).
//!   Dans cette implémentation, si KEY= est présent, il remplace BY.
//!
//! ## Autres règles
//! - NODUPKEY : déduplication sur les clés BY après tri (garder la
//!   première), NOTE "N observations with duplicate key values were
//!   deleted." ; NODUPRECS (alias NODUP) compare la LIGNE ENTIÈRE.
//! - out= absent → remplace le dataset d'entrée.
//! - Conserver les VarMeta du dataset d'entrée à l'identique.

use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::token::TokenKind;
use crate::procs::common::{self, decode_column};
use crate::session::Session;
use crate::value::Value;
use polars::prelude::*;
use std::cmp::Ordering;

/// Séquence de tri déclarée via SORTSEQ=.
#[derive(Debug, Clone, PartialEq)]
pub enum SortSeq {
    /// Tri binaire ASCII (comportement par défaut, identique à sans SORTSEQ).
    Ascii,
    /// Tri linguistique (locale). Simplifié : même collation que ASCII ici.
    Linguistic,
}

pub struct SortAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    /// Clés de tri issues de BY ou de KEY= (KEY= remplace BY si présent).
    pub by: Vec<(String, bool)>, // (var, descending)
    pub nodupkey: bool,
    pub noduprecs: bool,
    /// TAGSORT : hint de performance, ignoré sémantiquement.
    pub tagsort: bool,
    /// Séquence de tri (SORTSEQ=ASCII par défaut).
    pub sortseq: SortSeq,
}

/// Parse un statement `key=var [/ descending] ;`.
/// Le token courant est positionné sur `key`. On consomme jusqu'au `;` inclus.
/// Retourne une liste de `(var, descending)`.
fn parse_key_statement(ts: &mut StatementStream) -> Result<Vec<(String, bool)>> {
    ts.next(); // consume "key"
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            "expected '=' after KEY".to_string(),
            ts.peek().span,
        ));
    }
    ts.next(); // consume '='

    // Variable name.
    let var_tok = ts.peek().clone();
    let var_name = var_tok
        .ident()
        .ok_or_else(|| {
            SasError::parse(
                "expected a variable name after KEY=".to_string(),
                var_tok.span,
            )
        })?
        .to_string();
    ts.next(); // consume var name

    // Optional `/ descending`.
    let mut descending = false;
    if ts.peek().kind == TokenKind::Slash {
        ts.next(); // consume '/'
        // Look for DESCENDING (or ASCENDING, which is the default).
        loop {
            if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                break;
            }
            let kw = ts.peek().ident().map(|s| s.to_ascii_lowercase());
            match kw.as_deref() {
                Some("descending") => {
                    ts.next();
                    descending = true;
                }
                Some("ascending") => {
                    ts.next(); // ascending is the default, no change
                }
                _ => break,
            }
        }
    }

    // Consume trailing `;`.
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }

    Ok(vec![(var_name, descending)])
}

/// Parse `proc sort [data=a] [out=b] [nodupkey] [noduprecs|nodup]
///                 [tagsort] [sortseq=ASCII|LINGUISTIC] ;
/// by [descending] v1 [descending] v2... ;
/// key=var [/ descending] ;   (répétable)
/// run ;`.
/// Called AFTER "proc sort" has been consumed. Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<SortAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut nodupkey = false;
    let mut noduprecs = false;
    let mut tagsort = false;
    let mut sortseq = SortSeq::Ascii;

    // --- PROC SORT statement options, until `;` (combinateur partagé M31) ---
    common::parse_proc_options(ts, "SORT", |ts, kw| {
        Ok(match kw {
            "data" => {
                data = Some(common::parse_dataset_opt(ts, "DATA")?);
                true
            }
            "out" => {
                out = Some(common::parse_dataset_opt(ts, "OUT")?);
                true
            }
            "nodupkey" => {
                ts.next();
                nodupkey = true;
                true
            }
            "noduprecs" | "nodup" => {
                ts.next();
                noduprecs = true;
                true
            }
            "tagsort" => {
                // TAGSORT : performance hint, accepté, ignoré sémantiquement.
                ts.next();
                tagsort = true;
                true
            }
            "sortseq" => {
                // SORTSEQ=ASCII|LINGUISTIC
                ts.next(); // consume "sortseq"
                if ts.peek().kind != TokenKind::Eq {
                    return Err(SasError::parse(
                        "expected '=' after SORTSEQ".to_string(),
                        ts.peek().span,
                    ));
                }
                ts.next(); // consume '='
                let seq_tok = ts.peek().clone();
                let seq_name = seq_tok
                    .ident()
                    .ok_or_else(|| {
                        SasError::parse(
                            "expected a collating sequence name after SORTSEQ=".to_string(),
                            seq_tok.span,
                        )
                    })?
                    .to_ascii_uppercase();
                ts.next();
                sortseq = match seq_name.as_str() {
                    "ASCII" => SortSeq::Ascii,
                    "LINGUISTIC" => SortSeq::Linguistic,
                    other => {
                        return Err(SasError::runtime(format!(
                            "Unknown collating sequence '{other}' for SORTSEQ=. \
                             Supported values: ASCII, LINGUISTIC."
                        )));
                    }
                };
                true
            }
            _ => false,
        })
    })?;

    // --- sub-statements : BY and KEY= until run;/quit; (combinateur M31) ---
    let mut by: Vec<(String, bool)> = Vec::new();
    let mut key_vars: Vec<(String, bool)> = Vec::new();
    let mut saw_by = false;
    let mut saw_key = false;

    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "by" => {
                ts.next(); // consume "by"
                saw_by = true;
                by.extend(common::parse_by(ts)?);
                true
            }
            "key" => {
                saw_key = true;
                let keys = parse_key_statement(ts)?;
                key_vars.extend(keys);
                true
            }
            _ => false,
        })
    })?;

    // KEY= présent → remplace BY (comportement SAS 9.4 : KEY prend le dessus).
    let effective_keys = if saw_key {
        key_vars
    } else {
        by
    };

    if !saw_by && !saw_key {
        return Err(SasError::runtime(
            "No BY statement was specified for PROC SORT.",
        ));
    }
    if effective_keys.is_empty() {
        return Err(SasError::runtime(
            "The BY statement must specify at least one variable.",
        ));
    }

    Ok(SortAst {
        data,
        out,
        by: effective_keys,
        nodupkey,
        noduprecs,
        tagsort,
        sortseq,
    })
}

/// Execute PROC SORT. Called by `procs::execute_proc` which wraps with timing.
pub fn execute(ast: &SortAst, session: &mut Session) -> Result<()> {
    // SORTSEQ=LINGUISTIC : collation simplifiée (même que ASCII ici — sas_cmp
    // ordre binaire UTF-8). Note de simplification émise pour transparence.
    if ast.sortseq == SortSeq::Linguistic {
        session.log.note(
            "SORTSEQ=LINGUISTIC: linguistic collation not fully implemented; \
             falling back to binary (sas_cmp) ordering.",
        );
    }
    // TAGSORT ignoré sémantiquement (performance hint uniquement).
    let in_ref = common::resolve_last_dataset(&ast.data, session)?;
    let ds = common::open_resolved(&in_ref, session)?;

    let n_obs = ds.n_obs();

    // Resolve BY variable column indices (validate existence).
    // (n_obs used below to size the index vector.)
    let mut key_cols: Vec<usize> = Vec::with_capacity(ast.by.len());
    for (vname, _) in &ast.by {
        let idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(vname));
        match idx {
            Some(i) => key_cols.push(i),
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    vname.to_uppercase()
                )));
            }
        }
    }

    // Decode each key column ONCE into Vec<Value>.
    let key_values: Vec<Vec<Value>> = key_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;
    let descending: Vec<bool> = ast.by.iter().map(|(_, d)| *d).collect();

    // Stable sort of an index vector via sas_cmp, key by key.
    // sas_cmp already encodes the SAS missing collation (`._ < . < .A.. <
    // numbers`), so no companion rank column is needed.
    let mut order: Vec<usize> = (0..n_obs).collect();
    order.sort_by(|&a, &b| {
        for (k, key) in key_values.iter().enumerate() {
            let mut c = key[a].sas_cmp(&key[b]);
            if descending[k] {
                c = c.reverse();
            }
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    });

    // Reorder ALL columns in a single take.
    let idx_ca = IdxCa::from_vec(
        "".into(),
        order.iter().map(|&i| i as IdxSize).collect::<Vec<_>>(),
    );
    let sorted_df = ds.df.take(&idx_ca)?;

    // Build the post-sort dataset (keep input VarMeta verbatim).
    let mut result = SasDataset {
        df: sorted_df,
        vars: ds.vars.clone(),
    };

    // NODUPKEY / NODUPRECS deduplication (after sort).
    let mut deleted = 0usize;
    if ast.nodupkey || ast.noduprecs {
        // Re-decode the sorted columns we need for comparison.
        let cmp_cols: Vec<usize> = if ast.noduprecs {
            (0..result.vars.len()).collect()
        } else {
            key_cols.clone()
        };
        let cols: Vec<Vec<Value>> = cmp_cols
            .iter()
            .map(|&ci| decode_column(&result, ci))
            .collect::<Result<_>>()?;

        let n = result.n_obs();
        let mut keep: Vec<IdxSize> = Vec::with_capacity(n);
        for row in 0..n {
            let dup = row > 0
                && cols.iter().all(|c| {
                    let prev = row - 1;
                    c[prev].sas_cmp(&c[row]) == Ordering::Equal
                });
            if dup {
                deleted += 1;
            } else {
                keep.push(row as IdxSize);
            }
        }
        if deleted > 0 {
            let keep_ca = IdxCa::from_vec("".into(), keep);
            result.df = result.df.take(&keep_ca)?;
        }
    }

    // Determine output destination.
    let out_ref = ast.out.clone().unwrap_or_else(|| in_ref.clone());
    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();

    let out_provider = session.libs.get(&out_libref)?;
    out_provider.write(&out_table, &result)?;
    session.last_dataset = Some(format!("{out_libref}.{out_table}"));

    // NOTE de déduplication (pluriel invariable).
    if deleted > 0 {
        if ast.nodupkey {
            session.log.note(&format!(
                "{deleted} observations with duplicate key values were deleted."
            ));
        } else {
            session.log.note(&format!(
                "{deleted} duplicate observations were deleted."
            ));
        }
    }

    // PROC SORT n'émet pas de NOTE "observations read" en SAS standard ;
    // execute_proc ajoute la NOTE de timing "PROCEDURE SORT used".
    Ok(())
}

#[cfg(test)]
mod tests;
