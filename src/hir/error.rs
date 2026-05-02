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

    #[error("function value '{name}' can only be called, not used as a value (offset {offset})")]
    FunctionUsedAsValue { name: String, offset: usize },

    /// A closure value carrying upvalues was used in a position that
    /// would route it through `Callee::Indirect` (call argument or
    /// return value). Indirect dispatch cannot thread upvalues, so
    /// the closure must be reached via a direct call (Phase 2.5c.3,
    /// ADR 0044).
    #[error(
        "closure with upvalues cannot escape via {position} — direct call only (offset {offset})"
    )]
    ClosureEscapes { position: String, offset: usize },
}

impl HirError {
    /// Phase 2.9a (ADR 0045): byte offset for the diagnostic layer.
    pub fn offset(&self) -> usize {
        match self {
            HirError::UndefinedName { offset, .. }
            | HirError::ArityMismatch { offset, .. }
            | HirError::UnsupportedCall { offset }
            | HirError::TypeMismatch { offset, .. }
            | HirError::ReadOnlyAssign { offset, .. }
            | HirError::BreakOutsideLoop { offset }
            | HirError::ReturnOutsideFunction { offset }
            | HirError::UnknownFunction { offset, .. }
            | HirError::FunctionUsedAsValue { offset, .. }
            | HirError::ClosureEscapes { offset, .. } => *offset,
        }
    }
}
