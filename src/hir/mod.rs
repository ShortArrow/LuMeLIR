//! HIR — High-level Intermediate Representation.
//!
//! Lowers a [`crate::parser::Chunk`] (syntactic AST) into a representation
//! where names are resolved to [`LocalId`] indices and the only call form
//! is a known [`Builtin`]. Codegen consumes the HIR; the AST stays pure
//! syntax. See ADRs 0007 (Phase 2.0) and 0008 (Phase 2.1 scope stack).

mod error;
mod ir;

pub use error::HirError;
pub use ir::{
    Builtin, Callee, FuncId, HirChunk, HirExpr, HirExprKind, HirFunction, HirStmt, HirStmtKind,
    LocalId, LocalInfo,
};

use std::collections::{HashMap, HashSet};

use crate::lexer::Span;
use crate::parser::{BinOp, Chunk, Expr, ExprKind, Stmt, StmtKind, UnaryOp};

/// Static value-kind for a fully lowered HIR expression. Used by the
/// in-HIR type guard, the heterogeneous-`==` fold, and by codegen to
/// dispatch the `print` path and the per-slot alloca type.
///
/// Phase 2.5b.2 (ADR 0018) changes `Function`'s payload from `FuncId`
/// to `arity: usize`, since function values passed as parameters do
/// not have a statically-known FuncId. The actual `FuncId` (when
/// known) lives in [`LocalInfo::func_id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Number,
    Bool,
    Nil,
    Function(usize),
}

impl ValueKind {
    fn name(self) -> &'static str {
        match self {
            ValueKind::Number => "number",
            ValueKind::Bool => "bool",
            ValueKind::Nil => "nil",
            ValueKind::Function(_) => "function",
        }
    }
}

pub fn infer_kind(expr: &HirExpr, locals: &[LocalInfo], functions: &[HirFunction]) -> ValueKind {
    match &expr.kind {
        HirExprKind::Number(_) => ValueKind::Number,
        HirExprKind::Bool(_) => ValueKind::Bool,
        HirExprKind::Nil => ValueKind::Nil,
        HirExprKind::Local(LocalId(idx)) => locals[*idx].kind,
        HirExprKind::UnaryOp { op, .. } => match op {
            crate::parser::UnaryOp::Neg => ValueKind::Number,
            crate::parser::UnaryOp::Not => ValueKind::Bool,
        },
        HirExprKind::BinOp { op, lhs, .. } => match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                ValueKind::Number
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => {
                ValueKind::Bool
            }
            // `and`/`or` preserve the operand kind (lower-time guard
            // ensures both sides share a kind).
            BinOp::And | BinOp::Or => infer_kind(lhs, locals, functions),
        },
        HirExprKind::Call { callee, .. } => match callee {
            // print() has no useful value in our subset; treat as Number
            // so existing arithmetic guards remain consistent (it never
            // actually appears as a comparison operand).
            Callee::Builtin(Builtin::Print) => ValueKind::Number,
            // User function: look up its declared return kind. Phase
            // 2.5a forces this to Number when present; void calls
            // never appear in expression position legally.
            Callee::User(FuncId(id)) => functions[*id].ret_kind.unwrap_or(ValueKind::Number),
            // Indirect call (function-kind local): Phase 2.5b.2 fixes
            // returns to Number, so that's the answer.
            Callee::Indirect(_) => ValueKind::Number,
        },
        HirExprKind::FunctionRef(FuncId(id)) => {
            // Phase 2.5b.2: kind tracks arity, FuncId lives in LocalInfo.
            ValueKind::Function(functions[*id].params.len())
        }
    }
}

/// True iff `stmts` contains a `break` reachable from this loop's body —
/// i.e. without crossing a nested loop boundary. Used by
/// `lower_stmt(While|ForNumeric)` to decide whether to allocate a
/// `_broken` flag (ADR 0015).
fn body_contains_break(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_contains_break)
}

fn stmt_contains_break(s: &Stmt) -> bool {
    match &s.kind {
        StmtKind::Break => true,
        StmtKind::If {
            then_body,
            elifs,
            else_body,
            ..
        } => {
            body_contains_break(then_body)
                || elifs.iter().any(|(_, b)| body_contains_break(b))
                || else_body.as_ref().is_some_and(|b| body_contains_break(b))
        }
        StmtKind::Block(b) => body_contains_break(b),
        // Nested loops have their own break scope — their breaks do
        // not escape to this loop.
        StmtKind::While { .. } | StmtKind::ForNumeric { .. } => false,
        _ => false,
    }
}

fn wrap_with_broken_guard(stmt: HirStmt, broken_id: LocalId) -> HirStmt {
    let span = stmt.span;
    let cond = HirExpr {
        kind: HirExprKind::UnaryOp {
            op: UnaryOp::Not,
            operand: Box::new(HirExpr {
                kind: HirExprKind::Local(broken_id),
                span,
            }),
        },
        span,
    };
    HirStmt {
        kind: HirStmtKind::If {
            cond,
            then_body: vec![stmt],
            elifs: vec![],
            else_body: None,
        },
        span,
    }
}

