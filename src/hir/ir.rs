use crate::lexer::Span;
use crate::parser::BinOp;

/// Index into [`HirChunk::locals`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);

/// Per-local metadata. Phase 2.0 carries only the source name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInfo {
    pub name: String,
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
    Local(LocalId),
    BinOp {
        op: BinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
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
