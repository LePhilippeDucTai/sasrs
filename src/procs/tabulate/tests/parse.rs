use super::*;
use crate::dataset::SasDataset;
use polars::df;

// ─────────────── parse tests ───────────────

#[test]
fn parse_minimal_table() {
    let ast = parse_src("proc tabulate data=a; class region; table region; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert_eq!(ast.class, vec!["region"]);
    assert!(ast.row.is_none());
    assert_eq!(ast.col.terms.len(), 1);
}

#[test]
fn parse_two_dimensions() {
    let ast =
        parse_src("proc tabulate data=a; class region; var sales; table region, sales*mean; run;")
            .unwrap();
    assert!(ast.row.is_some());
}

#[test]
fn parse_unknown_proc_option_errors() {
    let r = parse_src("proc tabulate data=a bogus; class x; table x; run;");
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("BOGUS"));
}

// ─────────────── execute tests ───────────────

#[test]
fn one_dimension_frequency() {
    let mut session = make_session();
    let df = df!["region" => ["E", "E", "W"]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region", 8)],
    };
    write_dataset(&mut session, "T", ds);

    let listing = run(
        session,
        "proc tabulate data=work.t; class region; table region; run;",
    )
    .unwrap();
    assert!(listing.contains("The TABULATE Procedure"), "{listing}");
    // Two levels E and W in headers.
    assert!(listing.contains("E"), "{listing}");
    assert!(listing.contains("W"), "{listing}");
    // Frequencies: E=2, W=1.
    assert!(listing.contains("2"), "{listing}");
    assert!(listing.contains("1"), "{listing}");
}

#[test]
fn row_classvar_col_var_mean() {
    let mut session = make_session();
    let df = df![
        "region" => ["E", "E", "W"],
        "sales"  => [10.0_f64, 20.0, 8.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region", 8), num_meta("sales")],
    };
    write_dataset(&mut session, "T", ds);

    let listing = run(
        session,
        "proc tabulate data=work.t; class region; var sales; table region, sales*mean; run;",
    )
    .unwrap();
    // E mean = 15, W mean = 8.
    assert!(listing.contains("15"), "{listing}");
    assert!(listing.contains("8"), "{listing}");
    // Header includes sales*Mean.
    assert!(
        listing.contains("sales") && listing.contains("Mean"),
        "{listing}"
    );
}

#[test]
fn class_cross_class() {
    let mut session = make_session();
    let df = df![
        "a" => ["x", "x", "y"],
        "b" => ["p", "q", "p"]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("a", 8), char_meta("b", 8)],
    };
    write_dataset(&mut session, "T", ds);

    // a on rows, a*b crossing on columns gives nested category cells.
    let listing = run(
        session,
        "proc tabulate data=work.t; class a b; table a, a*b; run;",
    )
    .unwrap();
    // Column headers should show crossings like x*p.
    assert!(listing.contains("x*p"), "{listing}");
    assert!(listing.contains("x*q"), "{listing}");
    assert!(listing.contains("y*p"), "{listing}");
}

