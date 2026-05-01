use thiserror::Error;

/// Errors produced by [`super::lex`].
///
/// Variants carry byte offsets into the original source so diagnostics can
/// map them to line/column at the presentation layer. See ADR 0003.
#[derive(Debug, Error, PartialEq)]
pub enum LexError {
    #[error("unexpected character {ch:?} at byte offset {offset}")]
    Unexpected { ch: char, offset: usize },

    /// String literal whose closing quote is missing or eclipsed by EOF
    /// (Phase 2.7a, ADR 0024).
    #[error("unterminated string literal starting at byte offset {offset}")]
    UnterminatedString { offset: usize },

    /// Backslash escape with an unrecognised follower (Phase 2.7a).
    #[error("invalid escape sequence {seq:?} at byte offset {offset}")]
    InvalidEscape { seq: String, offset: usize },
}
