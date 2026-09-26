//! J04-P4 — Métadonnées après transformations SQL.
//!
//! Reproducer : un `CREATE TABLE AS SELECT` sur une table dont les
//! colonnes portent format/label/longueur (persistés dans le sidecar
//! J04-P1) produisait une table SUCÉE de toute métadonnée — la frame
//! Polars ne transporte pas les VarMeta. Ces tests verrouillent :
//!
//! - colonne reprise telle quelle (`*`, `t.*`, `x`, `t.x`, `AS`) →
//!   format/label/longueur conservés (round-trip parquet+sidecar compris) ;
//! - colonne calculée / agrégat → AUCUNE métadonnée héritée ;
//! - `FORMAT=`/`LABEL=`/`LENGTH=` du select-list → appliqués (et `LENGTH=`
//!   tronque réellement les valeurs caractère) ;
//! - `DELETE FROM` réécrit la table SANS perdre les métadonnées ;
//! - token de format invalide dans le select-list → ERROR.

use super::*;
use crate::sql::ast::SqlItemAttrs;
use crate::testkit::*;

fn var_meta<'a>(ds: &'a SasDataset, name: &str) -> &'a VarMeta {
    ds.vars
        .iter()
        .find(|v| v.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("column {name} missing in {:?}", ds.vars))
}

fn write_meta_table(session: &mut Session) {
    let df = df![
        "name" => ["Alfred", "Alice", "Barbara"],
        "age"  => [14.0_f64, 13.0, 13.0],
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "name".into(),
            ty: VarType::Char,
            length: 12,
            format: Some("$char12.".into()),
            label: Some("Full name".into()),
        },
        VarMeta {
            name: "age".into(),
            ty: VarType::Num,
            length: 8,
            format: Some("8.2".into()),
            label: Some("Age in years".into()),
        },
    ];
    write_table(session, "SRC", df, vars);
}

#[test]
fn sql_metadata_star_preserves_source_attributes() {
    let mut s = make_session();
    write_meta_table(&mut s);
    run_sql("create table t2 as select * from src;", &mut s);
    // La table est relue DEPUIS le disque : le sidecar doit avoir fait le
    // round-trip (c'est le test du chemin « lecture qui applique le
    // sidecar » pour les métadonnées).
    let ds = read_work(&mut s, "T2");
    let name = var_meta(&ds, "name");
    assert_eq!(name.ty, VarType::Char);
    assert_eq!(name.length, 12);
    assert_eq!(name.format.as_deref(), Some("$char12."));
    assert_eq!(name.label.as_deref(), Some("Full name"));
    let age = var_meta(&ds, "age");
    assert_eq!(age.format.as_deref(), Some("8.2"));
    assert_eq!(age.label.as_deref(), Some("Age in years"));
}

#[test]
fn sql_metadata_renamed_column_inherits() {
    let mut s = make_session();
    write_meta_table(&mut s);
    run_sql(
        "create table t3 as select name as fullname, age from src;",
        &mut s,
    );
    let ds = read_work(&mut s, "T3");
    let fullname = var_meta(&ds, "fullname");
    assert_eq!(fullname.ty, VarType::Char);
    assert_eq!(fullname.length, 12);
    assert_eq!(fullname.format.as_deref(), Some("$char12."));
    assert_eq!(fullname.label.as_deref(), Some("Full name"));
    // Les données suivent.
    assert_eq!(ds.n_obs(), 3);
}

#[test]
fn sql_metadata_qualified_star_and_ref() {
    let mut s = make_session();
    write_meta_table(&mut s);
    run_sql("create table t4 as select a.* from src as a;", &mut s);
    let ds = read_work(&mut s, "T4");
    assert_eq!(var_meta(&ds, "age").label.as_deref(), Some("Age in years"));

    run_sql(
        "create table t5 as select a.name, a.age as vieux from src a;",
        &mut s,
    );
    let ds = read_work(&mut s, "T5");
    assert_eq!(var_meta(&ds, "name").format.as_deref(), Some("$char12."));
    assert_eq!(var_meta(&ds, "vieux").format.as_deref(), Some("8.2"));
}