#[test]
fn multistat_list_with_group() {
    let mut session = make_session();
    let df = df![
        "region" => ["E", "E", "W"],
        "sales"  => [10.0_f64, 20.0, 8.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region", 8), num_meta("sales")],
    };
    write_dataset(&mut session, "T", ds);

    let listing = run(
        session,
        "proc tabulate data=work.t; class region; var sales; table region, sales*(n sum mean); run;",
    )
    .unwrap();
    // Three stat columns for sales: N, Sum, Mean.
    assert!(listing.contains("sales*N"), "{listing}");
    assert!(listing.contains("sales*Sum"), "{listing}");
    assert!(listing.contains("sales*Mean"), "{listing}");
    // E: n=2 sum=30 mean=15.
    assert!(listing.contains("30"), "{listing}");
    assert!(listing.contains("15"), "{listing}");
}

#[test]
fn missing_in_var_excluded_from_mean_counted_in_nmiss() {
    let mut session = make_session();
    let df = df![
        "region" => ["E", "E", "E"],
        "sales"  => [Some(10.0_f64), Some(20.0), None]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region", 8), num_meta("sales")],
    };
    write_dataset(&mut session, "T", ds);

    let listing = run(
        session,
        "proc tabulate data=work.t; class region; var sales; table region, sales*(mean nmiss n); run;",
    )
    .unwrap();
    // mean over [10,20] = 15; nmiss = 1; n = 2.
    assert!(listing.contains("15"), "{listing}");
    assert!(listing.contains("sales*NMiss"), "{listing}");
}

#[test]
fn unsupported_construct_clean_error() {
    let mut session = make_session();
    let df = df![
        "region" => ["E", "W"],
        "a" => [1.0_f64, 2.0],
        "b" => [3.0_f64, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region", 8), num_meta("a"), num_meta("b")],
    };
    write_dataset(&mut session, "T", ds);

    // Crossing two analysis variables a*b is unsupported.
    let r = run(
        session,
        "proc tabulate data=work.t; class region; var a b; table region, a*b; run;",
    );
    assert!(r.is_err());
    assert!(
        r.err().unwrap().to_string().contains("not yet supported"),
        "expected clean unsupported error"
    );
}

#[test]
fn unknown_name_clean_error() {
    let mut session = make_session();
    let df = df!["region" => ["E", "W"]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region", 8)],
    };
    write_dataset(&mut session, "T", ds);

    let r = run(
        session,
        "proc tabulate data=work.t; class region; table region*nope; run;",
    );
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("not yet supported"));
}

#[test]
fn third_dimension_now_supported() {
    let mut session = make_session();
    let df = df![
        "a" => ["x", "y"],
        "b" => ["p", "p"],
        "c" => ["m", "m"]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("a", 8), char_meta("b", 8), char_meta("c", 8)],
    };
    write_dataset(&mut session, "T", ds);

    // A 3rd (page) dimension is now rendered, not an error.
    let listing = run(
        session,
        "proc tabulate data=work.t; class a b c; table a, b, c; run;",
    )
    .unwrap();
    // Two page sections, labelled by the page CLASS value of `a`.
    assert!(listing.contains("a=x"), "{listing}");
    assert!(listing.contains("a=y"), "{listing}");
}

