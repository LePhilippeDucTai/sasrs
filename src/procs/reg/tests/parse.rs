use super::*;

#[test]
fn test_parse_model() {
    let ast = parse_reg("proc reg data=a; model y = x1 x2; run;").unwrap();
    assert_eq!(ast.models.len(), 1);
    let m = &ast.models[0].model;
    assert_eq!(m.dependents, vec!["y"]);
    assert_eq!(m.regressors, vec!["x1", "x2"]);
    assert!(!m.noint);
    assert!(!m.noprint);
    assert!(m.selection.is_none());
}

#[test]
fn test_parse_multiple_models() {
    let ast = parse_reg(
        "proc reg data=a; model y = x1; output out=o1 p=p1; model y = x1 x2; output out=o2 p=p2; run;",
    )
    .unwrap();
    assert_eq!(ast.models.len(), 2);
    // First model has one regressor and its OUTPUT.
    assert_eq!(ast.models[0].model.regressors, vec!["x1"]);
    assert_eq!(ast.models[0].outputs.len(), 1);
    assert_eq!(ast.models[0].outputs[0].out.name, "o1");
    assert_eq!(ast.models[0].outputs[0].predicted.as_deref(), Some("p1"));
    // Second model has two regressors and its own OUTPUT.
    assert_eq!(ast.models[1].model.regressors, vec!["x1", "x2"]);
    assert_eq!(ast.models[1].outputs.len(), 1);
    assert_eq!(ast.models[1].outputs[0].out.name, "o2");
}

#[test]
fn test_parse_output() {
    let ast =
        parse_reg("proc reg data=a; model y = x; output out=work.out predicted=p residual=r; run;")
            .unwrap();
    assert_eq!(ast.models.len(), 1);
    assert_eq!(ast.models[0].outputs.len(), 1);
    let o = &ast.models[0].outputs[0];
    assert_eq!(o.out.name, "out");
    assert_eq!(o.predicted.as_deref(), Some("p"));
    assert_eq!(o.residual.as_deref(), Some("r"));
}

#[test]
fn test_parse_selection_forward() {
    let ast = parse_reg(
        "proc reg data=a; model y = x1 x2 / selection=forward slentry=0.3; run;",
    )
    .unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::Forward);
    assert!((sel.slentry - 0.3).abs() < 1e-12);
}

#[test]
fn test_parse_selection_synonyms() {
    // sle=/sls= synonyms and stepwise.
    let ast = parse_reg(
        "proc reg data=a; model y = x1 x2 / selection=stepwise sle=0.2 sls=0.25; run;",
    )
    .unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::Stepwise);
    assert!((sel.slentry - 0.2).abs() < 1e-12);
    assert!((sel.slstay - 0.25).abs() < 1e-12);
}

#[test]
fn test_parse_selection_defaults() {
    let ast =
        parse_reg("proc reg data=a; model y = x1 / selection=backward; run;").unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::Backward);
    assert!((sel.slstay - 0.10).abs() < 1e-12);
}

#[test]
fn test_parse_noint() {
    let ast = parse_reg("proc reg data=a; model y = x / noint; run;").unwrap();
    assert!(ast.models[0].model.noint);
}

#[test]
fn test_parse_test_multi_eq() {
    let ast = parse_reg("proc reg data=a; model y = a b c; test a=b, c=0; run;").unwrap();
    let t = &ast.models[0].tests[0];
    assert!(t.label.is_none());
    assert_eq!(t.equations.len(), 2);
    // a = b  →  A - B = 0
    let e0 = eq_terms(&t.equations[0]);
    assert_eq!(e0, vec![(1.0, "A".into()), (-1.0, "B".into())]);
    assert!((t.equations[0].rhs).abs() < 1e-12);
    // c = 0  →  C = 0
    let e1 = eq_terms(&t.equations[1]);
    assert_eq!(e1, vec![(1.0, "C".into())]);
}

#[test]
fn test_parse_test_label() {
    let ast = parse_reg("proc reg data=a; model y = x1 x2; peak: test x1 = x2; run;").unwrap();
    let t = &ast.models[0].tests[0];
    assert_eq!(t.label.as_deref(), Some("peak"));
    assert_eq!(
        eq_terms(&t.equations[0]),
        vec![(1.0, "X1".into()), (-1.0, "X2".into())]
    );
}

