use thiserror::Error;

/// Errors produced by [`super::lower`].
#[derive(Debug, Error, PartialEq)]
pub enum HirError {
    #[error("undefined name '{name}' at byte offset {offset}")]
    UndefinedName { name: String, offset: usize },

    #[error("local '{name}' already defined in this scope (offset {offset})")]
    RedefinedLocal { name: String, offset: usize },

    #[error("unknown builtin '{name}' at byte offset {offset}")]
    UnknownBuiltin { name: String, offset: usize },

    #[error("builtin '{builtin}' expects {expected} argument(s), got {actual} (offset {offset})")]
    ArityMismatch {
        builtin: String,
        expected: usize,
        actual: usize,
        offset: usize,
    },

    #[error("unsupported call form at byte offset {offset}")]
    UnsupportedCall { offset: usize },
}
