use super::*;

/// Statistiques d'un score : `(test à 2 échantillons si k == 2, test k échantillons)`.
type ScorePair = (Option<ScoreTwoSample>, ScoreOneWay);

// ───────────────────────── OUT= dataset ─────────────────────────

/// One accumulated OUT= row (one VAR within one BY group). Statistics are
/// `None` for analyses not run.
pub(super) struct OutRow {
    pub(super) by_key: Vec<Value>,
    pub(super) var: String,
    /// Wilcoxon: (S_0, Z, P2, P1).
    pub(super) wil: Option<(f64, f64, f64, f64)>,
    /// Exact Wilcoxon: (XP1 = one-sided lower, XP2 = two-sided).
    pub(super) exact: Option<(f64, f64)>,
    /// Kruskal-Wallis: (chisq, df, p).
    pub(super) kw: Option<(f64, usize, f64)>,
    /// Median: (two-sample, one-way).
    pub(super) med: Option<(Option<ScoreTwoSample>, ScoreOneWay)>,
    /// Savage: (two-sample, one-way).
    pub(super) sav: Option<(Option<ScoreTwoSample>, ScoreOneWay)>,
    /// Van der Waerden: (two-sample, one-way).
    pub(super) vw: Option<(Option<ScoreTwoSample>, ScoreOneWay)>,
}

impl OutRow {
    pub(super) fn new(by_key: Vec<Value>, var: String) -> Self {
        OutRow {
            by_key,
            var,
            wil: None,
            exact: None,
            kw: None,
            med: None,
            sav: None,
            vw: None,
        }
    }
}

