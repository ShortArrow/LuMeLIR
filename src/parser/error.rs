use thiserror::Error;

use crate::lexer::{LexError, TokenKind};

/// Errors produced by [`super::parse`].
///
/// See ADR 0003 for the library-layer error-handling convention.
#[derive(Debug, Error, PartialEq)]
pub enum ParseError {
    #[error(transparent)]
    Lex(#[from] LexError),

    #[error("unexpected end of input at byte offset {offset}")]
    UnexpectedEof { offset: usize },

    #[error("unexpected token {actual:?} at byte offset {offset}")]
    UnexpectedToken { actual: TokenKind, offset: usize },
}
