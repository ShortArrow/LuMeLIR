//! HIR — High-level Intermediate Representation.
//!
//! Lowers a [`crate::parser::Chunk`] (syntactic AST) into a representation
//! where names are resolved to [`LocalId`] indices and the only call form
//! is a known [`Builtin`]. Codegen consumes the HIR; the AST stays pure
//! syntax. See ADRs 0007 (Phase 2.0) and 0008 (Phase 2.1 scope stack).

mod error;
mod ir;

pub use error::HirError;
pub use ir::{Builtin, HirChunk, HirExpr, HirExprKind, HirStmt, HirStmtKind, LocalId, LocalInfo};

use std::collections::HashMap;

use crate::parser::{BinOp, Chunk, Expr, ExprKind, Stmt, StmtKind};

/// Static value-kind for a fully lowered HIR expression. Used by the
/// in-HIR type guard, the heterogeneous-`==` fold, and by codegen to
/// dispatch the `print` path and the per-slot alloca type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Number,
    Bool,
    Nil,
}

impl ValueKind {
    fn name(self) -> &'static str {
        match self {
            ValueKind::Number => "number",
            ValueKind::Bool => "bool",
            ValueKind::Nil => "nil",
        }
    }
}

pub fn infer_kind(expr: &HirExpr, locals: &[LocalInfo]) -> ValueKind {
    match &expr.kind {
        HirExprKind::Number(_) => ValueKind::Number,
        HirExprKind::Bool(_) => ValueKind::Bool,
        HirExprKind::Nil => ValueKind::Nil,
        HirExprKind::Local(LocalId(idx)) => locals[*idx].kind,
        HirExprKind::UnaryOp { .. } => ValueKind::Number,
        HirExprKind::BinOp { op, .. } => match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                ValueKind::Number
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => {
                ValueKind::Bool
            }
        },
        // print() returns no value in Lua, but our HIR treats it as
        // an expression. It can never appear as an operand to a
        // comparison anyway (we'd have already errored on parse).
        HirExprKind::Call { .. } => ValueKind::Number,
    }
}

fn binop_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Pow => "^",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Eq => "==",
        BinOp::Ne => "~=",
    }
}

/// Lower a parsed [`Chunk`] into a [`HirChunk`] with resolved names.
pub fn lower(chunk: &Chunk) -> Result<HirChunk, HirError> {
    let mut ctx = LowerCtx::new();
    let stmts = ctx.lower_stmts(chunk)?;
    Ok(HirChunk {
        locals: ctx.locals,
        stmts,
    })
}

struct LowerCtx {
    locals: Vec<LocalInfo>,
    /// Scope stack: innermost scope is the last element. `local` always
    /// pushes into the top frame, allowing same-scope shadowing per Lua
    /// 5.4 semantics. See ADR 0008.
    scopes: Vec<HashMap<String, LocalId>>,
}