/// Phase 2.5b.2 (ADR 0018): Pre-scan a function body's AST to discover
/// which named parameters are used as callees (`g(args)`). For each
/// such parameter, infer `ValueKind::Function(arity)` where `arity` is
/// the number of arguments at the call site. Other parameters default
/// to `ValueKind::Number`. Conflicting arities are caught later by
/// `lower_call`'s arity check.
fn infer_param_kinds(body: &[Stmt], param_names: &[String]) -> Vec<ValueKind> {
    use std::collections::HashMap as Map;
    let mut kinds: Vec<ValueKind> = param_names.iter().map(|_| ValueKind::Number).collect();
    let name_to_idx: Map<&str, usize> = param_names
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();

    fn visit_stmt(stmt: &Stmt, name_to_idx: &Map<&str, usize>, kinds: &mut Vec<ValueKind>) {
        match &stmt.kind {
            StmtKind::Local { value, .. } | StmtKind::Assign { value, .. } => {
                visit_expr(value, name_to_idx, kinds);
            }
            StmtKind::ExprStmt(e) => visit_expr(e, name_to_idx, kinds),
            StmtKind::Return { value: Some(e) } => visit_expr(e, name_to_idx, kinds),
            StmtKind::Return { value: None } => {}
            StmtKind::Block(b) => {
                for s in b {
                    visit_stmt(s, name_to_idx, kinds);
                }
            }
            StmtKind::If {
                cond,
                then_body,
                elifs,
                else_body,
            } => {
                visit_expr(cond, name_to_idx, kinds);
                for s in then_body {
                    visit_stmt(s, name_to_idx, kinds);
                }
                for (c, b) in elifs {
                    visit_expr(c, name_to_idx, kinds);
                    for s in b {
                        visit_stmt(s, name_to_idx, kinds);
                    }
                }
                if let Some(b) = else_body {
                    for s in b {
                        visit_stmt(s, name_to_idx, kinds);
                    }
                }
            }
            StmtKind::While { cond, body } => {
                visit_expr(cond, name_to_idx, kinds);
                for s in body {
                    visit_stmt(s, name_to_idx, kinds);
                }
            }
            StmtKind::ForNumeric {
                start,
                stop,
                step,
                body,
                ..
            } => {
                visit_expr(start, name_to_idx, kinds);
                visit_expr(stop, name_to_idx, kinds);
                if let Some(s) = step {
                    visit_expr(s, name_to_idx, kinds);
                }
                for s in body {
                    visit_stmt(s, name_to_idx, kinds);
                }
            }
            StmtKind::Break => {}
            StmtKind::FunctionDef { .. } => {} // nested fn defs not in 2.5b.2
        }
    }

    fn visit_expr(expr: &Expr, name_to_idx: &Map<&str, usize>, kinds: &mut Vec<ValueKind>) {
        match &expr.kind {
            ExprKind::Call { callee, args } => {
                if let ExprKind::Ident(name) = &callee.kind {
                    if let Some(&idx) = name_to_idx.get(name.as_str()) {
                        kinds[idx] = ValueKind::Function(args.len());
                    }
                }
                visit_expr(callee, name_to_idx, kinds);
                for a in args {
                    visit_expr(a, name_to_idx, kinds);
                }
            }
            ExprKind::BinOp { lhs, rhs, .. } => {
                visit_expr(lhs, name_to_idx, kinds);
                visit_expr(rhs, name_to_idx, kinds);
            }
            ExprKind::UnaryOp { operand, .. } => {
                visit_expr(operand, name_to_idx, kinds);
            }
            ExprKind::FunctionExpr { .. } => {} // not recursed (own scope)
            _ => {}
        }
    }

    for s in body {
        visit_stmt(s, &name_to_idx, &mut kinds);
    }
    kinds
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
        BinOp::And => "and",
        BinOp::Or => "or",
    }
}

/// Lower a parsed [`Chunk`] into a [`HirChunk`] with resolved names.
///
/// Phase 2.5a: top-level `local function` definitions are lifted into
/// `chunk.functions`; the rest of the top-level statements (including
/// `print(...)` calls into those functions) become the implicit `main`
/// chunk's `stmts`.
pub fn lower(chunk: &Chunk) -> Result<HirChunk, HirError> {
    // Pass 1: register every top-level `local function` in the
    // function table so recursion and forward-reference work.
    let mut functions: Vec<HirFunction> = Vec::new();
    let mut function_names: HashMap<String, FuncId> = HashMap::new();
    for stmt in chunk {
        if let StmtKind::FunctionDef { name, params, .. } = &stmt.kind {
            let id = FuncId(functions.len());
            functions.push(HirFunction {
                name: name.clone(),
                mangled_name: format!("user_{}_{}", name, id.0),
                params: params
                    .iter()
                    .map(|p| LocalInfo {
                        name: p.clone(),
                        kind: ValueKind::Number,
                        func_id: None,
                    })
                    .collect(),
                locals: Vec::new(), // filled in pass 2
                body: Vec::new(),   // filled in pass 2
                ret_kind: None,     // resolved in pass 2
            });
            function_names.insert(name.clone(), id);
        }
    }

    // Pass 2: lower each function body in its own LowerCtx; then lower
    // the `main` chunk (skipping FunctionDef stmts — they've been
    // lifted out).
    for (idx, stmt) in chunk.iter().enumerate() {
        if let StmtKind::FunctionDef { params, body, .. } = &stmt.kind {
            let id = FuncId(idx_of_funcdef(chunk, idx));
            let mut fn_ctx = LowerCtx::for_function(&function_names, &functions, params, body);
            let body_hir = fn_ctx.lower_function_body(body)?;
            let ret_kind = fn_ctx.in_function_ret_kind;
            // Phase 2.5b.2: copy the inferred param kinds (the first
            // `params.len()` locals carry them) into the public
            // `HirFunction.params` slice so callers see the right
            // arity check at indirect-call sites.
            functions[id.0].params = fn_ctx.locals[..params.len()].to_vec();
            functions[id.0].locals = fn_ctx.locals;
            functions[id.0].body = body_hir;
            functions[id.0].ret_kind = ret_kind;
        }
    }

    let mut ctx = LowerCtx::new(function_names.clone(), functions);
    // Lower top-level stmts, skipping FunctionDef (already lifted).
    let mut stmts = Vec::new();
    for s in chunk {
        if matches!(s.kind, StmtKind::FunctionDef { .. }) {
            continue;
        }
        stmts.push(ctx.lower_stmt(s)?);
    }
    // ctx.functions accumulates anonymous functions registered during
    // lowering of `local f = function() ... end` (Phase 2.5b, ADR 0017).
    Ok(HirChunk {
        locals: ctx.locals,
        stmts,
        functions: ctx.functions,
    })
}

/// Helper: count `FunctionDef`s up to (and including) position `idx` to
/// get the FuncId. The stable ordering matches the pass-1 enumeration.
fn idx_of_funcdef(chunk: &[Stmt], idx: usize) -> usize {
    chunk
        .iter()
        .take(idx + 1)
        .filter(|s| matches!(s.kind, StmtKind::FunctionDef { .. }))
        .count()
        - 1
}