#[test]
fn test_parse_restrict_sum() {
    let ast = parse_reg("proc reg data=a; model y = a b; restrict a+b=1; run;").unwrap();
    let r = &ast.models[0].restricts[0];
    assert_eq!(r.equations.len(), 1);
    assert_eq!(
        eq_terms(&r.equations[0]),
        vec![(1.0, "A".into()), (1.0, "B".into())]
    );
    assert!((r.equations[0].rhs - 1.0).abs() < 1e-12);
}

#[test]
fn test_parse_restrict_coefficients() {
    // 2*x1 - x2 = 0
    let ast =
        parse_reg("proc reg data=a; model y = x1 x2; restrict 2*x1 - x2 = 0; run;").unwrap();
    let e = &ast.models[0].restricts[0].equations[0];
    assert_eq!(
        eq_terms(e),
        vec![(2.0, "X1".into()), (-1.0, "X2".into())]
    );
    assert!(e.rhs.abs() < 1e-12);
}

#[test]
fn test_parse_coef_no_star() {
    // `2 x1` (no star) is also a coefficient form.
    let ast =
        parse_reg("proc reg data=a; model y = x1 x2; restrict 2 x1 = x2 + 3; run;").unwrap();
    let e = &ast.models[0].restricts[0].equations[0];
    // 2*x1 - x2 = 3
    assert_eq!(
        eq_terms(e),
        vec![(2.0, "X1".into()), (-1.0, "X2".into())]
    );
    assert!((e.rhs - 3.0).abs() < 1e-12);
}

#[test]
fn test_parse_intercept_keyword() {
    let ast = parse_reg(
        "proc reg data=a; model y = x1 x2; restrict intercept = 0; run;",
    )
    .unwrap();
    let e = &ast.models[0].restricts[0].equations[0];
    assert_eq!(eq_terms(e), vec![(1.0, "INTERCEPT".into())]);
}

#[test]
fn test_parse_model_cl_options() {
    let ast =
        parse_reg("proc reg data=a; model y=x / clb alpha=0.10 cli clm; run;").unwrap();
    let m = &ast.models[0].model;
    assert!(m.clb);
    assert!(m.cli);
    assert!(m.clm);
    assert!((m.alpha - 0.10).abs() < 1e-12);
}

#[test]
fn test_parse_output_cl_keywords() {
    let ast = parse_reg(
        "proc reg data=a; model y=x; output out=o p=pred stdp=sp lclm=lm uclm=um lcl=l ucl=u stdi=si stdr=sr; run;",
    )
    .unwrap();
    let o = &ast.models[0].outputs[0];
    assert_eq!(o.predicted.as_deref(), Some("pred"));
    assert_eq!(o.stdp.as_deref(), Some("sp"));
    assert_eq!(o.lclm.as_deref(), Some("lm"));
    assert_eq!(o.uclm.as_deref(), Some("um"));
    assert_eq!(o.lcl.as_deref(), Some("l"));
    assert_eq!(o.ucl.as_deref(), Some("u"));
    assert_eq!(o.stdi.as_deref(), Some("si"));
    assert_eq!(o.stdr.as_deref(), Some("sr"));
}

#[test]
fn parse_plots_statement_flag() {
    let ast = parse_reg("proc reg data=a; model y = x; plots / only; run;").unwrap();
    assert!(ast.plots_requested);
}

#[test]
fn test_parse_model_r_influence() {
    let ast = parse_reg("proc reg data=a; model y=x / r influence; run;").unwrap();
    let m = &ast.models[0].model;
    assert!(m.r);
    assert!(m.influence);
}

#[test]
fn test_parse_output_influence_keywords() {
    let ast = parse_reg(
        "proc reg data=a; model y=x; output out=o student=rs rstudent=er cookd=cd h=hat press=pr dffits=df covratio=cv dfbetas=b; run;",
    )
    .unwrap();
    let o = &ast.models[0].outputs[0];
    assert_eq!(o.student.as_deref(), Some("rs"));
    assert_eq!(o.rstudent.as_deref(), Some("er"));
    assert_eq!(o.cookd.as_deref(), Some("cd"));
    assert_eq!(o.h.as_deref(), Some("hat"));
    assert_eq!(o.press.as_deref(), Some("pr"));
    assert_eq!(o.dffits.as_deref(), Some("df"));
    assert_eq!(o.covratio.as_deref(), Some("cv"));
    assert_eq!(o.dfbetas.as_deref(), Some("b"));
}

// ───────────────────────── M36.4 ─────────────────────────

