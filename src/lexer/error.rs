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

    /// Block comment whose closing `]]` is missing (Phase 2.8c, ADR 0034).
    #[error("unterminated block comment starting at byte offset {offset}")]
    UnterminatedComment { offset: usize },

    /// Long-bracket string whose closing `]==]` is missing
    /// (Phase 2.7j, ADR 0038).
    #[error("unterminated long-bracket string starting at byte offset {offset}")]
    UnterminatedBracket { offset: usize },
}

impl LexError {
    /// Phase 2.9a (ADR 0045): byte offset associated with this error,
    /// used by the CLI diagnostic layer to compute line/column.
    pub fn offset(&self) -> usize {
        match self {
            LexError::Unexpected { offset, .. }
            | LexError::UnterminatedString { offset }
            | LexError::InvalidEscape { offset, .. }
            | LexError::UnterminatedComment { offset }
            | LexError::UnterminatedBracket { offset } => *offset,
        }
    }
}
