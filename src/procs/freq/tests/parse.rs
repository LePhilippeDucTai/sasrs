use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

// ───────────────────────────── parse tests ─────────────────────────────

#[test]
fn parse_one_way() {
    let ast = parse_freq("proc freq data=a; tables x; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert_eq!(ast.tables.len(), 1);
    assert_eq!(ast.tables[0].vars, vec!["x"]);
    assert!(!ast.tables[0].missing);
    assert!(ast.tables[0].out.is_none());
}

#[test]
fn parse_multiple_specs_and_crosstab() {
    let ast = parse_freq("proc freq data=a; tables a b a*b; run;").unwrap();
    assert_eq!(ast.tables.len(), 3);
    assert_eq!(ast.tables[0].vars, vec!["a"]);
    assert_eq!(ast.tables[1].vars, vec!["b"]);
    assert_eq!(ast.tables[2].vars, vec!["a", "b"]);
}

#[test]
fn parse_missing_and_out() {
    let ast = parse_freq("proc freq data=a; tables a / missing out=work.o; run;").unwrap();
    assert_eq!(ast.tables.len(), 1);
    assert!(ast.tables[0].missing);
    let out = ast.tables[0].out.as_ref().unwrap();
    assert_eq!(out.libref.as_deref(), Some("work"));
    assert_eq!(out.name, "o");
}

#[test]
fn parse_out_requires_single_spec() {
    let r = parse_freq("proc freq data=a; tables a b / out=work.o; run;");
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(msg.contains("OUT="), "msg: {msg}");
}

#[test]
fn parse_ignores_display_options() {
    let ast = parse_freq("proc freq data=a; tables x / nopercent norow nocol nofreq nocum; run;")
        .unwrap();
    assert_eq!(ast.tables.len(), 1);
    assert_eq!(ast.tables[0].vars, vec!["x"]);
}

#[test]
fn parse_accepts_table_spelling() {
    let ast = parse_freq("proc freq data=a; table x; run;").unwrap();
    assert_eq!(ast.tables.len(), 1);
    assert_eq!(ast.tables[0].vars, vec!["x"]);
}

#[test]
fn parse_multiple_tables_statements_accumulate() {
    let ast = parse_freq("proc freq data=a; tables x; tables y*z; run;").unwrap();
    assert_eq!(ast.tables.len(), 2);
    assert_eq!(ast.tables[0].vars, vec!["x"]);
    assert_eq!(ast.tables[1].vars, vec!["y", "z"]);
}

// ---- parser ----

#[test]
fn parse_new_stat_options() {
    let ast = parse_freq("proc freq data=a; tables a*b / chisq fisher agree measures trend; run;")
        .unwrap();
    let t = &ast.tables[0];
    assert!(t.chisq && t.fisher && t.agree && t.measures && t.trend);
}

#[test]
fn parse_exact_and_relrisk_aliases() {
    let ast = parse_freq("proc freq data=a; tables a*b / exact relrisk; run;").unwrap();
    let t = &ast.tables[0];
    assert!(t.fisher, "exact -> fisher");
    assert!(t.measures, "relrisk -> measures");
}

#[test]
fn parse_weight_by_list() {
    let ast = parse_freq("proc freq data=a; weight wt; by g; tables x*y / list; run;").unwrap();
    assert_eq!(ast.weight.as_deref(), Some("wt"));
    assert_eq!(ast.by, vec![("g".to_string(), false)]);
    assert!(ast.tables[0].list);
    assert_eq!(ast.tables[0].vars, vec!["x", "y"]);
}

#[test]
fn parse_three_way_spec() {
    let ast = parse_freq("proc freq data=a; tables a*b*c; run;").unwrap();
    assert_eq!(ast.tables[0].vars, vec!["a", "b", "c"]);
}

// ───────────────────────────── tally tests ─────────────────────────────

#[test]
fn tally_excludes_missing_by_default() {
    let col = vec![
        Value::Num(2.0),
        Value::Num(1.0),
        Value::Num(2.0),
        Value::missing(),
    ];
    let rows: Vec<usize> = (0..col.len()).collect();
    let (cats, nm) = tally(&col, &rows, false, None);
    assert_eq!(nm, 1.0);
    // sas_cmp order: 1 then 2.
    assert_eq!(cats.len(), 2);
    assert_eq!(cats[0].value, Value::Num(1.0));
    assert_eq!(cats[0].freq, 1.0);
    assert_eq!(cats[1].value, Value::Num(2.0));
    assert_eq!(cats[1].freq, 2.0);
}

#[test]
fn tally_includes_missing_when_requested() {
    let col = vec![Value::Num(2.0), Value::missing(), Value::Num(2.0)];
    let rows: Vec<usize> = (0..col.len()).collect();
    let (cats, nm) = tally(&col, &rows, true, None);
    assert_eq!(nm, 1.0);
    // Missing sorts before numbers.
    assert_eq!(cats.len(), 2);
    assert!(cats[0].value.is_missing());
    assert_eq!(cats[0].freq, 1.0);
    assert_eq!(cats[1].value, Value::Num(2.0));
    assert_eq!(cats[1].freq, 2.0);
}

// ───────────────────────────── execute tests ───────────────────────────

#[test]
fn execute_one_way_default_excludes_missing() {
    let mut session = make_session();
    // x = 1,1,2,. -> non-missing denom = 3.
    let df = df!["x" => [Some(1.0_f64), Some(1.0), Some(2.0), None]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = FreqAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        tables: vec![tr(&["x"], false, None)],
        weight: None,
        by: Vec::new(),
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("The FREQ Procedure"), "{listing}");
    assert!(listing.contains("Frequency Missing = 1"), "{listing}");
    // 1: freq 2, percent 2/3 = 66.67; 2: freq 1, 33.33; cumulative 100.00.
    assert!(listing.contains("66.67"), "{listing}");
    assert!(listing.contains("33.33"), "{listing}");
    assert!(listing.contains("100.00"), "{listing}");
    // cumulative frequency 3 present.
    assert!(listing.contains('3'), "{listing}");
}

#[test]
fn execute_one_way_missing_option_includes_it() {
    let mut session = make_session();
    let df = df!["x" => [Some(1.0_f64), Some(1.0), Some(2.0), None]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = FreqAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        tables: vec![tr(&["x"], true, None)],
        weight: None,
        by: Vec::new(),
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // With MISSING the denom is 4: 1 -> 2/4 = 50.00; missing -> 1/4 = 25.00.
    assert!(listing.contains("50.00"), "{listing}");
    assert!(listing.contains("25.00"), "{listing}");
    // No "Frequency Missing" footnote when MISSING is set.
    assert!(!listing.contains("Frequency Missing"), "{listing}");
}

#[test]
fn execute_out_dataset() {
    let mut session = make_session();
    // x = a,a,b -> a:2 (66.67), b:1 (33.33).
    let df = df!["x" => ["a", "a", "b"]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = FreqAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        tables: vec![tr(
            &["x"],
            false,
            Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "O".into(),
            }),
        )],
        weight: None,
        by: Vec::new(),
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 2);
    let cat = read_col(&session, "O", "x");
    let count = read_col(&session, "O", "COUNT");
    let pct = read_col(&session, "O", "PERCENT");
    // sas_cmp order: a then b.
    assert_eq!(cat, vec![Value::Char("a".into()), Value::Char("b".into())]);
    assert_eq!(count, vec![Value::Num(2.0), Value::Num(1.0)]);
    if let (Value::Num(pa), Value::Num(pb)) = (&pct[0], &pct[1]) {
        assert!((pa - 200.0 / 3.0).abs() < 1e-9, "pa={pa}");
        assert!((pb - 100.0 / 3.0).abs() < 1e-9, "pb={pb}");
    } else {
        panic!("percent must be numeric: {pct:?}");
    }

    let log = session.log.into_string();
    assert!(
        log.contains("The data set WORK.O has 2 observations and 3 variables."),
        "log: {log}"
    );
}