impl LowerCtx {
    fn new() -> Self {
        Self {
            locals: Vec::new(),
            scopes: vec![HashMap::new()],
        }
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, HirError> {
        let mut out = Vec::with_capacity(stmts.len());
        for s in stmts {
            out.push(self.lower_stmt(s)?);
        }
        Ok(out)
    }

    /// Lower a body with a fresh lexical scope pushed/popped around it.
    /// Used for `do/end`, `if`/`elseif`/`else` bodies, and `while` bodies.
    fn lower_scoped_body(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, HirError> {
        self.scopes.push(HashMap::new());
        let result = self.lower_stmts(stmts);
        self.scopes.pop();
        result
    }

    fn resolve(&self, name: &str) -> Option<LocalId> {
        for frame in self.scopes.iter().rev() {
            if let Some(id) = frame.get(name) {
                return Some(*id);
            }
        }
        None
    }

    fn declare_local(&mut self, name: String, kind: ValueKind) -> LocalId {
        let id = LocalId(self.locals.len());
        self.locals.push(LocalInfo {
            name: name.clone(),
            kind,
        });
        self.scopes
            .last_mut()
            .expect("scope stack is never empty")
            .insert(name, id);
        id
    }

    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<HirStmt, HirError> {
        match &stmt.kind {
            StmtKind::Local { name, value } => {
                let value = self.lower_expr(value)?;
                let kind = infer_kind(&value, &self.locals);
                let id = self.declare_local(name.clone(), kind);
                Ok(HirStmt {
                    kind: HirStmtKind::LocalInit { id, value },
                    span: stmt.span,
                })
            }
            StmtKind::Assign { name, value } => {
                let id = self.resolve(name).ok_or_else(|| HirError::UndefinedName {
                    name: name.clone(),
                    offset: stmt.span.start,
                })?;
                let value = self.lower_expr(value)?;
                let slot_kind = self.locals[id.0].kind;
                let value_kind = infer_kind(&value, &self.locals);
                if slot_kind != value_kind {
                    return Err(HirError::TypeMismatch {
                        op: "=".to_owned(),
                        lhs_kind: slot_kind.name().to_owned(),
                        rhs_kind: value_kind.name().to_owned(),
                        offset: stmt.span.start,
                    });
                }
                Ok(HirStmt {
                    kind: HirStmtKind::Assign { id, value },
                    span: stmt.span,
                })
            }
            StmtKind::Block(body) => {
                self.scopes.push(HashMap::new());
                let result = self.lower_stmts(body);
                self.scopes.pop();
                let stmts = result?;
                Ok(HirStmt {
                    kind: HirStmtKind::Block { stmts },
                    span: stmt.span,
                })
            }
            StmtKind::If {
                cond,
                then_body,
                elifs,
                else_body,
            } => {
                let cond_hir = self.lower_expr(cond)?;
                let then_hir = self.lower_scoped_body(then_body)?;
                let elifs_hir = elifs
                    .iter()
                    .map(|(c, b)| {
                        let c_hir = self.lower_expr(c)?;
                        let b_hir = self.lower_scoped_body(b)?;
                        Ok((c_hir, b_hir))
                    })
                    .collect::<Result<Vec<_>, HirError>>()?;
                let else_hir = match else_body {
                    Some(body) => Some(self.lower_scoped_body(body)?),
                    None => None,
                };
                Ok(HirStmt {
                    kind: HirStmtKind::If {
                        cond: cond_hir,
                        then_body: then_hir,
                        elifs: elifs_hir,
                        else_body: else_hir,
                    },
                    span: stmt.span,
                })
            }
            StmtKind::While { cond, body } => {
                let cond_hir = self.lower_expr(cond)?;
                let body_hir = self.lower_scoped_body(body)?;
                Ok(HirStmt {
                    kind: HirStmtKind::While {
                        cond: cond_hir,
                        body: body_hir,
                    },
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
            ExprKind::Ident(name) => match self.resolve(name) {
                Some(id) => HirExprKind::Local(id),
                None => {
                    return Err(HirError::UndefinedName {
                        name: name.clone(),
                        offset: expr.span.start,
                    });
                }
            },
            ExprKind::Bool(b) => HirExprKind::Bool(*b),
            ExprKind::Nil => HirExprKind::Nil,
            ExprKind::BinOp { op, lhs, rhs } => {
                let lhs_hir = self.lower_expr(lhs)?;
                let rhs_hir = self.lower_expr(rhs)?;
                let lk = infer_kind(&lhs_hir, &self.locals);
                let rk = infer_kind(&rhs_hir, &self.locals);
                match op {
                    // Arithmetic: both sides must be Number.
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                        if !(lk == ValueKind::Number && rk == ValueKind::Number) {
                            return Err(HirError::TypeMismatch {
                                op: binop_symbol(*op).to_owned(),
                                lhs_kind: lk.name().to_owned(),
                                rhs_kind: rk.name().to_owned(),
                                offset: expr.span.start,
                            });
                        }
                        HirExprKind::BinOp {
                            op: *op,
                            lhs: Box::new(lhs_hir),
                            rhs: Box::new(rhs_hir),
                        }
                    }
                    // Ordering: both sides must be Number (nil/bool reject).
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        if !(lk == ValueKind::Number && rk == ValueKind::Number) {
                            return Err(HirError::TypeMismatch {
                                op: binop_symbol(*op).to_owned(),
                                lhs_kind: lk.name().to_owned(),
                                rhs_kind: rk.name().to_owned(),
                                offset: expr.span.start,
                            });
                        }
                        HirExprKind::BinOp {
                            op: *op,
                            lhs: Box::new(lhs_hir),
                            rhs: Box::new(rhs_hir),
                        }
                    }
                    // Equality: heterogeneous kinds and both-Nil fold to a
                    // constant (Lua semantics: `1 == nil` ≡ false). Same-
                    // kind non-Nil drops through to runtime cmpf in codegen.
                    BinOp::Eq | BinOp::Ne => {
                        let fold = lk != rk || (lk == ValueKind::Nil && rk == ValueKind::Nil);
                        if fold {
                            let equal = lk == rk; // both-Nil → true; heterogeneous → false
                            let folded = match op {
                                BinOp::Eq => equal,
                                BinOp::Ne => !equal,
                                _ => unreachable!(),
                            };
                            HirExprKind::Bool(folded)
                        } else {
                            HirExprKind::BinOp {
                                op: *op,
                                lhs: Box::new(lhs_hir),
                                rhs: Box::new(rhs_hir),
                            }
                        }
                    }
                }
            }
            ExprKind::UnaryOp { op, operand } => HirExprKind::UnaryOp {
                op: *op,
                operand: Box::new(self.lower_expr(operand)?),
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
    fn lower_same_scope_shadowing_is_allowed() {
        // Lua 5.4 allows re-declaring a local in the same scope; the
        // newer binding shadows the old one.
        let hir = lower_src("local x = 1\nlocal x = 2\nprint(x)")
            .expect("same-scope shadowing must lower");
        assert_eq!(hir.locals.len(), 2, "two distinct slots");
        let HirStmtKind::ExprStmt(call) = &hir.stmts[2].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected Call");
        };
        // The print(x) reference resolves to the *second* x (LocalId(1)).
        assert!(matches!(args[0].kind, HirExprKind::Local(LocalId(1))));
    }

    #[test]
    fn lower_assign_to_existing_local_resolves() {
        let hir = lower_src("local x = 1\nx = 2").expect("assign must lower");
        assert_eq!(hir.locals.len(), 1);
        let HirStmtKind::Assign { id, value } = &hir.stmts[1].kind else {
            panic!("expected Assign, got {:?}", hir.stmts[1].kind);
        };
        assert_eq!(*id, LocalId(0));
        assert!(matches!(value.kind, HirExprKind::Number(2.0)));
    }

    #[test]
    fn lower_assign_to_undefined_name_errors() {
        let err = lower_src("y = 1").expect_err("assign-to-undef must fail");
        match err {
            HirError::UndefinedName { name, .. } => assert_eq!(name, "y"),
            other => panic!("expected UndefinedName, got {other:?}"),
        }
    }

    #[test]
    fn lower_block_creates_inner_scope_and_pops_on_exit() {
        // Inner `local x` shadows outer `x` only within the block.
        let hir = lower_src("local x = 1\ndo local x = 99\nprint(x) end\nprint(x)")
            .expect("nested scope must lower");
        assert_eq!(hir.locals.len(), 2);
        let HirStmtKind::Block { stmts } = &hir.stmts[1].kind else {
            panic!("expected Block at stmts[1]");
        };
        let HirStmtKind::ExprStmt(inner_call) = &stmts[1].kind else {
            panic!("expected inner ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &inner_call.kind else {
            panic!("expected inner Call");
        };
        // Inside the block, x → LocalId(1) (the inner shadow).
        assert!(matches!(args[0].kind, HirExprKind::Local(LocalId(1))));

        let HirStmtKind::ExprStmt(outer_call) = &hir.stmts[2].kind else {
            panic!("expected outer ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &outer_call.kind else {
            panic!("expected outer Call");
        };
        // After the block, x → LocalId(0) (the outer binding).
        assert!(matches!(args[0].kind, HirExprKind::Local(LocalId(0))));
    }

    #[test]
    fn lower_local_inside_block_is_invisible_outside() {
        let err = lower_src("do local x = 1 end\nprint(x)").expect_err("inner local must not leak");
        assert!(matches!(err, HirError::UndefinedName { .. }));
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
    fn lower_unary_minus_preserves_op() {
        let hir = lower_src("print(-1)").expect("must lower");
        let HirStmtKind::ExprStmt(call) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected Call");
        };
        assert!(matches!(
            args[0].kind,
            HirExprKind::UnaryOp {
                op: crate::parser::UnaryOp::Neg,
                ..
            }
        ));
    }

    #[test]
    fn lower_all_arith_ops_pass_through() {
        // Smoke test: each new op survives lowering with its identity.
        for src in [
            "print(1 - 2)",
            "print(1 * 2)",
            "print(1 / 2)",
            "print(1 % 2)",
            "print(1 ^ 2)",
        ] {
            assert!(lower_src(src).is_ok(), "{src} must lower");
        }
    }

    #[test]
    fn lower_true_yields_hir_bool() {
        let hir = lower_src("print(true)").expect("must lower");
        let HirStmtKind::ExprStmt(call) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected Call");
        };
        assert!(matches!(args[0].kind, HirExprKind::Bool(true)));
    }

    #[test]
    fn lower_lt_passes_through_to_hir() {
        let hir = lower_src("print(1 < 2)").expect("must lower");
        let HirStmtKind::ExprStmt(call) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected Call");
        };
        assert!(matches!(
            args[0].kind,
            HirExprKind::BinOp {
                op: crate::parser::BinOp::Lt,
                ..
            }
        ));
    }

    // (Phase 2.2b's `lower_number_eq_bool_returns_type_mismatch` was
    // removed in 2.3a — heterogeneous `==` now folds to a constant
    // per ADR 0011. The replacement assertion lives in
    // `lower_number_eq_bool_now_folds_instead_of_erroring` below.)

    #[test]
    fn lower_lt_with_bool_lhs_returns_type_mismatch() {
        let err = lower_src("print(true < 1)").expect_err("bool < number must error");
        assert!(matches!(err, HirError::TypeMismatch { .. }));
    }

    #[test]
    fn lower_nil_yields_hir_nil() {
        let hir = lower_src("print(nil == nil)").expect("must lower");
        // The (nil == nil) is statically folded to Bool(true); we
        // observe it via the print arg shape.
        let HirStmtKind::ExprStmt(call) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected Call");
        };
        assert!(matches!(args[0].kind, HirExprKind::Bool(true)));
    }

    #[test]
    fn lower_local_with_bool_value_records_bool_kind() {
        let hir = lower_src("local b = true").expect("must lower");
        assert_eq!(hir.locals.len(), 1);
        assert_eq!(hir.locals[0].name, "b");
        assert_eq!(hir.locals[0].kind, ValueKind::Bool);
    }

    #[test]
    fn lower_local_with_nil_value_records_nil_kind() {
        let hir = lower_src("local n = nil").expect("must lower");
        assert_eq!(hir.locals.len(), 1);
        assert_eq!(hir.locals[0].kind, ValueKind::Nil);
    }

    #[test]
    fn lower_assign_changing_kind_returns_type_mismatch() {
        let err = lower_src("local x = 1\nx = nil").expect_err("kind change must reject");
        match err {
            HirError::TypeMismatch {
                lhs_kind, rhs_kind, ..
            } => {
                assert_eq!(lhs_kind, "number");
                assert_eq!(rhs_kind, "nil");
            }
            other => panic!("expected TypeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn lower_nil_eq_nil_folds_to_bool_true() {
        let hir = lower_src("print(nil == nil)").expect("must lower");
        let HirStmtKind::ExprStmt(call) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected Call");
        };
        assert!(matches!(args[0].kind, HirExprKind::Bool(true)));
    }

    #[test]
    fn lower_number_eq_nil_folds_to_bool_false() {
        let hir = lower_src("print(1 == nil)").expect("must lower");
        let HirStmtKind::ExprStmt(call) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected Call");
        };
        assert!(matches!(args[0].kind, HirExprKind::Bool(false)));
    }

    #[test]
    fn lower_number_eq_bool_now_folds_instead_of_erroring() {
        // ADR 0010 rejected this with TypeMismatch; ADR 0011 folds it.
        let hir = lower_src("print(1 == true)").expect("must lower (heterogeneous fold)");
        let HirStmtKind::ExprStmt(call) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected Call");
        };
        assert!(matches!(args[0].kind, HirExprKind::Bool(false)));
    }

    #[test]
    fn lower_nil_lt_number_returns_type_mismatch() {
        let err = lower_src("print(nil < 1)").expect_err("nil < number must reject");
        assert!(matches!(err, HirError::TypeMismatch { .. }));
    }

    #[test]
    fn lower_simple_if_resolves_body() {
        let hir = lower_src("if 1 then print(1) end").expect("must lower");
        assert_eq!(hir.stmts.len(), 1);
        let HirStmtKind::If {
            then_body,
            elifs,
            else_body,
            ..
        } = &hir.stmts[0].kind
        else {
            panic!("expected If, got {:?}", hir.stmts[0].kind);
        };
        assert_eq!(then_body.len(), 1);
        assert!(elifs.is_empty());
        assert!(else_body.is_none());
    }

    #[test]
    fn lower_if_elseif_else_chain() {
        let src = "if 1 then print(1) elseif 2 then print(2) else print(3) end";
        let hir = lower_src(src).expect("must lower");
        let HirStmtKind::If {
            then_body,
            elifs,
            else_body,
            ..
        } = &hir.stmts[0].kind
        else {
            panic!("expected If");
        };
        assert_eq!(then_body.len(), 1);
        assert_eq!(elifs.len(), 1);
        assert!(else_body.as_ref().unwrap().len() == 1);
    }

    #[test]
    fn lower_while_resolves_body() {
        let hir = lower_src("local i = 0\nwhile i < 3 do i = i + 1 end").expect("must lower");
        assert_eq!(hir.stmts.len(), 2);
        let HirStmtKind::While { body, .. } = &hir.stmts[1].kind else {
            panic!("expected While");
        };
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn lower_local_inside_if_uses_inner_scope() {
        // The inner `x` does not leak.
        let err = lower_src("if 1 then local x = 1 end\nprint(x)")
            .expect_err("inner local must not leak past `end`");
        assert!(matches!(err, HirError::UndefinedName { .. }));
    }

    #[test]
    fn lower_phase2_0_target_succeeds() {
        let hir = lower_src("local x = 1\nprint(x + 2)").expect("Phase 2.0 target lowers");
        assert_eq!(hir.locals.len(), 1);
        assert_eq!(hir.stmts.len(), 2);
    }
}