/// Parse: all collinearity / spec diagnostic options on one MODEL.
#[test]
fn test_parse_model_diagnostics() {
    let ast = parse_reg(
        "proc reg data=a; model y=x1 x2 / vif tol collin spec dw dwprob acov; run;",
    )
    .unwrap();
    let m = &ast.models[0].model;
    assert!(m.vif);
    assert!(m.tol);
    assert!(m.collin);
    assert!(!m.collinoint);
    assert!(m.spec);
    assert!(m.dw);
    assert!(m.dwprob);
    assert!(m.acov);
}

#[test]
fn test_parse_collinoint_and_hcc_synonym() {
    let ast =
        parse_reg("proc reg data=a; model y=x1 x2 / collinoint hcc; run;").unwrap();
    let m = &ast.models[0].model;
    assert!(m.collinoint);
    assert!(!m.collin);
    // HCC is a synonym for ACOV.
    assert!(m.acov);
}

/// Parse all M36.5 options off one MODEL statement.
#[test]
fn test_parse_m365_options() {
    let ast = parse_reg(
        "proc reg data=a; model y = x1 x2 / ss1 ss2 stb pcorr1 pcorr2 scorr1 scorr2 seqb press; run;",
    )
    .unwrap();
    let m = &ast.models[0].model;
    assert!(m.ss1 && m.ss2 && m.stb);
    assert!(m.pcorr1 && m.pcorr2 && m.scorr1 && m.scorr2);
    assert!(m.seqb && m.press_opt);
}

/// Default model leaves every M36.5 flag off (byte-identity guard).
#[test]
fn test_parse_m365_default_off() {
    let ast = parse_reg("proc reg data=a; model y = x1 x2; run;").unwrap();
    let m = &ast.models[0].model;
    assert!(!m.ss1 && !m.ss2 && !m.stb);
    assert!(!m.pcorr1 && !m.pcorr2 && !m.scorr1 && !m.scorr2);
    assert!(!m.seqb && !m.press_opt);
}

#[test]
fn test_parse_weight_freq_by_id() {
    let ast = parse_reg(
        "proc reg data=a; model y = x; weight wv; freq fv; by grp; id name; run;",
    )
    .unwrap();
    assert_eq!(ast.weight.as_deref(), Some("wv"));
    assert_eq!(ast.freq.as_deref(), Some("fv"));
    assert_eq!(ast.by, vec!["grp".to_string()]);
    assert_eq!(ast.id, vec!["name".to_string()]);
}

#[test]
fn test_parse_by_multiple_and_defaults() {
    let ast = parse_reg("proc reg data=a; model y = x; by a b; run;").unwrap();
    assert_eq!(ast.by, vec!["a".to_string(), "b".to_string()]);
    assert!(ast.weight.is_none());
    assert!(ast.freq.is_none());
    assert!(ast.id.is_empty());
}

// ───────────── M36.8 parse tests ─────────────

#[test]
fn test_parse_simple_corr_all() {
    let ast = parse_reg("proc reg data=a simple corr all; model y = x; run;").unwrap();
    assert!(ast.simple);
    assert!(ast.corr);
    let m = &ast.models[0].model;
    // ALL turns on the MODEL matrix options + CLM/CLI.
    assert!(m.xpx && m.inv && m.covb && m.corrb);
    assert!(m.clm && m.cli);
}

#[test]
fn test_parse_model_matrix_options() {
    let ast =
        parse_reg("proc reg data=a; model y = x / xpx i covb corrb; run;").unwrap();
    let m = &ast.models[0].model;
    assert!(m.xpx && m.inv && m.covb && m.corrb);
    // Untouched flags stay off (byte-identity guard).
    assert!(!m.clb && !m.vif);
}

#[test]
fn test_parse_outest_modifiers() {
    let ast =
        parse_reg("proc reg data=a outest=e covout outseb edf; model y = x; run;")
            .unwrap();
    let oe = ast.data_options.outest.as_ref().unwrap();
    assert_eq!(oe.out.name, "e");
    assert!(oe.covout && oe.outseb && oe.edf);
    assert!(!oe.tableout);
}

#[test]
fn test_parse_outsscp() {
    let ast = parse_reg("proc reg data=a outsscp=s; model y = x; run;").unwrap();
    assert_eq!(ast.data_options.outsscp.as_ref().unwrap().name, "s");
}

#[test]
fn test_parse_default_m368_off() {
    // A plain PROC/MODEL leaves every M36.8 flag off (byte-identity guard).
    let ast = parse_reg("proc reg data=a; model y = x; run;").unwrap();
    assert!(!ast.simple && !ast.corr);
    assert!(ast.data_options.outest.is_none());
    assert!(ast.data_options.outsscp.is_none());
    let m = &ast.models[0].model;
    assert!(!m.xpx && !m.inv && !m.covb && !m.corrb);
}

