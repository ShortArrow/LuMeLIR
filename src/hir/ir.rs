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
#[derive(Debug, Clone, PartialEq)]
pub struct HirChunk {
    pub locals: Vec<LocalInfo>,
    pub stmts: Vec<HirStmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirStmt {
    pub kind: HirStmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmtKind {
    LocalInit { id: LocalId, value: HirExpr },
    Assign { id: LocalId, value: HirExpr },
    Block { stmts: Vec<HirStmt> },
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
        builtin: Builtin,
        args: Vec<HirExpr>,
    },
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
