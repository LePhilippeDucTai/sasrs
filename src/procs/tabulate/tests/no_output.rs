use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

#[test]
fn no_output_dataset_set() {
    let mut session = make_session();
    let df = df!["region" => ["E", "W"]].unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("region")] };
    write_dataset(&mut session, "T", ds);
    let before = session.last_dataset.clone();

    let ast = parse_src("proc tabulate data=work.t; class region; table region; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    // last_dataset unchanged when no OUT= is requested.
    assert_eq!(session.last_dataset, before);
}

// ─────────────── M33.4: labels in headers ───────────────

#[test]
fn explicit_label_overrides_stat_header() {
    let mut session = make_session();
    class_fixture(&mut session);
    // mean='Average' replaces the "Mean" header text; sex levels unchanged.
    let listing = run(
        session,
        "proc tabulate data=work.c; class sex; var height; \
         table sex, height*mean='Average'; run;",
    )
    .unwrap();
    assert!(listing.contains("Average"), "{listing}");
    assert!(!listing.contains("Mean"), "{listing}");
}

#[test]
fn stored_varmeta_label_is_default_header() {
    let mut session = make_session();
    let df = df![
        "sex"    => ["M", "F", "M"],
        "height" => [69.0_f64, 56.5, 57.3]
    ]
    .unwrap();
    let mut hmeta = num_meta("height");
    hmeta.label = Some("Height (in)".to_string());
    let ds = SasDataset { df, vars: vec![char_meta("sex"), hmeta] };
    write_dataset(&mut session, "T", ds);
    // No explicit label on `height`, but its stored LABEL is the default.
    let listing = run(
        session,
        "proc tabulate data=work.t; class sex; var height; \
         table sex, height*sum; run;",
    )
    .unwrap();
    assert!(listing.contains("Height (in)*Sum"), "{listing}");
}

// ─────────────── M33.4: FORMAT= cell formatting ───────────────

#[test]
fn per_cell_format_8_2() {
    let mut session = make_session();
    class_fixture(&mut session);
    // height means: M=(69+57.3+62.5)/3=62.933.., F=(56.5+65.3)/2=60.9.
    // *f=8.2 → "62.93" and "60.90".
    let listing = run(
        session,
        "proc tabulate data=work.c; class sex; var height; \
         table sex, height*mean*f=8.2; run;",
    )
    .unwrap();
    assert!(listing.contains("62.93"), "{listing}");
    assert!(listing.contains("60.90"), "{listing}");
}

#[test]
fn per_cell_format_overrides_table_format() {
    let mut session = make_session();
    class_fixture(&mut session);
    // table default 8.0, but the cell asks for 8.2 → 62.93 wins.
    let listing = run(
        session,
        "proc tabulate data=work.c format=8.0; class sex; var height; \
         table sex, height*mean*f=8.2; run;",
    )
    .unwrap();
    assert!(listing.contains("62.93"), "{listing}");
}

#[test]
fn table_level_format_default() {
    let mut session = make_session();
    class_fixture(&mut session);
    // format=8.1 default applies to every cell. M mean=62.933->62.9,
    // F mean=60.9->60.9.
    let listing = run(
        session,
        "proc tabulate data=work.c format=8.1; class sex; var height; \
         table sex, height*mean; run;",
    )
    .unwrap();
    assert!(listing.contains("62.9"), "{listing}");
    assert!(listing.contains("60.9"), "{listing}");
}

// ─────────────── M33.4: OUT= dataset ───────────────

#[test]
fn out_dataset_shape_and_values() {
    let mut session = make_session();
    let df = df![
        "region" => ["E", "E", "W"],
        "sales"  => [10.0_f64, 20.0, 8.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("region"), num_meta("sales")] };
    write_dataset(&mut session, "T", ds);

    let ast = parse_src(
        "proc tabulate data=work.t out=work.o; class region; var sales; \
         table region, sales*mean; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.O"));

    let (out, _notes) = session.libs.get("WORK").unwrap().read("O").unwrap();
    // Columns: region, _TYPE_, _PAGE_, _TABLE_, sales_Mean.
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    assert_eq!(
        names,
        vec!["region", "_TYPE_", "_PAGE_", "_TABLE_", "sales_Mean"],
        "OUT= column shape"
    );
    // One row per column cell (region E, region W) = 2 rows.
    assert_eq!(out.n_obs(), 2);

    // Decode and check values: both rows _TYPE_="1", _PAGE_=1, _TABLE_=1.
    let region = decode_column(&out, 0).unwrap();
    let ty = out.df.column("_TYPE_").unwrap().str().unwrap();
    let mean = decode_column(&out, 4).unwrap();
    // Rows ordered by column-cell expansion: E then W (sas_cmp).
    assert_eq!(region[0], Value::Char("E".into()));
    assert_eq!(region[1], Value::Char("W".into()));
    assert_eq!(ty.get(0), Some("1"));
    assert_eq!(ty.get(1), Some("1"));
    // E mean = 15, W mean = 8.
    assert_eq!(mean[0], Value::Num(15.0));
    assert_eq!(mean[1], Value::Num(8.0));
}

#[test]
fn out_dataset_frequency_stat_name() {
    let mut session = make_session();
    let df = df!["region" => ["E", "E", "W"]].unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("region")] };
    write_dataset(&mut session, "T", ds);

    let ast = parse_src(
        "proc tabulate data=work.t out=work.o; class region; table region; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let (out, _n) = session.libs.get("WORK").unwrap().read("O").unwrap();
    // Pure-frequency cell → stat column named "N" (no analysis VAR).
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    assert_eq!(names, vec!["region", "_TYPE_", "_PAGE_", "_TABLE_", "N"]);
    let n = decode_column(&out, 4).unwrap();
    // E freq = 2, W freq = 1.
    assert_eq!(n[0], Value::Num(2.0));
    assert_eq!(n[1], Value::Num(1.0));
}
