//! HIR — High-level Intermediate Representation.
//!
//! Lowers a [`crate::parser::Chunk`] (syntactic AST) into a representation
//! where names are resolved to [`LocalId`] indices and the only call form
//! is a known [`Builtin`]. Codegen consumes the HIR; the AST stays pure
//! syntax. See ADR 0007.

mod error;
mod ir;

pub use error::HirError;
pub use ir::{Builtin, HirChunk, HirExpr, HirExprKind, HirStmt, HirStmtKind, LocalId, LocalInfo};

use std::collections::HashMap;

use crate::parser::{Chunk, Expr, ExprKind, Stmt, StmtKind};

/// Lower a parsed [`Chunk`] into a [`HirChunk`] with resolved names.
pub fn lower(chunk: &Chunk) -> Result<HirChunk, HirError> {
    let mut ctx = LowerCtx::default();
    let mut stmts = Vec::with_capacity(chunk.len());
    for stmt in chunk {
        stmts.push(ctx.lower_stmt(stmt)?);
    }
    Ok(HirChunk {
        locals: ctx.locals,
        stmts,
    })
}

#[derive(Default)]
struct LowerCtx {
    locals: Vec<LocalInfo>,
    scope: HashMap<String, LocalId>,
}

impl LowerCtx {
    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<HirStmt, HirError> {
        match &stmt.kind {
            StmtKind::Local { name, value } => {
                if self.scope.contains_key(name) {
                    return Err(HirError::RedefinedLocal {
                        name: name.clone(),
                        offset: stmt.span.start,
                    });
                }
                let value = self.lower_expr(value)?;
                let id = LocalId(self.locals.len());
                self.locals.push(LocalInfo { name: name.clone() });
                self.scope.insert(name.clone(), id);
                Ok(HirStmt {
                    kind: HirStmtKind::LocalInit { id, value },
                    span: stmt.span,
                })
            }
            StmtKind::ExprStmt(expr) => {
                let hir_expr = self.lower_expr(expr)?;
                Ok(HirStmt {
                    kind: HirStmtKind::ExprStmt(hir_expr),
                    span: stmt.span,
                })
            }
        }
    }

    fn lower_expr(&self, expr: &Expr) -> Result<HirExpr, HirError> {
        let kind = match &expr.kind {
            ExprKind::Number(n) => HirExprKind::Number(*n),
            ExprKind::Ident(name) => match self.scope.get(name) {
                Some(&id) => HirExprKind::Local(id),
                None => {
                    return Err(HirError::UndefinedName {
                        name: name.clone(),
                        offset: expr.span.start,
                    });
                }
            },
            ExprKind::BinOp { op, lhs, rhs } => HirExprKind::BinOp {
                op: *op,
                lhs: Box::new(self.lower_expr(lhs)?),
                rhs: Box::new(self.lower_expr(rhs)?),
            },
            ExprKind::Call { callee, args } => self.lower_call(callee, args, expr)?,
        };
        Ok(HirExpr {
            kind,
            span: expr.span,
        })
    }

    fn lower_call(
        &self,
        callee: &Expr,
        args: &[Expr],
        whole: &Expr,
    ) -> Result<HirExprKind, HirError> {
        let name = match &callee.kind {
            ExprKind::Ident(n) => n,
            _ => {
                return Err(HirError::UnsupportedCall {
                    offset: whole.span.start,
                });
            }
        };
        let builtin = match Builtin::from_name(name) {
            Some(b) => b,
            None => {
                return Err(HirError::UnknownBuiltin {
                    name: name.clone(),
                    offset: callee.span.start,
                });
            }
        };
        let arity = builtin.arity();
        if args.len() != arity {
            return Err(HirError::ArityMismatch {
                builtin: name.clone(),
                expected: arity,
                actual: args.len(),
                offset: whole.span.start,
            });
        }
        let lowered_args = args
            .iter()
            .map(|a| self.lower_expr(a))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(HirExprKind::Call {
            builtin,
            args: lowered_args,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    fn lower_src(src: &str) -> Result<HirChunk, HirError> {
        let chunk = parser::parse(src).expect("parse");
        lower(&chunk)
    }

    #[test]
    fn lower_print_constant_produces_print_call() {
        let hir = lower_src("print(42)").expect("lower");
        assert_eq!(hir.locals.len(), 0);
        assert_eq!(hir.stmts.len(), 1);
        let HirStmtKind::ExprStmt(e) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        match &e.kind {
            HirExprKind::Call { builtin, args } => {
                assert_eq!(*builtin, Builtin::Print);
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0].kind, HirExprKind::Number(42.0)));
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn lower_local_then_use_resolves_to_local_id() {
        let hir = lower_src("local x = 1\nprint(x)").expect("lower");
        assert_eq!(hir.locals.len(), 1);
        assert_eq!(hir.locals[0].name, "x");
        let HirStmtKind::ExprStmt(call) = &hir.stmts[1].kind else {
            panic!("expected ExprStmt for print(x)");
        };
        let HirExprKind::Call { builtin, args } = &call.kind else {
            panic!("expected Call for print(x)");
        };
        assert_eq!(*builtin, Builtin::Print);
        assert!(matches!(args[0].kind, HirExprKind::Local(LocalId(0))));
    }

    #[test]
    fn lower_local_value_can_reference_no_locals_yet() {
        // The initializer of `local x = ...` must not see `x` itself.
        let err = lower_src("local x = x").expect_err("self-reference must fail");
        assert!(matches!(err, HirError::UndefinedName { .. }));
    }

    #[test]
    fn lower_undefined_name_in_print_errors() {
        let err = lower_src("print(y)").expect_err("undefined name must fail");
        match err {
            HirError::UndefinedName { name, .. } => assert_eq!(name, "y"),
            other => panic!("expected UndefinedName, got {other:?}"),
        }
    }

    #[test]
    fn lower_redefining_local_errors() {
        let err = lower_src("local x = 1\nlocal x = 2").expect_err("redef must fail");
        assert!(matches!(err, HirError::RedefinedLocal { .. }));
    }

    #[test]
    fn lower_unknown_builtin_errors() {
        let err = lower_src("foo(1)").expect_err("unknown builtin must fail");
        assert!(matches!(err, HirError::UnknownBuiltin { .. }));
    }

    #[test]
    fn lower_print_arity_mismatch_errors() {
        let err = lower_src("print()").expect_err("print arity must match");
        assert!(matches!(err, HirError::ArityMismatch { .. }));
    }

    #[test]
    fn lower_phase2_0_target_succeeds() {
        let hir = lower_src("local x = 1\nprint(x + 2)").expect("Phase 2.0 target lowers");
        assert_eq!(hir.locals.len(), 1);
        assert_eq!(hir.stmts.len(), 2);
    }
}
