use crate::hir::ValueKind;
use crate::lexer::Span;
use crate::parser::{BinOp, UnaryOp};

/// Index into [`HirChunk::locals`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);

/// Per-local metadata. Phase 2.3a adds the static value kind that
/// determines the stack slot type at codegen time. Phase 2.5b.2 adds
/// `func_id`: when a Function-kind local was bound to a known
/// function (`local f = function() end` or alias), this carries that
/// `FuncId`; for function parameters whose value is only known at
/// runtime, it is `None` and the call site uses `Callee::Indirect`
/// (ADR 0018).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInfo {
    pub name: String,
    pub kind: ValueKind,
    pub func_id: Option<FuncId>,
}

/// A name-resolved program — the input to codegen.
///
/// `functions` carries every `local function` definition discovered at
/// the top level (Phase 2.5a; ADR 0016). `locals` and `stmts` describe
/// the implicit `main` chunk only.
#[derive(Debug, Clone, PartialEq)]
pub struct HirChunk {
    pub locals: Vec<LocalInfo>,
    pub stmts: Vec<HirStmt>,
    pub functions: Vec<HirFunction>,
}

/// Index into [`HirChunk::functions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FuncId(pub usize);

/// A user-defined function. The body is a fully-lowered statement
/// sequence with its own private `locals` table. Phase 2.5d (ADR
/// 0021) generalises the return type from `Option<ValueKind>` to a
/// `Vec<ValueKind>` — empty for void, length 1 for the historical
/// single-return case, length ≥2 for multi-return. See ADRs 0016,
/// 0019, 0020, 0021.
#[derive(Debug, Clone, PartialEq)]
pub struct HirFunction {
    /// Source-level Lua name.
    pub name: String,
    /// MLIR symbol name — `user_<name>_<idx>`.
    pub mangled_name: String,
    /// Declared parameters in source order. Each is also the prefix of
    /// `locals` so that `LocalId(i)` for `i < params.len()` refers to
    /// the i-th parameter slot.
    pub params: Vec<LocalInfo>,
    /// All locals (params first, then body-introduced locals + the
    /// synthetic `_returned` / `_ret_value_*` slots).
    pub locals: Vec<LocalInfo>,
    pub body: Vec<HirStmt>,
    /// Empty ⇒ void; length N ⇒ N return values, in source order.
    pub ret_kinds: Vec<ValueKind>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirStmt {
    pub kind: HirStmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmtKind {
    LocalInit {
        id: LocalId,
        value: HirExpr,
    },
    Assign {
        id: LocalId,
        value: HirExpr,
    },
    Block {
        stmts: Vec<HirStmt>,
    },
    /// `if cond then ... [elseif cond then ...]* [else ...]? end`.
    /// Body, elif arms, and else body are independent lexical scopes.
    If {
        cond: HirExpr,
        then_body: Vec<HirStmt>,
        elifs: Vec<(HirExpr, Vec<HirStmt>)>,
        else_body: Option<Vec<HirStmt>>,
    },
    /// `while cond do body end`. `break_id` is `Some` when the body
    /// contains a reachable `break`; codegen AND-extends `cond` with
    /// `not load(break_slot)` in that case (ADR 0015).
    While {
        cond: HirExpr,
        body: Vec<HirStmt>,
        break_id: Option<LocalId>,
    },
    /// `for var = start, stop[, step] do body end` (Lua 5.4 §3.3.5).
    /// `step` is always present in the HIR — the parser's `Option`
    /// is materialised into a `HirExpr::Number(1.0)` at lowering time.
    /// `var_id` is the loop variable's slot, scoped to `body` only and
    /// recorded in `LowerCtx::readonly_locals` while the body lowers.
    /// `break_id` is `Some` when the body contains a reachable `break`;
    /// codegen AND-extends the natural cond with `not load(break_slot)`
    /// in that case. See ADR 0015.
    ForNumeric {
        var_id: LocalId,
        start: HirExpr,
        stop: HirExpr,
        step: HirExpr,
        body: Vec<HirStmt>,
        break_id: Option<LocalId>,
    },
    /// `local a, b, ... = f(args)` (Phase 2.5d, ADR 0021): a single
    /// multi-result call whose results are bound 1-1 to the listed
    /// destination locals. Equivalent in observable behaviour to
    /// "evaluate the call, then store each result into the matching
    /// `dst_ids[i]` slot", but represented atomically because codegen
    /// must emit the call once and read multiple `result(i)` values.
    MultiAssignFromCall {
        dst_ids: Vec<LocalId>,
        callee: Callee,
        args: Vec<HirExpr>,
    },
    ExprStmt(HirExpr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExprKind {
    Number(f64),
    /// String literal. Codegen materialises each unique payload as
    /// an `llvm.mlir.global` and emits an `addressof` at every use
    /// site. Phase 2.7a (ADR 0024).
    Str(String),
    Bool(bool),
    Nil,
    Local(LocalId),
    BinOp {
        op: BinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<HirExpr>,
    },
    Call {
        callee: Callee,
        args: Vec<HirExpr>,
    },
    /// Reference to a user function by id (Phase 2.5b, ADR 0017).
    /// Produced by lowering an anonymous function expression
    /// `function() ... end` and stored into the matching Function-kind
    /// local; codegen treats it as an `i1 0` placeholder because the
    /// actual function is resolved by name at every call site.
    FunctionRef(FuncId),
}

/// Discriminates whether a [`HirExprKind::Call`] hits a built-in
/// function (Phase 2.0 baseline), a statically-known user-defined
/// function (Phase 2.5a; ADR 0016), or a runtime function value
/// reached through a Function-kind local — typically a parameter
/// (Phase 2.5b.2; ADR 0018).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Callee {
    Builtin(Builtin),
    User(FuncId),
    Indirect(LocalId),
}

/// Recognised builtin functions. Phase 2.0 had only `print`; Phase
/// 2.7c (ADR 0026) added `tostring`; Phase 2.7e (ADR 0028) adds
/// `tonumber`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Print,
    /// `tostring(x)` — converts Number/Bool/Nil/String to String.
    /// Function values are rejected (Phase 2.7c, ADR 0026).
    ToString,
    /// `tonumber(x)` — Number→identity, String→`sscanf("%lf")`
    /// with NaN sentinel on failure (Phase 2.7e, ADR 0028).
    /// Other kinds rejected.
    ToNumber,
}

impl Builtin {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "print" => Some(Builtin::Print),
            "tostring" => Some(Builtin::ToString),
            "tonumber" => Some(Builtin::ToNumber),
            _ => None,
        }
    }

    pub fn arity(self) -> usize {
        match self {
            Builtin::Print => 1,
            Builtin::ToString => 1,
            Builtin::ToNumber => 1,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Builtin::Print => "print",
            Builtin::ToString => "tostring",
            Builtin::ToNumber => "tonumber",
        }
    }
}
