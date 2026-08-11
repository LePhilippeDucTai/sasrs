//! Fast-path vectorisé OPTIONNEL des étapes DATA simples (`Session.vectorize`,
//! OFF par défaut). Au lieu de la boucle implicite ligne-à-ligne de `exec.rs`,
//! l'étape est traduite en un `LazyFrame` Polars (with_columns + collect).
//!
//! # Contrat : ÉQUIVALENCE avec le chemin ligne-à-ligne
//! Le fast-path ne s'active QUE pour les étapes que [`eligible`] prouve
//! traduisibles sans divergence (sinon `exec::execute` conserve le chemin
//! normal — le fast-path est une pure optimisation, jamais un changement de
//! sémantique). Périmètre v1, volontairement étroit :
//!   - UN seul dataset en entrée via SET, **sans** BY / MERGE / WHERE= / IN= ;
//!   - statements : uniquement assignations à cible NUMÉRIQUE + déclaratifs
//!     sans effet d'exécution (KEEP/DROP/FORMAT/LABEL/ATTRIB) ;
//!   - **pas** de subsetting IF, IF-THEN, DO, OUTPUT explicite, RETAIN, sum
//!     statement, LENGTH, ARRAY (→ repli sur la boucle) ;
//!   - **une seule** sortie, output implicite, aucune variable non initialisée,
//!     aucune valeur initiale (RETAIN/sum) ;
//!   - expressions limitées à : littéral numérique, COPIE de variable, et
//!     `+` `-` `*`. Pas de `/` ni `**` (division par zéro / racine complexe :
//!     sémantique SAS divergente), pas de fonctions, comparaisons, `||`.
//!   - L'appelant exige en plus `FIRSTOBS=1` et `OBS=MAX` (fenêtre d'entrée
//!     pleine), sinon repli.
//!
//! # Pièges respectés (PLAN.md §Checklist)
//!   - **Missings spéciaux** : une COPIE nue `y = x` préserve le NaN-payload
//!     (`.A`) via `col(x)` BRUT ; un opérande d'ARITHMÉTIQUE est neutralisé en
//!     place (`when(is_nan).then(null)`) car SAS rend `.` (ordinaire) dès qu'un
//!     missing entre dans un calcul. La colonne stockée n'est jamais mutée.
//!   - **NOTE "Missing values were generated..."** : émise ssi un opérande
//!     d'arithmétique est missing sur AU MOINS une ligne — équivaut au
//!     compteur `missing_generated > 0` de `exec.rs`. Calculée par un OU
//!     booléen capturé AVANT chaque réassignation (sémantique séquentielle).
//!   - **Ordre des NOTEs** identique à `exec::execute` : (missing generated) →
//!     "N observations read" → "has N observations and M variables".

use super::StepProgram;
use super::exec::{self, StepStats};
use super::pdv::Pdv;
use crate::ast::{BinaryOp, DsStmt, Expr};
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::session::Session;
use crate::value::VarType;
use polars::prelude::*;

/// Vrai si l'étape compilée est dans le périmètre du fast-path (cf. en-tête).
/// N'examine QUE la structure ; l'appelant ajoute les conditions de session
/// (FIRSTOBS=/OBS=).
pub fn eligible(prog: &StepProgram) -> bool {
    // Une seule entrée SET simple. Plusieurs statements SET (M40.2 :
    // `extra_inputs` non vide) = lecture parallèle multi-curseurs, hors
    // périmètre vectorisé.
    let Some(input) = &prog.input else {
        return false;
    };
    if !prog.extra_inputs.is_empty()
        || input.datasets.len() != 1
        || !input.by.is_empty()
        || input.merge
        || !input.in_flags.is_empty()
        || input.datasets[0].where_.is_some()
        // Options de niveau statement (M16.4) : END= modifie le PDV par
        // itération, NOBS= ajoute une affectation pré-boucle, POINT= remplace
        // la boucle implicite — toutes hors du périmètre vectorisé.
        || input.end_var.is_some()
        || input.nobs_slot.is_some()
        || input.point_slot.is_some()
    {
        return false;
    }
    // Une seule sortie, output implicite, rien de retenu/non initialisé.
    if prog.outputs.len() != 1
        || prog.has_explicit_output
        || !prog.uninitialized.is_empty()
        || !prog.initial_values.is_empty()
        || !prog.arrays.is_empty()
        // Objets hash (M17.1) : DECLARE/define* opèrent par itération, hors
        // périmètre vectorisé.
        || !prog.hash_objects.is_empty()
    {
        return false;
    }
    // Chaque statement doit être traduisible.
    prog.stmts.iter().all(|s| stmt_ok(s, &prog.pdv))
}

