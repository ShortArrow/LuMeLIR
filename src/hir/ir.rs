use crate::hir::ValueKind;
use crate::lexer::Span;
use crate::parser::{BinOp, UnaryOp};

/// Index into [`HirChunk::locals`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);

/// Per-local metadata. Phase 2.3a adds the static value kind that
/// determines the stack slot type at codegen time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInfo {
    pub name: String,
    pub kind: ValueKind,
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
/// sequence with its own private `locals` table. Phase 2.5a allows
/// only `Number` parameters and an optional `Number` return type;
/// later sub-phases (2.5b/c/d) widen this. See ADR 0016.
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
    /// synthetic `_returned` / `_ret_value` slots).
    pub locals: Vec<LocalInfo>,
    pub body: Vec<HirStmt>,
    /// `None` ⇒ `void` return; `Some(k)` ⇒ a value of kind `k`.
    pub ret_kind: Option<ValueKind>,
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
/// function (Phase 2.0 baseline) or a user-defined function (Phase
/// 2.5a; ADR 0016).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Callee {
    Builtin(Builtin),
    User(FuncId),
}

/// Recognised builtin functions. Phase 2.0 has only `print`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Print,
}

impl Builtin {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "print" => Some(Builtin::Print),
            _ => None,
        }
    }

    pub fn arity(self) -> usize {
        match self {
            Builtin::Print => 1,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Builtin::Print => "print",
        }
    }
}
