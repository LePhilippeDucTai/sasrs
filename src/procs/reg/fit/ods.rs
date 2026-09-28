//! J08-P3 — capture ODS OUTPUT des objets REG.
//!
//! Les trois tables du rapport de modèle portent leurs noms d'objets ODS SAS
//! documentés (« ODS Table Names », SAS/STAT UG PROC REG) :
//!
//! - `ANOVA` : colonnes Dependent, Model, Source, DF, SumOfSquares,
//!   MeanSquare, FValue, ProbF — une ligne par Source (Model, Error, total) ;
//! - `FitStatistics` : colonnes Dependent, Model, Label1, cValue1, nValue1,
//!   Label2, cValue2, nValue2 — une ligne par paire du listing (cValue =
//!   valeur affichée, nValue = valeur numérique pleine précision) ;
//! - `ParameterEstimates` : colonnes Dependent, Model, Variable, DF, Estimate,
//!   StdErr, tValue, Probt — une ligne par paramètre (intercept compris),
//!   plus les lignes RESTRICT (DF = −1).
//!
//! La capture est indépendante de l'affichage : elle a lieu même sous NOPRINT
//! et n'est pas filtrée par ODS SELECT/EXCLUDE (doc SAS — l'exclusion ne
//! gouverne que l'affichage). Plusieurs MODEL / dépendants s'empilent dans le
//! même dataset (union diagonale M38.3).

use super::*;

/// Capture les trois objets ODS du modèle courant si un `ODS OUTPUT` actif
/// les demande. No-op total sinon (chemin par défaut inchangé).
#[allow(clippy::too_many_arguments)]
pub(super) fn capture_ods_outputs(
    session: &mut Session,
    dep_name: &str,
    model_label: &str,
    reg_names: &[String],
    intercept: bool,
    stats: &AnovaStats,
    sse: f64,
    beta: &[f64],
    se_beta: &[f64],
    t_beta: &[f64],
    p_beta: &[f64],
    p_eff: usize,
    restricted: Option<&Restricted>,
) -> Result<()> {
    let want_anova = session.ods_output_active("ANOVA");
    let want_fit = session.ods_output_active("FitStatistics");
    let want_pe = session.ods_output_active("ParameterEstimates");
    if !(want_anova || want_fit || want_pe) {
        return Ok(());
    }
    // `Model` du dataset SAS : le libellé nu « MODELn » (sans le préfixe
    // d'affichage « Model: »).
    let bare_model = model_label.strip_prefix("Model: ").unwrap_or(model_label);

    if want_anova {
        let part = build_anova_part(dep_name, bare_model, stats, sse)?;
        session.append_ods_output("ANOVA", part)?;
    }
    if want_fit {
        let part = build_fit_stats_part(dep_name, bare_model, stats)?;
        session.append_ods_output("FitStatistics", part)?;
    }
    if want_pe {
        let part = build_pe_part(
            dep_name, bare_model, reg_names, intercept, beta, se_beta, t_beta, p_beta, p_eff,
            restricted,
        )?;
        session.append_ods_output("ParameterEstimates", part)?;
    }
    Ok(())
}

