use crate::lexer::Span;

/// A parsed Lua expression with its byte span into the original source.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// The discriminated shape of an [`Expr`].
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Number(f64),
    /// String literal, escapes already processed by the lexer
    /// (Phase 2.7a, ADR 0024).
    Str(String),
    Bool(bool),
    Nil,
    Ident(String),
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    BinOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    /// `function(params) body end` — anonymous function expression
    /// (Phase 2.5b, ADR 0017). Lowered to a `HirFunction` registered
    /// with the mangled name `user_anon_<idx>`.
    FunctionExpr {
        params: Vec<String>,
        body: Chunk,
    },
    /// `{ e1, e2, … }` table constructor (Phase 2.6a-min, ADR 0053
    /// for the empty form; ADR 0054 for the populated array form).
    /// Trailing comma allowed.
    Table(Vec<Expr>),
    /// `target[key]` array indexing (Phase 2.6a-arr, ADR 0054).
    /// `target` must be Table-kind, `key` Number-kind. Out-of-
    /// bounds reads trap at runtime.
    Index {
        target: Box<Expr>,
        key: Box<Expr>,
    },
}

/// Binary operators. Phase 2.2a covers all arithmetic operators
/// except `//` (floor div, deferred). Phase 2.2b adds the six relational
/// operators. Phase 2.3c adds the short-circuit logical operators.
/// Phase 2.2c (ADR 0022) adds floor div and the five bitwise operators.
/// See ADRs 0009, 0010, 0013, 0022.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    /// `//` floor division (Phase 2.2c, ADR 0022).
    FloorDiv,
    /// `&` bitwise AND (Phase 2.2c, ADR 0022).
    BitAnd,
    /// `|` bitwise OR (Phase 2.2c, ADR 0022).
    BitOr,
    /// `~` bitwise XOR (Phase 2.2c, ADR 0022). Distinguished from
    /// the unary `~` (`UnaryOp::BitNot`) by parser context.
    BitXor,
    /// `<<` left shift (Phase 2.2c, ADR 0022).
    Shl,
    /// `>>` arithmetic right shift (Phase 2.2c, ADR 0022).
    Shr,
    /// `..` string concatenation (Phase 2.7b, ADR 0025).
    /// Right-associative, between shift and additive in the
    /// precedence ladder.
    Concat,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

/// Unary prefix operators. Phase 2.2a introduces arithmetic negation.
/// Phase 2.3c adds logical `not`. Phase 2.2c (ADR 0022) adds bitwise
/// NOT (the unary `~`). Phase 2.7a (ADR 0024) adds the length
/// operator (the unary `#`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    /// `~x` bitwise complement (Phase 2.2c, ADR 0022).
    BitNot,
    /// `#s` string length in bytes (Phase 2.7a, ADR 0024).
    Len,
}

/// A statement. Phase 2.0 introduced `local` declarations and bare
/// expression statements; Phase 2.1 adds `Assign` and `Block` (do/end).
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Local {
        name: String,
        value: Expr,
    },
    /// `local NAME (, NAME)+ = EXPR (, EXPR)*` (Phase 2.5d, ADR 0021).
    /// `values.len() == 1` means a single Call expanded across all
    /// names; `values.len() == names.len()` means parallel binding;
    /// any other shape is a parse-time error.
    LocalMulti {
        names: Vec<String>,
        values: Vec<Expr>,
    },
    Assign {
        name: String,
        value: Expr,
    },
    /// `NAME (, NAME)+ = EXPR (, EXPR)*` (Phase 2.1a, ADR 0049).
    /// Per Lua semantics every RHS is evaluated before any LHS
    /// is written, so `a, b = b, a` is a real swap. Lower layers
    /// require `values.len() == names.len()` (parallel binding) —
    /// multi-result Call expansion is the LocalMulti / 2.5d
    /// territory and is out of scope here.
    AssignMulti {
        names: Vec<String>,
        values: Vec<Expr>,
    },
    Block(Chunk),
    /// `if cond then ... [elseif cond then ...]* [else ...]? end`.
    /// `elifs` keeps the chain explicit (one entry per `elseif` arm).
    If {
        cond: Expr,
        then_body: Chunk,
        elifs: Vec<(Expr, Chunk)>,
        else_body: Option<Chunk>,
    },
    /// `while cond do ... end`.
    While {
        cond: Expr,
        body: Chunk,
    },
    /// `repeat ... until cond` (Phase 2.4b, ADR 0035). The body
    /// runs at least once; the cond is evaluated at the bottom and
    /// — per Lua 5.4 §3.3.4 — sees locals declared in the body.
    Repeat {
        body: Chunk,
        cond: Expr,
    },
    /// `for var = start, stop[, step] do body end` (Lua 5.4 §3.3.5).
    /// `step` is `None` when the implicit `1` is used.
    ForNumeric {
        var: String,
        start: Expr,
        stop: Expr,
        step: Option<Expr>,
        body: Chunk,
    },
    /// `break` — exits the innermost enclosing loop. HIR rejects
    /// `break` outside of any loop with `BreakOutsideLoop`.
    Break,
    /// `local function NAME(PARAMS) BODY end` (Phase 2.5a, ADR 0016).
    /// First-class anonymous functions arrive in 2.5b.
    FunctionDef {
        name: String,
        params: Vec<String>,
        body: Chunk,
    },
    /// `return [expr]`. HIR rejects `return` outside any function with
    /// `ReturnOutsideFunction`.
    Return {
        value: Option<Expr>,
    },
    /// `return EXPR, EXPR (, EXPR)*` — two or more return values
    /// (Phase 2.5d, ADR 0021).
    ReturnMulti {
        values: Vec<Expr>,
    },
    ExprStmt(Expr),
}

/// A Lua chunk — the top-level unit produced by [`super::parse`].
pub type Chunk = Vec<Stmt>;