#[test]
fn sql_metadata_calculated_column_has_no_metadata() {
    let mut s = make_session();
    write_meta_table(&mut s);
    run_sql(
        "create table t6 as select age, age * 2 as dbl, count(*) as n from src group by age;",
        &mut s,
    );
    let ds = read_work(&mut s, "T6");
    // Colonne reprise → hérite.
    assert_eq!(var_meta(&ds, "age").label.as_deref(), Some("Age in years"));
    // Colonnes calculées → rien (ni format, ni label).
    let dbl = var_meta(&ds, "dbl");
    assert_eq!(dbl.format, None);
    assert_eq!(dbl.label, None);
    let n = var_meta(&ds, "n");
    assert_eq!(n.format, None);
    assert_eq!(n.label, None);
}

#[test]
fn sql_metadata_select_attrs_on_calculated_column() {
    let mut s = make_session();
    write_meta_table(&mut s);
    run_sql(
        "create table t7 as select age * 2 as dbl format=best8. label='Doubled' \
             from src;",
        &mut s,
    );
    let ds = read_work(&mut s, "T7");
    let dbl = var_meta(&ds, "dbl");
    assert_eq!(dbl.format.as_deref(), Some("best8."));
    assert_eq!(dbl.label.as_deref(), Some("Doubled"));
}

#[test]
fn sql_metadata_length_truncates_char_values() {
    let mut s = make_session();
    write_meta_table(&mut s);
    run_sql(
        "create table t8 as select name label='Court' length=3 from src;",
        &mut s,
    );
    let ds = read_work(&mut s, "T8");
    let name = var_meta(&ds, "name");
    assert_eq!(name.length, 3);
    assert_eq!(name.label.as_deref(), Some("Court"));
    // Troncuction réelle (sémantique SAS) : "Alfred" → "Alf".
    let col = ds.df.column("name").unwrap().str().unwrap();
    let v: Vec<&str> = col.into_no_null_iter().collect();
    assert_eq!(v, vec!["Alf", "Ali", "Bar"]);
}

#[test]
fn sql_metadata_attrs_survive_alias_and_order() {
    let mut s = make_session();
    write_meta_table(&mut s);
    // attributs AVANT l'alias, ordre libre.
    run_sql(
        "create table t9 as select name length=5 format=$5. as nm from src;",
        &mut s,
    );
    let ds = read_work(&mut s, "T9");
    let nm = var_meta(&ds, "nm");
    assert_eq!(nm.length, 5);
    assert_eq!(nm.format.as_deref(), Some("$5."));
}

#[test]
fn sql_metadata_delete_preserves_attributes() {
    let mut s = make_session();
    write_meta_table(&mut s);
    run_sql("delete from src where age > 13;", &mut s);
    let ds = read_work(&mut s, "SRC");
    assert_eq!(ds.n_obs(), 2);
    let name = var_meta(&ds, "name");
    assert_eq!(name.length, 12);
    assert_eq!(name.format.as_deref(), Some("$char12."));
    assert_eq!(name.label.as_deref(), Some("Full name"));
    assert_eq!(var_meta(&ds, "age").label.as_deref(), Some("Age in years"));
}

#[test]
fn sql_metadata_invalid_format_token_errors() {
    let mut s = make_session();
    write_meta_table(&mut s);
    // `.` seul : ni nom, ni largeur — token de format invalide.
    let file = SourceFile::new("create table bad as select age format=. from src;");
    let mut ts = StatementStream::new(&file).unwrap();
    let prog = parse_sql_program(&mut ts).unwrap();
    let err = execute(&prog, &mut s).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("The format . is not valid."),
        "unexpected error: {msg}"
    );
}

// ── parsing des attributs (AST pur) ────────────────────────────────────

#[test]
fn sql_metadata_parse_attrs_ast() {
    let file = SourceFile::new("select x format=date9. label='D' length=4 from t;");
    let mut ts = StatementStream::new(&file).unwrap();
    let prog = parse_sql_program(&mut ts).unwrap();
    let SqlStmt::Select(sel) = &prog.stmts[0] else {
        panic!("expected Select");
    };
    assert_eq!(
        sel.items[0].attrs,
        SqlItemAttrs {
            format: Some("date9.".into()),
            label: Some("D".into()),
            length: Some(4),
        }
    );
    // `format`/`label`/`length` nus restent des alias légitimes.
    let file = SourceFile::new("select x length from t;");
    let mut ts = StatementStream::new(&file).unwrap();
    let prog = parse_sql_program(&mut ts).unwrap();
    let SqlStmt::Select(sel) = &prog.stmts[0] else {
        panic!("expected Select");
    };
    assert_eq!(sel.items[0].alias.as_deref(), Some("length"));
    assert_eq!(sel.items[0].attrs, SqlItemAttrs::default());
}
