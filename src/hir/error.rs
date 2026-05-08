use thiserror::Error;

/// Errors produced by [`super::lower`].
#[derive(Debug, Error, PartialEq)]
pub enum HirError {
    #[error("undefined name '{name}'")]
    UndefinedName { name: String, offset: usize },

    #[error("builtin '{builtin}' expects {expected} argument(s), got {actual}")]
    ArityMismatch {
        builtin: String,
        expected: usize,
        actual: usize,
        offset: usize,
    },

    #[error("unsupported call form")]
    UnsupportedCall { offset: usize },

    #[error("operator '{op}' has incompatible operand types: lhs={lhs_kind}, rhs={rhs_kind}")]
    TypeMismatch {
        op: String,
        lhs_kind: String,
        rhs_kind: String,
        offset: usize,
    },

    #[error("loop variable '{name}' is read-only inside its `for` body")]
    ReadOnlyAssign { name: String, offset: usize },

    #[error("`break` is not inside any loop")]
    BreakOutsideLoop { offset: usize },

    #[error("`return` is not inside a function")]
    ReturnOutsideFunction { offset: usize },

    #[error("unknown function '{name}'")]
    UnknownFunction { name: String, offset: usize },

    #[error("function value '{name}' can only be called, not used as a value")]
    FunctionUsedAsValue { name: String, offset: usize },

    /// A closure value carrying upvalues was used in a position that
    /// would route it through `Callee::Indirect` (call argument or
    /// return value). Indirect dispatch cannot thread upvalues, so
    /// the closure must be reached via a direct call (Phase 2.5c.3,
    /// ADR 0044).
    #[error("closure with upvalues cannot escape via {position} — direct call only")]
    ClosureEscapes { position: String, offset: usize },

    /// Phase 2.6c-tag-callee-arity (ADR 0075): an indirect call
    /// through a TaggedValue local cannot prove the callee's
    /// arity at HIR time — the slot's payload is a bare function
    /// pointer with no signature descriptor. ADR 0072's
    /// `args.len()` reconstruction was unsound (UB on arity
    /// mismatch); ADR 0075 rejects the path entirely. Workaround:
    /// use a direct call or expand a static dispatch at the
    /// call site.
    ///
    /// **Status**: variant retained for binary-compatibility with
    /// older lowering tests. ADR 0082 supersedes this rejection
    /// with `IndirectCallNoCandidates` for the cases where no user
    /// function in the module matches the call site's signature.
    #[error(
        "indirect call through TaggedValue local '{local_name}' is not supported \
         (LIC-2.6c-tag-callee-arity-1; ADR 0075). Use a direct call or static dispatch."
    )]
    IndirectCallThroughTaggedLocal { local_name: String, offset: usize },

    /// Phase 2.5x-callee-dispatch (ADR 0082): an indirect call
    /// through a TaggedValue local could not be lowered because
    /// the call site's expected signature (`param_kinds` /
    /// `ret_kinds`) doesn't match any user function declared in the
    /// module. The compatible-candidate set is empty, so the
    /// per-call-site dispatch chain has no branches to emit.
    /// Workaround: declare a user function with the matching
    /// signature, or call the function value through a direct path
    /// (bind it to a `Function(arity)` local).
    #[error(
        "indirect call through TaggedValue local '{local_name}' has no compatible user \
         function in this module (param_kinds={param_kinds:?}, ret_kinds={ret_kinds:?}; \
         ADR 0082)"
    )]
    IndirectCallNoCandidates {
        local_name: String,
        param_kinds: Vec<crate::hir::ValueKind>,
        ret_kinds: Vec<crate::hir::ValueKind>,
        offset: usize,
    },

    /// Phase 2.5c-full Commit 2a-fix (ADR 0083 / ADR 0075 amend): a
    /// function value whose `ret_kinds` is not exactly `[Number]`
    /// cannot flow into a `Function`-kind parameter, because inside
    /// the receiving user fn it would be invoked via `Callee::Indirect`
    /// — a path whose codegen hardcodes an `f64` MLIR result type
    /// (the LLVM-dialect indirect call inferred from operand /
    /// result types). The `!llvm.ptr` Function-value erasure
    /// landed in Commit 2a removed the verifier-level safety net
    /// that previously catch'd this mismatch, so we reject it at
    /// HIR until ADR 0087 routes parameter-routed indirect calls
    /// through `Callee::IndirectDispatch` (which carries the full
    /// `ret_kinds`).
    #[error(
        "function '{source_name}' returning {ret_kinds:?} cannot be passed as a \
         Function-kind argument — only ret_kinds=[Number] is supported here (ADR 0075 \
         amend / ADR 0083 Commit 2a-fix; lifts when ADR 0087 ships)"
    )]
    IndirectCallNonNumberReturn {
        source_name: String,
        ret_kinds: Vec<crate::hir::ValueKind>,
        offset: usize,
    },

    /// Phase 2.5c-full Commit 3b prep fix (ADR 0083): a `local
    /// function f() ... end` whose body captures upvalues cannot
    /// be called from outside its body via the `function_names`
    /// fallback (= without resolving through a visible local
    /// binding) when the call site is **not** the capturing fn's
    /// own self-recursion. Mutual recursion of two capturing fns
    /// at the same scope hits this path because Function-kind
    /// upvalues are still rejected (`lookup_or_capture_upvalue`)
    /// and the synthetic FunctionDef-backing local isn't in the
    /// caller body's scopes. ADR 0087 (candidate) lifts this
    /// restriction once Function-kind upvalues are allowed.
    #[error(
        "mutual recursion between capturing functions is not supported \
         (ADR 0083 / future ADR 0087): '{local_name}' calls a capturing fn \
         from outside its body"
    )]
    MutualCapturingRecursion { local_name: String, offset: usize },
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
            | HirError::ClosureEscapes { offset, .. }
            | HirError::IndirectCallThroughTaggedLocal { offset, .. }
            | HirError::IndirectCallNoCandidates { offset, .. }
            | HirError::IndirectCallNonNumberReturn { offset, .. }
            | HirError::MutualCapturingRecursion { offset, .. } => *offset,
        }
    }
}