#[test]
fn execute_crosstab_counts_and_total() {
    let mut session = make_session();
    // 2x2: (a,1),(a,2),(b,1),(b,1)
    // rows a: 1->1, 2->1 ; b: 1->2, 2->0. grand=4.
    let df = df![
        "r" => ["a", "a", "b", "b"],
        "c" => [1.0_f64, 2.0, 1.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("r"), num_meta("c")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = FreqAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        tables: vec![tr(&["r", "c"], false, None)],
        weight: None,
        by: Vec::new(),
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Table of r by c"), "{listing}");
    // Grand total 4 and column total for c=1 is 3.
    assert!(listing.contains("Total"), "{listing}");
    // The 4 stacked-cell legend.
    assert!(listing.contains("Row Pct"), "{listing}");
    // Grand-total percent 100.00 must appear.
    assert!(listing.contains("100.00"), "{listing}");
}

// ---- end-to-end through execute() ----

#[test]
fn execute_fisher_measures_agree_end_to_end() {
    let mut session = make_session();
    // Build [[20,10],[5,25]] from raw columns.
    let mut r: Vec<&str> = Vec::new();
    let mut c: Vec<f64> = Vec::new();
    for _ in 0..20 {
        r.push("a");
        c.push(1.0);
    }
    for _ in 0..10 {
        r.push("a");
        c.push(2.0);
    }
    for _ in 0..5 {
        r.push("b");
        c.push(1.0);
    }
    for _ in 0..25 {
        r.push("b");
        c.push(2.0);
    }
    let df = df!["r" => r, "c" => c].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("r"), num_meta("c")],
    };
    write_dataset(&mut session, "T", ds);

    let mut req = tr(&["r", "c"], false, None);
    req.fisher = true;
    req.measures = true;
    req.agree = true;
    let ast = FreqAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        tables: vec![req],
        weight: None,
        by: Vec::new(),
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Fisher's Exact Test"), "{listing}");
    assert!(
        listing.contains("Estimates of the Relative Risk"),
        "{listing}"
    );
    assert!(listing.contains("Simple Kappa Coefficient"), "{listing}");
    assert!(listing.contains("10.0000"), "OR=10:\n{listing}");
}

