use super::*;
use crate::dataset::SasDataset;
use crate::session::Session;
use crate::source::SourceFile;
use crate::testkit::*;
use polars::df;

fn parse_src(src: &str) -> Result<TabulateAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "tabulate"
    parse(&mut ts)
}

/// Parse + execute through a session, returning the listing string.
fn run(mut session: Session, src: &str) -> Result<String> {
    let ast = parse_src(src)?;
    execute(&ast, &mut session)?;
    Ok(session.listing.take_string())
}

// ─────────────── M21.4: page dimension ───────────────

/// Build the classic sashelp.class-like fixture (subset of rows is fine).
fn class_fixture(session: &mut Session) {
    let df = df![
        "sex"    => ["M", "F", "M", "F", "M"],
        "age"    => [14.0_f64, 13.0, 12.0, 13.0, 14.0],
        "height" => [69.0_f64, 56.5, 57.3, 65.3, 62.5],
        "weight" => [112.5_f64, 84.0, 83.0, 98.0, 84.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![
            char_meta("sex", 8),
            num_meta("age"),
            num_meta("height"),
            num_meta("weight"),
        ],
    };
    write_dataset(session, "C", ds);
}

mod no_output;
mod parse;