struct LowerCtx {
    locals: Vec<LocalInfo>,
    /// Scope stack: innermost scope is the last element. `local` always
    /// pushes into the top frame, allowing same-scope shadowing per Lua
    /// 5.4 semantics. See ADR 0008.
    scopes: Vec<HashMap<String, LocalId>>,
    /// Locals that cannot be reassigned with `=` — currently only
    /// numeric `for` loop variables (Lua 5.4 §3.3.5). See ADR 0014.
    readonly_locals: HashSet<LocalId>,
    /// Stack of optional `_broken` flag locals, one entry per active
    /// loop. Top is the innermost loop. A loop without `break` in its
    /// body pushes `None` — body statements then lower without the
    /// guard wrap, but `break` inside such a body would error
    /// (impossible because we only push `None` when there is no
    /// `break` to target it). See ADR 0015.
    loop_break_targets: Vec<Option<LocalId>>,
    /// Function namespace inherited from the top-level pass (Phase
    /// 2.5a). Resolved at every `Call` to dispatch user vs builtin.
    function_names: HashMap<String, FuncId>,
    /// Mirror of [`HirChunk::functions`] — needed by `infer_kind` for
    /// user-call return-type lookup. Phase 2.5a clones it into each
    /// `LowerCtx`; the cost is negligible at this scale.
    functions: Vec<HirFunction>,
    /// `Some((returned_id, ret_value_id))` while lowering inside a
    /// function body; `None` at top level. `ret_value_id` is `None`
    /// for void functions (no value ever stored).
    in_function: Option<(LocalId, Option<LocalId>)>,
    /// The return kind discovered while lowering the current function
    /// body: `None` (uninitialised) → `Some(Number)` once a value-
    /// returning `return` is seen. Phase 2.5a fixes value returns to
    /// Number so this never widens.
    in_function_ret_kind: Option<ValueKind>,
}

impl LowerCtx {
    fn new(function_names: HashMap<String, FuncId>, functions: Vec<HirFunction>) -> Self {
        Self {
            locals: Vec::new(),
            scopes: vec![HashMap::new()],
            readonly_locals: HashSet::new(),
            loop_break_targets: Vec::new(),
            function_names,
            functions,
            in_function: None,
            in_function_ret_kind: None,
        }
    }

    /// Build a `LowerCtx` for lowering a function body in isolation
    /// (separate locals, scopes, loop break stack). The function's
    /// parameters are pre-declared as the first locals.
    ///
    /// Phase 2.5b.2 (ADR 0018): the body AST is pre-scanned with
    /// [`infer_param_kinds`] so any parameter used as a callee gets
    /// `ValueKind::Function(arity)` instead of the default Number.
    fn for_function(
        function_names: &HashMap<String, FuncId>,
        functions: &[HirFunction],
        params: &[String],
        body: &[Stmt],
    ) -> Self {
        let mut ctx = Self::new(function_names.clone(), functions.to_vec());
        let kinds = infer_param_kinds(body, params);
        for (p, k) in params.iter().zip(kinds.iter()) {
            ctx.declare_local(p.clone(), *k);
        }
        ctx
    }

