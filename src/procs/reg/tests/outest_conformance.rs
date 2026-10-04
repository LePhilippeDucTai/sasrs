use super::*;

/// Issue #22: PARMS stores -1 in the response column, independently of SSE.
/// SAS/STAT, REG, Output Data Sets, OUTEST= Data Set:
/// https://support.sas.com/documentation/cdl/en/statug/67523/HTML/default/statug_reg_details04.htm
/// The rational OLS oracle is derived in the corpus case's ORACLE.md, without
/// calling the production fit. Scaling the response changes SSE, not the marker.
#[test]
fn outest_parms_dependent_marker_is_independent_of_sse() {
    for scale in [1.0, 10.0] {
        let mut session = make_session();
        let responses: Vec<f64> = [2.1, 3.9, 6.2, 7.9, 10.1, 12.0]
            .iter()
            .map(|y| y * scale)
            .collect();
        let ds = SasDataset {
            df: df![
                "fert" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
                "yield" => responses.clone()
            ]
            .unwrap(),
            vars: vec![num_meta("fert"), num_meta("yield")],
        };
        write_dataset(&mut session, "SAMPLE", ds);
        let ast = parse_reg(
            "proc reg data=work.sample outest=est;
             model yield = fert;
             output out=pred p=yhat r=resid;
             run;",
        )
        .unwrap();
        execute(&ast, &mut session).unwrap();

        let (est, _) = session.libs.get("WORK").unwrap().read("EST").unwrap();
        assert_eq!(est.df.height(), 1);
        for (name, want) in [
            ("_MODEL_", "MODEL1"),
            ("_TYPE_", "PARMS"),
            ("_DEPVAR_", "yield"),
        ] {
            assert_eq!(
                est.df.column(name).unwrap().str().unwrap().get(0),
                Some(want)
            );
        }
        let number = |name: &str| est.df.column(name).unwrap().f64().unwrap().get(0).unwrap();
        assert_eq!(number("yield"), -1.0);
        let expected_sse = scale * scale * 191.0 / 2625.0;
        for (name, want) in [
            ("Intercept", scale * 4.0 / 75.0),
            ("fert", scale * 349.0 / 175.0),
            ("_RMSE_", (expected_sse / 4.0).sqrt()),
        ] {
            assert!((number(name) - want).abs() < 1e-9, "{name}, scale={scale}");
        }

        let (pred, _) = session.libs.get("WORK").unwrap().read("PRED").unwrap();
        assert_eq!(pred.df.height(), responses.len());
        let yhat = pred.df.column("yhat").unwrap().f64().unwrap();
        let resid = pred.df.column("resid").unwrap().f64().unwrap();
        let mut actual_sse = 0.0;
        for (i, response) in responses.iter().enumerate() {
            let expected_yhat = scale * (4.0 / 75.0 + 349.0 / 175.0 * (i + 1) as f64);
            let actual_resid = resid.get(i).unwrap();
            assert!((yhat.get(i).unwrap() - expected_yhat).abs() < 1e-9);
            assert!((actual_resid - (response - expected_yhat)).abs() < 1e-9);
            actual_sse += actual_resid * actual_resid;
        }
        assert!((actual_sse - expected_sse).abs() < 1e-9);
        assert!((number("_RMSE_").powi(2) * 4.0 - actual_sse).abs() < 1e-9);
    }
}
