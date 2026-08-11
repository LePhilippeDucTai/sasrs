use super::*;

// ───────────────────────── ODS / OUT= datasets ─────────────────────────

/// Resolve the destination for the TTest output (ODS OUTPUT TTest, else OUT=).
pub(super) fn output_target(ast: &TTestAst, session: &Session) -> Option<DatasetRef> {
    session
        .ods_output_target("TTest")
        .or_else(|| ast.data_options.output.clone())
}

pub(super) fn write_out_dataset(
    session: &mut Session,
    target: &DatasetRef,
    out_ds: SasDataset,
) -> Result<()> {
    let out_libref = target.libref_or_work();
    let out_table = target.name.to_uppercase();
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

pub(super) fn maybe_write_one_sample_output(
    ast: &TTestAst,
    session: &mut Session,
    rows: &[(String, OneSampleResult)],
) -> Result<()> {
    let Some(target) = output_target(ast, session) else {
        return Ok(());
    };
    let var: Vec<Option<String>> = rows.iter().map(|(n, _)| Some(n.clone())).collect();
    let n: Vec<Option<f64>> = rows.iter().map(|(_, r)| Some(r.n as f64)).collect();
    let mean: Vec<Option<f64>> = rows
        .iter()
        .map(|(_, r)| (r.n > 0).then_some(r.mean))
        .collect();
    let std: Vec<Option<f64>> = rows.iter().map(|(_, r)| r.std).collect();
    let stderr: Vec<Option<f64>> = rows.iter().map(|(_, r)| r.se).collect();
    let df: Vec<Option<f64>> = rows.iter().map(|(_, r)| Some(r.df)).collect();
    let tval: Vec<Option<f64>> = rows.iter().map(|(_, r)| r.t).collect();
    let probt: Vec<Option<f64>> = rows.iter().map(|(_, r)| r.p).collect();

    let columns: Vec<Column> = vec![
        Series::new("Variable".into(), var).into(),
        Series::new("N".into(), n).into(),
        Series::new("Mean".into(), mean).into(),
        Series::new("StdDev".into(), std).into(),
        Series::new("StdErr".into(), stderr).into(),
        Series::new("DF".into(), df).into(),
        Series::new("tValue".into(), tval).into(),
        Series::new("Probt".into(), probt).into(),
    ];
    let vars = vec![
        char_var_meta("Variable", 32),
        num_var_meta("N"),
        num_var_meta("Mean"),
        num_var_meta("StdDev"),
        num_var_meta("StdErr"),
        num_var_meta("DF"),
        num_var_meta("tValue"),
        num_var_meta("Probt"),
    ];
    let df_out = DataFrame::new(columns)?;
    write_out_dataset(session, &target, SasDataset { df: df_out, vars })?;
    // M38.3 — si la cible venait d'un `ODS OUTPUT TTest=…` (prioritaire sur
    // OUT= dans `output_target`), signaler la production pour que la frontière
    // de step n'émette pas « Output 'TTest' was not created ». No-op sinon.
    session.mark_ods_output_created("TTest");
    Ok(())
}

pub(super) fn maybe_write_paired_output(
    ast: &TTestAst,
    session: &mut Session,
    rows: &[(String, OneSampleResult)],
) -> Result<()> {
    // Paired output shares the one-sample layout (difference statistics).
    maybe_write_one_sample_output(ast, session, rows)
}

pub(super) fn maybe_write_two_sample_output(
    ast: &TTestAst,
    session: &mut Session,
    rows: &[(String, TwoSampleResult)],
    label_a: &str,
    label_b: &str,
) -> Result<()> {
    let Some(target) = output_target(ast, session) else {
        return Ok(());
    };
    // One row per variable per method (Pooled, Satterthwaite).
    let mut var: Vec<Option<String>> = Vec::new();
    let mut method: Vec<Option<String>> = Vec::new();
    let mut n1: Vec<Option<f64>> = Vec::new();
    let mut n2: Vec<Option<f64>> = Vec::new();
    let mut df: Vec<Option<f64>> = Vec::new();
    let mut tval: Vec<Option<f64>> = Vec::new();
    let mut probt: Vec<Option<f64>> = Vec::new();
    let _ = (label_a, label_b);
    for (name, r) in rows {
        for (m, res) in [("Pooled", &r.pooled), ("Satterthwaite", &r.satterthwaite)] {
            var.push(Some(name.clone()));
            method.push(Some(m.to_string()));
            n1.push(Some(r.n_a as f64));
            n2.push(Some(r.n_b as f64));
            match res {
                Some((t, d, p)) => {
                    df.push(Some(*d));
                    tval.push(Some(*t));
                    probt.push(Some(*p));
                }
                None => {
                    df.push(None);
                    tval.push(None);
                    probt.push(None);
                }
            }
        }
    }
    let columns: Vec<Column> = vec![
        Series::new("Variable".into(), var).into(),
        Series::new("Method".into(), method).into(),
        Series::new("N1".into(), n1).into(),
        Series::new("N2".into(), n2).into(),
        Series::new("DF".into(), df).into(),
        Series::new("tValue".into(), tval).into(),
        Series::new("Probt".into(), probt).into(),
    ];
    let vars = vec![
        char_var_meta("Variable", 32),
        char_var_meta("Method", 13),
        num_var_meta("N1"),
        num_var_meta("N2"),
        num_var_meta("DF"),
        num_var_meta("tValue"),
        num_var_meta("Probt"),
    ];
    let df_out = DataFrame::new(columns)?;
    write_out_dataset(session, &target, SasDataset { df: df_out, vars })?;
    // M38.3 — voir maybe_write_one_sample_output.
    session.mark_ods_output_created("TTest");
    Ok(())
}
