use thiserror::Error;

use crate::lexer::{LexError, TokenKind};

/// Errors produced by [`super::parse`].
///
/// See ADR 0003 for the library-layer error-handling convention.
#[derive(Debug, Error, PartialEq)]
pub enum ParseError {
    #[error(transparent)]
    Lex(#[from] LexError),

    #[error("unexpected end of input")]
    UnexpectedEof { offset: usize },

    #[error("unexpected token {actual:?}")]
    UnexpectedToken { actual: TokenKind, offset: usize },

    /// Phase 2.8e-iter-ipairs (ADR 0078): a `for ... in EXPR do ...`
    /// form whose iterator slot is not the supported
    /// `ipairs(table_expr)` shape. `pairs(t)` and arbitrary
    /// callable iterators are LIC-tracked
    /// (LIC-2.8e-iter-pairs-1 / LIC-2.8e-iter-generic-1) until a
    /// future phase reopens generic-for protocol.
    #[error(
        "unsupported iterator in `for ... in`: only `ipairs(table)` is recognised in this phase (LIC-2.8e-iter-pairs-1 / LIC-2.8e-iter-generic-1)"
    )]
    UnsupportedIterator { offset: usize },
}

impl ParseError {
    /// Phase 2.9a (ADR 0045): byte offset for the diagnostic layer.
    /// `Lex` defers to the wrapped lexer error.
    pub fn offset(&self) -> usize {
        match self {
            ParseError::Lex(e) => e.offset(),
            ParseError::UnexpectedEof { offset }
            | ParseError::UnexpectedToken { offset, .. }
            | ParseError::UnsupportedIterator { offset } => *offset,
        }
    }
}
