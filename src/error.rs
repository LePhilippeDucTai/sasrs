use crate::token::Span;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SasError {
    #[error("{msg}")]
    Parse { msg: String, span: Span },

    #[error("{msg}")]
    Runtime { msg: String },

    #[error("file error: {0}")]
    Io(#[from] std::io::Error),

    #[error("data engine error: {0}")]
    Polars(#[from] polars::error::PolarsError),
}

impl SasError {
    pub fn parse(msg: impl Into<String>, span: Span) -> Self {
        SasError::Parse {
            msg: msg.into(),
            span,
        }
    }

    pub fn runtime(msg: impl Into<String>) -> Self {
        SasError::Runtime { msg: msg.into() }
    }
}

pub type Result<T> = std::result::Result<T, SasError>;
