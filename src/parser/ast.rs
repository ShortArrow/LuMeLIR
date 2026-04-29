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
}

/// Binary operators. Phase 2.2a covers all arithmetic operators
/// except `//` (floor div, deferred). Phase 2.2b adds the six relational
/// operators. See ADRs 0009 and 0010.
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
}

/// Unary prefix operators. Phase 2.2a introduces arithmetic negation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
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
    ExprStmt(Expr),
}

/// A Lua chunk — the top-level unit produced by [`super::parse`].
pub type Chunk = Vec<Stmt>;
