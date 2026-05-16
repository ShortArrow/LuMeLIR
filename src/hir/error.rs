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

    // Phase 2.5c-full Commit 3c (ADR 0083 supersedes 0044): the
    // `ClosureEscapes` variant is retired. Closure-with-upvalues
    // values are now sound in any position because the cell-ptr-
    // first ABI (heap cell + heap upvalue boxes) keeps captured-
    // binding reads alive across frame teardown.
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
    /// HIR until a future ADR (Function-kind upvalue support /
    /// TaggedValue arith coerce) routes parameter-routed indirect
    /// calls through `Callee::IndirectDispatch` (which carries the
    /// full `ret_kinds`).
    #[error(
        "function '{source_name}' returning {ret_kinds:?} cannot be passed as a \
         Function-kind argument — only ret_kinds=[Number] is supported here (ADR 0075 \
         amend / ADR 0083 Commit 2a-fix; lifts in future ADR — Function-kind upvalue support)"
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
    /// caller body's scopes. A future ADR (Function-kind upvalue
    /// support) lifts this restriction once Function-kind upvalues
    /// are allowed.
    #[error(
        "mutual recursion between capturing functions is not supported \
         (ADR 0083 / future ADR — Function-kind upvalue support): \
         '{local_name}' calls a capturing fn from outside its body"
    )]
    MutualCapturingRecursion { local_name: String, offset: usize },

    /// Phase 2.6+-methods (ADR 0092): a method-call receiver
    /// contains an expression shape that the MVP shape-walker
    /// rejects — `Call`, `MethodCall`, `FunctionExpr`, `BinOp`, or
    /// `UnaryOp`. The receiver must be a simple form (Ident,
    /// literal, table constructor, or a chain of `[]` / `.` index
    /// suffixes built on those) so the receiver-once evaluation
    /// invariant via `materialize_to_synth_local` stays predictable.
    /// Future ADRs may relax this once side-effect ordering is
    /// resolved for compound receivers.
    #[error(
        "method-call receiver is too complex for MVP shape walker \
         (ADR 0092): must be Ident, Index suffixes, or literal"
    )]
    ComplexMethodReceiver { offset: usize },
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
            | HirError::IndirectCallThroughTaggedLocal { offset, .. }
            | HirError::IndirectCallNoCandidates { offset, .. }
            | HirError::IndirectCallNonNumberReturn { offset, .. }
            | HirError::MutualCapturingRecursion { offset, .. }
            | HirError::ComplexMethodReceiver { offset } => *offset,
        }
    }
}
