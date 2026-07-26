use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::source::SourceFile;
use crate::sql::parser::parse_sql_program;
use crate::value::VarType;
use polars::df;

fn num(name: &str) -> VarMeta {
    VarMeta {
        name: name.into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

fn chr(name: &str, len: usize) -> VarMeta {
    VarMeta {
        name: name.into(),
        ty: VarType::Char,
        length: len,
        format: None,
        label: None,
    }
}

fn write_table(session: &mut Session, name: &str, df: DataFrame, vars: Vec<VarMeta>) {
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
}

fn write_people(session: &mut Session) {
    let df = df![
        "name" => ["Al", "Bo", "Cy", "Di"],
        "sex"  => ["M", "M", "F", "F"],
        "age"  => [10.0_f64, 14.0, 13.0, 11.0],
        "height" => [50.0_f64, 60.0, 55.0, 52.0],
    ]
    .unwrap();
    write_table(
        session,
        "T",
        df,
        vec![chr("name", 8), chr("sex", 1), num("age"), num("height")],
    );
}

/// Parse and execute a PROC SQL body (the statements between `proc sql;`
/// and `quit;`).
fn run_sql(src: &str, session: &mut Session) {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file).unwrap();
    let prog = parse_sql_program(&mut ts).unwrap();
    execute(&prog, session).unwrap();
}

fn read_work(session: &mut Session, name: &str) -> SasDataset {
    session.libs.get("WORK").unwrap().read(name).unwrap().0
}

// ── M20.4 : UPDATE ... SET ──────────────────────────────────────────────

fn ages(ds: &SasDataset) -> Vec<f64> {
    ds.df
        .column("age")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect()
}

mod create;
mod update_multi;
