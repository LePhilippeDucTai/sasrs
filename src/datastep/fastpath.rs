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

use super::pdv::Pdv;
use super::StepProgram;
use super::exec::StepStats;
use crate::ast::{BinaryOp, DsStmt, Expr};
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::missing::value_to_num;
use crate::session::Session;
use crate::value::{Value, VarType};
use polars::prelude::*;

/// Vrai si l'étape compilée est dans le périmètre du fast-path (cf. en-tête).
/// N'examine QUE la structure ; l'appelant ajoute les conditions de session
/// (FIRSTOBS=/OBS=).
pub fn eligible(prog: &StepProgram) -> bool {
    // Une seule entrée SET simple.
    let Some(input) = &prog.input else {
        return false;
    };
    if input.datasets.len() != 1
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
        DsStmt::Set { .. }
        | DsStmt::Keep(_)
        | DsStmt::Drop(_)
        | DsStmt::Format(_)
        | DsStmt::Label(_)
        | DsStmt::Attrib(_) => true,
        DsStmt::Assign { var, expr } => {
            // Cible numérique existante + RHS entièrement lowerable.
            match pdv.slot(var) {
                Some(slot) if pdv.vars()[slot].ty == VarType::Num => {
                    lower_rhs(expr, pdv).is_some()
                }
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
        let name = pdv.vars()[slot].name.as_str();
        let series = match pdv.vars()[slot].ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> =
                    ds0.columns[ci].iter().map(value_to_num).collect();
                Series::new(name.into(), vals)
            }
            VarType::Char => {
                let vals: Vec<String> = ds0.columns[ci]
                    .iter()
                    .map(|v| match v {
                        Value::Char(s) => s.clone(),
                        _ => String::new(),
                    })
                    .collect();
                Series::new(name.into(), vals)
            }
        };
        columns.push(series.into());
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
    session.libs.get(&spec.libref)?.write(&spec.table, &ds)?;
    session.last_dataset = Some(spec.display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        spec.display,
        n_out,
        spec.kept_slots.len()
    ));
    stats
        .written
        .push((spec.display.clone(), n_out, spec.kept_slots.len()));

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datastep::compile;
    use crate::datastep::exec::execute;
    use crate::missing::encode_special;
    use crate::parser::StatementStream;
    use crate::source::SourceFile;
    use crate::value::MissingKind;
    use polars::df;
    use std::path::PathBuf;

    fn session(vectorize: bool) -> Session {
        let mut s = Session::new(None, PathBuf::from("."), true).unwrap();
        s.vectorize = vectorize;
        s
    }

    /// Entrée avec un missing ordinaire (`.`) ET un spécial (`.A`) en numérique,
    /// plus une colonne caractère — pour éprouver la propagation des missings
    /// et la préservation du NaN-payload.
    fn write_input(session: &Session) {
        let df = df!(
            "Age" => [
                Some(14.0),
                None,
                Some(encode_special(MissingKind::Letter(0))), // .A
                Some(13.0),
            ],
            "Name" => ["Alfred", "Alice", "Carol", "Barbara"],
        )
        .unwrap();
        let vars = vec![
            VarMeta { name: "Age".into(), ty: VarType::Num, length: 8, format: None, label: None },
            VarMeta { name: "Name".into(), ty: VarType::Char, length: 7, format: None, label: None },
        ];
        session
            .libs
            .get("WORK")
            .unwrap()
            .write("inp", &SasDataset { df, vars })
            .unwrap();
    }

    fn parse_compile(src: &str, session: &mut Session) -> StepProgram {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        let ast = crate::parser::datastep::parse_data_step(&mut ts).unwrap();
        compile(&ast, session).unwrap()
    }

    /// Exécute `src` sur une session neuve (fast-path selon `vectorize`) et
    /// renvoie (sortie WORK.OUT, log). Le log d'`execute` ne contient que des
    /// NOTEs (l'écho est fait par l'exécuteur de haut niveau), donc les deux
    /// chemins sont directement comparables.
    fn run_capture(src: &str, vectorize: bool) -> (SasDataset, String) {
        let mut s = session(vectorize);
        write_input(&s);
        let prog = parse_compile(src, &mut s);
        if vectorize {
            assert!(eligible(&prog), "étape attendue éligible au fast-path : {src}");
        }
        execute(prog, &mut s).unwrap();
        let out = s.libs.get("WORK").unwrap().read("out").unwrap().0;
        (out, s.log.into_string())
    }

    fn is_eligible(src: &str) -> bool {
        let mut s = session(true);
        write_input(&s);
        let prog = parse_compile(src, &mut s);
        eligible(&prog)
    }

    /// Compare deux sorties colonne par colonne — les f64 comparés BIT À BIT
    /// pour distinguer `.` (null) de `.A` (NaN-payload) et de tout nombre.
    fn assert_same_output(a: &SasDataset, b: &SasDataset) {
        assert_eq!(a.df.width(), b.df.width(), "largeur différente");
        assert_eq!(a.n_obs(), b.n_obs(), "nb obs différent");
        for (ca, cb) in a.df.get_columns().iter().zip(b.df.get_columns()) {
            assert_eq!(ca.name(), cb.name(), "nom de colonne");
            assert_eq!(ca.dtype(), cb.dtype(), "dtype de {}", ca.name());
            match ca.dtype() {
                DataType::Float64 => {
                    let fa = ca.f64().unwrap();
                    let fb = cb.f64().unwrap();
                    for i in 0..fa.len() {
                        match (fa.get(i), fb.get(i)) {
                            (None, None) => {}
                            (Some(x), Some(y)) => assert_eq!(
                                x.to_bits(),
                                y.to_bits(),
                                "col {} ligne {i} (bits)",
                                ca.name()
                            ),
                            _ => panic!("null/non-null divergent col {} ligne {i}", ca.name()),
                        }
                    }
                }
                DataType::String => {
                    let sa = ca.str().unwrap();
                    let sb = cb.str().unwrap();
                    for i in 0..sa.len() {
                        assert_eq!(sa.get(i), sb.get(i), "col {} ligne {i}", ca.name());
                    }
                }
                other => panic!("dtype inattendu {other}"),
            }
        }
    }

    // ── Équivalence fast-path ⇔ boucle ligne-à-ligne ────────────────────────

    #[test]
    fn equivalence_arithmetic_with_missings() {
        let src = "data out; set inp; x = age * 2; run;";
        let (off, log_off) = run_capture(src, false);
        let (on, log_on) = run_capture(src, true);
        assert_same_output(&off, &on);
        assert_eq!(log_off, log_on, "logs divergents");
        // Sanity : x = age*2, missing propagé pour `.` ET `.A` ; NOTE missing.
        let x = on.df.column("x").unwrap().f64().unwrap();
        assert_eq!(x.get(0), Some(28.0));
        assert_eq!(x.get(1), None);
        assert_eq!(x.get(2), None);
        assert_eq!(x.get(3), Some(26.0));
        assert!(log_on.contains("Missing values were generated"));
    }

    #[test]
    fn equivalence_copy_preserves_special_missing() {
        let src = "data out; set inp; y = age; run;";
        let (off, log_off) = run_capture(src, false);
        let (on, log_on) = run_capture(src, true);
        assert_same_output(&off, &on);
        assert_eq!(log_off, log_on);
        // La copie nue préserve le payload .A et ne génère AUCUN missing.
        let y = on.df.column("y").unwrap().f64().unwrap();
        assert_eq!(y.get(0), Some(14.0));
        assert_eq!(y.get(1), None);
        assert!(y.get(2).unwrap().is_nan());
        assert_eq!(
            y.get(2).unwrap().to_bits(),
            encode_special(MissingKind::Letter(0)).to_bits()
        );
        assert!(!log_on.contains("Missing values were generated"));
    }

    #[test]
    fn equivalence_sequential_dependency() {
        // y dépend de x assignée juste avant (ordre séquentiel).
        let src = "data out; set inp; x = age + 1; y = x * 2; run;";
        let (off, _) = run_capture(src, false);
        let (on, _) = run_capture(src, true);
        assert_same_output(&off, &on);
    }

    #[test]
    fn equivalence_literal_and_keep() {
        let src = "data out; set inp; flag = 1; keep Name flag; run;";
        let (off, log_off) = run_capture(src, false);
        let (on, log_on) = run_capture(src, true);
        assert_same_output(&off, &on);
        assert_eq!(log_off, log_on);
        assert_eq!(on.df.width(), 2); // Name, flag
        assert!(!log_on.contains("Missing values were generated"));
    }

    // ── Le garde-fou rejette tout ce qui sort du périmètre ──────────────────

    #[test]
    fn gate_rejects_subsetting_if() {
        assert!(!is_eligible("data out; set inp; if age > 13; run;"));
    }

    #[test]
    fn gate_rejects_division() {
        assert!(!is_eligible("data out; set inp; x = age / 2; run;"));
    }

    #[test]
    fn gate_rejects_char_assignment() {
        assert!(!is_eligible("data out; set inp; z = 'hi'; run;"));
    }

    #[test]
    fn gate_rejects_explicit_output() {
        assert!(!is_eligible("data out; set inp; output; run;"));
    }

    #[test]
    fn gate_rejects_no_input() {
        assert!(!is_eligible("data out; x = 1; run;"));
    }

    /// Avec le flag ON mais une étape NON éligible (subsetting IF), `execute`
    /// doit retomber sur la boucle ligne-à-ligne et rester correct.
    #[test]
    fn ineligible_step_falls_back_under_flag() {
        let src = "data out; set inp; if age > 13; run;";
        assert!(!is_eligible(src));
        let mut s = session(true);
        write_input(&s);
        let prog = parse_compile(src, &mut s);
        execute(prog, &mut s).unwrap();
        let out = s.libs.get("WORK").unwrap().read("out").unwrap().0;
        // age > 13 : seul Alfred (14) ; `.`, `.A` et 13 sont faux.
        assert_eq!(out.n_obs(), 1);
        assert_eq!(out.df.column("Name").unwrap().str().unwrap().get(0), Some("Alfred"));
    }
}
