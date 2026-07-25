use super::*;
use crate::ast::GlobalStmt;
use crate::source::SourceFile;

fn parse(src: &str) -> Result<GlobalStmt> {
    let sf = SourceFile::new(src);
    let mut ts = StatementStream::new(&sf).unwrap();
    parse_global(&mut ts)
}

// ── ODS GRAPHICS (M29.1) ─────────────────────────────────────────────────

use crate::ast::{OdsGraphicsStmt, OdsGraphicsToggle};
use crate::ods_graphics::ImageFmt;

fn graphics_stmt(src: &str) -> OdsGraphicsStmt {
    match parse(src).unwrap() {
        GlobalStmt::OdsGraphics(s) => s,
        other => panic!("expected OdsGraphics, got {other:?}"),
    }
}

mod libname;
mod parse;