// ───────────────────────── M36.11 PLOTS= / PLOT tests ─────────────────────────

#[test]
fn parse_plots_diagnostics() {
    let ast = parse_reg("proc reg data=a; model y=x; plots=diagnostics; run;").unwrap();
    assert!(ast.plot_requests.diagnostics);
    assert!(ast.plot_requests.explicit);
    assert!(!ast.plot_requests.none);
    assert!(ast.plot_requests.any());
}

#[test]
fn parse_plots_list_residuals_fit() {
    let ast = parse_reg("proc reg data=a; model y=x; plots=(residuals fit); run;").unwrap();
    assert!(ast.plot_requests.residuals);
    assert!(ast.plot_requests.fit);
    assert!(!ast.plot_requests.diagnostics);
}

#[test]
fn parse_plots_unpack_modifier() {
    let ast = parse_reg("proc reg data=a; model y=x; plots(unpack)=diagnostics; run;").unwrap();
    assert!(ast.plot_requests.unpack);
    assert!(ast.plot_requests.diagnostics);
}

#[test]
fn parse_plots_only_modifier() {
    let ast = parse_reg("proc reg data=a; model y=x; plots(only)=fit; run;").unwrap();
    assert!(ast.plot_requests.only);
    assert!(ast.plot_requests.fit);
}

#[test]
fn parse_plots_none() {
    let ast = parse_reg("proc reg data=a; model y=x; plots=none; run;").unwrap();
    assert!(ast.plot_requests.none);
    assert!(!ast.plot_requests.any());
}

#[test]
fn parse_plots_all() {
    let ast = parse_reg("proc reg data=a; model y=x; plots=all; run;").unwrap();
    assert!(ast.plot_requests.all);
    assert!(ast.plot_requests.any());
}

#[test]
fn parse_plots_at_proc_level() {
    let ast = parse_reg("proc reg data=a plots=diagnostics; model y=x; run;").unwrap();
    assert!(ast.plot_requests.diagnostics);
}

#[test]
fn parse_plots_unknown_keyword_ignored() {
    let ast =
        parse_reg("proc reg data=a; model y=x; plots=(diagnostics bogusplot); run;").unwrap();
    assert!(ast.plot_requests.diagnostics);
    // Bogus keyword consumed cleanly, no other family flipped.
    assert!(!ast.plot_requests.residuals && !ast.plot_requests.fit);
}

#[test]
fn parse_plot_statement_simple() {
    let ast = parse_reg("proc reg data=a; model weight=height; plot weight*height; run;")
        .unwrap();
    assert_eq!(ast.plot_statements.len(), 1);
    assert_eq!(ast.plot_statements[0].y, PlotVar::Named("WEIGHT".into()));
    assert_eq!(ast.plot_statements[0].x, PlotVar::Named("HEIGHT".into()));
}

#[test]
fn parse_plot_statement_keyword_vars() {
    let ast =
        parse_reg("proc reg data=a; model y=x; plot residual.*predicted.; run;").unwrap();
    assert_eq!(ast.plot_statements.len(), 1);
    assert_eq!(ast.plot_statements[0].y, PlotVar::Residual);
    assert_eq!(ast.plot_statements[0].x, PlotVar::Predicted);
}

#[test]
fn parse_plot_statement_short_keyword_vars() {
    let ast = parse_reg("proc reg data=a; model y=x; plot r.*p.; run;").unwrap();
    assert_eq!(ast.plot_statements[0].y, PlotVar::Residual);
    assert_eq!(ast.plot_statements[0].x, PlotVar::Predicted);
}

#[test]
fn parse_plot_statement_multiple_pairs() {
    let ast = parse_reg(
        "proc reg data=a; model y=x z; plot y*x z*y / overlay; run;",
    )
    .unwrap();
    assert_eq!(ast.plot_statements.len(), 2);
    assert_eq!(ast.plot_statements[0].y, PlotVar::Named("Y".into()));
    assert_eq!(ast.plot_statements[0].x, PlotVar::Named("X".into()));
    assert_eq!(ast.plot_statements[1].y, PlotVar::Named("Z".into()));
    assert_eq!(ast.plot_statements[1].x, PlotVar::Named("Y".into()));
}

#[test]
fn parse_plot_statement_with_symbol() {
    let ast =
        parse_reg("proc reg data=a; model y=x; plot residual.*predicted.='*'; run;").unwrap();
    assert_eq!(ast.plot_statements.len(), 1);
    assert_eq!(ast.plot_statements[0].y, PlotVar::Residual);
}
