use super::*;
use crate::source::SourceFile;
use crate::testkit::*;

fn parse_format_src(src: &str) -> Result<FormatAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "format"
    parse(&mut ts)
}

// ── INVALUE execute tests (M18.2) ─────────────────────────────────────────

fn run_format_src(src: &str) -> crate::session::Session {
    let mut session = make_session();
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // format
    let ast = parse(&mut ts).unwrap();
    execute(&ast, &mut session).unwrap();
    session
}

fn run_det(src: &str) -> crate::RunOutcome {
    crate::run(
        src,
        crate::RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    )
}

mod cntl;
mod execute;
mod parse;
