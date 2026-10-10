use super::*;
use crate::source::SourceFile;

fn parse_glm(src: &str) -> Result<GlmAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // glm
    parse(&mut ts)
}

mod contract;
mod convergence;
mod one;
mod output_statement;
mod type1_type3;
