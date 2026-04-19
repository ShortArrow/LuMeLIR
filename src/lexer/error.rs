use thiserror::Error;

/// Errors produced by [`super::lex`].
///
/// Variants carry byte offsets into the original source so diagnostics can
/// map them to line/column at the presentation layer. See ADR 0003.
#[derive(Debug, Error, PartialEq)]
pub enum LexError {
    #[error("unexpected character {ch:?} at byte offset {offset}")]
    Unexpected { ch: char, offset: usize },
}