/// Un statement est-il dans le périmètre ? (SET = déclaration d'entrée, ignoré
/// à l'exécution ; déclaratifs sans effet ; assignation numérique lowerable.)
fn stmt_ok(stmt: &DsStmt, pdv: &Pdv) -> bool {
    match stmt {
        // L'entrée (déjà matérialisée) et les déclaratifs purs : aucun effet à
        // l'exécution dans le chemin ligne-à-ligne — sans risque d'ignorer ici.
        // INFORMAT (M40.3) est déclaratif (il n'agit qu'à travers un INPUT,
        // lui-même hors périmètre). Un WHERE statement, lui, tombe dans le
        // `_ => false` — de toute façon `eligible()` a déjà rejeté l'étape
        // (le WHERE est replié dans le `where_` du dataset d'entrée).
        DsStmt::Set { .. }
        | DsStmt::Keep(_)
        | DsStmt::Drop(_)
        | DsStmt::Format(_)
        | DsStmt::Informat(_)
        | DsStmt::Label(_)
        | DsStmt::Attrib(_) => true,
        DsStmt::Assign { var, expr } => {
            // Cible numérique existante + RHS entièrement lowerable.
            match pdv.slot(var) {
                Some(slot) if pdv.vars()[slot].ty == VarType::Num => lower_rhs(expr, pdv).is_some(),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Traduit le RHS d'une assignation. Cas spécial : une COPIE NUE `y = x` rend
/// `col(x)` brut (préserve le NaN-payload des missings spéciaux), au lieu de le
/// neutraliser comme un opérande d'arithmétique.
fn lower_rhs(expr: &Expr, pdv: &Pdv) -> Option<polars::prelude::Expr> {
    if let Expr::Var(v) = expr {
        let slot = pdv.slot(v)?;
        if pdv.vars()[slot].ty != VarType::Num {
            return None;
        }
        return Some(col(pdv.vars()[slot].name.as_str()));
    }
    lower_num(expr, pdv)
}

/// Traduit une expression numérique en expression Polars « null-safe » : tout
/// opérande variable est neutralisé (`NaN → null`) pour que la propagation
/// `null` de Polars reproduise la propagation `.` de SAS. Renvoie `None` pour
/// toute forme hors périmètre (→ repli sur la boucle).
fn lower_num(expr: &Expr, pdv: &Pdv) -> Option<polars::prelude::Expr> {
    match expr {
        Expr::Num(n) => Some(lit(*n)),
        Expr::Var(v) => {
            let slot = pdv.slot(v)?;
            if pdv.vars()[slot].ty != VarType::Num {
                return None;
            }
            let name = pdv.vars()[slot].name.as_str();
            // Neutralise les missings spéciaux (NaN-payload) → null.
            Some(
                when(col(name).is_nan())
                    .then(lit(NULL))
                    .otherwise(col(name)),
            )
        }
        Expr::Binary { op, left, right } => {
            let l = lower_num(left, pdv)?;
            let r = lower_num(right, pdv)?;
            match op {
                BinaryOp::Add => Some(l + r),
                BinaryOp::Sub => Some(l - r),
                BinaryOp::Mul => Some(l * r),
                // Div / Power / Concat / comparaisons / logique : divergence ou
                // hors périmètre.
                _ => None,
            }
        }
        // Littéraux chaîne, missings littéraux, unaires, fonctions, IN, index :
        // hors périmètre v1.
        _ => None,
    }
}

/// Accumule, pour CHAQUE opération arithmétique de `expr`, le booléen « un
/// opérande est missing » (= `lower_num(operande).is_null()`). L'OU de tous ces
/// drapeaux, agrégé sur les lignes, reproduit `missing_generated > 0`.
fn collect_arith_flags(expr: &Expr, pdv: &Pdv, acc: &mut Vec<polars::prelude::Expr>) -> Option<()> {
    if let Expr::Binary { op, left, right } = expr
        && matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul)
    {
        let l = lower_num(left, pdv)?;
        let r = lower_num(right, pdv)?;
        acc.push(l.is_null().or(r.is_null()));
        collect_arith_flags(left, pdv, acc)?;
        collect_arith_flags(right, pdv, acc)?;
    }
    Some(())
}

/// Exécute l'étape par le fast-path. Préconditions : [`eligible`] vraie (et
/// fenêtre d'entrée pleine, vérifiée par l'appelant).
pub fn run(prog: StepProgram, session: &mut Session) -> Result<StepStats> {
    let StepProgram {
        pdv,
        stmts,
        input,
        outputs,
        labels,
        ..
    } = prog;

    let input = input.expect("eligible() garantit une entrée");
    let ds0 = &input.datasets[0];
    let n_rows = ds0.n_rows;

    // 1. Frame de base : colonnes d'entrée (typées par le PDV) + colonnes des
    // variables créées par assignation, initialisées à missing (null) comme la
    // remise à blanc des non-retenues en début d'itération SAS.
    let mut from_input = vec![false; pdv.vars().len()];
    let mut columns: Vec<Column> = Vec::with_capacity(pdv.vars().len());
    for (ci, &slot) in ds0.var_slots.iter().enumerate() {
        from_input[slot] = true;
        let v = &pdv.vars()[slot];
        columns.push(exec::column_from_values(
            &v.name,
            v.ty,
            ds0.columns[ci].iter(),
        ));
    }
    // Variables créées par assignation (jamais en entrée) : colonne null f64.
    // (eligible() garantit qu'elles sont numériques et qu'il n'y a pas de
    // variable seulement référencée — donc aucune colonne manquante.)
    for (slot, v) in pdv.vars().iter().enumerate() {
        if !from_input[slot] {
            let s = Float64Chunked::full_null(v.name.as_str().into(), n_rows).into_series();
            columns.push(s.into());
        }
    }
    let base = DataFrame::new(columns)?;
    let mut lf = base.lazy();

    // 2. Drapeau "missing généré" : booléen courant, mis à jour AVANT chaque
    // assignation (capture des opérandes à leur état pré-réassignation).
    lf = lf.with_column(lit(false).alias("__mg"));

    for stmt in &stmts {
        if let DsStmt::Assign { var, expr } = stmt {
            let slot = pdv
                .slot(var)
                .ok_or_else(|| SasError::runtime(format!("Variable {var} is not addressable.")))?;
            let name = pdv.vars()[slot].name.clone();

            // a. Capturer le missing-généré de cette assignation AVANT de
            // réassigner (les opérandes voient l'état courant des colonnes).
            let mut flags = Vec::new();
            collect_arith_flags(expr, &pdv, &mut flags)
                .ok_or_else(|| SasError::runtime("fastpath: RHS non traduisible"))?;
            if let Some(or_flag) = flags.into_iter().reduce(|a, b| a.or(b)) {
                lf = lf.with_column((col("__mg").or(or_flag)).alias("__mg"));
            }

            // b. Appliquer l'assignation.
            let rhs = lower_rhs(expr, &pdv)
                .ok_or_else(|| SasError::runtime("fastpath: RHS non traduisible"))?;
            lf = lf.with_column(rhs.alias(name.as_str()));
        }
    }

    // 3. Matérialiser une seule fois.
    let result = lf.collect()?;

    // 4. NOTE "missing generated" (ordre : avant les lectures, comme exec.rs).
    let mg = result
        .column("__mg")?
        .as_materialized_series()
        .bool()?
        .any();
    if mg {
        session.log.note(
            "Missing values were generated as a result of performing an operation on missing values.",
        );
    }

    let mut stats = StepStats {
        read: Vec::new(),
        written: Vec::new(),
    };

    // 5. "N observations read" — pas de WHERE= : toutes les lignes sont lues.
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_rows, ds0.display
    ));
    stats.read.push((ds0.display.clone(), n_rows));

    // 6. Écriture de la sortie unique (projection des kept_slots en ordre PDV,
    // renommage RENAME= par out_names, métadonnées PDV + labels).
    let spec = &outputs[0];
    let mut out_cols: Vec<Column> = Vec::with_capacity(spec.kept_slots.len());
    let mut vars: Vec<VarMeta> = Vec::with_capacity(spec.kept_slots.len());
    for (slot, out_name) in spec.kept_slots.iter().zip(&spec.out_names) {
        let v = &pdv.vars()[*slot];
        let series = result
            .column(v.name.as_str())?
            .as_materialized_series()
            .clone()
            .with_name(out_name.as_str().into());
        out_cols.push(series.into());
        vars.push(VarMeta {
            name: out_name.clone(),
            ty: v.ty,
            length: v.length,
            format: v.format.clone(),
            label: labels.get(&v.name.to_uppercase()).cloned(),
        });
    }
    let n_out = out_cols.first().map_or(n_rows, |c| c.len());
    let df = DataFrame::new(out_cols)?;
    let ds = SasDataset { df, vars };
    exec::write_dataset_with_note(
        session,
        &spec.libref,
        &spec.table,
        &spec.display,
        &ds,
        n_out,
        Some(&mut stats),
    )?;

    Ok(stats)
}

#[cfg(test)]
mod tests;