/// Tranche typée de la table ODS « ANOVA ».
fn build_anova_part(
    dep_name: &str,
    model: &str,
    stats: &AnovaStats,
    sse: f64,
) -> Result<SasDataset> {
    let &AnovaStats {
        ssm,
        sst,
        model_df,
        error_df,
        total_df,
        total_label,
        msm,
        mse,
        f_stat,
        p_f,
        ..
    } = stats;
    // (Source, DF, SS, MS, F, Prob)
    #[allow(clippy::type_complexity)]
    let rows: Vec<(&str, f64, f64, Option<f64>, Option<f64>, Option<f64>)> = vec![
        ("Model", model_df, ssm, Some(msm), Some(f_stat), Some(p_f)),
        ("Error", error_df, sse, Some(mse), None, None),
        (total_label, total_df, sst, None, None, None),
    ];
    let n = rows.len();
    let dep_col: Vec<Option<String>> = vec![Some(dep_name.to_string()); n];
    let model_col: Vec<Option<String>> = vec![Some(model.to_string()); n];
    let source: Vec<Option<String>> = rows.iter().map(|r| Some(r.0.to_string())).collect();
    let df: Vec<Option<f64>> = rows.iter().map(|r| Some(r.1)).collect();
    let ss: Vec<Option<f64>> = rows.iter().map(|r| Some(r.2)).collect();
    let ms: Vec<Option<f64>> = rows.iter().map(|r| r.3).collect();
    let fv: Vec<Option<f64>> = rows.iter().map(|r| r.4).collect();
    let prob: Vec<Option<f64>> = rows.iter().map(|r| r.5).collect();

    let columns: Vec<Column> = vec![
        Series::new("Dependent".into(), dep_col).into(),
        Series::new("Model".into(), model_col).into(),
        Series::new("Source".into(), source).into(),
        Series::new("DF".into(), df).into(),
        Series::new("SumOfSquares".into(), ss).into(),
        Series::new("MeanSquare".into(), ms).into(),
        Series::new("FValue".into(), fv).into(),
        Series::new("ProbF".into(), prob).into(),
    ];
    let vars = vec![
        char_var_meta("Dependent", 32),
        char_var_meta("Model", 8),
        char_var_meta("Source", 16),
        num_var_meta("DF"),
        num_var_meta("SumOfSquares"),
        num_var_meta("MeanSquare"),
        num_var_meta("FValue"),
        num_var_meta("ProbF"),
    ];
    let df = DataFrame::new(columns)?;
    Ok(SasDataset { df, vars })
}

/// Tranche typée de la table ODS « FitStatistics » : paires
/// (Label, cValue, nValue) horizontales, une ligne par ligne du listing.
#[allow(clippy::type_complexity)]
fn build_fit_stats_part(dep_name: &str, model: &str, stats: &AnovaStats) -> Result<SasDataset> {
    let &AnovaStats {
        y_mean,
        r2,
        adj_r2,
        root_mse,
        cv,
        ..
    } = stats;
    let rows: Vec<(String, String, f64, Option<(String, String, f64)>)> = vec![
        (
            "Root MSE".into(),
            fmt5(root_mse),
            root_mse,
            Some(("R-Square".into(), fmt_fit4(r2), r2)),
        ),
        (
            "Dependent Mean".into(),
            fmt5(y_mean),
            y_mean,
            Some(("Adj R-Sq".into(), fmt_fit4(adj_r2), adj_r2)),
        ),
        ("Coeff Var".into(), fmt5(cv), cv, None),
    ];
    let n = rows.len();
    let dep_col: Vec<Option<String>> = vec![Some(dep_name.to_string()); n];
    let model_col: Vec<Option<String>> = vec![Some(model.to_string()); n];
    let label1: Vec<Option<String>> = rows.iter().map(|r| Some(r.0.clone())).collect();
    let cvalue1: Vec<Option<String>> = rows.iter().map(|r| Some(r.1.clone())).collect();
    let nvalue1: Vec<Option<f64>> = rows.iter().map(|r| Some(r.2)).collect();
    let label2: Vec<Option<String>> = rows
        .iter()
        .map(|r| r.3.as_ref().map(|p| p.0.clone()))
        .collect();
    let cvalue2: Vec<Option<String>> = rows
        .iter()
        .map(|r| r.3.as_ref().map(|p| p.1.clone()))
        .collect();
    let nvalue2: Vec<Option<f64>> = rows.iter().map(|r| r.3.as_ref().map(|p| p.2)).collect();

    let char_len = |vals: &[Option<String>]| {
        vals.iter()
            .flatten()
            .map(|s| s.len())
            .max()
            .unwrap_or(1)
            .max(1)
    };
    let columns: Vec<Column> = vec![
        Series::new("Dependent".into(), dep_col).into(),
        Series::new("Model".into(), model_col).into(),
        Series::new("Label1".into(), label1.clone()).into(),
        Series::new("cValue1".into(), cvalue1.clone()).into(),
        Series::new("nValue1".into(), nvalue1).into(),
        Series::new("Label2".into(), label2.clone()).into(),
        Series::new("cValue2".into(), cvalue2.clone()).into(),
        Series::new("nValue2".into(), nvalue2).into(),
    ];
    let vars = vec![
        char_var_meta("Dependent", 32),
        char_var_meta("Model", 8),
        char_var_meta("Label1", char_len(&label1)),
        char_var_meta("cValue1", char_len(&cvalue1)),
        num_var_meta("nValue1"),
        char_var_meta("Label2", char_len(&label2)),
        char_var_meta("cValue2", char_len(&cvalue2)),
        num_var_meta("nValue2"),
    ];
    let df = DataFrame::new(columns)?;
    Ok(SasDataset { df, vars })
}

