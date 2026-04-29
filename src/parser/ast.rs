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
}

/// Binary operators. Phase 1 has addition only; more will join it as the
/// grammar grows (see ADR 0004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
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
    Local { name: String, value: Expr },
    Assign { name: String, value: Expr },
    Block(Chunk),
    ExprStmt(Expr),
}

/// A Lua chunk — the top-level unit produced by [`super::parse`].
pub type Chunk = Vec<Stmt>;