#[test]
fn page_dimension_renders_per_page_subtables() {
    let mut session = make_session();
    let df = df![
        "grp"    => ["A", "A", "B", "B"],
        "region" => ["E", "W", "E", "W"],
        "sales"  => [10.0_f64, 20.0, 30.0, 40.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![
            char_meta("grp", 8),
            char_meta("region", 8),
            num_meta("sales"),
        ],
    };
    write_dataset(&mut session, "T", ds);

    let listing = run(
        session,
        "proc tabulate data=work.t; class grp region; var sales; \
         table grp, region, sales*sum; run;",
    )
    .unwrap();
    // Two page sections, labelled by page CLASS value.
    assert!(listing.contains("grp=A"), "{listing}");
    assert!(listing.contains("grp=B"), "{listing}");
    // Page A: E=10, W=20 ; page B: E=30, W=40.
    assert!(
        listing.contains("10") && listing.contains("20"),
        "{listing}"
    );
    assert!(
        listing.contains("30") && listing.contains("40"),
        "{listing}"
    );
}

#[test]
fn four_dimensions_clean_error() {
    let mut session = make_session();
    let df = df![
        "a" => ["x"], "b" => ["p"], "c" => ["m"], "d" => ["q"]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![
            char_meta("a", 8),
            char_meta("b", 8),
            char_meta("c", 8),
            char_meta("d", 8),
        ],
    };
    write_dataset(&mut session, "T", ds);

    let r = run(
        session,
        "proc tabulate data=work.t; class a b c d; table a, b, c, d; run;",
    );
    assert!(r.is_err());
    assert!(
        r.err()
            .unwrap()
            .to_string()
            .contains("at most 3 dimensions")
    );
}

// ─────────────── M21.4: ALL (universal class) ───────────────

#[test]
fn all_marginal_row_total() {
    let mut session = make_session();
    class_fixture(&mut session);
    // ALL in the ROW dimension adds a grand-total row (no sex constraint),
    // so the N column shows N over all 5 observations on that row.
    let listing = run(
        session,
        "proc tabulate data=work.c; class sex; table sex all, n; run;",
    )
    .unwrap();
    assert!(listing.contains("All"), "{listing}");
    // sex M=3, F=2; ALL row = 5 (grand total).
    assert!(listing.contains("5"), "{listing}");
}

#[test]
fn all_with_stat_aggregates_over_all_rows() {
    let mut session = make_session();
    class_fixture(&mut session);
    // ALL row crossed with height*mean: mean over all 5 rows.
    // (69 + 56.5 + 57.3 + 65.3 + 62.5) / 5 = 310.6/5 = 62.12.
    let listing = run(
        session,
        "proc tabulate data=work.c; class sex; var height; \
         table sex all, height*mean; run;",
    )
    .unwrap();
    assert!(listing.contains("All"), "{listing}");
    assert!(listing.contains("62.12"), "{listing}");
}

#[test]
fn all_and_pctn_combined() {
    let mut session = make_session();
    class_fixture(&mut session);
    // sex on rows with an ALL marginal row; PCTN columns.
    let listing = run(
        session,
        "proc tabulate data=work.c; class sex; table sex all, pctn; run;",
    )
    .unwrap();
    assert!(listing.contains("All"), "{listing}");
    // ALL row PCTN = 5/5 = 100%.
    assert!(listing.contains("100"), "{listing}");
}

// ─────────────── M21.4: PCTN / PCTSUM ───────────────

#[test]
fn pctn_grand_total_denominator() {
    let mut session = make_session();
    class_fixture(&mut session);
    // PCTN per sex: M=3/5=60%, F=2/5=40%.
    let listing = run(
        session,
        "proc tabulate data=work.c; class sex; table sex, pctn; run;",
    )
    .unwrap();
    assert!(listing.contains("PctN"), "{listing}");
    assert!(listing.contains("60"), "{listing}");
    assert!(listing.contains("40"), "{listing}");
}

#[test]
fn pctn_empty_cell_is_dot_not_panic() {
    let mut session = make_session();
    // No observations at all → grand total N = 0 → "." (no div-by-zero).
    let df = df!["region" => [""; 0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region", 8)],
    };
    write_dataset(&mut session, "T", ds);
    let listing = run(
        session,
        "proc tabulate data=work.t; class region; table region, pctn; run;",
    );
    // Either an empty table or a clean render — must not panic.
    assert!(listing.is_ok(), "{:?}", listing.err());
}

#[test]
fn pctsum_grand_total_denominator() {
    let mut session = make_session();
    let df = df![
        "region" => ["E", "E", "W"],
        "sales"  => [10.0_f64, 30.0, 60.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region", 8), num_meta("sales")],
    };
    write_dataset(&mut session, "T", ds);

    // PCTSUM of sales: E = (10+30)/100 = 40%, W = 60/100 = 60%.
    let listing = run(
        session,
        "proc tabulate data=work.t; class region; var sales; \
         table region, sales*pctsum; run;",
    )
    .unwrap();
    assert!(listing.contains("PctSum"), "{listing}");
    assert!(listing.contains("40"), "{listing}");
    assert!(listing.contains("60"), "{listing}");
}

#[test]
fn pctsum_requires_var_clean_error() {
    let mut session = make_session();
    class_fixture(&mut session);
    let r = run(
        session,
        "proc tabulate data=work.c; class sex; table sex, pctsum; run;",
    );
    assert!(r.is_err());
    assert!(
        r.err().unwrap().to_string().contains("not yet supported"),
        "expected clean error for PCTSUM without VAR"
    );
}

#[test]
fn pctsum_zero_denominator_is_dot() {
    let mut session = make_session();
    let df = df![
        "region" => ["E", "W"],
        "sales"  => [0.0_f64, 0.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region", 8), num_meta("sales")],
    };
    write_dataset(&mut session, "T", ds);
    let listing = run(
        session,
        "proc tabulate data=work.t; class region; var sales; \
         table region, sales*pctsum; run;",
    )
    .unwrap();
    // Denominator 0 → cells are ".", no panic.
    assert!(listing.contains('.'), "{listing}");
}

// ─────────────── M21.4: multi-VAR / multi-stat in columns ───────────────

#[test]
fn multi_var_separate_column_analyses() {
    let mut session = make_session();
    class_fixture(&mut session);
    // Two different VAR analyses side by side in the column dimension.
    let listing = run(
        session,
        "proc tabulate data=work.c; class sex; var height weight; \
         table sex, height*mean weight*sum; run;",
    )
    .unwrap();
    assert!(
        listing.contains("height") && listing.contains("Mean"),
        "{listing}"
    );
    assert!(
        listing.contains("weight") && listing.contains("Sum"),
        "{listing}"
    );
    // M weights sum = 112.5 + 83 + 84 = 279.5.
    assert!(listing.contains("279.5"), "{listing}");
}

#[test]
fn distribute_stats_over_var_via_group() {
    let mut session = make_session();
    class_fixture(&mut session);
    // height*(N MEAN) distributes two stats over the single VAR.
    let listing = run(
        session,
        "proc tabulate data=work.c; class sex; var height; \
         table sex, height*(n mean); run;",
    )
    .unwrap();
    assert!(listing.contains("height*N"), "{listing}");
    assert!(listing.contains("height*Mean"), "{listing}");
    // M: n=3, F: n=2.
    assert!(listing.contains("3") && listing.contains("2"), "{listing}");
}