/// Tranche typée de la table ODS « ParameterEstimates ».
#[allow(clippy::too_many_arguments)]
fn build_pe_part(
    dep_name: &str,
    model: &str,
    reg_names: &[String],
    intercept: bool,
    beta: &[f64],
    se_beta: &[f64],
    t_beta: &[f64],
    p_beta: &[f64],
    p_eff: usize,
    restricted: Option<&Restricted>,
) -> Result<SasDataset> {
    // (Variable, DF, Estimate, StdErr, tValue, Probt)
    let mut rows: Vec<(String, f64, f64, f64, f64, f64)> = Vec::with_capacity(p_eff + 1);
    for j in 0..p_eff {
        let var_name = if intercept {
            if j == 0 {
                "Intercept".to_string()
            } else {
                reg_names[j - 1].clone()
            }
        } else {
            reg_names[j].clone()
        };
        rows.push((var_name, 1.0, beta[j], se_beta[j], t_beta[j], p_beta[j]));
    }
    // Lignes RESTRICT : DF = −1 (SAS), Estimate = λ_i.
    if let Some(r) = restricted {
        for (_label, lam, se, t, pv) in &r.lambda_rows {
            rows.push(("RESTRICT".into(), -1.0, *lam, *se, *t, *pv));
        }
    }

    let n = rows.len();
    let dep_col: Vec<Option<String>> = vec![Some(dep_name.to_string()); n];
    let model_col: Vec<Option<String>> = vec![Some(model.to_string()); n];
    let variable: Vec<Option<String>> = rows.iter().map(|r| Some(r.0.clone())).collect();
    let df: Vec<Option<f64>> = rows.iter().map(|r| Some(r.1)).collect();
    let estimate: Vec<Option<f64>> = rows.iter().map(|r| Some(r.2)).collect();
    let stderr: Vec<Option<f64>> = rows.iter().map(|r| Some(r.3)).collect();
    let tvalue: Vec<Option<f64>> = rows.iter().map(|r| Some(r.4)).collect();
    let probt: Vec<Option<f64>> = rows.iter().map(|r| Some(r.5)).collect();

    let var_len = variable
        .iter()
        .flatten()
        .map(|s| s.len())
        .max()
        .unwrap_or(1)
        .max(8);
    let columns: Vec<Column> = vec![
        Series::new("Dependent".into(), dep_col).into(),
        Series::new("Model".into(), model_col).into(),
        Series::new("Variable".into(), variable).into(),
        Series::new("DF".into(), df).into(),
        Series::new("Estimate".into(), estimate).into(),
        Series::new("StdErr".into(), stderr).into(),
        Series::new("tValue".into(), tvalue).into(),
        Series::new("Probt".into(), probt).into(),
    ];
    let vars = vec![
        char_var_meta("Dependent", 32),
        char_var_meta("Model", 8),
        char_var_meta("Variable", var_len.max(32)),
        num_var_meta("DF"),
        num_var_meta("Estimate"),
        num_var_meta("StdErr"),
        num_var_meta("tValue"),
        num_var_meta("Probt"),
    ];
    let df = DataFrame::new(columns)?;
    Ok(SasDataset { df, vars })
}
