use super::*;
use crate::dataset::VarMeta;
use crate::session::Session;
use crate::source::SourceFile;
use crate::value::VarType;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn num_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

fn char_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.into(),
        ty: VarType::Char,
        length: 1,
        format: None,
        label: None,
    }
}

fn parse_glm(src: &str) -> Result<GlmAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // glm
    parse(&mut ts)
}

mod one;
mod type1_type3;