#[test]
fn execute_one_way_chisq_end_to_end() {
    let mut session = make_session();
    let mut x: Vec<f64> = Vec::new();
    for _ in 0..10 {
        x.push(1.0);
    }
    for _ in 0..20 {
        x.push(2.0);
    }
    for _ in 0..30 {
        x.push(3.0);
    }
    for _ in 0..40 {
        x.push(4.0);
    }
    let df = df!["x" => x].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let mut req = tr(&["x"], false, None);
    req.chisq = true;
    let ast = FreqAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        tables: vec![req],
        weight: None,
        by: Vec::new(),
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(
        listing.contains("Chi-Square Test for Equal Proportions"),
        "{listing}"
    );
    assert!(listing.contains("20.0000"), "{listing}");
}

// ───────────────────────── chisq_sf tests ─────────────────────────

#[test]
fn chisq_sf_known_values() {
    // 95th percentile of chi-square(1) is 3.841 -> upper tail ~ 0.05.
    assert!(
        (chisq_sf(3.841, 1.0) - 0.05).abs() < 1e-3,
        "{}",
        chisq_sf(3.841, 1.0)
    );
    // At 0 the survival function is 1.
    assert!((chisq_sf(0.0, 1.0) - 1.0).abs() < 1e-12);
    // Far in the tail -> ~0.
    assert!(chisq_sf(100.0, 1.0) < 1e-3);
}

// ---- CHISQ one-way goodness of fit ----

#[test]
fn chisq_one_way_equal_proportions() {
    // 4 categories, counts 10,20,30,40. N=100, exp=25 each.
    // chisq = (15²+5²+5²+15²)/25 = (225+25+25+225)/25 = 500/25 = 20.
    // DF=3, p = chisq_sf(20,3) ~ 0.00017.
    let cats = vec![
        Category {
            value: Value::Num(1.0),
            freq: 10.0,
        },
        Category {
            value: Value::Num(2.0),
            freq: 20.0,
        },
        Category {
            value: Value::Num(3.0),
            freq: 30.0,
        },
        Category {
            value: Value::Num(4.0),
            freq: 40.0,
        },
    ];
    let out = run_block(|s| chisq_one_way_block(s, &cats));
    assert!(
        out.contains("Chi-Square Test for Equal Proportions"),
        "{out}"
    );
    assert!(out.contains("20.0000"), "{out}");
    let p = chisq_sf(20.0, 3.0);
    assert!((p - 0.00017).abs() < 1e-4, "p={p}");
}

#[test]
fn chisq_one_way_uniform_is_zero() {
    let cats = vec![
        Category {
            value: Value::Num(1.0),
            freq: 25.0,
        },
        Category {
            value: Value::Num(2.0),
            freq: 25.0,
        },
        Category {
            value: Value::Num(3.0),
            freq: 25.0,
        },
        Category {
            value: Value::Num(4.0),
            freq: 25.0,
        },
    ];
    let out = run_block(|s| chisq_one_way_block(s, &cats));
    assert!(out.contains("0.0000"), "{out}");
}

#[test]
fn chisq_one_way_degenerate_note() {
    let cats = vec![Category {
        value: Value::Num(1.0),
        freq: 5.0,
    }];
    let out = run_block(|s| chisq_one_way_block(s, &cats));
    assert!(out.contains("not computable"), "{out}");
}

#[test]
fn one_way_nofreq_drops_frequency_column() {
    let l = one_way_listing(|r| r.nofreq = true);
    // The standalone "Frequency" header is gone, but "Cumulative
    // Frequency" (NOCUM not set) remains.
    let default = one_way_listing(|_| {});
    let default_freq = default.matches("Frequency").count();
    assert_eq!(l.matches("Frequency").count(), default_freq - 1, "{l}");
    assert!(l.contains("Percent"), "{l}");
    assert!(l.contains("Cumulative Frequency"), "{l}");
}

#[test]
fn one_way_nopercent_drops_percent_columns() {
    let l = one_way_listing(|r| r.nopercent = true);
    assert!(!l.contains("Percent"), "{l}");
    assert!(l.contains("Frequency"), "{l}");
    assert!(l.contains("Cumulative Frequency"), "{l}");
}

#[test]
fn one_way_nocum_drops_cumulative_columns() {
    let l = one_way_listing(|r| r.nocum = true);
    assert!(!l.contains("Cumulative"), "{l}");
    assert!(l.contains("Frequency"), "{l}");
    assert!(l.contains("Percent"), "{l}");
}