    /// Lower a function body. Allocates the synthetic `_returned` and
    /// `_ret_value` slots, sets `in_function`, and applies the same
    /// body-guard wrap pattern used by `break` (ADR 0015) so that
    /// post-`return` statements are skipped at runtime.
    fn lower_function_body(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, HirError> {
        // Synthetic flag + value slot, declared at the top of the
        // body's outermost scope (after parameters).
        let returned_id = self.declare_local("_returned".to_owned(), ValueKind::Bool);
        let ret_value_id = self.declare_local("_ret_value".to_owned(), ValueKind::Number);
        self.in_function = Some((returned_id, Some(ret_value_id)));

        // The implicit Span for synthetic locals — pick the chunk's
        // first stmt's span if available, else a zero-width sentinel.
        let span = stmts.first().map(|s| s.span).unwrap_or(Span::new(0, 0));
        let mut out = Vec::with_capacity(stmts.len() + 2);
        out.push(HirStmt {
            kind: HirStmtKind::LocalInit {
                id: returned_id,
                value: HirExpr {
                    kind: HirExprKind::Bool(false),
                    span,
                },
            },
            span,
        });
        out.push(HirStmt {
            kind: HirStmtKind::LocalInit {
                id: ret_value_id,
                value: HirExpr {
                    kind: HirExprKind::Number(0.0),
                    span,
                },
            },
            span,
        });

        // Lower body statements with the `_returned` guard wrap (same
        // pattern as `break`). Reuse `loop_break_targets` machinery by
        // pushing the returned flag — every body stmt becomes
        // `if not _returned then stmt`.
        self.loop_break_targets.push(Some(returned_id));
        let lowered = self.lower_stmts(stmts)?;
        self.loop_break_targets.pop();

        // Apply the guard wrap manually (lower_stmts doesn't wrap on
        // its own — only `lower_stmts_maybe_guarded` does, and that
        // helper is loop-specific). Reuse `wrap_with_broken_guard`
        // since the shape is identical.
        for stmt in lowered {
            out.push(wrap_with_broken_guard(stmt, returned_id));
        }
        Ok(out)
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
    /// When the innermost active loop has a break flag, every body
    /// statement is wrapped in `if not load(_broken) then ... end` so
    /// post-`break` code is skipped at runtime (ADR 0015).
    fn lower_scoped_body(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, HirError> {
        self.scopes.push(HashMap::new());
        let result = self.lower_stmts_maybe_guarded(stmts);
        self.scopes.pop();
        result
    }

    /// Same as `lower_scoped_body` but the caller is responsible for the
    /// scope push/pop. Used by `for`-loop lowering, which needs to
    /// declare the read-only loop variable inside the body's own scope.
    fn lower_scoped_body_no_push(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, HirError> {
        self.lower_stmts_maybe_guarded(stmts)
    }

    fn lower_stmts_maybe_guarded(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, HirError> {
        // The innermost loop's break flag (if any) — `None` for either
        // top-level lowering or a loop whose body has no `break`.
        let broken_id = self.loop_break_targets.last().copied().flatten();
        let mut out = Vec::with_capacity(stmts.len());
        for stmt in stmts {
            let lowered = self.lower_stmt(stmt)?;
            let final_stmt = match broken_id {
                Some(id) => wrap_with_broken_guard(lowered, id),
                None => lowered,
            };
            out.push(final_stmt);
        }
        Ok(out)
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
        self.declare_local_with_func_id(name, kind, None)
    }

    fn declare_local_with_func_id(
        &mut self,
        name: String,
        kind: ValueKind,
        func_id: Option<FuncId>,
    ) -> LocalId {
        let id = LocalId(self.locals.len());
        self.locals.push(LocalInfo {
            name: name.clone(),
            kind,
            func_id,
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
                let kind = infer_kind(&value, &self.locals, &self.functions);
                // Phase 2.5b.2: when the rhs is a function reference
                // (literal `function() end` or alias of a Function-kind
                // local), record its FuncId so static dispatch works.
                let func_id = match &value.kind {
                    HirExprKind::FunctionRef(fid) => Some(*fid),
                    HirExprKind::Local(LocalId(idx)) => self.locals[*idx].func_id,
                    _ => None,
                };
                let id = self.declare_local_with_func_id(name.clone(), kind, func_id);
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
                if self.readonly_locals.contains(&id) {
                    return Err(HirError::ReadOnlyAssign {
                        name: name.clone(),
                        offset: stmt.span.start,
                    });
                }
                let value = self.lower_expr(value)?;
                let slot_kind = self.locals[id.0].kind;
                let value_kind = infer_kind(&value, &self.locals, &self.functions);
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
                let needs_break = body_contains_break(body);
                let break_id =
                    if needs_break {
                        Some(self.declare_local(
                            format!("_broken_{}", self.locals.len()),
                            ValueKind::Bool,
                        ))
                    } else {
                        None
                    };
                self.loop_break_targets.push(break_id);
                let cond_hir = self.lower_expr(cond)?;
                let body_hir = self.lower_scoped_body(body)?;
                self.loop_break_targets.pop();
                let while_stmt = HirStmt {
                    kind: HirStmtKind::While {
                        cond: cond_hir,
                        body: body_hir,
                        break_id,
                    },
                    span: stmt.span,
                };
                if let Some(id) = break_id {
                    let init = HirStmt {
                        kind: HirStmtKind::LocalInit {
                            id,
                            value: HirExpr {
                                kind: HirExprKind::Bool(false),
                                span: stmt.span,
                            },
                        },
                        span: stmt.span,
                    };
                    Ok(HirStmt {
                        kind: HirStmtKind::Block {
                            stmts: vec![init, while_stmt],
                        },
                        span: stmt.span,
                    })
                } else {
                    Ok(while_stmt)
                }
            }
            StmtKind::FunctionDef { .. } => {
                // FunctionDef at top level is lifted out into
                // `chunk.functions` by `lower()`. Inside a function
                // body, nested function definitions are not yet
                // supported (Phase 2.5b will introduce them).
                unimplemented!(
                    "Nested function definitions arrive in Phase 2.5b — \
                     top-level `local function` is hoisted in `lower()`"
                );
            }
            StmtKind::Return { value } => {
                let (returned_id, ret_value_id) =
                    self.in_function.ok_or(HirError::ReturnOutsideFunction {
                        offset: stmt.span.start,
                    })?;
                let mut block_stmts = Vec::new();
                if let Some(expr) = value {
                    let v = self.lower_expr(expr)?;
                    let v_kind = infer_kind(&v, &self.locals, &self.functions);
                    if v_kind != ValueKind::Number {
                        return Err(HirError::TypeMismatch {
                            op: "return".to_owned(),
                            lhs_kind: "number".to_owned(),
                            rhs_kind: v_kind.name().to_owned(),
                            offset: stmt.span.start,
                        });
                    }
                    self.in_function_ret_kind = Some(ValueKind::Number);
                    let ret_value_id = ret_value_id
                        .expect("_ret_value slot allocated whenever any return has a value");
                    block_stmts.push(HirStmt {
                        kind: HirStmtKind::Assign {
                            id: ret_value_id,
                            value: v,
                        },
                        span: stmt.span,
                    });
                }
                block_stmts.push(HirStmt {
                    kind: HirStmtKind::Assign {
                        id: returned_id,
                        value: HirExpr {
                            kind: HirExprKind::Bool(true),
                            span: stmt.span,
                        },
                    },
                    span: stmt.span,
                });
                Ok(HirStmt {
                    kind: HirStmtKind::Block { stmts: block_stmts },
                    span: stmt.span,
                })
            }
            StmtKind::Break => {
                // `break` lowers to `Assign { _broken_<n>, true }` —
                // the AND-extension of the loop cond and the per-stmt
                // guard wrap take care of skipping the rest. ADR 0015.
                //
                // Innermost loop's break target is on top of the stack.
                // It must be `Some` because `body_contains_break` runs
                // before the loop pushes, ensuring `break_id` is set
                // for any loop whose body holds a `break` reachable
                // from this point.
                let broken_id = self.loop_break_targets.last().copied().flatten().ok_or(
                    HirError::BreakOutsideLoop {
                        offset: stmt.span.start,
                    },
                )?;
                Ok(HirStmt {
                    kind: HirStmtKind::Assign {
                        id: broken_id,
                        value: HirExpr {
                            kind: HirExprKind::Bool(true),
                            span: stmt.span,
                        },
                    },
                    span: stmt.span,
                })
            }
            StmtKind::ForNumeric {
                var,
                start,
                stop,
                step,
                body,
            } => {
                // start/stop/step lower in the *outer* scope (they are
                // evaluated once before the loop variable comes into
                // existence — Lua 5.4 §3.3.5).
                let start_hir = self.lower_expr(start)?;
                let stop_hir = self.lower_expr(stop)?;
                let step_hir = match step {
                    Some(e) => self.lower_expr(e)?,
                    None => HirExpr {
                        kind: HirExprKind::Number(1.0),
                        span: Span::new(stmt.span.end, stmt.span.end),
                    },
                };
                // All three must be Number.
                for (label, ex) in [
                    ("start", &start_hir),
                    ("stop", &stop_hir),
                    ("step", &step_hir),
                ] {
                    let k = infer_kind(ex, &self.locals, &self.functions);
                    if k != ValueKind::Number {
                        return Err(HirError::TypeMismatch {
                            op: format!("for-{label}"),
                            lhs_kind: "number".to_owned(),
                            rhs_kind: k.name().to_owned(),
                            offset: ex.span.start,
                        });
                    }
                }
                // Body scope + read-only loop variable + optional break flag.
                let needs_break = body_contains_break(body);
                let break_id =
                    if needs_break {
                        Some(self.declare_local(
                            format!("_broken_{}", self.locals.len()),
                            ValueKind::Bool,
                        ))
                    } else {
                        None
                    };
                self.loop_break_targets.push(break_id);
                self.scopes.push(HashMap::new());
                let var_id = self.declare_local(var.clone(), ValueKind::Number);
                self.readonly_locals.insert(var_id);
                let body_result = self.lower_scoped_body_no_push(body);
                self.readonly_locals.remove(&var_id);
                self.scopes.pop();
                self.loop_break_targets.pop();
                let body_hir = body_result?;
                let for_stmt = HirStmt {
                    kind: HirStmtKind::ForNumeric {
                        var_id,
                        start: start_hir,
                        stop: stop_hir,
                        step: step_hir,
                        body: body_hir,
                        break_id,
                    },
                    span: stmt.span,
                };
                if let Some(id) = break_id {
                    let init = HirStmt {
                        kind: HirStmtKind::LocalInit {
                            id,
                            value: HirExpr {
                                kind: HirExprKind::Bool(false),
                                span: stmt.span,
                            },
                        },
                        span: stmt.span,
                    };
                    Ok(HirStmt {
                        kind: HirStmtKind::Block {
                            stmts: vec![init, for_stmt],
                        },
                        span: stmt.span,
                    })
                } else {
                    Ok(for_stmt)
                }
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

    fn lower_expr(&mut self, expr: &Expr) -> Result<HirExpr, HirError> {
        let kind = match &expr.kind {
            ExprKind::Number(n) => HirExprKind::Number(*n),
            ExprKind::Ident(name) => match self.resolve(name) {
                Some(id) => HirExprKind::Local(id),
                None => {
                    // Phase 2.5b: a top-level `local function f` registers
                    // `f` in `function_names` but does *not* (in 2.5a)
                    // create a local. Resolve identifiers that hit this
                    // map as a `FunctionRef` so they can be aliased via
                    // `local g = f` and called via `f(args)`.
                    if let Some(&fid) = self.function_names.get(name) {
                        HirExprKind::FunctionRef(fid)
                    } else {
                        return Err(HirError::UndefinedName {
                            name: name.clone(),
                            offset: expr.span.start,
                        });
                    }
                }
            },
            ExprKind::Bool(b) => HirExprKind::Bool(*b),
            ExprKind::Nil => HirExprKind::Nil,
            ExprKind::BinOp { op, lhs, rhs } => {
                let lhs_hir = self.lower_expr(lhs)?;
                let rhs_hir = self.lower_expr(rhs)?;
                let lk = infer_kind(&lhs_hir, &self.locals, &self.functions);
                let rk = infer_kind(&rhs_hir, &self.locals, &self.functions);
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
                    // Logical and/or: both sides must share a kind. Result
                    // kind matches both. (Heterogeneous defers to dynamic
                    // typing in a later phase — ADR 0013.)
                    BinOp::And | BinOp::Or => {
                        if lk != rk {
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
                }
            }
            ExprKind::UnaryOp { op, operand } => HirExprKind::UnaryOp {
                op: *op,
                operand: Box::new(self.lower_expr(operand)?),
            },
            ExprKind::FunctionExpr { params, body } => {
                // Register a fresh HirFunction with `name = ""` and
                // mangled `user_anon_<idx>` (ADR 0017). The body is
                // lowered in a separate LowerCtx so it has its own
                // scope/locals/break-stack.
                let id = FuncId(self.functions.len());
                let mangled = format!("user_anon_{}", id.0);
                self.functions.push(HirFunction {
                    name: String::new(),
                    mangled_name: mangled,
                    params: params
                        .iter()
                        .map(|p| LocalInfo {
                            name: p.clone(),
                            kind: ValueKind::Number,
                            func_id: None,
                        })
                        .collect(),
                    locals: Vec::new(),
                    body: Vec::new(),
                    ret_kind: None,
                });
                let mut fn_ctx =
                    LowerCtx::for_function(&self.function_names, &self.functions, params, body);
                let body_hir = fn_ctx.lower_function_body(body)?;
                let ret_kind = fn_ctx.in_function_ret_kind;
                self.functions[id.0].params = fn_ctx.locals[..params.len()].to_vec();
                self.functions[id.0].locals = fn_ctx.locals;
                self.functions[id.0].body = body_hir;
                self.functions[id.0].ret_kind = ret_kind;
                HirExprKind::FunctionRef(id)
            }
            ExprKind::Call { callee, args } => self.lower_call(callee, args, expr)?,
        };
        Ok(HirExpr {
            kind,
            span: expr.span,
        })
    }

    fn lower_call(
        &mut self,
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
        // Phase 2.5b/2.5b.2: a Function-kind local takes precedence
        // over the function-name table. The kind's arity must match
        // `args.len()`. If the local has a known FuncId we dispatch
        // statically; otherwise (function passed as a parameter) we
        // emit `Callee::Indirect`.
        if let Some(local_id) = self.resolve(name) {
            if let ValueKind::Function(arity) = self.locals[local_id.0].kind {
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
                for arg in &lowered_args {
                    let k = infer_kind(arg, &self.locals, &self.functions);
                    if !matches!(k, ValueKind::Number | ValueKind::Function(_)) {
                        return Err(HirError::TypeMismatch {
                            op: format!("call-{name}"),
                            lhs_kind: "number or function".to_owned(),
                            rhs_kind: k.name().to_owned(),
                            offset: arg.span.start,
                        });
                    }
                }
                let callee = match self.locals[local_id.0].func_id {
                    Some(fid) => Callee::User(fid),
                    None => Callee::Indirect(local_id),
                };
                return Ok(HirExprKind::Call {
                    callee,
                    args: lowered_args,
                });
            }
        }
        // User functions take precedence over builtins. (Phase 2.5a
        // doesn't allow shadowing `print` since users can't define a
        // function called `print` without explicit conflict — but we
        // resolve user names first for forward-compatibility with
        // 2.5b's first-class function values.)
        if let Some(&fid) = self.function_names.get(name) {
            // Snapshot what the lower_call closure needs from the
            // function's signature *before* recursing through
            // `lower_expr` (which mutably borrows `self`).
            let param_kinds: Vec<ValueKind> = self.functions[fid.0]
                .params
                .iter()
                .map(|p| p.kind)
                .collect();
            let expected = param_kinds.len();
            if args.len() != expected {
                return Err(HirError::ArityMismatch {
                    builtin: name.clone(),
                    expected,
                    actual: args.len(),
                    offset: whole.span.start,
                });
            }
            let lowered_args = args
                .iter()
                .map(|a| self.lower_expr(a))
                .collect::<Result<Vec<_>, _>>()?;
            // Phase 2.5b.2: each arg's kind must match the corresponding
            // param's kind (Number param ↔ Number arg, Function(arity)
            // param ↔ Function(arity) arg).
            for (i, arg) in lowered_args.iter().enumerate() {
                let arg_kind = infer_kind(arg, &self.locals, &self.functions);
                let expected_kind = param_kinds[i];
                let compatible = match (expected_kind, arg_kind) {
                    (ValueKind::Number, ValueKind::Number) => true,
                    (ValueKind::Function(a), ValueKind::Function(b)) => a == b,
                    _ => false,
                };
                if !compatible {
                    return Err(HirError::TypeMismatch {
                        op: format!("call-{name}"),
                        lhs_kind: expected_kind.name().to_owned(),
                        rhs_kind: arg_kind.name().to_owned(),
                        offset: arg.span.start,
                    });
                }
            }
            return Ok(HirExprKind::Call {
                callee: Callee::User(fid),
                args: lowered_args,
            });
        }
        let builtin = match Builtin::from_name(name) {
            Some(b) => b,
            None => {
                return Err(HirError::UnknownFunction {
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
        // Phase 2.5b: builtin args may be Number/Bool/Nil but never a
        // Function value (function values cannot be printed or otherwise
        // observed as values yet). Reject explicitly.
        for arg in &lowered_args {
            if let ValueKind::Function(_) = infer_kind(arg, &self.locals, &self.functions) {
                let arg_name = match &arg.kind {
                    HirExprKind::Local(LocalId(idx)) => self.locals[*idx].name.clone(),
                    HirExprKind::FunctionRef(_) => "<anonymous>".to_owned(),
                    _ => "<unknown>".to_owned(),
                };
                return Err(HirError::FunctionUsedAsValue {
                    name: arg_name,
                    offset: arg.span.start,
                });
            }
        }
        Ok(HirExprKind::Call {
            callee: Callee::Builtin(builtin),
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
            HirExprKind::Call { callee, args } => {
                assert!(matches!(callee, Callee::Builtin(Builtin::Print)));
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
        let HirExprKind::Call { callee, args } = &call.kind else {
            panic!("expected Call for print(x)");
        };
        assert!(matches!(callee, Callee::Builtin(Builtin::Print)));
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
    fn lower_unknown_function_errors() {
        // Phase 2.5a renamed `UnknownBuiltin` to `UnknownFunction`
        // because user-defined functions now share the dispatch path.
        let err = lower_src("foo(1)").expect_err("unknown call target must fail");
        assert!(matches!(err, HirError::UnknownFunction { .. }));
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
    fn lower_not_returns_bool_kind() {
        // `not 1` → UnaryOp::Not(Number) — kind must be Bool.
        let hir = lower_src("print(not 1)").expect("must lower");
        let HirStmtKind::ExprStmt(call) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected Call");
        };
        let arg = &args[0];
        assert_eq!(
            infer_kind(arg, &hir.locals, &hir.functions),
            ValueKind::Bool
        );
    }

    #[test]
    fn lower_and_with_same_kind_passes() {
        let hir = lower_src("print(true and false)").expect("must lower");
        let HirStmtKind::ExprStmt(call) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected Call");
        };
        assert!(matches!(
            args[0].kind,
            HirExprKind::BinOp {
                op: crate::parser::BinOp::And,
                ..
            }
        ));
    }

    #[test]
    fn lower_and_with_different_kinds_returns_type_mismatch() {
        let err = lower_src("print(1 and true)").expect_err("heterogeneous and must reject");
        assert!(matches!(err, HirError::TypeMismatch { .. }));
    }

    #[test]
    fn lower_or_with_different_kinds_returns_type_mismatch() {
        let err = lower_src("print(nil or 1)").expect_err("heterogeneous or must reject");
        assert!(matches!(err, HirError::TypeMismatch { .. }));
    }

    #[test]
    fn lower_for_numeric_default_step_inserts_constant_one() {
        let hir = lower_src("for i = 1, 3 do print(i) end").expect("must lower");
        let HirStmtKind::ForNumeric {
            start,
            stop,
            step,
            body,
            ..
        } = &hir.stmts[0].kind
        else {
            panic!("expected ForNumeric, got {:?}", hir.stmts[0].kind);
        };
        assert!(matches!(start.kind, HirExprKind::Number(1.0)));
        assert!(matches!(stop.kind, HirExprKind::Number(3.0)));
        // Implicit step → synthesised Number(1.0).
        assert!(matches!(step.kind, HirExprKind::Number(1.0)));
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn lower_for_numeric_with_explicit_step() {
        let hir = lower_src("for i = 10, 1, -2 do print(i) end").expect("must lower");
        let HirStmtKind::ForNumeric { step, .. } = &hir.stmts[0].kind else {
            panic!("expected ForNumeric");
        };
        // -2 lowers to UnaryOp::Neg over Number(2.0).
        assert!(matches!(
            step.kind,
            HirExprKind::UnaryOp {
                op: crate::parser::UnaryOp::Neg,
                ..
            }
        ));
    }

    #[test]
    fn lower_for_with_non_number_start_returns_type_mismatch() {
        let err =
            lower_src("for i = true, 3 do print(i) end").expect_err("non-Number start must reject");
        assert!(matches!(err, HirError::TypeMismatch { .. }));
    }

    #[test]
    fn lower_for_with_assign_to_loop_var_returns_readonly() {
        let err =
            lower_src("for i = 1, 3 do i = 99 end").expect_err("assigning to loop var must reject");
        match err {
            HirError::ReadOnlyAssign { name, .. } => assert_eq!(name, "i"),
            other => panic!("expected ReadOnlyAssign, got {other:?}"),
        }
    }

    #[test]
    fn lower_for_loop_var_invisible_outside() {
        let err = lower_src("for i = 1, 3 do end\nprint(i)").expect_err("loop var must not leak");
        assert!(matches!(err, HirError::UndefinedName { .. }));
    }

    #[test]
    fn lower_break_outside_loop_returns_error() {
        let err = lower_src("break").expect_err("break outside loop must reject");
        assert!(matches!(err, HirError::BreakOutsideLoop { .. }));
    }

    #[test]
    fn lower_break_inside_while_lowers_without_error() {
        // The hidden flag is wrapped inside an enclosing Block so the
        // public HirChunk has a single top-level Block stmt that
        // contains a LocalInit + While.
        let hir = lower_src("while true do break end").expect("must lower");
        assert_eq!(hir.stmts.len(), 1);
        let HirStmtKind::Block { stmts } = &hir.stmts[0].kind else {
            panic!("expected enclosing Block for while+break");
        };
        // First stmt: LocalInit of the hidden _broken local (Bool).
        assert!(matches!(stmts[0].kind, HirStmtKind::LocalInit { .. }));
        // Second stmt: the actual While.
        assert!(matches!(stmts[1].kind, HirStmtKind::While { .. }));
    }

    #[test]
    fn lower_break_inside_for_sets_break_id() {
        let hir = lower_src("for i = 1, 5 do if i == 3 then break end end").expect("must lower");
        let HirStmtKind::Block { stmts } = &hir.stmts[0].kind else {
            panic!("expected enclosing Block for for+break");
        };
        assert!(matches!(stmts[0].kind, HirStmtKind::LocalInit { .. }));
        let HirStmtKind::ForNumeric { break_id, .. } = &stmts[1].kind else {
            panic!("expected ForNumeric");
        };
        assert!(
            break_id.is_some(),
            "for-loop with break must carry break_id"
        );
    }

    #[test]
    fn lower_break_inside_if_in_while_targets_outer_while() {
        let hir = lower_src("while true do if true then break end end").expect("must lower");
        // Should lower without BreakOutsideLoop.
        let HirStmtKind::Block { stmts } = &hir.stmts[0].kind else {
            panic!("expected Block");
        };
        assert!(matches!(stmts[1].kind, HirStmtKind::While { .. }));
    }

    #[test]
    fn lower_nested_loops_break_targets_innermost() {
        // Inner break should reference the inner loop's flag — the
        // outer body has no `break` of its own, so the outer While
        // stays unwrapped while the inner becomes a Block { LocalInit
        // _broken, While { break_id: Some(_) } }.
        let hir = lower_src("while true do while true do break end end").expect("must lower");
        let HirStmtKind::While { body, break_id, .. } = &hir.stmts[0].kind else {
            panic!("expected outer While at top level");
        };
        assert!(break_id.is_none(), "outer body has no break");
        // Outer body's only stmt is the inner block.
        assert_eq!(body.len(), 1);
        let HirStmtKind::Block { stmts } = &body[0].kind else {
            panic!("expected inner to be wrapped in Block");
        };
        assert_eq!(stmts.len(), 2);
    }

    #[test]
    fn lower_break_in_do_block_outside_loop_errors() {
        let err =
            lower_src("do break end").expect_err("break in do-block outside loop must reject");
        assert!(matches!(err, HirError::BreakOutsideLoop { .. }));
    }

    #[test]
    fn lower_function_def_registers_in_function_table() {
        let hir = lower_src("local function f() end").expect("must lower");
        assert_eq!(hir.functions.len(), 1);
        assert_eq!(hir.functions[0].name, "f");
        assert_eq!(hir.functions[0].mangled_name, "user_f_0");
        assert!(hir.functions[0].params.is_empty());
        assert_eq!(hir.functions[0].ret_kind, None);
    }

    #[test]
    fn lower_function_call_resolves_to_user_func_id() {
        let hir = lower_src("local function f() return 1 end\nprint(f())").expect("must lower");
        // The print arg is a Call to user f.
        let HirStmtKind::ExprStmt(call) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { callee, args } = &call.kind else {
            panic!("expected Call");
        };
        assert!(matches!(callee, Callee::Builtin(Builtin::Print)));
        // The single arg is a user-function call.
        assert_eq!(args.len(), 1);
        let HirExprKind::Call { callee: inner, .. } = &args[0].kind else {
            panic!("expected nested Call");
        };
        assert!(matches!(inner, Callee::User(FuncId(0))));
    }

    #[test]
    fn lower_function_recursion_supported() {
        let src = "local function f(n) if n == 0 then return 0 end\nreturn f(n) end";
        let hir = lower_src(src).expect("recursion must lower (name registered before body)");
        assert_eq!(hir.functions.len(), 1);
        assert_eq!(hir.functions[0].params.len(), 1);
    }

    #[test]
    fn lower_call_to_unknown_function_errors() {
        let err = lower_src("foo()").expect_err("unknown function name must reject");
        // Either UnknownBuiltin or UnknownFunction is acceptable as a
        // surface error — the message should mention `foo`.
        let msg = format!("{err}");
        assert!(msg.contains("foo"), "got: {msg}");
    }

    #[test]
    fn lower_return_outside_function_errors() {
        let err = lower_src("return 1").expect_err("top-level return must reject");
        assert!(matches!(err, HirError::ReturnOutsideFunction { .. }));
    }

    #[test]
    fn lower_return_with_value_inside_function() {
        let hir = lower_src("local function f() return 42 end").expect("must lower");
        assert_eq!(hir.functions.len(), 1);
        let body = &hir.functions[0].body;
        // The body's first user statement is the return; the lowering
        // wraps it in the same body-guard pattern as break, so we just
        // inspect the function's ret_kind here.
        assert_eq!(hir.functions[0].ret_kind, Some(ValueKind::Number));
        assert!(!body.is_empty());
    }

    #[test]
    fn lower_function_body_locals_are_independent_of_outer() {
        // Outer `x` is a Number; the function body's `x` parameter is
        // its own slot, not the outer one. The function shouldn't see
        // the outer x at all.
        let hir = lower_src("local x = 1\nlocal function f(x) return x end\nprint(f(2))")
            .expect("must lower");
        // Outer chunk has one local (`x`).
        assert_eq!(hir.locals.len(), 1);
        assert_eq!(hir.locals[0].name, "x");
        // Function f has its own params and locals (independent).
        assert_eq!(hir.functions[0].params.len(), 1);
    }

    #[test]
    fn lower_anonymous_function_registers_in_table_with_anon_name() {
        let hir = lower_src("local f = function() return 1 end").expect("must lower");
        assert_eq!(hir.functions.len(), 1);
        assert_eq!(hir.functions[0].mangled_name, "user_anon_0");
        // Source-level name is intentionally empty for anonymous fns.
        assert!(hir.functions[0].name.is_empty());
    }

    #[test]
    fn lower_local_init_with_function_expr_records_function_kind() {
        let hir = lower_src("local f = function(x) return x end").expect("must lower");
        assert_eq!(hir.locals.len(), 1);
        assert_eq!(hir.locals[0].name, "f");
        // Phase 2.5b.2: Function(arity); FuncId moved to func_id field.
        assert!(matches!(hir.locals[0].kind, ValueKind::Function(1)));
        assert_eq!(hir.locals[0].func_id, Some(FuncId(0)));
    }

    #[test]
    fn lower_alias_local_propagates_function_kind() {
        let hir =
            lower_src("local f = function() return 1 end\nlocal g = f").expect("alias must lower");
        assert_eq!(hir.locals.len(), 2);
        // Both locals share Function(0) (zero-arity) and the same FuncId.
        assert!(matches!(hir.locals[0].kind, ValueKind::Function(0)));
        assert!(matches!(hir.locals[1].kind, ValueKind::Function(0)));
        assert_eq!(hir.locals[0].func_id, Some(FuncId(0)));
        assert_eq!(hir.locals[1].func_id, Some(FuncId(0)));
    }

    #[test]
    fn lower_call_via_function_typed_local_resolves_to_func_id() {
        let hir = lower_src("local f = function(x) return x end\nprint(f(7))").expect("must lower");
        // The print's arg is a Call into the Function-typed local.
        let HirStmtKind::ExprStmt(call) = &hir.stmts[1].kind else {
            panic!("expected print(f(7)) ExprStmt at stmts[1]");
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected print Call");
        };
        let HirExprKind::Call { callee: inner, .. } = &args[0].kind else {
            panic!("expected nested Call");
        };
        assert!(matches!(inner, Callee::User(FuncId(0))));
    }

    #[test]
    fn lower_function_used_as_value_errors() {
        let err = lower_src("local f = function() end\nprint(f)")
            .expect_err("function passed as plain value must reject");
        let msg = format!("{err}");
        assert!(
            msg.contains("function") || msg.contains("FunctionUsedAsValue"),
            "got: {msg}"
        );
    }

    #[test]
    fn lower_function_passed_as_arg_now_succeeds_in_2_5b2() {
        // Phase 2.5b: this errored (no `func.call_indirect` infrastructure).
        // Phase 2.5b.2: passing a function with matching arity is the
        // whole point of this phase, so it must succeed.
        let result = lower_src(
            "local f = function() return 1 end\nlocal function apply(g) return g() end\napply(f)",
        );
        assert!(result.is_ok(), "f as arg should lower in 2.5b.2");
    }

    #[test]
    fn lower_function_kind_carries_arity() {
        // Phase 2.5b.2: Function payload is arity, not FuncId.
        let hir = lower_src("local f = function(x, y) return x + y end").expect("lower");
        assert!(matches!(hir.locals[0].kind, ValueKind::Function(2)));
        assert_eq!(hir.locals[0].func_id, Some(FuncId(0)));
    }

    #[test]
    fn lower_function_arg_param_has_no_func_id() {
        // The g param in `apply(g, x)` is Function(1) with func_id None.
        let hir = lower_src(
            "local function apply(g, x) return g(x) end\nlocal f = function(x) return x end\nprint(apply(f, 5))",
        )
        .expect("lower");
        // `apply` is functions[0]; its first param `g` should be
        // Function(1) with func_id None.
        let apply_fn = &hir.functions[0];
        assert_eq!(apply_fn.name, "apply");
        assert!(matches!(apply_fn.params[0].kind, ValueKind::Function(1)));
        assert_eq!(apply_fn.params[0].func_id, None);
        assert!(matches!(apply_fn.params[1].kind, ValueKind::Number));
    }

    #[test]
    fn lower_call_via_function_arg_uses_indirect_callee() {
        let hir = lower_src(
            "local function apply(g, x) return g(x) end\nlocal f = function(x) return x end\nprint(apply(f, 5))",
        )
        .expect("lower");
        let apply_fn = &hir.functions[0];
        // The apply body has a return that lowers to: assign _ret_value
        // = call(g, x). Find the user-call inside.
        // Quick structural check: the call to g goes through Indirect.
        let body_str = format!("{:?}", apply_fn.body);
        assert!(
            body_str.contains("Indirect"),
            "expected `Callee::Indirect` somewhere in apply's body, got:\n{body_str}"
        );
    }

    #[test]
    fn lower_arity_mismatch_on_function_arg_errors() {
        // f has arity 2 but apply's g param is called with 1 arg.
        // The call `apply(f, 5)` should fail because f's arity (2)
        // doesn't match what apply expects of g (1, inferred from g(x)).
        let result = lower_src(
            "local function apply(g, x) return g(x) end\nlocal f = function(a, b) return a + b end\nprint(apply(f, 5))",
        );
        assert!(result.is_err(), "arity mismatch must reject");
    }

    #[test]
    fn lower_phase2_0_target_succeeds() {
        let hir = lower_src("local x = 1\nprint(x + 2)").expect("Phase 2.0 target lowers");
        assert_eq!(hir.locals.len(), 1);
        assert_eq!(hir.stmts.len(), 2);
    }
}
