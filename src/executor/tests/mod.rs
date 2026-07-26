use crate::{RunOptions, run};

fn run_det(src: &str) -> crate::RunOutcome {
    run(
        src,
        RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    )
}

// ── ODS GRAPHICS (M29.1) ─────────────────────────────────────────────────

/// Construit une Session déterministe et exécute UN statement `ODS GRAPHICS`
/// dessus, en renvoyant la Session pour inspection de `session.ods_graphics`.
fn run_graphics_stmt(src: &str) -> crate::session::Session {
    use crate::parser::StatementStream;
    use crate::parser::global::parse_global;
    use crate::source::SourceFile;
    let mut session = crate::session::Session::new(None, std::env::temp_dir(), true).unwrap();
    let sf = SourceFile::new(src);
    let mut ts = StatementStream::new(&sf).unwrap();
    let stmt = parse_global(&mut ts).unwrap();
    super::exec_global(&stmt, &mut session);
    session
}

// ── M38.1 : TITLE/FOOTNOTE multi-niveaux de bout en bout ──────────────────

/// Exécute une suite de statements globaux sur une même session déterministe.
fn run_globals(srcs: &[&str]) -> crate::session::Session {
    use crate::parser::StatementStream;
    use crate::parser::global::parse_global;
    use crate::source::SourceFile;
    let mut session = crate::session::Session::new(None, std::env::temp_dir(), true).unwrap();
    for src in srcs {
        let sf = SourceFile::new(*src);
        let mut ts = StatementStream::new(&sf).unwrap();
        let stmt = parse_global(&mut ts).unwrap();
        super::exec_global(&stmt, &mut session);
    }
    session
}

mod call;
mod end;
mod options;
