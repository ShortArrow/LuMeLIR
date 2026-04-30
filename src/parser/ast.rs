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
}

/// Binary operators. Phase 2.2a covers all arithmetic operators
/// except `//` (floor div, deferred). Phase 2.2b adds the six relational
/// operators. Phase 2.3c adds the short-circuit logical operators.
/// See ADRs 0009, 0010, 0013.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
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
/// Phase 2.3c adds logical `not`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
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