/// Build and persist the OUT= dataset, set `_LAST_`, and emit the creation NOTE.
pub(super) fn write_out_dataset(
    session: &mut Session,
    target: &DatasetRef,
    by_names: &[String],
    rows: &[OutRow],
) -> Result<()> {
    let n_rows = rows.len();

    // Determine which statistic column families are present across all rows.
    let any = |f: &dyn Fn(&OutRow) -> bool| rows.iter().any(f);
    let has_wil = any(&|r| r.wil.is_some());
    let has_exact = any(&|r| r.exact.is_some());
    let has_kw = any(&|r| r.kw.is_some());
    let has_med = any(&|r| r.med.is_some());
    let has_sav = any(&|r| r.sav.is_some());
    let has_vw = any(&|r| r.vw.is_some());
    // Z columns only when a 2-sample statistic exists (k == 2).
    let has_med_z = any(&|r| r.med.as_ref().is_some_and(|(t, _)| t.is_some()));
    let has_sav_z = any(&|r| r.sav.as_ref().is_some_and(|(t, _)| t.is_some()));
    let has_vw_z = any(&|r| r.vw.as_ref().is_some_and(|(t, _)| t.is_some()));

    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // BY columns first (decoded as char display strings — faithful enough).
    for (bi, bname) in by_names.iter().enumerate() {
        let col: Vec<Option<String>> = rows
            .iter()
            .map(|r| Some(by_cell_string(&r.by_key[bi])))
            .collect();
        columns.push(Series::new(bname.as_str().into(), col).into());
        vars.push(char_var_meta(bname, 32));
    }

    // _VAR_ (char 32).
    let var_col: Vec<Option<String>> = rows.iter().map(|r| Some(r.var.clone())).collect();
    columns.push(Series::new("_VAR_".into(), var_col).into());
    vars.push(char_var_meta("_VAR_", 32));

    // Helper to push a numeric statistic column.
    let push_num = |columns: &mut Vec<Column>,
                    vars: &mut Vec<VarMeta>,
                    name: &str,
                    values: Vec<Option<f64>>| {
        columns.push(Series::new(name.into(), values).into());
        vars.push(num_var_meta(name));
    };

    let finite = |v: f64| if v.is_finite() { Some(v) } else { None };

    if has_wil {
        push_num(
            &mut columns,
            &mut vars,
            "_WIL_",
            rows.iter()
                .map(|r| r.wil.map(|w| w.0).and_then(finite))
                .collect(),
        );
        push_num(
            &mut columns,
            &mut vars,
            "Z_WIL",
            rows.iter()
                .map(|r| r.wil.map(|w| w.1).and_then(finite))
                .collect(),
        );
        push_num(
            &mut columns,
            &mut vars,
            "P2_WIL",
            rows.iter()
                .map(|r| r.wil.map(|w| w.2).and_then(finite))
                .collect(),
        );
        push_num(
            &mut columns,
            &mut vars,
            "P1_WIL",
            rows.iter()
                .map(|r| r.wil.map(|w| w.3).and_then(finite))
                .collect(),
        );
    }
    if has_exact {
        push_num(
            &mut columns,
            &mut vars,
            "XP1_WIL",
            rows.iter()
                .map(|r| r.exact.map(|e| e.0).and_then(finite))
                .collect(),
        );
        push_num(
            &mut columns,
            &mut vars,
            "XP2_WIL",
            rows.iter()
                .map(|r| r.exact.map(|e| e.1).and_then(finite))
                .collect(),
        );
    }
    if has_kw {
        push_num(
            &mut columns,
            &mut vars,
            "_KW_",
            rows.iter()
                .map(|r| r.kw.map(|w| w.0).and_then(finite))
                .collect(),
        );
        push_num(
            &mut columns,
            &mut vars,
            "DF_KW",
            rows.iter().map(|r| r.kw.map(|w| w.1 as f64)).collect(),
        );
        push_num(
            &mut columns,
            &mut vars,
            "P_KW",
            rows.iter()
                .map(|r| r.kw.map(|w| w.2).and_then(finite))
                .collect(),
        );
    }

    // Generic per-score-method emission.
    let emit_score = |columns: &mut Vec<Column>,
                      vars: &mut Vec<VarMeta>,
                      present: bool,
                      has_z: bool,
                      stat_name: &str,
                      z_name: &str,
                      p2_name: &str,
                      p_name: &str,
                      df_name: &str,
                      get: &dyn Fn(&OutRow) -> Option<&ScorePair>| {
        if !present {
            return;
        }
        // _STAT_ = 2-sample statistic (only meaningful when k == 2).
        push_num(
            columns,
            vars,
            stat_name,
            rows.iter()
                .map(|r| {
                    get(r)
                        .and_then(|(t, _)| t.as_ref())
                        .map(|t| t.stat)
                        .and_then(finite)
                })
                .collect(),
        );
        if has_z {
            push_num(
                columns,
                vars,
                z_name,
                rows.iter()
                    .map(|r| {
                        get(r)
                            .and_then(|(t, _)| t.as_ref())
                            .map(|t| t.z)
                            .and_then(finite)
                    })
                    .collect(),
            );
            push_num(
                columns,
                vars,
                p2_name,
                rows.iter()
                    .map(|r| {
                        get(r)
                            .and_then(|(t, _)| t.as_ref())
                            .map(|t| t.p2)
                            .and_then(finite)
                    })
                    .collect(),
            );
        }
        push_num(
            columns,
            vars,
            p_name,
            rows.iter()
                .map(|r| get(r).map(|(_, o)| o.p).and_then(finite))
                .collect(),
        );
        push_num(
            columns,
            vars,
            df_name,
            rows.iter()
                .map(|r| get(r).map(|(_, o)| o.df as f64))
                .collect(),
        );
    };

    emit_score(
        &mut columns,
        &mut vars,
        has_med,
        has_med_z,
        "_MED_",
        "Z_MED",
        "P2_MED",
        "P_MED",
        "DF_MED",
        &|r| r.med.as_ref(),
    );
    emit_score(
        &mut columns,
        &mut vars,
        has_sav,
        has_sav_z,
        "_SAV_",
        "Z_SAV",
        "P2_SAV",
        "P_SAV",
        "DF_SAV",
        &|r| r.sav.as_ref(),
    );
    emit_score(
        &mut columns,
        &mut vars,
        has_vw,
        has_vw_z,
        "_VW_",
        "Z_VW",
        "P2_VW",
        "P_VW",
        "DF_VW",
        &|r| r.vw.as_ref(),
    );

    let n_vars = vars.len();
    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = target.libref_or_work();
    let out_table = target.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));
    Ok(())
}

/// Render a BY-key value as a display string for the OUT= dataset.
pub(super) fn by_cell_string(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.trim_end().to_string(),
    }
}
