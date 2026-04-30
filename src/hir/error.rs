use thiserror::Error;

/// Errors produced by [`super::lower`].
#[derive(Debug, Error, PartialEq)]
pub enum HirError {
    #[error("undefined name '{name}' at byte offset {offset}")]
    UndefinedName { name: String, offset: usize },

    #[error("builtin '{builtin}' expects {expected} argument(s), got {actual} (offset {offset})")]
    ArityMismatch {
        builtin: String,
        expected: usize,
        actual: usize,
        offset: usize,
    },

    #[error("unsupported call form at byte offset {offset}")]
    UnsupportedCall { offset: usize },

    #[error(
        "operator '{op}' has incompatible operand types: lhs={lhs_kind}, rhs={rhs_kind} (offset {offset})"
    )]
    TypeMismatch {
        op: String,
        lhs_kind: String,
        rhs_kind: String,
        offset: usize,
    },

    #[error("loop variable '{name}' is read-only inside its `for` body (offset {offset})")]
    ReadOnlyAssign { name: String, offset: usize },

    #[error("`break` is not inside any loop (offset {offset})")]
    BreakOutsideLoop { offset: usize },

    #[error("`return` is not inside a function (offset {offset})")]
    ReturnOutsideFunction { offset: usize },

    #[error("unknown function '{name}' at byte offset {offset}")]
    UnknownFunction { name: String, offset: usize },
}
