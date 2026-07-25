use super::super::*;
use super::*;

// ───────────────────────────── parse tests ─────────────────────────────

#[test]
fn parse_header_stats() {
    let ast = parse_means("proc means data=a n mean std min max; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert!(!ast.noprint);
    assert_eq!(ast.stats, vec!["n", "mean", "std", "min", "max"]);
}

#[test]
fn parse_noprint() {
    let ast = parse_means("proc means data=a noprint; run;").unwrap();
    assert!(ast.noprint);
}

#[test]
fn parse_class_and_var() {
    let ast =
        parse_means("proc means data=a; class g h; var x y; run;").unwrap();
    assert_eq!(ast.class, vec!["g", "h"]);
    assert_eq!(ast.var, vec!["x", "y"]);
}

#[test]
fn parse_output_specs() {
    let ast = parse_means(
        "proc means data=a; var height; output out=b mean(height)=avg_h n(height)=n_h; run;",
    )
    .unwrap();
    let out = ast.output.as_ref().unwrap();
    assert_eq!(out.out.name, "b");
    assert_eq!(
        out.specs,
        vec![
            ("mean".to_string(), "height".to_string(), "avg_h".to_string()),
            ("n".to_string(), "height".to_string(), "n_h".to_string()),
        ]
    );
}

#[test]
fn parse_unknown_option_errors() {
    let r = parse_means("proc means data=a bogus; run;");
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(msg.contains("BOGUS"), "msg: {msg}");
}

// ───────────────────────── WAYS / TYPES tests (M33.3) ────────────────────

#[test]
fn parse_ways_and_types() {
    let ast = parse_means(
        "proc means data=a; class g h; ways 0 1 2; types (g) (g*h) h; run;",
    )
    .unwrap();
    assert_eq!(ast.ways, vec![0, 1, 2]);
    assert_eq!(
        ast.types,
        vec![
            vec!["g".to_string()],
            vec!["g".to_string(), "h".to_string()],
            vec!["h".to_string()],
        ]
    );
}

#[test]
fn parse_printalltypes_option() {
    let ast = parse_means("proc means data=a printalltypes; class g; run;").unwrap();
    assert!(ast.printalltypes);
    let ast2 = parse_means("proc means data=a; class g; run;").unwrap();
    assert!(!ast2.printalltypes);
}

#[test]
fn parse_alpha_option() {
    let ast = parse_means("proc means data=a alpha=0.1 clm; var x; run;").unwrap();
    assert!((ast.alpha - 0.1).abs() < 1e-12);
    assert!(ast.stats.contains(&"clm".to_string()));
}

#[test]
fn parse_alpha_default() {
    let ast = parse_means("proc means data=a; var x; run;").unwrap();
    assert!((ast.alpha - 0.05).abs() < 1e-12);
}

#[test]
fn parse_alpha_invalid_errors() {
    for bad in ["alpha=0", "alpha=1", "alpha=1.5"] {
        let r = parse_means(&format!("proc means data=a {bad}; run;"));
        assert!(r.is_err(), "{bad} should error");
        assert!(
            r.err().unwrap().to_string().contains("between 0 and 1"),
            "{bad}"
        );
    }
}

#[test]
fn parse_weight_statement() {
    let ast = parse_means("proc means data=a; var x; weight w; run;").unwrap();
    assert_eq!(ast.weight.as_deref(), Some("w"));
    assert_eq!(ast.var, vec!["x"]);
}

// ───────────────────────────── compute tests ───────────────────────────

#[test]
fn compute_basic_stats_with_a_missing() {
    // values: 2, 4, 6, missing -> non-missing [2,4,6], nmiss=1
    let xs = vec![2.0, 4.0, 6.0];
    assert_eq!(compute("n", &xs, 1, 0.05), Value::Num(3.0));
    assert_eq!(compute("nmiss", &xs, 1, 0.05), Value::Num(1.0));
    assert_eq!(compute("mean", &xs, 1, 0.05), Value::Num(4.0));
    assert_eq!(compute("min", &xs, 1, 0.05), Value::Num(2.0));
    assert_eq!(compute("max", &xs, 1, 0.05), Value::Num(6.0));
    assert_eq!(compute("sum", &xs, 1, 0.05), Value::Num(12.0));
    assert_eq!(compute("range", &xs, 1, 0.05), Value::Num(4.0));
    assert_eq!(compute("median", &xs, 1, 0.05), Value::Num(4.0));
    // std of [2,4,6]: variance = ((2-4)^2+(4-4)^2+(6-4)^2)/2 = 8/2 = 4 -> std 2
    assert_eq!(compute("std", &xs, 1, 0.05), Value::Num(2.0));
}

// ─────────────────────── percentile keyword tests (M33.3) ───────────────

#[test]
fn compute_percentiles_def5_hand_oracle() {
    // sashelp.class heights (n=19) sorted ascending:
    //  51.3 56.3 56.5 57.3 57.5 59.0 59.8 62.5 62.5 62.8
    //  63.5 64.3 64.8 65.3 66.5 66.5 67.0 69.0 72.0
    // Definition 5: np=19*p, j=floor(np), g=np-j;
    //   g==0 → (x[j]+x[j+1])/2 ; else → x[j+1]  (1-indexed)
    let xs = vec![
        69.0, 56.5, 65.3, 62.8, 63.5, 57.3, 59.8, 62.5, 62.5, 59.0, 51.3, 64.3, 56.3, 66.5,
        72.0, 64.8, 67.0, 57.5, 66.5,
    ];
    // P25: np=4.75, j=4, g=.75 → x[5]=57.5
    assert_eq!(compute("p25", &xs, 0, 0.05), Value::Num(57.5));
    assert_eq!(compute("q1", &xs, 0, 0.05), Value::Num(57.5));
    // P50/median: np=9.5, j=9, g=.5 → x[10]=62.8
    assert_eq!(compute("p50", &xs, 0, 0.05), Value::Num(62.8));
    assert_eq!(compute("median", &xs, 0, 0.05), Value::Num(62.8));
    // P75: np=14.25, j=14, g=.25 → x[15]=66.5
    assert_eq!(compute("p75", &xs, 0, 0.05), Value::Num(66.5));
    assert_eq!(compute("q3", &xs, 0, 0.05), Value::Num(66.5));
    // P95: np=18.05, j=18, g=.05 → x[19]=72.0
    assert_eq!(compute("p95", &xs, 0, 0.05), Value::Num(72.0));
    // QRANGE = P75 − P25 = 66.5 − 57.5 = 9.0
    assert_eq!(compute("qrange", &xs, 0, 0.05), Value::Num(9.0));
    // empty → missing for all percentile keywords
    let empty: Vec<f64> = vec![];
    assert!(compute("p25", &empty, 0, 0.05).is_missing());
    assert!(compute("qrange", &empty, 0, 0.05).is_missing());
}

#[test]
fn compute_percentile_discontinuity_average() {
    // [1,2,3,4]: P50 np=2, g=0 → (x[2]+x[3])/2 = 2.5 (matches median).
    let xs = vec![1.0, 2.0, 3.0, 4.0];
    assert_eq!(compute("p50", &xs, 0, 0.05), Value::Num(2.5));
    // P25 np=1, g=0 → (x[1]+x[2])/2 = 1.5
    assert_eq!(compute("p25", &xs, 0, 0.05), Value::Num(1.5));
}

#[test]
fn compute_median_even() {
    let xs = vec![1.0, 2.0, 3.0, 4.0];
    assert_eq!(compute("median", &xs, 0, 0.05), Value::Num(2.5));
}

#[test]
fn compute_edge_n0_and_n1() {
    let empty: Vec<f64> = vec![];
    assert_eq!(compute("n", &empty, 0, 0.05), Value::Num(0.0));
    assert!(compute("mean", &empty, 0, 0.05).is_missing());
    assert!(compute("std", &empty, 0, 0.05).is_missing());
    assert!(compute("min", &empty, 0, 0.05).is_missing());
    assert!(compute("range", &empty, 0, 0.05).is_missing());
    assert_eq!(compute("sum", &empty, 0, 0.05), Value::Num(0.0));

    let one = vec![5.0];
    assert_eq!(compute("n", &one, 0, 0.05), Value::Num(1.0));
    assert_eq!(compute("mean", &one, 0, 0.05), Value::Num(5.0));
    // std needs n>=2.
    assert!(compute("std", &one, 0, 0.05).is_missing());
    assert!(compute("stderr", &one, 0, 0.05).is_missing());
    assert_eq!(compute("min", &one, 0, 0.05), Value::Num(5.0));
}

#[test]
fn compute_cv_and_stderr() {
    // [2,4,6]: mean 4, std 2 -> cv = 100*2/4 = 50; stderr = 2/sqrt(3)
    let xs = vec![2.0, 4.0, 6.0];
    assert_eq!(compute("cv", &xs, 0, 0.05), Value::Num(50.0));
    if let Value::Num(se) = compute("stderr", &xs, 0, 0.05) {
        assert!((se - 2.0 / 3.0_f64.sqrt()).abs() < 1e-12);
    } else {
        panic!("stderr should be numeric");
    }
}

#[test]
fn compute_clm_hand_computed() {
    // values [2,4,4,4,5,5,7,9]: mean 5, n=8. SAS uses the SAMPLE std
    // (VARDEF=DF): var = 32/7 → std = 2.13809, stderr = std/sqrt(8) =
    // 0.75593, t_{0.975,7} = 2.36462 → h = 1.78749. (The task brief's
    // 3.3278/6.6722 assumed std=2, which is not the sample std of this
    // data; SAS reports the values below.)
    let xs = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
    let lo = match compute("lclm", &xs, 0, 0.05) {
        Value::Num(f) => f,
        _ => panic!("lclm numeric"),
    };
    let hi = match compute("uclm", &xs, 0, 0.05) {
        Value::Num(f) => f,
        _ => panic!("uclm numeric"),
    };
    assert!((lo - 3.21251).abs() < 1e-3, "lclm={lo}");
    assert!((hi - 6.78749).abs() < 1e-3, "uclm={hi}");
}

#[test]
fn compute_clm_alpha_widens_interval() {
    let xs = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
    let lo05 = match compute("lclm", &xs, 0, 0.05) {
        Value::Num(f) => f,
        _ => unreachable!(),
    };
    let lo01 = match compute("lclm", &xs, 0, 0.01) {
        Value::Num(f) => f,
        _ => unreachable!(),
    };
    // Smaller alpha → wider interval → lower lower-limit.
    assert!(lo01 < lo05, "lo01={lo01} lo05={lo05}");
}

#[test]
fn compute_clm_requires_n2() {
    let one = vec![5.0];
    assert!(compute("lclm", &one, 0, 0.05).is_missing());
    assert!(compute("uclm", &one, 0, 0.05).is_missing());
    let empty: Vec<f64> = vec![];
    assert!(compute("lclm", &empty, 0, 0.05).is_missing());
    // "clm" has no single-value meaning.
    assert!(compute("clm", &one, 0, 0.05).is_missing());
}

// ───────────────────────────── WEIGHT tests ────────────────────────────

#[test]
fn compute_weighted_hand_values() {
    // values [1,2,3] weights [1,2,3]:
    //   SumWgt=6, Sum=14, mean=14/6=2.33333...
    //   CSS_w = 1*(1-m)^2 + 2*(2-m)^2 + 3*(3-m)^2 = 3.33333...
    //   Variance = CSS_w/(n-1) = 3.33333/2 = 1.66667
    //   Std = sqrt(1.66667) = 1.2909944
    //   StdErr = Std/sqrt(6) = 0.5270463
    //   CV = 100*Std/mean = 55.3283
    //   USS_w = 1*1 + 2*4 + 3*9 = 36
    let pairs = vec![(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)];
    assert_eq!(compute_weighted("n", &pairs, 0, 0.05), Value::Num(3.0));
    assert_eq!(compute_weighted("nmiss", &pairs, 0, 0.05), Value::Num(0.0));
    assert_eq!(compute_weighted("sum", &pairs, 0, 0.05), Value::Num(14.0));
    assert_eq!(compute_weighted("min", &pairs, 0, 0.05), Value::Num(1.0));
    assert_eq!(compute_weighted("max", &pairs, 0, 0.05), Value::Num(3.0));

    let m = match compute_weighted("mean", &pairs, 0, 0.05) {
        Value::Num(f) => f,
        _ => panic!("mean numeric"),
    };
    assert!((m - 14.0 / 6.0).abs() < 1e-12, "mean = {m}");

    let std = match compute_weighted("std", &pairs, 0, 0.05) {
        Value::Num(f) => f,
        _ => panic!("std numeric"),
    };
    assert!((std - (5.0_f64 / 3.0).sqrt()).abs() < 1e-12, "std = {std}");

    let se = match compute_weighted("stderr", &pairs, 0, 0.05) {
        Value::Num(f) => f,
        _ => panic!("stderr numeric"),
    };
    assert!(
        (se - (5.0_f64 / 3.0).sqrt() / 6.0_f64.sqrt()).abs() < 1e-12,
        "stderr = {se}"
    );

    let cv = match compute_weighted("cv", &pairs, 0, 0.05) {
        Value::Num(f) => f,
        _ => panic!("cv numeric"),
    };
    let expected_cv = 100.0 * (5.0_f64 / 3.0).sqrt() / (14.0 / 6.0);
    assert!((cv - expected_cv).abs() < 1e-9, "cv = {cv}");
}

#[test]
fn compute_weighted_n1_std_missing() {
    let pairs = vec![(5.0, 2.0)];
    assert_eq!(compute_weighted("n", &pairs, 0, 0.05), Value::Num(1.0));
    assert_eq!(compute_weighted("mean", &pairs, 0, 0.05), Value::Num(5.0));
    assert!(compute_weighted("std", &pairs, 0, 0.05).is_missing());
    assert!(compute_weighted("stderr", &pairs, 0, 0.05).is_missing());
}

#[test]
fn percentile_keywords_recognized() {
    for k in [
        "p1", "p5", "p10", "p20", "p25", "p30", "p40", "p50", "p60", "p70", "p75", "p80",
        "p90", "p95", "p99", "q1", "q3", "qrange",
    ] {
        assert!(is_stat_keyword(k), "{k} should be a stat keyword");
    }
    let ast = parse_means("proc means data=a p25 median p75 p95 qrange; run;").unwrap();
    assert_eq!(
        ast.stats,
        vec!["p25", "median", "p75", "p95", "qrange"]
    );
}

#[test]
fn percentile_report_headers() {
    assert_eq!(percentile_header("p25").as_deref(), Some("25th Pctl"));
    assert_eq!(percentile_header("p1").as_deref(), Some("1st Pctl"));
    assert_eq!(percentile_header("p5").as_deref(), Some("5th Pctl"));
    assert_eq!(percentile_header("q3").as_deref(), Some("75th Pctl"));
    assert_eq!(percentile_header("p50").as_deref(), Some("Median"));
    assert_eq!(percentile_header("median").as_deref(), Some("Median"));
    assert_eq!(percentile_header("qrange").as_deref(), Some("Quartile Range"));
    assert_eq!(percentile_header("mean"), None);
}

#[test]
fn type_mask_convention() {
    // k=2: class order [g, h]. LSB ⇔ LAST class (h).
    //   {} → 0 ; {h}(i=1) → 1 ; {g}(i=0) → 2 ; {g,h} → 3
    assert_eq!(type_mask(&[], 2), 0);
    assert_eq!(type_mask(&[1], 2), 1);
    assert_eq!(type_mask(&[0], 2), 2);
    assert_eq!(type_mask(&[0, 1], 2), 3);
}

#[test]
fn allowed_types_ways_selects_by_popcount() {
    // k=2; WAYS 1 → the two single-class types {1,2}.
    let mut ast = means_ast_var_x();
    ast.class = vec!["g".into(), "h".into()];
    ast.ways = vec![1];
    let set = allowed_types(&ast, &ast.class, 2).unwrap().unwrap();
    assert_eq!(set, [1u64, 2].iter().copied().collect());

    // WAYS 0 2 → overall (0) + full crossing (3).
    ast.ways = vec![0, 2];
    let set = allowed_types(&ast, &ast.class, 2).unwrap().unwrap();
    assert_eq!(set, [0u64, 3].iter().copied().collect());
}

#[test]
fn allowed_types_types_selects_specific_crossings() {
    // class [g,h]; TYPES (g) (g*h) → masks 2 and 3.
    let mut ast = means_ast_var_x();
    ast.class = vec!["g".into(), "h".into()];
    ast.types = vec![vec!["g".into()], vec!["g".into(), "h".into()]];
    let set = allowed_types(&ast, &ast.class, 2).unwrap().unwrap();
    assert_eq!(set, [2u64, 3].iter().copied().collect());

    // unknown class name in TYPES → error.
    ast.types = vec![vec!["zzz".into()]];
    assert!(allowed_types(&ast, &ast.class, 2).is_err());
}

#[test]
fn allowed_types_none_when_unrestricted() {
    let ast = means_ast_var_x();
    assert!(allowed_types(&ast, &ast.class, 0).unwrap().is_none());
}
