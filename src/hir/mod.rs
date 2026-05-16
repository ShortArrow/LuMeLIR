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
    IndirectSig, LocalId, LocalInfo, ParentScope, UpvalueInfo,
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
    /// String value (Phase 2.7a, ADR 0024). Storage is a `!llvm.ptr`
    /// to a static C-string global; length is recovered via libc
    /// `strlen` at the `#` use site rather than tracked statically.
    String,
    /// Table value (Phase 2.6a-min, ADR 0053). Reference type —
    /// represented as `!llvm.ptr` pointing at a heap-allocated
    /// `[length: i64]`-prefixed region. Phase 2.6a-min only
    /// supports the empty form; populated arrays / hashes /
    /// metatables are out-of-scope until later sub-phases.
    Table,
    /// Phase 2.6c-tag-locals (ADR 0063): a local slot that may
    /// carry a Number value or Nil. Storage is the same 16-byte
    /// tagged layout used by array elements / hash entries —
    /// `{i64 tag, f64 value}` — so `emit_value_slot_*` helpers
    /// transfer over. Produced by `lower_stmt(LocalInit | Assign)`
    /// when the value is `HirExprKind::Index` (or its rewritten
    /// `IndexTagged`); future sub-phases will extend to
    /// MaybeNilBool / MaybeNilString etc.
    TaggedValue,
}

impl ValueKind {
    fn name(self) -> &'static str {
        match self {
            ValueKind::Number => "number",
            ValueKind::Bool => "bool",
            ValueKind::Nil => "nil",
            ValueKind::Function(_) => "function",
            ValueKind::String => "string",
            ValueKind::Table => "table",
            ValueKind::TaggedValue => "number?",
        }
    }
}

/// Number-compatible kinds: a plain `Number` or a
/// `TaggedValue` (the latter traps at the Local read site if
/// the tag is Nil; in HIR the kinds are interchangeable for
/// arithmetic / comparison / print). Phase 2.6c-tag-locals
/// (ADR 0063).
fn is_number_compatible(k: ValueKind) -> bool {
    matches!(k, ValueKind::Number | ValueKind::TaggedValue)
}

/// Phase 2.6b-hash-keys (ADR 0079) / 2.8e-iter-tk (ADR 0084):
/// Lua spec §3.4.5 — every non-nil, non-NaN value can be used as
/// a hash key. We accept Number / String / Bool / Function / Table
/// statically and `TaggedValue` for the dynamic-kind case (e.g.
/// `for k, v in pairs(t) do t[k] = v + 100 end`). Nil is HIR-
/// rejected for static keys; runtime nil via TaggedValue is
/// trapped by codegen with `s_table_index_nil` (ADR 0084). NaN
/// remains a runtime miss with the generic missing-key trap
/// (LIC-2.6b-hash-key-nan-runtime-1).
fn is_hash_key_eligible(k: ValueKind) -> bool {
    matches!(
        k,
        ValueKind::Number
            | ValueKind::String
            | ValueKind::Bool
            | ValueKind::Function(_)
            | ValueKind::Table
            | ValueKind::TaggedValue
    )
}

/// Phase 2.7p-arith-string-coerce (ADR 0077, Codex Tidy First):
/// if `expr` has static kind `String`, wrap it in
/// `HirExprKind::ArithStringCoerce` so codegen runs `tonumber`
/// at runtime and traps on parse failure (Lua spec §3.4.1).
/// Otherwise pass `expr` through unchanged. Applied to both
/// operands of every arithmetic / bitwise BinOp before kind
/// validation so the wrapper's `Number` kind satisfies
/// `is_number_compatible` downstream.
fn coerce_arith_operand_if_string(
    expr: HirExpr,
    locals: &[LocalInfo],
    functions: &[HirFunction],
) -> HirExpr {
    if matches!(infer_kind(&expr, locals, functions), ValueKind::String) {
        let span = expr.span;
        HirExpr {
            kind: HirExprKind::ArithStringCoerce(Box::new(expr)),
            span,
        }
    } else {
        expr
    }
}

/// Phase 2.6c-tag-locals (ADR 0063): when a `LocalInit` /
/// `Assign`'s RHS is a plain `HirExprKind::Index`, rewrite it
/// into `HirExprKind::IndexTagged` so the local widens to
/// `TaggedValue`. Idempotent on every other shape.
fn widen_index_for_local_init(value: HirExpr) -> HirExpr {
    match value.kind {
        HirExprKind::Index { target, key } => HirExpr {
            kind: HirExprKind::IndexTagged { target, key },
            span: value.span,
        },
        _ => value,
    }
}

/// Phase 2.6c-tag-fn-tbl (ADR 0071): identify the underlying
/// `FuncId` for a Function-kind expression — either a direct
/// `FunctionRef` or a `Local` whose `func_id` was recorded at
/// declaration time. Used to query `upvalues` for the
/// closure-escape check on `IndexAssign` / `Table` constructor
/// values.
fn function_ref_id(expr: &HirExpr, locals: &[LocalInfo]) -> Option<FuncId> {
    match &expr.kind {
        HirExprKind::FunctionRef(fid) => Some(*fid),
        HirExprKind::Local(LocalId(idx)) => locals[*idx].func_id,
        _ => None,
    }
}

pub fn infer_kind(expr: &HirExpr, locals: &[LocalInfo], functions: &[HirFunction]) -> ValueKind {
    match &expr.kind {
        HirExprKind::Number(_) => ValueKind::Number,
        HirExprKind::Bool(_) => ValueKind::Bool,
        HirExprKind::Nil => ValueKind::Nil,
        HirExprKind::Str(_) => ValueKind::String,
        HirExprKind::Table(_) => ValueKind::Table,
        // Phase 2.6a-arr (ADR 0054): Number-only arrays mean
        // every read returns a Number.
        HirExprKind::Index { .. } => ValueKind::Number,
        // Phase 2.6c-isnil-query (ADR 0061): non-trapping nil
        // probe returns Bool. Phase 2.6c-tag-hetero-eq (ADR
        // 0066) unifies the previous IsNilQuery / IsNilLocal
        // pair into IsNil(operand).
        HirExprKind::IsNil(_) => ValueKind::Bool,
        // Phase 2.7p-arith-string-coerce (ADR 0077): the
        // wrapper materialises a Number from a String operand at
        // runtime; trap on parse failure (Lua spec §3.4.1).
        HirExprKind::ArithStringCoerce(_) => ValueKind::Number,
        // Phase 2.6c-tag-locals (ADR 0063).
        HirExprKind::IndexTagged { .. } => ValueKind::TaggedValue,
        HirExprKind::Local(LocalId(idx)) => locals[*idx].kind,
        HirExprKind::UnaryOp { op, .. } => match op {
            crate::parser::UnaryOp::Neg => ValueKind::Number,
            crate::parser::UnaryOp::Not => ValueKind::Bool,
            crate::parser::UnaryOp::BitNot => ValueKind::Number,
            crate::parser::UnaryOp::Len => ValueKind::Number,
        },
        HirExprKind::BinOp { op, lhs, .. } => match op {
            BinOp::Add
            | BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::Mod
            | BinOp::Pow
            | BinOp::FloorDiv
            | BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::Shl
            | BinOp::Shr => ValueKind::Number,
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => {
                ValueKind::Bool
            }
            // `and`/`or` preserve the operand kind (lower-time guard
            // ensures both sides share a kind).
            BinOp::And | BinOp::Or => infer_kind(lhs, locals, functions),
            // Phase 2.7b (ADR 0025): `..` always produces String.
            BinOp::Concat => ValueKind::String,
        },
        HirExprKind::Call { callee, .. } => match callee {
            // print() has no useful value in our subset; treat as Number
            // so existing arithmetic guards remain consistent (it never
            // actually appears as a comparison operand).
            Callee::Builtin(Builtin::Print) => ValueKind::Number,
            Callee::Builtin(Builtin::ToString) => ValueKind::String,
            Callee::Builtin(Builtin::ToNumber) => ValueKind::Number,
            Callee::Builtin(Builtin::Type) => ValueKind::String,
            Callee::Builtin(Builtin::Assert) => ValueKind::Bool,
            // Phase 2.7h (ADR 0033): error never returns at run-
            // time. The kind is a Number placeholder for static
            // typing only — code after `error(...)` is unreachable.
            Callee::Builtin(Builtin::Error) => ValueKind::Number,
            // Phase 2.8e-iter-next (ADR 0081): `next(...)` in
            // single-value position truncates to the first result —
            // the next key, which is a TaggedValue.
            Callee::Builtin(Builtin::Next) => ValueKind::TaggedValue,
            // User function: look up its declared return kind. Phase
            // 2.5a forces this to Number when present; void calls
            // never appear in expression position legally.
            // For multi-return callees in expression position, Lua
            // truncates to the first result. Phase 2.5d (ADR 0021).
            Callee::User { fid, .. } => functions[fid.0]
                .ret_kinds
                .first()
                .copied()
                .unwrap_or(ValueKind::Number),
            // Indirect call (function-kind local): Phase 2.5b.2 fixes
            // returns to Number, so that's the answer.
            Callee::Indirect(_) => ValueKind::Number,
            // Phase 2.5x-callee-dispatch (ADR 0082): the dispatch's
            // `sig.ret_kinds[0]` carries the call-site's expected
            // return kind. Truncates to the first result for
            // single-value position (Lua spec); MultiAssignFromCall
            // observes the full vector via `lower_local_multi`.
            Callee::IndirectDispatch { sig, .. } => {
                sig.ret_kinds.first().copied().unwrap_or(ValueKind::Number)
            }
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
        StmtKind::While { .. }
        | StmtKind::ForNumeric { .. }
        | StmtKind::ForIpairs { .. }
        | StmtKind::ForPairs { .. }
        | StmtKind::ForGeneric { .. }
        | StmtKind::Repeat { .. } => false,
        _ => false,
    }
}

/// Wrap a loop statement (`While`/`Repeat`/`ForNumeric`) with a
/// preceding `LocalInit` of the `_broken` flag when one was
/// allocated. Returning a fresh `Block` is the canonical
/// representation: the outer chunk sees a single statement, the
/// flag is initialised exactly once before the loop fires, and the
/// loop's `break_id` reads the same slot. Pure relative to its
/// inputs (Phase 2.4b shared between `While` and `Repeat`, ADR 0035).
fn wrap_with_break_init(loop_stmt: HirStmt, break_id: Option<LocalId>, span: Span) -> HirStmt {
    match break_id {
        None => loop_stmt,
        Some(id) => {
            let init = HirStmt {
                kind: HirStmtKind::LocalInit {
                    id,
                    value: HirExpr {
                        kind: HirExprKind::Bool(false),
                        span,
                    },
                },
                span,
            };
            HirStmt {
                kind: HirStmtKind::Block {
                    stmts: vec![init, loop_stmt],
                },
                span,
            }
        }
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

/// Phase 2.7c (ADR 0026): auto-wrap a non-String value in a
/// `tostring(...)` builtin call so `..` accepts mixed operands.
/// Function-kind values are still rejected — there is no useful
/// string representation in the current scope.
fn coerce_to_string(expr: HirExpr, kind: ValueKind, offset: usize) -> Result<HirExpr, HirError> {
    match kind {
        ValueKind::String => Ok(expr),
        // Phase 2.6a-min (ADR 0053): table concat needs `__tostring`
        // metatable resolution which doesn't exist yet — reject for
        // symmetry with Function-kind concat rejection.
        ValueKind::Function(_) | ValueKind::Table => Err(HirError::TypeMismatch {
            op: "..".to_owned(),
            lhs_kind: "string".to_owned(),
            rhs_kind: kind.name().to_owned(),
            offset,
        }),
        ValueKind::Number | ValueKind::Bool | ValueKind::Nil | ValueKind::TaggedValue => {
            let span = expr.span;
            Ok(HirExpr {
                kind: HirExprKind::Call {
                    callee: Callee::Builtin(Builtin::ToString),
                    args: vec![expr],
                },
                span,
            })
        }
    }
}

/// Phase 2.5d (ADR 0021): scan a function body for the largest
/// number of return values produced by any `return` in the body
/// (recursing through nested control flow but not into nested
/// `FunctionDef` bodies, which have their own return scope). Used by
/// [`LowerCtx::lower_function_body`] to allocate one `_ret_value_N`
/// slot per return position.
fn ast_max_return_arity(stmts: &[Stmt]) -> usize {
    fn visit(s: &Stmt, max: &mut usize) {
        match &s.kind {
            StmtKind::Return { value: Some(_) } => *max = (*max).max(1),
            StmtKind::Return { value: None } => {}
            StmtKind::ReturnMulti { values } => *max = (*max).max(values.len()),
            StmtKind::Block(b) => {
                for st in b {
                    visit(st, max);
                }
            }
            StmtKind::If {
                then_body,
                elifs,
                else_body,
                ..
            } => {
                for st in then_body {
                    visit(st, max);
                }
                for (_, b) in elifs {
                    for st in b {
                        visit(st, max);
                    }
                }
                if let Some(b) = else_body {
                    for st in b {
                        visit(st, max);
                    }
                }
            }
            StmtKind::While { body, .. }
            | StmtKind::ForNumeric { body, .. }
            | StmtKind::ForIpairs { body, .. }
            | StmtKind::ForPairs { body, .. }
            | StmtKind::ForGeneric { body, .. } => {
                for st in body {
                    visit(st, max);
                }
            }
            // Nested FunctionDef bodies have their own return scope.
            _ => {}
        }
    }
    let mut max = 0;
    for s in stmts {
        visit(s, &mut max);
    }
    max
}

/// Phase 2.5e (ADR 0020): static value-kind of an AST expression used
/// as a literal call argument, for cross-function param-kind
/// inference. Only the four kinds with literal forms (`true`/`false`,
/// `nil`, number literals, and unary-minus over them) are recognised
/// — anything else falls back to `Number`, matching the historical
/// default.
fn ast_arg_kind(expr: &Expr) -> ValueKind {
    match &expr.kind {
        ExprKind::Bool(_) => ValueKind::Bool,
        ExprKind::Nil => ValueKind::Nil,
        ExprKind::Number(_) => ValueKind::Number,
        // Phase 2.7b (ADR 0025): a string literal at a call site
        // refines that parameter to ValueKind::String, so functions
        // like `greet(name)` lower with a String-kind `name`.
        ExprKind::Str(_) => ValueKind::String,
        // Phase 2.6a-min (ADR 0053): a table constructor literal at
        // a call site refines the param to ValueKind::Table.
        ExprKind::Table(_) => ValueKind::Table,
        ExprKind::UnaryOp { op, operand }
            if matches!(op, UnaryOp::Neg) && matches!(operand.kind, ExprKind::Number(_)) =>
        {
            ValueKind::Number
        }
        _ => ValueKind::Number,
    }
}

/// Phase 2.5e (ADR 0020): walk the chunk AST to discover every call
/// site whose callee is a top-level `FunctionDef` name, recording the
/// static literal kind of each argument. The first call site for a
/// function determines that function's param kinds; later call sites
/// with different kinds are caught at HIR-time by `lower_call`'s
/// existing arg-vs-param kind check. Falls back to `Number` for any
/// unobserved param.
fn infer_user_function_param_kinds(
    chunk: &[Stmt],
    function_names: &HashMap<String, FuncId>,
    method_funcs: &HashMap<(String, String), FuncId>,
    arities: &[usize],
) -> Vec<Vec<ValueKind>> {
    let mut kinds: Vec<Vec<ValueKind>> = arities
        .iter()
        .map(|n| vec![ValueKind::Number; *n])
        .collect();
    let mut seen: Vec<bool> = vec![false; arities.len()];

    fn visit_stmt(
        s: &Stmt,
        names: &HashMap<String, FuncId>,
        method_funcs: &HashMap<(String, String), FuncId>,
        kinds: &mut Vec<Vec<ValueKind>>,
        seen: &mut Vec<bool>,
    ) {
        match &s.kind {
            StmtKind::Local { value, .. } | StmtKind::Assign { value, .. } => {
                visit_expr(value, names, method_funcs, kinds, seen);
            }
            StmtKind::ExprStmt(e) => visit_expr(e, names, method_funcs, kinds, seen),
            StmtKind::Return { value: Some(e) } => visit_expr(e, names, method_funcs, kinds, seen),
            StmtKind::Return { value: None } => {}
            StmtKind::Block(b) => {
                for st in b {
                    visit_stmt(st, names, method_funcs, kinds, seen);
                }
            }
            StmtKind::If {
                cond,
                then_body,
                elifs,
                else_body,
            } => {
                visit_expr(cond, names, method_funcs, kinds, seen);
                for st in then_body {
                    visit_stmt(st, names, method_funcs, kinds, seen);
                }
                for (c, b) in elifs {
                    visit_expr(c, names, method_funcs, kinds, seen);
                    for st in b {
                        visit_stmt(st, names, method_funcs, kinds, seen);
                    }
                }
                if let Some(b) = else_body {
                    for st in b {
                        visit_stmt(st, names, method_funcs, kinds, seen);
                    }
                }
            }
            StmtKind::While { cond, body } => {
                visit_expr(cond, names, method_funcs, kinds, seen);
                for st in body {
                    visit_stmt(st, names, method_funcs, kinds, seen);
                }
            }
            StmtKind::ForNumeric {
                start,
                stop,
                step,
                body,
                ..
            } => {
                visit_expr(start, names, method_funcs, kinds, seen);
                visit_expr(stop, names, method_funcs, kinds, seen);
                if let Some(s) = step {
                    visit_expr(s, names, method_funcs, kinds, seen);
                }
                for st in body {
                    visit_stmt(st, names, method_funcs, kinds, seen);
                }
            }
            StmtKind::ForIpairs { table, body, .. } => {
                visit_expr(table, names, method_funcs, kinds, seen);
                for st in body {
                    visit_stmt(st, names, method_funcs, kinds, seen);
                }
            }
            StmtKind::ForPairs { table, body, .. } => {
                visit_expr(table, names, method_funcs, kinds, seen);
                for st in body {
                    visit_stmt(st, names, method_funcs, kinds, seen);
                }
            }
            StmtKind::ForGeneric {
                iter,
                state,
                ctl,
                body,
                ..
            } => {
                visit_expr(iter, names, method_funcs, kinds, seen);
                visit_expr(state, names, method_funcs, kinds, seen);
                visit_expr(ctl, names, method_funcs, kinds, seen);
                for st in body {
                    visit_stmt(st, names, method_funcs, kinds, seen);
                }
            }
            StmtKind::Repeat { body, cond } => {
                for st in body {
                    visit_stmt(st, names, method_funcs, kinds, seen);
                }
                visit_expr(cond, names, method_funcs, kinds, seen);
            }
            // FunctionDef bodies are also walked — recursive calls and
            // calls into sibling top-level functions count.
            StmtKind::FunctionDef { body, .. } => {
                for st in body {
                    visit_stmt(st, names, method_funcs, kinds, seen);
                }
            }
            // Phase 2.6+-methods (ADR 0092): method-def bodies are
            // walked the same as FunctionDef. Receiver-arg refinement
            // via MethodCall is intentionally NOT done (carry-over
            // from ADR 0091 Index-callee — call-site refinement
            // doesn't extend to Index-callee Calls).
            StmtKind::MethodDef { body, .. } => {
                for st in body {
                    visit_stmt(st, names, method_funcs, kinds, seen);
                }
            }
            StmtKind::LocalMulti { values, .. } | StmtKind::AssignMulti { values, .. } => {
                for v in values {
                    visit_expr(v, names, method_funcs, kinds, seen);
                }
            }
            StmtKind::ReturnMulti { values } => {
                for v in values {
                    visit_expr(v, names, method_funcs, kinds, seen);
                }
            }
            StmtKind::IndexAssign { target, key, value } => {
                visit_expr(target, names, method_funcs, kinds, seen);
                visit_expr(key, names, method_funcs, kinds, seen);
                visit_expr(value, names, method_funcs, kinds, seen);
            }
            StmtKind::Break => {}
        }
    }

    fn visit_expr(
        e: &Expr,
        names: &HashMap<String, FuncId>,
        method_funcs: &HashMap<(String, String), FuncId>,
        kinds: &mut Vec<Vec<ValueKind>>,
        seen: &mut Vec<bool>,
    ) {
        match &e.kind {
            ExprKind::Call { callee, args } => {
                if let ExprKind::Ident(name) = &callee.kind
                    && let Some(&FuncId(idx)) = names.get(name)
                    && !seen[idx]
                    && args.len() == kinds[idx].len()
                {
                    for (i, a) in args.iter().enumerate() {
                        kinds[idx][i] = ast_arg_kind(a);
                    }
                    seen[idx] = true;
                }
                visit_expr(callee, names, method_funcs, kinds, seen);
                for a in args {
                    visit_expr(a, names, method_funcs, kinds, seen);
                }
            }
            ExprKind::BinOp { lhs, rhs, .. } => {
                visit_expr(lhs, names, method_funcs, kinds, seen);
                visit_expr(rhs, names, method_funcs, kinds, seen);
            }
            ExprKind::UnaryOp { operand, .. } => {
                visit_expr(operand, names, method_funcs, kinds, seen);
            }
            ExprKind::FunctionExpr { body, .. } => {
                for st in body {
                    visit_stmt(st, names, method_funcs, kinds, seen);
                }
            }
            // Phase 2.6+-method-arg-refine (ADR 0093): MethodCall
            // refinement via `method_funcs` lookup. Receiver must be
            // an Ident (otherwise no static FuncId resolution today);
            // explicit args (index 1..N) refine from literal kinds.
            // `self` at index 0 stays at the placeholder kind — the
            // ADR 0092 policy re-seeds Table at `lower_method_def`'s
            // for_function call, regardless of what placeholder kind
            // ends up there. First-call-site-wins semantics
            // (`seen[idx]`) match the FunctionDef arm above.
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                if let ExprKind::Ident(recv_name) = &receiver.kind
                    && let Some(&FuncId(idx)) =
                        method_funcs.get(&(recv_name.clone(), method.clone()))
                    && !seen[idx]
                    && args.len() + 1 == kinds[idx].len()
                {
                    for (i, a) in args.iter().enumerate() {
                        kinds[idx][i + 1] = ast_arg_kind(a);
                    }
                    seen[idx] = true;
                }
                visit_expr(receiver, names, method_funcs, kinds, seen);
                for a in args {
                    visit_expr(a, names, method_funcs, kinds, seen);
                }
            }
            _ => {}
        }
    }

    for s in chunk {
        visit_stmt(s, function_names, method_funcs, &mut kinds, &mut seen);
    }
    kinds
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
            StmtKind::Repeat { body, cond } => {
                for s in body {
                    visit_stmt(s, name_to_idx, kinds);
                }
                visit_expr(cond, name_to_idx, kinds);
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
            StmtKind::ForIpairs { table, body, .. } => {
                visit_expr(table, name_to_idx, kinds);
                for s in body {
                    visit_stmt(s, name_to_idx, kinds);
                }
            }
            StmtKind::ForPairs { table, body, .. } => {
                visit_expr(table, name_to_idx, kinds);
                for s in body {
                    visit_stmt(s, name_to_idx, kinds);
                }
            }
            StmtKind::ForGeneric {
                iter,
                state,
                ctl,
                body,
                ..
            } => {
                visit_expr(iter, name_to_idx, kinds);
                visit_expr(state, name_to_idx, kinds);
                visit_expr(ctl, name_to_idx, kinds);
                for s in body {
                    visit_stmt(s, name_to_idx, kinds);
                }
            }
            StmtKind::Break => {}
            StmtKind::FunctionDef { .. } => {} // nested fn defs not in 2.5b.2
            // Phase 2.6+-methods (ADR 0092): method-def bodies are
            // their own scope, mirroring FunctionDef. The outer
            // scope's param-kind inference does not descend.
            StmtKind::MethodDef { .. } => {}
            StmtKind::LocalMulti { values, .. } | StmtKind::AssignMulti { values, .. } => {
                for v in values {
                    visit_expr(v, name_to_idx, kinds);
                }
            }
            StmtKind::ReturnMulti { values } => {
                for v in values {
                    visit_expr(v, name_to_idx, kinds);
                }
            }
            StmtKind::IndexAssign { target, key, value } => {
                visit_expr(target, name_to_idx, kinds);
                visit_expr(key, name_to_idx, kinds);
                visit_expr(value, name_to_idx, kinds);
            }
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
            // Phase 2.6+-methods (ADR 0092): MethodCall participates
            // in param-kind inference only via descend into receiver
            // and args (Function-kind callee-position refinement
            // intentionally NOT applied — receiver is not a param
            // name we're refining).
            ExprKind::MethodCall { receiver, args, .. } => {
                visit_expr(receiver, name_to_idx, kinds);
                for a in args {
                    visit_expr(a, name_to_idx, kinds);
                }
            }
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
        BinOp::FloorDiv => "//",
        BinOp::Mod => "%",
        BinOp::Pow => "^",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "~",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Concat => "..",
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
/// Phase 2.5f (ADR 0036): pass-1 registration step shared between
/// top-level `lower()` and the nested-FunctionDef arm of
/// `lower_function_body`. Allocates a `FuncId`, pushes a placeholder
/// `HirFunction` (params with default Number kinds; body filled in
/// at pass 2), and inserts the name into `function_names` so
/// recursion + sibling forward-reference work. Pure relative to
/// its mutable arguments.
fn register_function_signature(
    name: &str,
    params: &[String],
    function_names: &mut HashMap<String, FuncId>,
    functions: &mut Vec<HirFunction>,
    parent_scope: ParentScope,
) -> FuncId {
    let id = FuncId(functions.len());
    functions.push(HirFunction {
        name: name.to_owned(),
        mangled_name: format!("user_{}_{}", name, id.0),
        params: params
            .iter()
            .map(|p| LocalInfo {
                name: p.clone(),
                kind: ValueKind::Number,
                func_id: None,
                is_captured: false,
            })
            .collect(),
        upvalues: Vec::new(),
        locals: Vec::new(),
        body: Vec::new(),
        ret_kinds: Vec::new(),
        parent_scope,
    });
    function_names.insert(name.to_owned(), id);
    id
}

/// Phase 2.6+-method-arg-refine (ADR 0093): pass-1 registration step
/// for `MethodDef` statements, mirroring `register_function_signature`.
/// Pre-allocates a `FuncId` and a placeholder `HirFunction` (mangled
/// `user_anon_<idx>` matching ADR 0092's lowering naming) so the
/// chunk-walker `infer_user_function_param_kinds` (Pass 1.5) can refine
/// args by reading the index BEFORE the body is lowered (Pass 2).
/// `lower_method_def` (ADR 0092) then re-uses this FuncId instead of
/// freshly allocating one.
///
/// `effective_params` are the post-self-prepend params (caller is
/// responsible for inserting `"self"` when `is_colon`). The index key
/// is `(receiver, method)` — last-wins on collision (same semantics
/// as `function_names`'s `insert` for shadowed FunctionDef names;
/// future ADR can lift to source-order resolution).
fn register_method_signature(
    receiver: &str,
    method: &str,
    effective_params: &[String],
    method_funcs: &mut HashMap<(String, String), FuncId>,
    functions: &mut Vec<HirFunction>,
    parent_scope: ParentScope,
) -> FuncId {
    let id = FuncId(functions.len());
    functions.push(HirFunction {
        name: String::new(),
        mangled_name: format!("user_anon_{}", id.0),
        params: effective_params
            .iter()
            .map(|p| LocalInfo {
                name: p.clone(),
                kind: ValueKind::Number,
                func_id: None,
                is_captured: false,
            })
            .collect(),
        upvalues: Vec::new(),
        locals: Vec::new(),
        body: Vec::new(),
        ret_kinds: Vec::new(),
        parent_scope,
    });
    method_funcs.insert((receiver.to_owned(), method.to_owned()), id);
    id
}

/// Phase 2.5f (ADR 0036): pass-2 body lowering step shared between
/// top-level `lower()` and the nested-FunctionDef arm of
/// `lower_function_body`. Lowers `body` into a fresh `LowerCtx`,
/// fills in `functions[fid.0]`, and hoists any anonymous functions
/// registered during inner lowering into the outer table.
fn lower_into_function(
    fid: FuncId,
    params: &[String],
    body: &[Stmt],
    function_names: &HashMap<String, FuncId>,
    method_funcs: &HashMap<(String, String), FuncId>,
    functions: &mut Vec<HirFunction>,
    outer_visible: HashMap<String, (LocalId, ValueKind)>,
) -> Result<(), HirError> {
    let pre_count = functions.len();
    let external_kinds: Vec<ValueKind> = functions[fid.0].params.iter().map(|p| p.kind).collect();
    let mut sub_ctx = LowerCtx::for_function(
        function_names,
        method_funcs,
        functions,
        params,
        body,
        &external_kinds,
        outer_visible,
        fid,
    );
    let body_hir = sub_ctx.lower_function_body(body)?;
    let ret_kinds = sub_ctx.in_function_ret_kinds.unwrap_or_default();
    functions[fid.0].params = sub_ctx.locals[..params.len()].to_vec();
    functions[fid.0].upvalues = sub_ctx.upvalues;
    functions[fid.0].locals = sub_ctx.locals;
    functions[fid.0].body = body_hir;
    functions[fid.0].ret_kinds = ret_kinds;
    for new_fn in sub_ctx.functions.into_iter().skip(pre_count) {
        functions.push(new_fn);
    }
    Ok(())
}

pub fn lower(chunk: &Chunk) -> Result<HirChunk, HirError> {
    // Pass 1: register every top-level `local function` in the
    // function table so recursion and forward-reference work.
    let mut functions: Vec<HirFunction> = Vec::new();
    let mut function_names: HashMap<String, FuncId> = HashMap::new();
    for stmt in chunk {
        if let StmtKind::FunctionDef { name, params, .. } = &stmt.kind {
            register_function_signature(
                name,
                params,
                &mut function_names,
                &mut functions,
                ParentScope::Chunk,
            );
        }
    }
    // Phase 2.6+-method-arg-refine (ADR 0093): pass-1 also registers
    // every top-level MethodDef so the chunk-walker can refine call-
    // site arg kinds before lowering. Indexed by `(receiver, method)`
    // — last-wins on shadowing, same as `function_names` (carry-over).
    // FunctionDef and MethodDef walks are kept sequential (not
    // interleaved) so the `funcdef_seq` counter in Pass 2 still maps
    // 1:1 onto FunctionDef FuncIds in source order.
    let mut method_funcs: HashMap<(String, String), FuncId> = HashMap::new();
    for stmt in chunk {
        if let StmtKind::MethodDef {
            receiver,
            method,
            is_colon,
            params,
            ..
        } = &stmt.kind
        {
            let mut effective_params: Vec<String> = Vec::with_capacity(params.len() + 1);
            if *is_colon {
                effective_params.push("self".to_owned());
            }
            effective_params.extend_from_slice(params);
            register_method_signature(
                receiver,
                method,
                &effective_params,
                &mut method_funcs,
                &mut functions,
                ParentScope::Chunk,
            );
        }
    }
    // Phase 2.5e (ADR 0020): pre-scan all call sites for top-level
    // function names, refining each function's param kinds from
    // literal arg kinds at the first observed call. Without this,
    // every param defaults to Number and Bool/Nil call args get
    // rejected by `lower_call`'s kind check.
    let arities: Vec<usize> = functions.iter().map(|f| f.params.len()).collect();
    let inferred = infer_user_function_param_kinds(chunk, &function_names, &method_funcs, &arities);
    for (i, kinds) in inferred.iter().enumerate() {
        for (j, k) in kinds.iter().enumerate() {
            functions[i].params[j].kind = *k;
        }
    }

    // Pass 2 (Phase 2.5c.1, ADR 0042): walk the chunk in source
    // order, lowering each function body at the position where its
    // FunctionDef appears so the body can capture chunk-level
    // locals declared above it. Earlier phases lowered all bodies
    // first with an empty `outer_visible`, which made top-level
    // captures statically unreachable; ADR 0037 documented that
    // gap, ADR 0042 closes it.
    let mut ctx = LowerCtx::new(function_names.clone(), method_funcs.clone(), functions);
    // Phase 2.5c-full Commit 3b prep fix (ADR 0083): pass 1.5 — declare
    // a synthetic chunk local for every top-level `local function f`.
    // This is the same shape `local f = function() end` already
    // produces and makes `self.resolve(name)` succeed for every
    // forward / chunk-level call site, so the upcoming
    // `emit_call_user_with_cell` cutover can load the closure cell
    // ptr from the local's slot instead of guessing whether a
    // `function_names` fallback was self-recursion or not. The
    // declaration must precede the source-order walk so forward
    // references resolve.
    for stmt in chunk {
        if let StmtKind::FunctionDef { name, params, .. } = &stmt.kind {
            let arity = params.len();
            let fid = ctx.function_names[name];
            ctx.declare_local_with_func_id(name.clone(), ValueKind::Function(arity), Some(fid));
        }
    }
    let mut stmts = Vec::new();
    let mut funcdef_seq: usize = 0;
    for s in chunk {
        if let StmtKind::FunctionDef {
            name, params, body, ..
        } = &s.kind
        {
            let fid = FuncId(funcdef_seq);
            funcdef_seq += 1;
            let outer_visible = ctx.outer_visible_snapshot();
            lower_into_function(
                fid,
                params,
                body,
                &ctx.function_names,
                &ctx.method_funcs,
                &mut ctx.functions,
                outer_visible,
            )?;
            // Phase 2.5c-full Commit 3b prep fix (ADR 0083): emit a
            // synthetic LocalInit at the FunctionDef's source position
            // so the closure cell ptr is materialised into the
            // synthetic local's slot. For non-capturing fns this is
            // an alias-skip (LocalInit Function-kind storage rule);
            // capturing fns will store the malloc'd cell ptr once
            // Commit 3b body lands the storage rule update.
            let local_id = ctx
                .resolve(name)
                .expect("synthetic local declared in pass 1.5 above");
            stmts.push(HirStmt {
                kind: HirStmtKind::LocalInit {
                    id: local_id,
                    value: HirExpr {
                        kind: HirExprKind::FunctionRef(fid),
                        span: s.span,
                    },
                },
                span: s.span,
            });
            continue;
        }
        stmts.push(ctx.lower_stmt(s)?);
    }
    // ctx.functions accumulates anonymous functions registered during
    // lowering of `local f = function() ... end` (Phase 2.5b, ADR 0017).
    let mut chunk_locals = ctx.locals;
    let mut all_functions = ctx.functions;
    // Phase 2.5c-full Commit 3 (ADR 0083): post-pass that flips
    // `LocalInfo::is_captured = true` on every outer local that is
    // referenced as `outer_local_id` of any closure's `UpvalueInfo`.
    // The flag drives codegen's choice between a stack alloca slot
    // and a heap upvalue box (so writes through the box are visible
    // across closures sharing the same outer local). Resolved via
    // `HirFunction::parent_scope` since `LocalId` is scope-relative.
    let upvalue_records: Vec<(ParentScope, Vec<LocalId>)> = all_functions
        .iter()
        .map(|f| {
            (
                f.parent_scope,
                f.upvalues.iter().map(|uv| uv.outer_local_id).collect(),
            )
        })
        .collect();
    for (parent_scope, outer_ids) in upvalue_records {
        for outer_id in outer_ids {
            match parent_scope {
                ParentScope::Chunk => {
                    chunk_locals[outer_id.0].is_captured = true;
                }
                ParentScope::Function(fid) => {
                    all_functions[fid.0].locals[outer_id.0].is_captured = true;
                    // The params slice is a prefix of locals, so mirror
                    // the flag onto the matching params entry to keep
                    // the two views consistent.
                    if outer_id.0 < all_functions[fid.0].params.len() {
                        all_functions[fid.0].params[outer_id.0].is_captured = true;
                    }
                }
            }
        }
    }
    // Phase 2.5c-full Commit 3b prep fix (ADR 0083): post-pass to
    // reject mutual capturing recursion. After every body is
    // lowered, walk all `Callee::User { fid, holding_local: None }`
    // call sites: if the target is capturing and the call isn't
    // self-recursion (target.fid != enclosing fn), reject. The
    // check has to be a post-pass because `target.upvalues` is
    // populated only after `lower_into_function` for that target
    // returns, which can be later in source order than the call
    // site.
    for caller_fid in 0..all_functions.len() {
        // Take the body out of `all_functions[caller_fid]` so the
        // loop iterates an owned `Vec<HirStmt>` while still being
        // able to immutably borrow the rest of `all_functions` for
        // diagnostic name lookup. Restore it after the check.
        let body = std::mem::take(&mut all_functions[caller_fid].body);
        check_mutual_capturing_recursion_in_stmts(&body, Some(FuncId(caller_fid)), &all_functions)?;
        all_functions[caller_fid].body = body;
    }
    check_mutual_capturing_recursion_in_stmts(&stmts, None, &all_functions)?;
    Ok(HirChunk {
        locals: chunk_locals,
        stmts,
        functions: all_functions,
    })
}

/// Phase 2.5c-full Commit 3b prep fix (ADR 0083): walk a stmt slice
/// for `Callee::User { holding_local: None }` and reject mutual
/// capturing recursion. Used by the `lower()` post-pass.
fn check_mutual_capturing_recursion_in_stmts(
    stmts: &[HirStmt],
    enclosing_fid: Option<FuncId>,
    functions: &[HirFunction],
) -> Result<(), HirError> {
    for stmt in stmts {
        check_mutual_in_stmt(stmt, enclosing_fid, functions)?;
    }
    Ok(())
}

fn check_mutual_in_stmt(
    stmt: &HirStmt,
    enclosing_fid: Option<FuncId>,
    functions: &[HirFunction],
) -> Result<(), HirError> {
    match &stmt.kind {
        HirStmtKind::LocalInit { value, .. } | HirStmtKind::Assign { value, .. } => {
            check_mutual_in_expr(value, enclosing_fid, functions, stmt.span.start)
        }
        HirStmtKind::ExprStmt(e) => {
            check_mutual_in_expr(e, enclosing_fid, functions, stmt.span.start)
        }
        HirStmtKind::Block { stmts } => {
            check_mutual_capturing_recursion_in_stmts(stmts, enclosing_fid, functions)
        }
        HirStmtKind::If {
            cond,
            then_body,
            elifs,
            else_body,
        } => {
            check_mutual_in_expr(cond, enclosing_fid, functions, stmt.span.start)?;
            check_mutual_capturing_recursion_in_stmts(then_body, enclosing_fid, functions)?;
            for (c, b) in elifs {
                check_mutual_in_expr(c, enclosing_fid, functions, stmt.span.start)?;
                check_mutual_capturing_recursion_in_stmts(b, enclosing_fid, functions)?;
            }
            if let Some(else_body) = else_body {
                check_mutual_capturing_recursion_in_stmts(else_body, enclosing_fid, functions)?;
            }
            Ok(())
        }
        HirStmtKind::While { cond, body, .. } => {
            check_mutual_in_expr(cond, enclosing_fid, functions, stmt.span.start)?;
            check_mutual_capturing_recursion_in_stmts(body, enclosing_fid, functions)
        }
        HirStmtKind::Repeat { body, cond, .. } => {
            check_mutual_capturing_recursion_in_stmts(body, enclosing_fid, functions)?;
            check_mutual_in_expr(cond, enclosing_fid, functions, stmt.span.start)
        }
        HirStmtKind::ForNumeric {
            start,
            stop,
            step,
            body,
            ..
        } => {
            check_mutual_in_expr(start, enclosing_fid, functions, stmt.span.start)?;
            check_mutual_in_expr(stop, enclosing_fid, functions, stmt.span.start)?;
            check_mutual_in_expr(step, enclosing_fid, functions, stmt.span.start)?;
            check_mutual_capturing_recursion_in_stmts(body, enclosing_fid, functions)
        }
        HirStmtKind::MultiAssignFromCall { callee, args, .. } => {
            check_mutual_in_callee(callee, enclosing_fid, functions, stmt.span.start)?;
            for a in args {
                check_mutual_in_expr(a, enclosing_fid, functions, stmt.span.start)?;
            }
            Ok(())
        }
        HirStmtKind::IndexAssign { target, key, value } => {
            check_mutual_in_expr(target, enclosing_fid, functions, stmt.span.start)?;
            check_mutual_in_expr(key, enclosing_fid, functions, stmt.span.start)?;
            check_mutual_in_expr(value, enclosing_fid, functions, stmt.span.start)
        }
    }
}

fn check_mutual_in_expr(
    expr: &HirExpr,
    enclosing_fid: Option<FuncId>,
    functions: &[HirFunction],
    fallback_offset: usize,
) -> Result<(), HirError> {
    match &expr.kind {
        HirExprKind::Call { callee, args } => {
            check_mutual_in_callee(callee, enclosing_fid, functions, fallback_offset)?;
            for a in args {
                check_mutual_in_expr(a, enclosing_fid, functions, fallback_offset)?;
            }
            Ok(())
        }
        HirExprKind::BinOp { lhs, rhs, .. } => {
            check_mutual_in_expr(lhs, enclosing_fid, functions, fallback_offset)?;
            check_mutual_in_expr(rhs, enclosing_fid, functions, fallback_offset)
        }
        HirExprKind::UnaryOp { operand, .. } => {
            check_mutual_in_expr(operand, enclosing_fid, functions, fallback_offset)
        }
        HirExprKind::Index { target, key } => {
            check_mutual_in_expr(target, enclosing_fid, functions, fallback_offset)?;
            check_mutual_in_expr(key, enclosing_fid, functions, fallback_offset)
        }
        HirExprKind::IndexTagged { target, key } => {
            check_mutual_in_expr(target, enclosing_fid, functions, fallback_offset)?;
            check_mutual_in_expr(key, enclosing_fid, functions, fallback_offset)
        }
        HirExprKind::IsNil(operand) | HirExprKind::ArithStringCoerce(operand) => {
            check_mutual_in_expr(operand, enclosing_fid, functions, fallback_offset)
        }
        HirExprKind::Table(elems) => {
            for elem in elems {
                check_mutual_in_expr(elem, enclosing_fid, functions, fallback_offset)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn check_mutual_in_callee(
    callee: &Callee,
    enclosing_fid: Option<FuncId>,
    functions: &[HirFunction],
    fallback_offset: usize,
) -> Result<(), HirError> {
    if let Callee::User {
        fid,
        holding_local: None,
    } = callee
    {
        let target = &functions[fid.0];
        let target_capturing = !target.upvalues.is_empty();
        let is_self_call = enclosing_fid == Some(*fid);
        if target_capturing && !is_self_call {
            // Phase 2.5c-full Commit 3b prep fix v2 (Codex P2):
            // surface the Lua-level name so diagnostics point at
            // the function the user wrote. Anonymous functions
            // (no source name) fall back to the mangled `user_anon_N`
            // tag.
            let local_name = if target.name.is_empty() {
                target.mangled_name.clone()
            } else {
                target.name.clone()
            };
            return Err(HirError::MutualCapturingRecursion {
                local_name,
                offset: fallback_offset,
            });
        }
    }
    Ok(())
}

struct LowerCtx {
    locals: Vec<LocalInfo>,
    /// Phase 2.5c-min (ADR 0037): captured locals from the enclosing
    /// scope at the moment this LowerCtx was created via
    /// `for_function`. `name → (outer LocalId, kind)`. An ident
    /// lookup that misses `scopes` and `function_names` consults
    /// this map; a hit registers an upvalue and declares a fresh
    /// local in the current ctx (param-style binding).
    outer_visible: HashMap<String, (LocalId, ValueKind)>,
    /// Upvalues recorded during this body's lowering. Copied to
    /// `HirFunction.upvalues` after the body finishes lowering.
    upvalues: Vec<UpvalueInfo>,
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
    /// Phase 2.6+-method-arg-refine (ADR 0093): MethodDef namespace
    /// inherited from the top-level pass. `lower_method_def` reads
    /// `(receiver, method) -> FuncId` to find its pre-allocated slot
    /// instead of pushing a fresh `HirFunction`. Same last-wins
    /// shadowing semantics as `function_names`.
    method_funcs: HashMap<(String, String), FuncId>,
    /// Mirror of [`HirChunk::functions`] — needed by `infer_kind` for
    /// user-call return-type lookup. Phase 2.5a clones it into each
    /// `LowerCtx`; the cost is negligible at this scale.
    functions: Vec<HirFunction>,
    /// `Some((returned_id, ret_value_ids))` while lowering inside a
    /// function body; `None` at top level. Phase 2.5d (ADR 0021):
    /// `ret_value_ids` is a `Vec` of `_ret_value_N` slot ids — one
    /// per return position. Empty for void functions, length 1 for
    /// the historical single-return case, length ≥2 for multi-return.
    in_function: Option<(LocalId, Vec<LocalId>)>,
    /// Return kinds discovered while lowering the current function
    /// body. `None` until the first return is seen; `Some(vec![])`
    /// for a void return; `Some(vec![k])` for single; `Some(vec![k1,
    /// k2, ...])` for multi-return. Subsequent returns must agree on
    /// arity and per-position kind.
    in_function_ret_kinds: Option<Vec<ValueKind>>,
    /// Phase 2.5c-full Commit 3 (ADR 0083): the FuncId of the
    /// function whose body this ctx is lowering, or `None` at
    /// chunk level. Used by [`Self::current_parent_scope`] to
    /// stamp `parent_scope` on every new HirFunction this ctx
    /// registers, so the post-pass can resolve each upvalue's
    /// `outer_local_id` to the right `locals` table.
    containing_fn: Option<FuncId>,
    /// Phase 2.6+-callee-norm (ADR 0091 v2): HIR pre-stmt hoisting
    /// buffer. `lower_call`'s Index-callee path pre-binds the Index
    /// result to a synthetic `__callee_<N>` local; the LocalInit
    /// stmt pushes here, and `lower_stmt` drains the buffer at every
    /// stmt boundary, wrapping the inner stmt in a
    /// `Block { hoists..., inner }` when any hoists accumulated.
    /// Snapshot/restore at stmt entry keeps each stmt's hoists local
    /// to its own boundary; nested `lower_stmt` calls drain at THEIR
    /// own boundaries, not the outer caller's. The mechanism is
    /// general-purpose — future expression-level desugars (method
    /// colon sugar, let-binding rewrites, `__call` metamethod) reuse
    /// it.
    pending_pre_stmts: Vec<HirStmt>,
    /// Phase 2.6+-callee-norm (ADR 0091 v2): monotonic counter for
    /// synthesized `__callee_<N>` local names. Prevents collision
    /// when multiple Index-callee Calls land in the same surrounding
    /// scope.
    callee_seq: usize,
}

impl LowerCtx {
    fn new(
        function_names: HashMap<String, FuncId>,
        method_funcs: HashMap<(String, String), FuncId>,
        functions: Vec<HirFunction>,
    ) -> Self {
        Self {
            locals: Vec::new(),
            outer_visible: HashMap::new(),
            upvalues: Vec::new(),
            scopes: vec![HashMap::new()],
            readonly_locals: HashSet::new(),
            loop_break_targets: Vec::new(),
            function_names,
            method_funcs,
            functions,
            in_function: None,
            in_function_ret_kinds: None,
            containing_fn: None,
            pending_pre_stmts: Vec::new(),
            callee_seq: 0,
        }
    }

    /// Phase 2.5c-full Commit 3 (ADR 0083): the lexical parent
    /// scope of any new `HirFunction` registered via this ctx.
    fn current_parent_scope(&self) -> ParentScope {
        match self.containing_fn {
            None => ParentScope::Chunk,
            Some(fid) => ParentScope::Function(fid),
        }
    }

    /// Build a `LowerCtx` for lowering a function body in isolation
    /// (separate locals, scopes, loop break stack). The function's
    /// parameters are pre-declared as the first locals.
    ///
    /// Phase 2.5b.2 (ADR 0018): the body AST is pre-scanned with
    /// [`infer_param_kinds`] so any parameter used as a callee gets
    /// `ValueKind::Function(arity)`. Phase 2.5e (ADR 0020): the
    /// caller may also pass `external_kinds` from a chunk-level
    /// pre-scan of call sites, supplying Bool/Nil/Number kinds. The
    /// body-pre-scan wins for Function (body-callsite is decisive);
    /// `external_kinds` wins otherwise.
    #[allow(clippy::too_many_arguments)]
    fn for_function(
        function_names: &HashMap<String, FuncId>,
        method_funcs: &HashMap<(String, String), FuncId>,
        functions: &[HirFunction],
        params: &[String],
        body: &[Stmt],
        external_kinds: &[ValueKind],
        outer_visible: HashMap<String, (LocalId, ValueKind)>,
        containing_fn: FuncId,
    ) -> Self {
        let mut ctx = Self::new(
            function_names.clone(),
            method_funcs.clone(),
            functions.to_vec(),
        );
        ctx.outer_visible = outer_visible;
        ctx.containing_fn = Some(containing_fn);
        let body_kinds = infer_param_kinds(body, params);
        for (i, p) in params.iter().enumerate() {
            let kind = match body_kinds[i] {
                ValueKind::Function(_) => body_kinds[i],
                _ => external_kinds[i],
            };
            ctx.declare_local(p.clone(), kind);
        }
        ctx
    }

    /// Build a snapshot of names currently visible in this ctx for
    /// upvalue lookup by a nested function/closure (Phase 2.5c-min,
    /// ADR 0037). Walks the scope stack so the latest binding for
    /// each name wins, matching the existing `resolve` semantics.
    fn outer_visible_snapshot(&self) -> HashMap<String, (LocalId, ValueKind)> {
        let mut out: HashMap<String, (LocalId, ValueKind)> = HashMap::new();
        for frame in &self.scopes {
            for (name, id) in frame {
                out.insert(name.clone(), (*id, self.locals[id.0].kind));
            }
        }
        out
    }

    /// Phase 2.5c-min (ADR 0037): try to capture `name` as an
    /// upvalue from the enclosing scope. On a hit, registers a new
    /// upvalue (deduplicated), declares a fresh local in the
    /// current ctx that shadows future lookups, and returns its
    /// `LocalId`. On a miss, returns `Ok(None)` — the caller
    /// continues to the `function_names` / `UndefinedName` cascade.
    /// Phase 2.5c-min restricts captures to `ValueKind::Number`;
    /// other kinds surface a `TypeMismatch`.
    fn lookup_or_capture_upvalue(
        &mut self,
        name: &str,
        span: Span,
    ) -> Result<Option<LocalId>, HirError> {
        let Some(&(outer_id, outer_kind)) = self.outer_visible.get(name) else {
            return Ok(None);
        };
        // Phase 2.5c.2 (ADR 0043): allow Number/Bool/Nil/String
        // upvalues. Function-kind captures still error because
        // codegen has no path to wire a function value through the
        // alloca-backed inner slot — the existing Function-kind
        // local treatment relies on `LocalInfo.func_id`, which the
        // capture site doesn't reproduce.
        if matches!(outer_kind, ValueKind::Function(_)) {
            return Err(HirError::TypeMismatch {
                op: "upvalue capture".to_owned(),
                lhs_kind: "number/bool/nil/string".to_owned(),
                rhs_kind: outer_kind.name().to_owned(),
                offset: span.start,
            });
        }
        // De-dup: repeated references to the same outer name share
        // a single captured local.
        if let Some(existing) = self.upvalues.iter().find(|u| u.name == name) {
            return Ok(Some(existing.inner_local_id));
        }
        // First capture: declare a fresh local in the current ctx
        // so subsequent uses resolve via `scopes`, then record the
        // upvalue with its inner LocalId so codegen can wire the
        // block argument into the right slot.
        let inner_local_id = self.declare_local(name.to_owned(), outer_kind);
        self.upvalues.push(UpvalueInfo {
            name: name.to_owned(),
            kind: outer_kind,
            outer_local_id: outer_id,
            inner_local_id,
        });
        Ok(Some(inner_local_id))
    }

    /// Lower a function body. Allocates the synthetic `_returned` and
    /// `_ret_value_N` slots, sets `in_function`, and applies the same
    /// body-guard wrap pattern used by `break` (ADR 0015) so that
    /// post-`return` statements are skipped at runtime. Phase 2.5d
    /// (ADR 0021): N comes from a pre-scan of the body's max return
    /// arity, so multi-return bodies allocate one slot per position.
    fn lower_function_body(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, HirError> {
        let returned_id = self.declare_local("_returned".to_owned(), ValueKind::Bool);
        let ret_arity = ast_max_return_arity(stmts);
        let mut ret_value_ids: Vec<LocalId> = Vec::with_capacity(ret_arity);
        for i in 0..ret_arity {
            let id = self.declare_local(format!("_ret_value_{i}"), ValueKind::Number);
            ret_value_ids.push(id);
        }
        self.in_function = Some((returned_id, ret_value_ids.clone()));

        // Phase 2.5f (ADR 0036): pre-register every body-level
        // `local function NAME ...` so sibling forward references
        // (`g` calling `h` declared later in the same body) work.
        // Mirrors the pass-1 / pass-2 split that `lower()` already
        // does for top-level FunctionDefs.
        let parent_scope = self.current_parent_scope();
        for s in stmts {
            if let StmtKind::FunctionDef { name, params, .. } = &s.kind {
                register_function_signature(
                    name,
                    params,
                    &mut self.function_names,
                    &mut self.functions,
                    parent_scope,
                );
            }
        }
        // Phase 2.5c-full Commit 3b prep fix (ADR 0083): pass 1.5 —
        // declare a synthetic body-local for each `local function f`
        // declared in this body. Aligned with the chunk-level
        // synthetic local in `lower()`, this makes
        // `self.resolve(name)` succeed for all forward / nested
        // call sites and lets the upcoming
        // `emit_call_user_with_cell` cutover load the closure cell
        // ptr from the local's slot.
        for s in stmts {
            if let StmtKind::FunctionDef { name, params, .. } = &s.kind {
                let arity = params.len();
                let fid = self.function_names[name];
                self.declare_local_with_func_id(
                    name.clone(),
                    ValueKind::Function(arity),
                    Some(fid),
                );
            }
        }

        let span = stmts.first().map(|s| s.span).unwrap_or(Span::new(0, 0));
        self.loop_break_targets.push(Some(returned_id));
        let lowered = self.lower_stmts(stmts)?;
        self.loop_break_targets.pop();

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
        // Kind-appropriate default for each `_ret_value_N` slot —
        // Function-kind has no sensible default and is skipped, the
        // body-guard pattern guarantees a real value before load.
        for &slot_id in &ret_value_ids {
            let init_value = match self.locals[slot_id.0].kind {
                ValueKind::Number => Some(HirExprKind::Number(0.0)),
                ValueKind::Bool => Some(HirExprKind::Bool(false)),
                ValueKind::Nil => Some(HirExprKind::Nil),
                // Function / String / Table: no sensible default
                // (ptr alloca, body-guard guarantees a real value
                // before load fires).
                ValueKind::Function(_) | ValueKind::String | ValueKind::Table => None,
                // Phase 2.6c-tag-locals-fn (ADR 0074): a
                // `_ret_value_N` slot widens to TaggedValue when
                // the function has heterogeneous return paths.
                // The default is Nil-tagged so an early implicit
                // exit (no `return` reached) yields nil — matches
                // Lua's "missing return = nil" rule.
                ValueKind::TaggedValue => Some(HirExprKind::Nil),
            };
            if let Some(kind) = init_value {
                out.push(HirStmt {
                    kind: HirStmtKind::LocalInit {
                        id: slot_id,
                        value: HirExpr { kind, span },
                    },
                    span,
                });
            }
        }

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
            is_captured: false,
        });
        self.scopes
            .last_mut()
            .expect("scope stack is never empty")
            .insert(name, id);
        id
    }

    /// Phase 2.6+-callee-norm (ADR 0091 v2): drain wrapper around the
    /// match-arms body (renamed [`Self::lower_stmt_match_arms`]).
    /// Snapshots the current `pending_pre_stmts`, runs the inner
    /// lowering (which may push hoisted LocalInit stmts during
    /// `lower_call`'s Index-callee desugar), then drains. When hoists
    /// accumulated, wraps the inner stmt in a `Block { hoists...,
    /// inner }` so the synthetic `__callee_<N>` locals are evaluated
    /// once in the right surrounding scope. The snapshot/restore
    /// dance keeps each stmt's hoists local to its own boundary —
    /// recursive `lower_stmt` calls (e.g. for If-body / While-body /
    /// inner Block stmts) drain at their own boundaries, not at the
    /// outer caller's.
    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<HirStmt, HirError> {
        let outer_pre = std::mem::take(&mut self.pending_pre_stmts);
        let inner_result = self.lower_stmt_match_arms(stmt);
        let mut my_pre = std::mem::replace(&mut self.pending_pre_stmts, outer_pre);
        let inner = inner_result?;
        if my_pre.is_empty() {
            return Ok(inner);
        }
        my_pre.push(inner);
        Ok(HirStmt {
            kind: HirStmtKind::Block { stmts: my_pre },
            span: stmt.span,
        })
    }

    fn lower_stmt_match_arms(&mut self, stmt: &Stmt) -> Result<HirStmt, HirError> {
        match &stmt.kind {
            StmtKind::Local { name, value } => {
                let value = self.lower_expr(value)?;
                // Phase 2.6c-tag-locals (ADR 0063): a plain table
                // read in LocalInit position widens the local to
                // TaggedValue. Rewriting the value into
                // `IndexTagged` lets `infer_kind` pick the wider
                // kind and lets codegen take the non-trapping
                // path inside `emit_stmt(LocalInit)`.
                let value = widen_index_for_local_init(value);
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
                let value = self.lower_expr(value)?;
                self.lower_assign_target(name, value, stmt.span)
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
                Ok(wrap_with_break_init(while_stmt, break_id, stmt.span))
            }
            // Phase 2.4b (ADR 0035): `repeat body until cond`. Body
            // and cond share a lexical scope; lower them as a
            // single scoped unit so until-cond can resolve names
            // declared in the body.
            StmtKind::Repeat { body, cond } => {
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
                let body_hir = self.lower_stmts_maybe_guarded(body)?;
                let cond_hir = self.lower_expr(cond)?;
                self.scopes.pop();
                self.loop_break_targets.pop();
                let repeat_stmt = HirStmt {
                    kind: HirStmtKind::Repeat {
                        body: body_hir,
                        cond: cond_hir,
                        break_id,
                    },
                    span: stmt.span,
                };
                Ok(wrap_with_break_init(repeat_stmt, break_id, stmt.span))
            }
            // Phase 2.5f (ADR 0036): nested `local function NAME ...`.
            // Top-level FunctionDef is hoisted in `lower()` and
            // never reaches this arm; what we lower here is a
            // FunctionDef found *inside* another function's body.
            // `lower_function_body`'s pass-1 has already registered
            // the name in `self.function_names` and a placeholder
            // `HirFunction` in `self.functions`. We delegate the
            // body lowering / locals copy / anon-function hoist to
            // the same `lower_into_function` helper used at top-
            // level, and emit an empty Block as the definition's
            // runtime no-op (the function code is read from
            // `chunk.functions`, not the enclosing body).
            StmtKind::FunctionDef { name, params, body } => {
                let &fid = self.function_names.get(name).expect(
                    "lower_function_body's pass 1 always registers nested \
                     FunctionDef names",
                );
                // Phase 2.5c-min: the nested local function can
                // capture the enclosing function's currently-visible
                // locals.
                let outer_visible = self.outer_visible_snapshot();
                lower_into_function(
                    fid,
                    params,
                    body,
                    &self.function_names,
                    &self.method_funcs,
                    &mut self.functions,
                    outer_visible,
                )?;
                // Phase 2.5c-full Commit 3b prep fix (ADR 0083): the
                // synthetic body-local for `name` was declared in
                // `lower_function_body`'s pass 1.5; emit a
                // LocalInit at the FunctionDef's source position so
                // the closure cell ptr lands in its slot.
                let local_id = self
                    .resolve(name)
                    .expect("synthetic local declared in lower_function_body's pass 1.5");
                Ok(HirStmt {
                    kind: HirStmtKind::LocalInit {
                        id: local_id,
                        value: HirExpr {
                            kind: HirExprKind::FunctionRef(fid),
                            span: stmt.span,
                        },
                    },
                    span: stmt.span,
                })
            }
            StmtKind::Return { value } => {
                let values: Vec<HirExpr> = match value {
                    Some(e) => vec![self.lower_expr(e)?],
                    None => Vec::new(),
                };
                self.lower_return_with_values(values, stmt.span)
            }
            StmtKind::ReturnMulti { values } => {
                let lowered: Vec<HirExpr> = values
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                self.lower_return_with_values(lowered, stmt.span)
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
                Ok(wrap_with_break_init(for_stmt, break_id, stmt.span))
            }
            // Phase 2.8e-iter-ipairs (ADR 0078): desugar
            // `for IDX, VAL in ipairs(TABLE) do BODY end`
            // to existing primitives so codegen needs no new arm:
            //
            //   do
            //     local __t = TABLE
            //     local IDX = 1
            //     local _broken_N = false
            //     while true do
            //       local VAL = __t[IDX]   -- IndexTagged → TaggedValue
            //       if VAL == nil then _broken_N = true end
            //       BODY
            //       IDX = IDX + 1
            //     end
            //   end
            //
            // The While codegen (ADR 0015) AND-extends `cond` with
            // `not load(break_slot)`, so a `while true` paired with
            // a break flag terminates after the nil check fires.
            StmtKind::ForIpairs {
                idx_name,
                val_name,
                table,
                body,
            } => {
                let span = stmt.span;
                // Lower the table expression in the OUTER scope.
                let table_hir = self.lower_expr(table)?;
                let table_kind = infer_kind(&table_hir, &self.locals, &self.functions);
                if table_kind != ValueKind::Table {
                    return Err(HirError::TypeMismatch {
                        op: "for-in-ipairs".to_owned(),
                        lhs_kind: "table".to_owned(),
                        rhs_kind: table_kind.name().to_owned(),
                        offset: table.span.start,
                    });
                }
                // Synthetic locals live in a fresh inner scope so
                // user-shadowed names don't collide with __t /
                // _broken.
                self.scopes.push(HashMap::new());
                let table_local = self.declare_local(
                    format!("__lumelir_iter_t_{}", self.locals.len()),
                    ValueKind::Table,
                );
                let idx_id = self.declare_local(idx_name.clone(), ValueKind::Number);
                let val_id = self.declare_local(val_name.clone(), ValueKind::TaggedValue);
                let break_id =
                    self.declare_local(format!("_broken_{}", self.locals.len()), ValueKind::Bool);

                // User body lowers in the same scope so it sees IDX
                // and VAL. break inside the body targets break_id.
                // `lower_scoped_body_no_push` wraps each user stmt
                // in `if not _broken then STMT end` (ADR 0015) so a
                // mid-body `break` skips the remainder of the body.
                self.loop_break_targets.push(Some(break_id));
                let body_result = self.lower_scoped_body_no_push(body);
                self.loop_break_targets.pop();
                self.scopes.pop();
                let user_body_hir = body_result?;

                // Build synthetic HIR statements.
                let local_init = |id, value| HirStmt {
                    kind: HirStmtKind::LocalInit { id, value },
                    span,
                };
                let table_init = local_init(table_local, table_hir);
                let idx_init = local_init(
                    idx_id,
                    HirExpr {
                        kind: HirExprKind::Number(1.0),
                        span,
                    },
                );
                let break_init = local_init(
                    break_id,
                    HirExpr {
                        kind: HirExprKind::Bool(false),
                        span,
                    },
                );
                let table_local_expr = HirExpr {
                    kind: HirExprKind::Local(table_local),
                    span,
                };
                let idx_local_expr = HirExpr {
                    kind: HirExprKind::Local(idx_id),
                    span,
                };
                let val_local_expr = HirExpr {
                    kind: HirExprKind::Local(val_id),
                    span,
                };
                // local VAL = __t[IDX] (IndexTagged widens VAL to
                // TaggedValue per ADR 0063).
                let val_init = local_init(
                    val_id,
                    HirExpr {
                        kind: HirExprKind::IndexTagged {
                            target: Box::new(table_local_expr.clone()),
                            key: Box::new(idx_local_expr.clone()),
                        },
                        span,
                    },
                );
                // if IsNil(VAL) then _broken := true else BODY; IDX += 1 end
                //
                // BODY runs only when VAL is non-nil, matching
                // Lua spec for ipairs (stop at first nil, don't
                // expose the nil to user code).
                let break_assign = HirStmt {
                    kind: HirStmtKind::Assign {
                        id: break_id,
                        value: HirExpr {
                            kind: HirExprKind::Bool(true),
                            span,
                        },
                    },
                    span,
                };
                let idx_inc = HirStmt {
                    kind: HirStmtKind::Assign {
                        id: idx_id,
                        value: HirExpr {
                            kind: HirExprKind::BinOp {
                                op: BinOp::Add,
                                lhs: Box::new(idx_local_expr.clone()),
                                rhs: Box::new(HirExpr {
                                    kind: HirExprKind::Number(1.0),
                                    span,
                                }),
                            },
                            span,
                        },
                    },
                    span,
                };
                let mut else_body: Vec<HirStmt> = Vec::with_capacity(user_body_hir.len() + 1);
                else_body.extend(user_body_hir);
                else_body.push(idx_inc);
                let nil_check = HirStmt {
                    kind: HirStmtKind::If {
                        cond: HirExpr {
                            kind: HirExprKind::IsNil(Box::new(val_local_expr.clone())),
                            span,
                        },
                        then_body: vec![break_assign],
                        elifs: Vec::new(),
                        else_body: Some(else_body),
                    },
                    span,
                };
                let while_body: Vec<HirStmt> = vec![val_init, nil_check];
                let while_loop = HirStmt {
                    kind: HirStmtKind::While {
                        cond: HirExpr {
                            kind: HirExprKind::Bool(true),
                            span,
                        },
                        body: while_body,
                        break_id: Some(break_id),
                    },
                    span,
                };
                Ok(HirStmt {
                    kind: HirStmtKind::Block {
                        stmts: vec![table_init, idx_init, break_init, while_loop],
                    },
                    span,
                })
            }
            // Phase 2.8e-iter-next (ADR 0081):
            // `for K, V in pairs(TABLE) do BODY end` HIR-desugars to
            // a `MultiAssignFromCall` over `Builtin::Next`, replacing
            // the opaque `HirStmtKind::ForPairs` shape that ADR 0080
            // shipped. The synthetic body is:
            //
            //   do
            //     local __t = TABLE
            //     local __ctl = nil               -- TaggedValue
            //     local _broken_N = false
            //     while true do
            //       local k, v = next(__t, __ctl) -- Builtin::Next
            //       if IsNil(k) then _broken_N = true
            //       else BODY ; __ctl = k end
            //     end
            //   end
            //
            // Each user statement in BODY is wrapped with `if not
            // _broken_N then STMT end` by `lower_scoped_body_no_push`
            // (ADR 0015 break pattern), so a mid-body `break` skips
            // the rest of the same iteration; the next `while`
            // re-check terminates the loop because the `break` flag
            // is AND-extended into `cond` by the While codegen.
            StmtKind::ForPairs {
                key_name,
                val_name,
                table,
                body,
            } => {
                let span = stmt.span;
                let table_hir = self.lower_expr(table)?;
                let table_kind = infer_kind(&table_hir, &self.locals, &self.functions);
                if table_kind != ValueKind::Table {
                    return Err(HirError::TypeMismatch {
                        op: "for-in-pairs".to_owned(),
                        lhs_kind: "table".to_owned(),
                        rhs_kind: table_kind.name().to_owned(),
                        offset: table.span.start,
                    });
                }
                self.scopes.push(HashMap::new());
                let table_local = self.declare_local(
                    format!("__lumelir_iter_t_{}", self.locals.len()),
                    ValueKind::Table,
                );
                let ctl_id = self.declare_local(
                    format!("__lumelir_iter_ctl_{}", self.locals.len()),
                    ValueKind::TaggedValue,
                );
                let key_id = self.declare_local(key_name.clone(), ValueKind::TaggedValue);
                let val_id = self.declare_local(val_name.clone(), ValueKind::TaggedValue);
                let break_id =
                    self.declare_local(format!("_broken_{}", self.locals.len()), ValueKind::Bool);

                self.loop_break_targets.push(Some(break_id));
                let body_result = self.lower_scoped_body_no_push(body);
                self.loop_break_targets.pop();
                self.scopes.pop();
                let user_body_hir = body_result?;

                let local_init = |id, value| HirStmt {
                    kind: HirStmtKind::LocalInit { id, value },
                    span,
                };
                let table_init = local_init(table_local, table_hir);
                let ctl_init = local_init(
                    ctl_id,
                    HirExpr {
                        kind: HirExprKind::Nil,
                        span,
                    },
                );
                let break_init = local_init(
                    break_id,
                    HirExpr {
                        kind: HirExprKind::Bool(false),
                        span,
                    },
                );
                let table_local_expr = HirExpr {
                    kind: HirExprKind::Local(table_local),
                    span,
                };
                let ctl_local_expr = HirExpr {
                    kind: HirExprKind::Local(ctl_id),
                    span,
                };
                let key_local_expr = HirExpr {
                    kind: HirExprKind::Local(key_id),
                    span,
                };
                // `local k, v = next(__t, __ctl)` — every iteration
                // reassigns both slots through the
                // MultiAssignFromCall path (no LocalInit, since the
                // slots were already declared above and we are inside
                // the synthetic while body).
                let next_step = HirStmt {
                    kind: HirStmtKind::MultiAssignFromCall {
                        dst_ids: vec![key_id, val_id],
                        callee: Callee::Builtin(Builtin::Next),
                        args: vec![table_local_expr, ctl_local_expr],
                    },
                    span,
                };
                // `_broken_N = true` (set when next returns nil key).
                let break_assign = HirStmt {
                    kind: HirStmtKind::Assign {
                        id: break_id,
                        value: HirExpr {
                            kind: HirExprKind::Bool(true),
                            span,
                        },
                    },
                    span,
                };
                // `__ctl = k` (advance the iterator state for the
                // next call).
                let ctl_advance = HirStmt {
                    kind: HirStmtKind::Assign {
                        id: ctl_id,
                        value: key_local_expr.clone(),
                    },
                    span,
                };
                let mut else_body: Vec<HirStmt> = Vec::with_capacity(user_body_hir.len() + 1);
                else_body.extend(user_body_hir);
                else_body.push(ctl_advance);
                let nil_check = HirStmt {
                    kind: HirStmtKind::If {
                        cond: HirExpr {
                            kind: HirExprKind::IsNil(Box::new(key_local_expr)),
                            span,
                        },
                        then_body: vec![break_assign],
                        elifs: Vec::new(),
                        else_body: Some(else_body),
                    },
                    span,
                };
                let while_body: Vec<HirStmt> = vec![next_step, nil_check];
                let while_loop = HirStmt {
                    kind: HirStmtKind::While {
                        cond: HirExpr {
                            kind: HirExprKind::Bool(true),
                            span,
                        },
                        body: while_body,
                        break_id: Some(break_id),
                    },
                    span,
                };
                Ok(HirStmt {
                    kind: HirStmtKind::Block {
                        stmts: vec![table_init, ctl_init, break_init, while_loop],
                    },
                    span,
                })
            }
            // Phase 2.8e-iter-generic (ADR 0085): `for k, v in ITER,
            // STATE, CTL do BODY end`. Synthetic block desugar that
            // mirrors ForPairs (ADR 0081) but with iter / state / ctl
            // pinning + per-iteration `__iter(__state, __ctl)` call
            // dispatched to the right `Callee` based on iter's HIR
            // shape.
            //
            // Phase 1 scope: iter must resolve to `Builtin::Next`,
            // a top-level user function, a Function-kind local, or
            // a TaggedValue local with at least one compatible
            // non-closure user function. Closure-as-iter is rejected
            // until ADR 0083 lands.
            StmtKind::ForGeneric {
                names,
                iter,
                state,
                ctl,
                body,
            } => {
                let span = stmt.span;
                if names.len() != 2 {
                    return Err(HirError::ArityMismatch {
                        builtin: "for-in-generic".to_owned(),
                        expected: 2,
                        actual: names.len(),
                        offset: span.start,
                    });
                }
                let key_name = names[0].clone();
                let val_name = names[1].clone();

                // Lower state and ctl in the OUTER scope (these are
                // ordinary expressions). Iter is special — see below.
                let state_hir = self.lower_expr(state)?;
                let ctl_hir = self.lower_expr(ctl)?;
                let state_kind = infer_kind(&state_hir, &self.locals, &self.functions);
                let ctl_kind = infer_kind(&ctl_hir, &self.locals, &self.functions);

                // Resolve the iter expression to a callee shape.
                // Special case `next` ident → `Builtin::Next`.
                // Otherwise lower normally and dispatch on the
                // resulting kind.
                let iter_is_next_builtin = matches!(
                    &iter.kind,
                    ExprKind::Ident(n) if n == "next"
                );
                let iter_hir_opt = if iter_is_next_builtin {
                    None
                } else {
                    Some(self.lower_expr(iter)?)
                };
                let iter_kind = iter_hir_opt
                    .as_ref()
                    .map(|h| infer_kind(h, &self.locals, &self.functions));

                // Determine the iter's effective ret_kinds — drives
                // both the dst kind for key/val and the resolved
                // callee. Phase 1 scope: must have exactly 2 return
                // positions, both `TaggedValue` so a `nil` first
                // result can terminate the loop.
                let iter_ret_kinds: Vec<ValueKind> = if iter_is_next_builtin {
                    vec![ValueKind::TaggedValue, ValueKind::TaggedValue]
                } else {
                    let kind = iter_kind.expect("iter_kind set when iter_hir_opt is Some");
                    match (&iter_hir_opt.as_ref().unwrap().kind, kind) {
                        (HirExprKind::FunctionRef(fid), _) => {
                            self.functions[fid.0].ret_kinds.clone()
                        }
                        (HirExprKind::Local(LocalId(idx)), ValueKind::Function(_)) => {
                            match self.locals[*idx].func_id {
                                Some(fid) => self.functions[fid.0].ret_kinds.clone(),
                                None => {
                                    // Function param: ret is fixed to single Number per
                                    // ADR 0019. Not a valid iter shape.
                                    return Err(HirError::TypeMismatch {
                                        op: "for-in-generic iter".to_owned(),
                                        lhs_kind: "function returning (TaggedValue, TaggedValue)"
                                            .to_owned(),
                                        rhs_kind: "function parameter (single Number return)"
                                            .to_owned(),
                                        offset: iter.span.start,
                                    });
                                }
                            }
                        }
                        (_, ValueKind::TaggedValue) => {
                            vec![ValueKind::TaggedValue, ValueKind::TaggedValue]
                        }
                        (_, other) => {
                            return Err(HirError::TypeMismatch {
                                op: "for-in-generic iter".to_owned(),
                                lhs_kind: "function or callable".to_owned(),
                                rhs_kind: other.name().to_owned(),
                                offset: iter.span.start,
                            });
                        }
                    }
                };
                if iter_ret_kinds.len() != 2 {
                    return Err(HirError::ArityMismatch {
                        builtin: "for-in-generic iter return".to_owned(),
                        expected: 2,
                        actual: iter_ret_kinds.len(),
                        offset: iter.span.start,
                    });
                }
                if !matches!(iter_ret_kinds[0], ValueKind::TaggedValue | ValueKind::Nil) {
                    // Without `nil` reachable as the first result,
                    // generic-for can never terminate. TaggedValue
                    // covers the typical widened-return case;
                    // statically `Nil` covers the trivial `return
                    // nil, nil` shape.
                    return Err(HirError::TypeMismatch {
                        op: "for-in-generic iter return".to_owned(),
                        lhs_kind: "TaggedValue or Nil (first result must allow nil termination)"
                            .to_owned(),
                        rhs_kind: iter_ret_kinds[0].name().to_owned(),
                        offset: iter.span.start,
                    });
                }

                // Synthetic locals live in a fresh inner scope so
                // user-shadowed names don't collide.
                self.scopes.push(HashMap::new());
                let state_id = self.declare_local(
                    format!("__lumelir_iter_state_{}", self.locals.len()),
                    state_kind,
                );
                let ctl_id = self.declare_local(
                    format!("__lumelir_iter_ctl_{}", self.locals.len()),
                    ValueKind::TaggedValue,
                );
                let iter_id_opt = if iter_is_next_builtin {
                    None
                } else {
                    let kind = iter_kind.expect("iter_kind set when iter_hir_opt is Some");
                    let func_id = match (&iter_hir_opt.as_ref().unwrap().kind, kind) {
                        (HirExprKind::FunctionRef(fid), _) => Some(*fid),
                        (HirExprKind::Local(LocalId(idx)), ValueKind::Function(_)) => {
                            self.locals[*idx].func_id
                        }
                        _ => None,
                    };
                    Some(self.declare_local_with_func_id(
                        format!("__lumelir_iter_fn_{}", self.locals.len()),
                        kind,
                        func_id,
                    ))
                };
                let key_id = self.declare_local(key_name.clone(), iter_ret_kinds[0]);
                let val_id = self.declare_local(val_name.clone(), iter_ret_kinds[1]);
                let break_id =
                    self.declare_local(format!("_broken_{}", self.locals.len()), ValueKind::Bool);

                // Build the call's callee.
                let callee = if iter_is_next_builtin {
                    Callee::Builtin(Builtin::Next)
                } else {
                    let iter_id = iter_id_opt.unwrap();
                    match self.locals[iter_id.0].kind {
                        ValueKind::Function(arity) => {
                            if arity != 2 {
                                self.scopes.pop();
                                return Err(HirError::ArityMismatch {
                                    builtin: "for-in-generic iter".to_owned(),
                                    expected: 2,
                                    actual: arity,
                                    offset: iter.span.start,
                                });
                            }
                            match self.locals[iter_id.0].func_id {
                                Some(fid) => Callee::User {
                                    fid,
                                    holding_local: Some(iter_id),
                                },
                                None => Callee::Indirect(iter_id),
                            }
                        }
                        ValueKind::TaggedValue => {
                            // Phase 2.5c-full Commit 3c (ADR 0083):
                            // closure-as-iter is supported now. The
                            // dispatch chain threads each candidate's
                            // cell ptr via the cell-ptr-first ABI,
                            // so capturing iters reach their captured
                            // bindings through the entry-block unpack.
                            let sig = IndirectSig {
                                param_kinds: vec![state_kind, ctl_kind],
                                ret_kinds: vec![ValueKind::TaggedValue, ValueKind::TaggedValue],
                            };
                            let candidates: Vec<FuncId> = self
                                .functions
                                .iter()
                                .enumerate()
                                .filter_map(|(i, f)| {
                                    let pk: Vec<ValueKind> =
                                        f.params.iter().map(|p| p.kind).collect();
                                    if pk == sig.param_kinds && f.ret_kinds == sig.ret_kinds {
                                        Some(FuncId(i))
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if candidates.is_empty() {
                                let local_name = self.locals[iter_id.0].name.clone();
                                self.scopes.pop();
                                return Err(HirError::IndirectCallNoCandidates {
                                    local_name,
                                    param_kinds: sig.param_kinds,
                                    ret_kinds: sig.ret_kinds,
                                    offset: iter.span.start,
                                });
                            }
                            Callee::IndirectDispatch {
                                local_id: iter_id,
                                sig,
                                candidates,
                            }
                        }
                        other => {
                            self.scopes.pop();
                            return Err(HirError::TypeMismatch {
                                op: "for-in-generic iter".to_owned(),
                                lhs_kind: "function or callable".to_owned(),
                                rhs_kind: other.name().to_owned(),
                                offset: iter.span.start,
                            });
                        }
                    }
                };

                self.loop_break_targets.push(Some(break_id));
                let body_result = self.lower_scoped_body_no_push(body);
                self.loop_break_targets.pop();
                self.scopes.pop();
                let user_body_hir = body_result?;

                let local_init = |id, value| HirStmt {
                    kind: HirStmtKind::LocalInit { id, value },
                    span,
                };
                let mut block_stmts: Vec<HirStmt> = Vec::with_capacity(5);
                block_stmts.push(local_init(state_id, state_hir));
                block_stmts.push(local_init(ctl_id, ctl_hir));
                if let Some(iter_id) = iter_id_opt {
                    block_stmts.push(local_init(iter_id, iter_hir_opt.unwrap()));
                }
                block_stmts.push(local_init(
                    break_id,
                    HirExpr {
                        kind: HirExprKind::Bool(false),
                        span,
                    },
                ));
                let state_local_expr = HirExpr {
                    kind: HirExprKind::Local(state_id),
                    span,
                };
                let ctl_local_expr = HirExpr {
                    kind: HirExprKind::Local(ctl_id),
                    span,
                };
                let key_local_expr = HirExpr {
                    kind: HirExprKind::Local(key_id),
                    span,
                };
                let next_step = HirStmt {
                    kind: HirStmtKind::MultiAssignFromCall {
                        dst_ids: vec![key_id, val_id],
                        callee,
                        args: vec![state_local_expr, ctl_local_expr],
                    },
                    span,
                };
                let break_assign = HirStmt {
                    kind: HirStmtKind::Assign {
                        id: break_id,
                        value: HirExpr {
                            kind: HirExprKind::Bool(true),
                            span,
                        },
                    },
                    span,
                };
                let ctl_advance = HirStmt {
                    kind: HirStmtKind::Assign {
                        id: ctl_id,
                        value: key_local_expr.clone(),
                    },
                    span,
                };
                let mut else_body: Vec<HirStmt> = Vec::with_capacity(user_body_hir.len() + 1);
                else_body.extend(user_body_hir);
                else_body.push(ctl_advance);
                let nil_check = HirStmt {
                    kind: HirStmtKind::If {
                        cond: HirExpr {
                            kind: HirExprKind::IsNil(Box::new(key_local_expr)),
                            span,
                        },
                        then_body: vec![break_assign],
                        elifs: Vec::new(),
                        else_body: Some(else_body),
                    },
                    span,
                };
                let while_loop = HirStmt {
                    kind: HirStmtKind::While {
                        cond: HirExpr {
                            kind: HirExprKind::Bool(true),
                            span,
                        },
                        body: vec![next_step, nil_check],
                        break_id: Some(break_id),
                    },
                    span,
                };
                block_stmts.push(while_loop);
                Ok(HirStmt {
                    kind: HirStmtKind::Block { stmts: block_stmts },
                    span,
                })
            }
            StmtKind::ExprStmt(expr) => {
                let hir_expr = self.lower_expr(expr)?;
                Ok(HirStmt {
                    kind: HirStmtKind::ExprStmt(hir_expr),
                    span: stmt.span,
                })
            }
            StmtKind::LocalMulti { names, values } => {
                self.lower_local_multi(names, values, stmt.span)
            }
            StmtKind::AssignMulti { names, values } => {
                self.lower_assign_multi(names, values, stmt.span)
            }
            // Phase 2.6a-wr (ADR 0055): `target[key] = value` table
            // element write. Mirror of `ExprKind::Index` on the read
            // side — same kind constraints (target Table, key Number,
            // value Number); codegen emits the same bounds check.
            StmtKind::IndexAssign { target, key, value } => {
                let target_hir = self.lower_expr(target)?;
                let key_hir = self.lower_expr(key)?;
                let value_hir = self.lower_expr(value)?;
                let target_kind = infer_kind(&target_hir, &self.locals, &self.functions);
                if target_kind != ValueKind::Table {
                    return Err(HirError::TypeMismatch {
                        op: "[]=".to_owned(),
                        lhs_kind: "table".to_owned(),
                        rhs_kind: target_kind.name().to_owned(),
                        offset: target.span.start,
                    });
                }
                let key_kind = infer_kind(&key_hir, &self.locals, &self.functions);
                if !is_hash_key_eligible(key_kind) {
                    return Err(HirError::TypeMismatch {
                        op: "[]=".to_owned(),
                        lhs_kind: "non-nil hashable kind".to_owned(),
                        rhs_kind: key_kind.name().to_owned(),
                        offset: key.span.start,
                    });
                }
                // Phase 2.6c-tag-hash (ADR 0060): String-keyed
                // assignment additionally accepts Nil values as a
                // soft-delete signal (`t.k = nil`). Number-keyed
                // (array) writes still reject Nil because there's
                // no observable use for it — array hole creation
                // happens implicitly via the upper-bound lift.
                // Phase 2.6c-tag-hetero (ADR 0064): both array and
                // hash writes additionally accept Bool / String
                // values. Phase 2.6c-tag-fn-tbl (ADR 0071) opens
                // Function and Table values too; closure-with-
                // upvalues remains rejected via the existing
                // `ClosureEscapes` check (LIC-2.6c-tag-hetero-
                // closure-escape-1).
                let value_kind = infer_kind(&value_hir, &self.locals, &self.functions);
                // Phase 2.6b-hash-keys (ADR 0079): the value-kind
                // matrix factors out the array-vs-hash split. The
                // array path (Number key) historically rejects
                // `nil` because array slots use length-based hole
                // semantics; every other key kind takes the hash
                // path and accepts `nil` as the soft-delete /
                // hard-tombstone signal.
                let value_kind_ok = matches!(
                    value_kind,
                    ValueKind::Number
                        | ValueKind::Bool
                        | ValueKind::String
                        | ValueKind::Function(_)
                        | ValueKind::Table
                ) || (key_kind != ValueKind::Number
                    && value_kind == ValueKind::Nil);
                if !value_kind_ok {
                    return Err(HirError::TypeMismatch {
                        op: "[]=".to_owned(),
                        lhs_kind: "non-nil value (or nil for hash delete)".to_owned(),
                        rhs_kind: value_kind.name().to_owned(),
                        offset: value.span.start,
                    });
                }
                // Phase 2.5c-full Commit 3c (ADR 0083 supersede 0044):
                // closure-with-upvalues no longer escape-rejects on
                // table store. Heap cell + heap upvalue boxes (3b
                // body Steps 6-7) keep reads sound after the table
                // outlives the closure's creation scope.
                //
                // Phase 2.6c-tag-locals-fn (ADR 0074): functions
                // whose ret_kinds widen to TaggedValue are still
                // rejected — the tagged-slot Function payload is
                // a bare ptr with no signature info, and the
                // call-site arity reconstruction (ADR 0072) cannot
                // rebuild the `(...)→(i64,i64)` shape.
                // LIC-2.6c-tag-locals-fn-indirect-1.
                if let Some(fid) = function_ref_id(&value_hir, &self.locals) {
                    if self.functions[fid.0]
                        .ret_kinds
                        .iter()
                        .any(|k| matches!(k, ValueKind::TaggedValue))
                    {
                        return Err(HirError::TypeMismatch {
                            op: "table value (function with tagged return)".to_owned(),
                            lhs_kind: "function with non-tagged return".to_owned(),
                            rhs_kind: "function returning TaggedValue".to_owned(),
                            offset: value.span.start,
                        });
                    }
                }
                Ok(HirStmt {
                    kind: HirStmtKind::IndexAssign {
                        target: target_hir,
                        key: key_hir,
                        value: value_hir,
                    },
                    span: stmt.span,
                })
            }
            // Phase 2.6+-methods (ADR 0092): MethodDef desugars at
            // the HIR chokepoint to `IndexAssign(target=Ident(receiver),
            // key=Str(method), value=FunctionRef(synth_fid))`. For
            // colon form, `self` (kind TaggedValue) is prepended to
            // the effective params and `external_kinds[0]` is seeded
            // with TaggedValue so `for_function`'s body_kinds vs
            // external_kinds merge at src/hir/mod.rs:1336-1339 picks
            // the seeded kind (unless body usage explicitly upgrades
            // to Function, in which case Function wins — semantically
            // correct since `self` IS being called).
            StmtKind::MethodDef {
                receiver,
                method,
                is_colon,
                params,
                body,
            } => self.lower_method_def(receiver, method, *is_colon, params, body, stmt.span),
        }
    }

    /// Phase 2.1c: shared resolver for assignment targets. Three
    /// HIR sites — single Assign, parallel multi-target Assign,
    /// multi-target from Call — all need the same logic:
    ///
    /// - **Existing local**: cross-check kind, reject readonly.
    /// - **Unresolved name**: auto-declare at chunk scope per
    ///   ADR 0048 *if* we're outside a function body and the
    ///   name doesn't shadow a top-level FunctionDef; otherwise
    ///   error.
    ///
    /// Returns the destination `LocalId` plus a flag distinguishing
    /// "was this just declared?" — callers that want to emit
    /// `LocalInit` (vs `Assign`) for the freshly-declared case use
    /// the flag; callers wiring into `MultiAssignFromCall` ignore
    /// it (codegen treats both the same).
    fn resolve_or_declare_target(
        &mut self,
        name: &str,
        expected_kind: ValueKind,
        func_id: Option<FuncId>,
        span: Span,
    ) -> Result<(LocalId, bool), HirError> {
        match self.resolve(name) {
            Some(id) => {
                if self.readonly_locals.contains(&id) {
                    return Err(HirError::ReadOnlyAssign {
                        name: name.to_owned(),
                        offset: span.start,
                    });
                }
                if self.locals[id.0].kind != expected_kind {
                    return Err(HirError::TypeMismatch {
                        op: "=".to_owned(),
                        lhs_kind: self.locals[id.0].kind.name().to_owned(),
                        rhs_kind: expected_kind.name().to_owned(),
                        offset: span.start,
                    });
                }
                Ok((id, false))
            }
            None => {
                if self.in_function.is_some() {
                    return Err(HirError::UndefinedName {
                        name: name.to_owned(),
                        offset: span.start,
                    });
                }
                if self.function_names.contains_key(name) {
                    return Err(HirError::TypeMismatch {
                        op: "=".to_owned(),
                        lhs_kind: "function".to_owned(),
                        rhs_kind: "value".to_owned(),
                        offset: span.start,
                    });
                }
                let id = LocalId(self.locals.len());
                self.locals.push(LocalInfo {
                    name: name.to_owned(),
                    kind: expected_kind,
                    func_id,
                    is_captured: false,
                });
                self.scopes[0].insert(name.to_owned(), id);
                Ok((id, true))
            }
        }
    }

    /// Phase 2.1c: thin wrapper that turns a (name, value) pair
    /// into a complete `Assign` / `LocalInit` HirStmt via
    /// `resolve_or_declare_target`. Used by single-target Assign
    /// and the parallel multi-target path.
    fn lower_assign_target(
        &mut self,
        name: &str,
        value: HirExpr,
        span: Span,
    ) -> Result<HirStmt, HirError> {
        // Phase 2.6c-tag-locals (ADR 0063): same Index→IndexTagged
        // widening as in `Local { ... }` lowering, applied to the
        // `Assign` path (and the auto-declare-at-top-level path
        // routed through here per ADR 0048).
        let value = widen_index_for_local_init(value);
        let value_kind = infer_kind(&value, &self.locals, &self.functions);
        let func_id = match &value.kind {
            HirExprKind::FunctionRef(fid) => Some(*fid),
            HirExprKind::Local(LocalId(idx)) => self.locals[*idx].func_id,
            _ => None,
        };
        let (id, was_declared) = self.resolve_or_declare_target(name, value_kind, func_id, span)?;
        let kind = if was_declared {
            HirStmtKind::LocalInit { id, value }
        } else {
            HirStmtKind::Assign { id, value }
        };
        Ok(HirStmt { kind, span })
    }

    /// Phase 2.1b (ADR 0050): resolve each target name in a
    /// multi-target reassignment. Thin wrapper around
    /// `resolve_or_declare_target` that ignores the
    /// freshly-declared flag (codegen's `MultiAssignFromCall`
    /// treats fresh and existing slots the same way).
    fn resolve_or_declare_multi_targets(
        &mut self,
        names: &[String],
        kinds: &[ValueKind],
        span: Span,
    ) -> Result<Vec<LocalId>, HirError> {
        names
            .iter()
            .zip(kinds.iter())
            .map(|(n, k)| {
                self.resolve_or_declare_target(n, *k, None, span)
                    .map(|(id, _)| id)
            })
            .collect()
    }

    /// Phase 2.1a (ADR 0049): lower a multi-target reassignment
    /// `a, b = e1, e2`. Per Lua semantics, evaluate every RHS into
    /// a temporary first so a swap (`a, b = b, a`) reads the
    /// pre-assignment values. Then store each temporary into the
    /// matching target. Targets must already exist (or be auto-
    /// declared at chunk scope per ADR 0048's rule); each target's
    /// kind must match the value at its position.
    fn lower_assign_multi(
        &mut self,
        names: &[String],
        values: &[Expr],
        span: Span,
    ) -> Result<HirStmt, HirError> {
        // Phase 2.1b (ADR 0050): `a, b = call()` — a single Call
        // RHS expanding across N targets. Mirror `lower_local_multi`'s
        // shape but resolve / auto-declare each target instead of
        // declaring fresh.
        if values.len() == 1 && names.len() > 1 {
            let lowered = self.lower_expr(&values[0])?;
            if let HirExprKind::Call { mut callee, args } = lowered.kind {
                // Phase 2.5x-callee-dispatch (ADR 0082): in multi-
                // assign position the lowered IndirectDispatch's
                // single-value-default `ret_kinds = [Number]` is
                // wrong. Re-search the module for user fns whose
                // params match AND whose ret_kinds.len() ==
                // names.len(). All matching candidates must share
                // identical ret_kinds; otherwise the dispatch chain
                // would be ambiguous and we treat the call site as
                // having no compatible candidates.
                if let Callee::IndirectDispatch {
                    local_id,
                    sig,
                    candidates: _,
                } = &callee
                {
                    let n = names.len();
                    let sig_param_kinds = sig.param_kinds.clone();
                    let local_name = self.locals[local_id.0].name.clone();
                    let arity_matches: Vec<&HirFunction> = self
                        .functions
                        .iter()
                        .filter(|f| {
                            let f_param_kinds: Vec<ValueKind> =
                                f.params.iter().map(|p| p.kind).collect();
                            f_param_kinds == sig_param_kinds && f.ret_kinds.len() == n
                        })
                        .collect();
                    let multi_ret_kinds: Vec<ValueKind> = match arity_matches.first() {
                        Some(first) => first.ret_kinds.clone(),
                        None => Vec::new(),
                    };
                    let all_share = arity_matches.iter().all(|f| f.ret_kinds == multi_ret_kinds);
                    if multi_ret_kinds.is_empty() || !all_share {
                        return Err(HirError::IndirectCallNoCandidates {
                            local_name,
                            param_kinds: sig_param_kinds.clone(),
                            ret_kinds: vec![ValueKind::Number; n],
                            offset: span.start,
                        });
                    }
                    // Re-build the dispatch with the multi-value
                    // ret_kinds and re-filter candidates.
                    let new_sig = IndirectSig {
                        param_kinds: sig_param_kinds.clone(),
                        ret_kinds: multi_ret_kinds.clone(),
                    };
                    let new_candidates: Vec<FuncId> = self
                        .functions
                        .iter()
                        .enumerate()
                        .filter_map(|(i, f)| {
                            let f_param_kinds: Vec<ValueKind> =
                                f.params.iter().map(|p| p.kind).collect();
                            if f_param_kinds == new_sig.param_kinds
                                && f.ret_kinds == new_sig.ret_kinds
                            {
                                Some(FuncId(i))
                            } else {
                                None
                            }
                        })
                        .collect();
                    callee = Callee::IndirectDispatch {
                        local_id: *local_id,
                        sig: new_sig,
                        candidates: new_candidates,
                    };
                }
                let ret_kinds: Vec<ValueKind> = match &callee {
                    Callee::User { fid, .. } => self.functions[fid.0].ret_kinds.clone(),
                    // Phase 2.8e-iter-next (ADR 0081): builtin-callee
                    // multi-assign. Until `Builtin::Next` lands every
                    // builtin returns at most one value, so this
                    // branch only widens the shape — callers still
                    // hit the arity mismatch below when names.len()
                    // doesn't match `b.ret_kinds().len()`.
                    Callee::Builtin(b) => b.ret_kinds().to_vec(),
                    Callee::IndirectDispatch { sig, .. } => sig.ret_kinds.clone(),
                    Callee::Indirect(_) => {
                        return Err(HirError::ArityMismatch {
                            builtin: "multi-assign".to_owned(),
                            expected: names.len(),
                            actual: 1,
                            offset: span.start,
                        });
                    }
                };
                if ret_kinds.len() != names.len() {
                    return Err(HirError::ArityMismatch {
                        builtin: "multi-assign".to_owned(),
                        expected: ret_kinds.len(),
                        actual: names.len(),
                        offset: span.start,
                    });
                }
                let dst_ids = self.resolve_or_declare_multi_targets(names, &ret_kinds, span)?;
                return Ok(HirStmt {
                    kind: HirStmtKind::MultiAssignFromCall {
                        dst_ids,
                        callee,
                        args,
                    },
                    span,
                });
            }
            // Single non-Call RHS with multiple targets — fall
            // through to the arity-mismatch error path.
        }
        if names.len() != values.len() {
            return Err(HirError::ArityMismatch {
                builtin: "multi-assign".to_owned(),
                expected: names.len(),
                actual: values.len(),
                offset: span.start,
            });
        }
        let lowered: Vec<HirExpr> = values
            .iter()
            .map(|v| self.lower_expr(v))
            .collect::<Result<_, _>>()?;
        // Stage 1: snapshot each RHS into a fresh temp local. The
        // temp's kind comes from the lowered expr — same as how
        // single Assign infers it.
        let mut tmp_ids: Vec<LocalId> = Vec::with_capacity(lowered.len());
        let mut block_stmts: Vec<HirStmt> = Vec::with_capacity(lowered.len() * 2);
        for v in &lowered {
            let kind = infer_kind(v, &self.locals, &self.functions);
            let id = self.declare_local(format!("_multi_tmp_{}", self.locals.len()), kind);
            tmp_ids.push(id);
            block_stmts.push(HirStmt {
                kind: HirStmtKind::LocalInit {
                    id,
                    value: v.clone(),
                },
                span,
            });
        }
        // Stage 2: write each temp to its target via the shared
        // resolver — handles both existing-local kind check and
        // chunk-scope auto-declare uniformly (Phase 2.1c).
        for (name, tmp_id) in names.iter().zip(tmp_ids.iter()) {
            let value = HirExpr {
                kind: HirExprKind::Local(*tmp_id),
                span,
            };
            block_stmts.push(self.lower_assign_target(name, value, span)?);
        }
        Ok(HirStmt {
            kind: HirStmtKind::Block { stmts: block_stmts },
            span,
        })
    }

    /// Phase 2.5d (ADR 0021): shared implementation for both
    /// single-result `Return { value }` and multi-result
    /// `ReturnMulti { values }`. Cross-checks arity and per-position
    /// kind against any prior return in the same body, upgrades
    /// `_ret_value_N` slot kinds, and emits the body-guard pattern's
    /// `_ret_value_N := ...; _returned := true` block.
    fn lower_return_with_values(
        &mut self,
        lowered: Vec<HirExpr>,
        span: Span,
    ) -> Result<HirStmt, HirError> {
        let (returned_id, ret_value_ids) = self
            .in_function
            .as_ref()
            .map(|(r, ids)| (*r, ids.clone()))
            .ok_or(HirError::ReturnOutsideFunction { offset: span.start })?;
        // Phase 2.5c-full Commit 3c (ADR 0083 supersede 0044):
        // ADR 0044's `ClosureEscapes` reject for return values is
        // gone. Heap-allocated upvalue boxes (3b body Step 6) and
        // a heap-allocated closure cell (3b body Step 7) survive
        // the frame teardown, so a returned closure's reads stay
        // sound. The caller receives the cell ptr through the
        // standard `Function(_)`-kind multi-return slot.
        let kinds: Vec<ValueKind> = lowered
            .iter()
            .map(|e| infer_kind(e, &self.locals, &self.functions))
            .collect();
        if let Some(prev) = self.in_function_ret_kinds.clone() {
            if prev.len() != kinds.len() {
                return Err(HirError::ArityMismatch {
                    builtin: "return".to_owned(),
                    expected: prev.len(),
                    actual: kinds.len(),
                    offset: span.start,
                });
            }
            // Phase 2.6c-tag-locals-fn (ADR 0074): heterogeneous
            // return kinds at the same position widen the slot
            // (and thus the function's ret_kinds entry) to
            // TaggedValue instead of being rejected. Once a
            // position is TaggedValue, it stays TaggedValue
            // (idempotent — every subsequent return path stores
            // through the dispatched-store helper).
            //
            // Multi-return × TaggedValue position interleaving
            // (`return 1, nil` vs `return nil, 1`) is still
            // allowed by this pure widening logic, but the
            // codegen ABI is not yet ready for it; the gating
            // happens at the codegen layer
            // (LIC-2.6c-tag-locals-fn-multi-1).
            for (i, (p, k)) in prev.iter().zip(kinds.iter()).enumerate() {
                if *p == *k || *p == ValueKind::TaggedValue {
                    continue;
                }
                let widened = self.in_function_ret_kinds.as_mut().unwrap();
                widened[i] = ValueKind::TaggedValue;
                let slot_id = ret_value_ids[i];
                self.locals[slot_id.0].kind = ValueKind::TaggedValue;
            }
        } else {
            // First return seen — upgrade each `_ret_value_N` slot kind.
            for (slot_id, k) in ret_value_ids.iter().zip(kinds.iter()) {
                self.locals[slot_id.0].kind = *k;
            }
            self.in_function_ret_kinds = Some(kinds.clone());
        }
        let mut block_stmts: Vec<HirStmt> = Vec::with_capacity(lowered.len() + 1);
        for (slot_id, v) in ret_value_ids.iter().zip(lowered) {
            block_stmts.push(HirStmt {
                kind: HirStmtKind::Assign {
                    id: *slot_id,
                    value: v,
                },
                span,
            });
        }
        block_stmts.push(HirStmt {
            kind: HirStmtKind::Assign {
                id: returned_id,
                value: HirExpr {
                    kind: HirExprKind::Bool(true),
                    span,
                },
            },
            span,
        });
        Ok(HirStmt {
            kind: HirStmtKind::Block { stmts: block_stmts },
            span,
        })
    }

    /// Phase 2.5d (ADR 0021): lower `local NAMES = VALUES`. Two
    /// shapes are supported:
    ///
    /// - `len(values) == len(names)` — parallel binding, lowers to a
    ///   `Block` of independent `LocalInit` statements.
    /// - `len(values) == 1` and that value is a multi-result Call
    ///   whose ret arity matches `len(names)` — emits a single
    ///   `MultiAssignFromCall` so codegen evaluates the call once.
    ///
    /// Any other shape is rejected as `ArityMismatch`.
    fn lower_local_multi(
        &mut self,
        names: &[String],
        values: &[Expr],
        span: Span,
    ) -> Result<HirStmt, HirError> {
        if values.len() == names.len() {
            // Parallel: lower each value, declare locals 1-1.
            // Lower all RHS first under the original scope so name
            // shadowing matches single-binding semantics.
            let lowered: Vec<HirExpr> = values
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let mut stmts: Vec<HirStmt> = Vec::with_capacity(names.len());
            for (n, v) in names.iter().zip(lowered) {
                let kind = infer_kind(&v, &self.locals, &self.functions);
                let func_id = match &v.kind {
                    HirExprKind::FunctionRef(fid) => Some(*fid),
                    HirExprKind::Local(LocalId(idx)) => self.locals[*idx].func_id,
                    _ => None,
                };
                let id = self.declare_local_with_func_id(n.clone(), kind, func_id);
                stmts.push(HirStmt {
                    kind: HirStmtKind::LocalInit { id, value: v },
                    span,
                });
            }
            return Ok(HirStmt {
                kind: HirStmtKind::Block { stmts },
                span,
            });
        }
        if values.len() == 1 {
            // Multi-bind from a single Call. The Call must be a User-
            // or Builtin-dispatch (Indirect ret arities aren't
            // tracked statically); its ret arity must equal
            // `names.len()`. Phase 2.8e-iter-next (ADR 0081) opened
            // the Builtin path so `local k, v = next(t, c)` works.
            let lowered = self.lower_expr(&values[0])?;
            let HirExprKind::Call { callee, args } = lowered.kind else {
                return Err(HirError::ArityMismatch {
                    builtin: "local =".to_owned(),
                    expected: names.len(),
                    actual: 1,
                    offset: span.start,
                });
            };
            let ret_kinds: Vec<ValueKind> = match callee {
                Callee::User { fid, .. } => self.functions[fid.0].ret_kinds.clone(),
                Callee::Builtin(b) => b.ret_kinds().to_vec(),
                Callee::IndirectDispatch { ref sig, .. } => sig.ret_kinds.clone(),
                Callee::Indirect(_) => {
                    return Err(HirError::ArityMismatch {
                        builtin: "local =".to_owned(),
                        expected: names.len(),
                        actual: 1,
                        offset: span.start,
                    });
                }
            };
            if ret_kinds.len() != names.len() {
                return Err(HirError::ArityMismatch {
                    builtin: "local =".to_owned(),
                    expected: ret_kinds.len(),
                    actual: names.len(),
                    offset: span.start,
                });
            }
            let mut dst_ids: Vec<LocalId> = Vec::with_capacity(names.len());
            for (n, k) in names.iter().zip(ret_kinds.iter()) {
                dst_ids.push(self.declare_local(n.clone(), *k));
            }
            return Ok(HirStmt {
                kind: HirStmtKind::MultiAssignFromCall {
                    dst_ids,
                    callee,
                    args,
                },
                span,
            });
        }
        Err(HirError::ArityMismatch {
            builtin: "local =".to_owned(),
            expected: names.len(),
            actual: values.len(),
            offset: span.start,
        })
    }

    fn lower_expr(&mut self, expr: &Expr) -> Result<HirExpr, HirError> {
        let kind = match &expr.kind {
            ExprKind::Number(n) => HirExprKind::Number(*n),
            ExprKind::Str(s) => HirExprKind::Str(s.clone()),
            ExprKind::Ident(name) => match self.resolve(name) {
                Some(id) => HirExprKind::Local(id),
                None => {
                    if let Some(&fid) = self.function_names.get(name) {
                        // Phase 2.5b: a top-level `local function f`
                        // registers `f` in `function_names` but does
                        // *not* (in 2.5a) create a local.
                        HirExprKind::FunctionRef(fid)
                    } else if let Some(local_id) =
                        self.lookup_or_capture_upvalue(name, expr.span)?
                    {
                        // Phase 2.5c-min (ADR 0037): the name is in
                        // the enclosing scope — capture it.
                        HirExprKind::Local(local_id)
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
                    // Arithmetic + bitwise (Phase 2.2c, ADR 0022):
                    // both sides must be Number. Bitwise ops convert
                    // to i64 at codegen time; HIR enforces Number-only.
                    BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::Mod
                    | BinOp::Pow
                    | BinOp::FloorDiv
                    | BinOp::BitAnd
                    | BinOp::BitOr
                    | BinOp::BitXor
                    | BinOp::Shl
                    | BinOp::Shr => {
                        // Phase 2.7p-arith-string-coerce (ADR 0077):
                        // String operands wrap in
                        // `ArithStringCoerce` so kind validation
                        // sees them as Number; the wrapper traps
                        // at runtime on parse failure.
                        let lhs_hir =
                            coerce_arith_operand_if_string(lhs_hir, &self.locals, &self.functions);
                        let rhs_hir =
                            coerce_arith_operand_if_string(rhs_hir, &self.locals, &self.functions);
                        let lk = infer_kind(&lhs_hir, &self.locals, &self.functions);
                        let rk = infer_kind(&rhs_hir, &self.locals, &self.functions);
                        // Phase 2.6c-tag-locals (ADR 0063):
                        // TaggedValue is interchangeable with
                        // Number at the HIR layer. The Local
                        // read site emits a tag check that traps
                        // when the actual value is Nil.
                        if !(is_number_compatible(lk) && is_number_compatible(rk)) {
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
                    // Ordering: both sides must share a comparable
                    // kind. Phase 2.7d (ADR 0027) widens the rule
                    // from "Number only" to "Number-Number or
                    // String-String"; cross-kind still rejects.
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        let ok = (is_number_compatible(lk) && is_number_compatible(rk))
                            || (lk == ValueKind::String && rk == ValueKind::String);
                        if !ok {
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
                    //
                    // Phase 2.6c-isnil-query (ADR 0061): `Index == nil` /
                    // `nil == Index` (and `~=`) bypasses the fold to
                    // produce a non-trapping `IsNilQuery`, so OOB array
                    // reads and missing hash keys can answer the Lua-
                    // spec true without triggering the trapping read
                    // path. Detected before the fold; otherwise the
                    // heterogeneous-kind rule would silently miscompile
                    // them to `Bool(false)`.
                    BinOp::Eq | BinOp::Ne => {
                        // Phase 2.6c-tag-hetero-eq (ADR 0066): one
                        // unified `IsNil` variant for any tagged-
                        // value source. Two operand shapes lower
                        // through here:
                        //   - `Index { target, key }` (ADR 0061,
                        //     non-trapping table read)
                        //   - `Local(TaggedValue)` (ADR 0063, slot
                        //     tag check)
                        // Detect the `<source> == nil` (or the
                        // reverse) pattern and lower to
                        // `IsNil(<source>)`. Other Eq/Ne patterns
                        // fall through to the fold or runtime
                        // dispatch path below.
                        let nil_operand: Option<HirExpr> = match (&lhs_hir.kind, &rhs_hir.kind) {
                            (HirExprKind::Index { .. }, HirExprKind::Nil) => Some(lhs_hir.clone()),
                            (HirExprKind::Nil, HirExprKind::Index { .. }) => Some(rhs_hir.clone()),
                            (HirExprKind::Local(LocalId(idx)), HirExprKind::Nil)
                                if matches!(self.locals[*idx].kind, ValueKind::TaggedValue) =>
                            {
                                Some(lhs_hir.clone())
                            }
                            (HirExprKind::Nil, HirExprKind::Local(LocalId(idx)))
                                if matches!(self.locals[*idx].kind, ValueKind::TaggedValue) =>
                            {
                                Some(rhs_hir.clone())
                            }
                            _ => None,
                        };
                        if let Some(operand) = nil_operand {
                            let query = HirExpr {
                                kind: HirExprKind::IsNil(Box::new(operand)),
                                span: expr.span,
                            };
                            match op {
                                BinOp::Eq => query.kind,
                                BinOp::Ne => HirExprKind::UnaryOp {
                                    op: UnaryOp::Not,
                                    operand: Box::new(query),
                                },
                                _ => unreachable!(),
                            }
                        } else {
                            // Phase 2.6c-tag-hetero-fix (ADR 0065):
                            // skip the heterogeneous-kind Eq fold
                            // whenever either side is `TaggedValue`.
                            // The runtime tag of a TaggedValue can
                            // match a Number / Bool / String literal,
                            // so folding to `false` on static-kind
                            // mismatch is a silent miscompile.
                            // Codegen handles the runtime dispatch.
                            let either_tagged =
                                lk == ValueKind::TaggedValue || rk == ValueKind::TaggedValue;
                            let fold = !either_tagged
                                && (lk != rk || (lk == ValueKind::Nil && rk == ValueKind::Nil));
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
                    // Phase 2.7b (ADR 0025): `..` produces a String.
                    // Phase 2.7c (ADR 0026): non-String operands —
                    // Number, Bool, Nil — are silently wrapped in
                    // a `tostring(...)` call so `"x"..1` works the
                    // same way it does in stock Lua. Function-kind
                    // operands remain a TypeMismatch.
                    BinOp::Concat => {
                        let lhs_coerced = coerce_to_string(lhs_hir, lk, expr.span.start)?;
                        let rhs_coerced = coerce_to_string(rhs_hir, rk, expr.span.start)?;
                        HirExprKind::BinOp {
                            op: *op,
                            lhs: Box::new(lhs_coerced),
                            rhs: Box::new(rhs_coerced),
                        }
                    }
                }
            }
            ExprKind::UnaryOp { op, operand } => {
                let operand_hir = self.lower_expr(operand)?;
                // Phase 2.7a (ADR 0024) / 2.6a-min (ADR 0053): `#x`
                // accepts either a String (length via libc strlen)
                // or a Table (length read from the heap header's
                // first i64 slot).
                if matches!(op, UnaryOp::Len) {
                    let k = infer_kind(&operand_hir, &self.locals, &self.functions);
                    if !matches!(k, ValueKind::String | ValueKind::Table) {
                        return Err(HirError::TypeMismatch {
                            op: "#".to_owned(),
                            lhs_kind: "string or table".to_owned(),
                            rhs_kind: k.name().to_owned(),
                            offset: expr.span.start,
                        });
                    }
                }
                HirExprKind::UnaryOp {
                    op: *op,
                    operand: Box::new(operand_hir),
                }
            }
            ExprKind::FunctionExpr { params, body } => {
                // Register a fresh HirFunction with `name = ""` and
                // mangled `user_anon_<idx>` (ADR 0017). The body is
                // lowered in a separate LowerCtx so it has its own
                // scope/locals/break-stack.
                let id = FuncId(self.functions.len());
                let mangled = format!("user_anon_{}", id.0);
                let parent_scope = self.current_parent_scope();
                self.functions.push(HirFunction {
                    name: String::new(),
                    mangled_name: mangled,
                    params: params
                        .iter()
                        .map(|p| LocalInfo {
                            name: p.clone(),
                            kind: ValueKind::Number,
                            func_id: None,
                            is_captured: false,
                        })
                        .collect(),
                    upvalues: Vec::new(),
                    locals: Vec::new(),
                    body: Vec::new(),
                    ret_kinds: Vec::new(),
                    parent_scope,
                });
                // Anonymous functions have no caller-name to scan
                // for call-site arg kinds; default to Number for all
                // params. Body-pre-scan still upgrades to Function
                // when needed.
                let external_kinds = vec![ValueKind::Number; params.len()];
                // Phase 2.5c-min: the body can capture the
                // currently-visible bindings from this LowerCtx.
                let outer_visible = self.outer_visible_snapshot();
                let mut fn_ctx = LowerCtx::for_function(
                    &self.function_names,
                    &self.method_funcs,
                    &self.functions,
                    params,
                    body,
                    &external_kinds,
                    outer_visible,
                    id,
                );
                let body_hir = fn_ctx.lower_function_body(body)?;
                let ret_kinds = fn_ctx.in_function_ret_kinds.unwrap_or_default();
                self.functions[id.0].params = fn_ctx.locals[..params.len()].to_vec();
                self.functions[id.0].upvalues = fn_ctx.upvalues;
                self.functions[id.0].locals = fn_ctx.locals;
                self.functions[id.0].body = body_hir;
                self.functions[id.0].ret_kinds = ret_kinds;
                HirExprKind::FunctionRef(id)
            }
            ExprKind::Call { callee, args } => self.lower_call(callee, args, expr)?,
            // Phase 2.6a-min (ADR 0053) / 2.6a-arr (ADR 0054) /
            // 2.6c-tag-hetero (ADR 0064) / 2.6c-tag-fn-tbl
            // (ADR 0071): table constructor. All six kinds —
            // Number / Bool / String / Nil / Function / Table —
            // land in a 16-byte tagged slot. Closure with
            // upvalues is rejected via the existing
            // `ClosureEscapes` analysis.
            ExprKind::Table(elems) => {
                let lowered: Vec<HirExpr> = elems
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                for elem in &lowered {
                    let k = infer_kind(elem, &self.locals, &self.functions);
                    let elem_ok = matches!(
                        k,
                        ValueKind::Number
                            | ValueKind::Bool
                            | ValueKind::String
                            | ValueKind::Nil
                            | ValueKind::Function(_)
                            | ValueKind::Table
                    );
                    if !elem_ok {
                        return Err(HirError::TypeMismatch {
                            op: "table element".to_owned(),
                            lhs_kind: "number/bool/string/nil/function/table".to_owned(),
                            rhs_kind: k.name().to_owned(),
                            offset: elem.span.start,
                        });
                    }
                    // Phase 2.5c-full Commit 3c (ADR 0083 supersede 0044):
                    // closure-with-upvalues no longer escape-rejects
                    // on table-element insertion. ADR 0074's tagged-
                    // return rejection stays — its rationale (signature
                    // erasure on TaggedValue Function payload) is
                    // independent of the escape concern.
                    if let Some(fid) = function_ref_id(elem, &self.locals) {
                        if self.functions[fid.0]
                            .ret_kinds
                            .iter()
                            .any(|k| matches!(k, ValueKind::TaggedValue))
                        {
                            return Err(HirError::TypeMismatch {
                                op: "table element (function with tagged return)".to_owned(),
                                lhs_kind: "function with non-tagged return".to_owned(),
                                rhs_kind: "function returning TaggedValue".to_owned(),
                                offset: elem.span.start,
                            });
                        }
                    }
                }
                HirExprKind::Table(lowered)
            }
            // Phase 2.6a-arr (ADR 0054) / 2.6b-hash (ADR 0058):
            // `target[key]` index read. Number key → array path,
            // String key → hash path. Codegen dispatches on the
            // key's static kind.
            ExprKind::Index { target, key } => {
                let target_hir = self.lower_expr(target)?;
                let key_hir = self.lower_expr(key)?;
                let target_kind = infer_kind(&target_hir, &self.locals, &self.functions);
                if target_kind != ValueKind::Table {
                    return Err(HirError::TypeMismatch {
                        op: "[]".to_owned(),
                        lhs_kind: "table".to_owned(),
                        rhs_kind: target_kind.name().to_owned(),
                        offset: target.span.start,
                    });
                }
                let key_kind = infer_kind(&key_hir, &self.locals, &self.functions);
                if !is_hash_key_eligible(key_kind) {
                    return Err(HirError::TypeMismatch {
                        op: "[]".to_owned(),
                        lhs_kind: "non-nil hashable kind".to_owned(),
                        rhs_kind: key_kind.name().to_owned(),
                        offset: key.span.start,
                    });
                }
                HirExprKind::Index {
                    target: Box::new(target_hir),
                    key: Box::new(key_hir),
                }
            }
            // Phase 2.6+-methods (ADR 0092): MethodCall desugar at
            // HIR chokepoint. Steps:
            //   (i)   Receiver-shape walker rejects ComplexMethodReceiver
            //         and non-Ident receivers (MVP scope — only Ident
            //         receivers participate in dispatch param-kind
            //         matching; future ADR broadens once dispatch
            //         widening lands).
            //   (ii)  Receiver consumed via Ident fast-path: the
            //         caller already stored the Table value in a
            //         local with kind Table, which matches the
            //         colon-method's `external_kinds[0] = Table`
            //         seed (see `lower_method_def`).
            //   (iii) Synthesise `Call { callee: Index { recv, Str(method) },
            //         args: [recv, ...args] }` and recurse through
            //         `lower_call`, which classifies the new shape as
            //         IndexCallee and routes through ADR 0082's
            //         IndirectDispatch (LocalId-source invariant
            //         preserved).
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                check_method_receiver_shape(receiver)?;
                let recv_name = match &receiver.kind {
                    ExprKind::Ident(n) => n.clone(),
                    _ => {
                        return Err(HirError::ComplexMethodReceiver {
                            offset: receiver.span.start,
                        });
                    }
                };
                let recv_ident_expr = Expr::new(ExprKind::Ident(recv_name), receiver.span);
                let method_key_expr = Expr::new(ExprKind::Str(method.clone()), expr.span);
                let synth_callee = Expr::new(
                    ExprKind::Index {
                        target: Box::new(recv_ident_expr.clone()),
                        key: Box::new(method_key_expr),
                    },
                    expr.span,
                );
                let mut new_args = Vec::with_capacity(args.len() + 1);
                new_args.push(recv_ident_expr);
                for a in args {
                    new_args.push(a.clone());
                }
                let call_kind = self.lower_call(&synth_callee, &new_args, expr)?;
                return Ok(HirExpr {
                    kind: call_kind,
                    span: expr.span,
                });
            }
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
        // Phase 2.6+-callee-norm (ADR 0091 v2): pre-step is to
        // classify the callee shape. DirectIdent flows through the
        // existing path; IndexCallee reconstructs the Index AST and
        // pre-binds it to a synthetic local via the shared
        // `materialize_to_synth_local` helper (renamed by ADR 0092
        // so MethodCall can reuse it for receiver materialisation),
        // then recurses with the synthetic as the new callee.
        // Anything else surfaces as the existing `UnsupportedCall`.
        match classify_callee_form(callee) {
            Ok(CalleeForm::DirectIdent) => { /* fall through */ }
            Ok(CalleeForm::IndexCallee { target, key }) => {
                let index_ast = Expr::new(
                    ExprKind::Index {
                        target: Box::new(target.clone()),
                        key: Box::new(key.clone()),
                    },
                    whole.span,
                );
                let synth_id = self.materialize_to_synth_local(&index_ast, whole.span)?;
                let synth_name = self.locals[synth_id.0].name.clone();
                let synth_callee = Expr::new(ExprKind::Ident(synth_name), whole.span);
                return self.lower_call(&synth_callee, args, whole);
            }
            Err(e) => return Err(e),
        }
        // Local helper for the ADR 0082 indirect-dispatch path. Pure
        // function over the module's user-function table; no table /
        // local provenance analysis (Codex pre-ADR-0082 review:
        // `IndexAssign` breaks any provenance immediately).
        fn compatible_user_functions(sig: &IndirectSig, functions: &[HirFunction]) -> Vec<FuncId> {
            functions
                .iter()
                .enumerate()
                .filter_map(|(i, f)| {
                    let f_param_kinds: Vec<ValueKind> = f.params.iter().map(|p| p.kind).collect();
                    if f_param_kinds == sig.param_kinds && f.ret_kinds == sig.ret_kinds {
                        Some(FuncId(i))
                    } else {
                        None
                    }
                })
                .collect()
        }

        // Phase 2.5c-full Commit 2a-fix (ADR 0083 / ADR 0075 amend):
        // a `Function`-kind argument's source fn must declare
        // `ret_kinds == [Number]`. Inside the receiving user fn the
        // value is invoked via `Callee::Indirect`, whose codegen
        // hardcodes an `f64` MLIR result type — Bool / Nil / String /
        // Table / multi-return ABIs would silently miscompile after
        // Commit 2a's `!llvm.ptr` Function-value erasure stripped the
        // verifier's safety net. By induction Function-kind locals
        // with no `func_id` (parameters) are already Number-only at
        // their binding site, so we skip those here and only check
        // statically resolvable sources (FunctionRef, known-FuncId
        // Local).
        fn check_function_arg_ret_kinds(
            target_param_kinds: &[ValueKind],
            lowered_args: &[HirExpr],
            locals: &[LocalInfo],
            functions: &[HirFunction],
        ) -> Result<(), HirError> {
            for (i, arg) in lowered_args.iter().enumerate() {
                if !matches!(target_param_kinds.get(i), Some(ValueKind::Function(_))) {
                    continue;
                }
                let source_fid = match &arg.kind {
                    HirExprKind::FunctionRef(fid) => Some(*fid),
                    HirExprKind::Local(LocalId(idx)) => locals[*idx].func_id,
                    _ => None,
                };
                if let Some(fid) = source_fid {
                    let ret_kinds = functions[fid.0].ret_kinds.clone();
                    if !matches!(ret_kinds.as_slice(), [ValueKind::Number]) {
                        return Err(HirError::IndirectCallNonNumberReturn {
                            source_name: functions[fid.0].mangled_name.clone(),
                            ret_kinds,
                            offset: arg.span.start,
                        });
                    }
                }
            }
            Ok(())
        }
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
                // Phase 2.5c-full Commit 3b prep fix (ADR 0083):
                // when the local resolves to a known user fn (e.g.
                // synthetic FunctionDef local), check per-arg
                // Function-kind arity compatibility against the
                // target's params. This matches the function_names
                // fallback path's check and was previously
                // unreachable when `local function f` had no local
                // slot (function_names path always handled it).
                if let Some(target_fid) = self.locals[local_id.0].func_id {
                    let target_param_kinds: Vec<ValueKind> = self.functions[target_fid.0]
                        .params
                        .iter()
                        .map(|p| p.kind)
                        .collect();
                    for (i, arg) in lowered_args.iter().enumerate() {
                        let arg_kind = infer_kind(arg, &self.locals, &self.functions);
                        if let Some(expected) = target_param_kinds.get(i) {
                            let compatible = match (*expected, arg_kind) {
                                (ValueKind::Number, ValueKind::Number) => true,
                                (ValueKind::Bool, ValueKind::Bool) => true,
                                (ValueKind::Nil, ValueKind::Nil) => true,
                                (ValueKind::String, ValueKind::String) => true,
                                (ValueKind::Table, ValueKind::Table) => true,
                                (ValueKind::Function(a), ValueKind::Function(b)) => a == b,
                                _ => false,
                            };
                            if !compatible {
                                return Err(HirError::TypeMismatch {
                                    op: format!("call-{name}"),
                                    lhs_kind: expected.name().to_owned(),
                                    rhs_kind: arg_kind.name().to_owned(),
                                    offset: arg.span.start,
                                });
                            }
                        }
                    }
                }
                // Phase 2.5c.3 (ADR 0044): a closure carrying
                // upvalues passed as a value reaches its eventual
                // call site via Callee::Indirect, which has no
                // path to thread upvalues. Reject statically.
                // (Phase 2.5c-full Commit 3b prep fix: the narrow
                // `Number | Function` arg-kind check that lived
                // here before E1 has been replaced by the
                // per-target compatibility check above; the latter
                // matches the function_names path's check exactly
                // and now correctly accepts Bool/Nil/String/Table
                // args when the target's params expect those kinds.)
                //
                // Phase 2.5c-full Commit 3c (ADR 0083 supersede 0044):
                // closure-with-upvalues args no longer escape-reject.
                // The cell-ptr-first ABI (3b body) means a Function
                // arg is a heap cell ptr that survives any frame
                // teardown the callee performs.
                // Phase 2.5c-full Commit 3b (ADR 0083): the local
                // resolved here IS the holding binding for the
                // closure cell. Codegen will load the cell ptr
                // from `slots[local_id]` (capturing) or fall back
                // to the singleton (non-capturing).
                let callee = match self.locals[local_id.0].func_id {
                    Some(fid) => Callee::User {
                        fid,
                        holding_local: Some(local_id),
                    },
                    None => Callee::Indirect(local_id),
                };
                // Phase 2.5c-full Commit 2a-fix (ADR 0083 / ADR 0075
                // amend): when this resolves to a known user fn,
                // restrict Function-kind args to ret_kinds=[Number].
                // The Callee::Indirect branch (parameter passing
                // through) needs no extra check — its enclosing
                // fn's parameter binding already enforced the rule.
                if let Callee::User { fid, .. } = callee {
                    let target_param_kinds: Vec<ValueKind> = self.functions[fid.0]
                        .params
                        .iter()
                        .map(|p| p.kind)
                        .collect();
                    check_function_arg_ret_kinds(
                        &target_param_kinds,
                        &lowered_args,
                        &self.locals,
                        &self.functions,
                    )?;
                }
                // Phase 2.5c-full Commit 3b body atomic Step 2 (ADR
                // 0083): trailing-uv append removed. cell ptr now
                // flows through the call's first argument from
                // codegen, sourced via `Callee::User::holding_local`
                // when the binding is in scope or via the entry
                // `cell_ptr` block-arg for self-recursion.
                return Ok(HirExprKind::Call {
                    callee,
                    args: lowered_args,
                });
            }
            // Phase 2.5x-callee-dispatch (ADR 0082, part-supersedes
            // ADR 0075 / ADR 0072): a TaggedValue local whose
            // runtime tag is TAG_FUNCTION reaches the call site as
            // a dispatcher. We compute the compatible-set of user
            // functions whose `(param_kinds, ret_kinds)` matches
            // the call site's signature at compile time, then emit
            // a per-call-site `if loaded_ptr == @user_fn_X then
            // func.call @user_fn_X(args)` chain (no
            // `func.call_indirect` cast — Codex pre-ADR-0082
            // review's forward-edge integrity recommendation).
            //
            // Single-value position truncates `ret_kinds` to
            // `[Number]` by default. Multi-value position
            // (`local k, v = g(...)`) is handled by
            // `lower_local_multi` / `lower_assign_multi`, which
            // re-run the candidate filter with the multi-target
            // ret_kinds before reaching codegen.
            if matches!(self.locals[local_id.0].kind, ValueKind::TaggedValue) {
                let lowered_args = args
                    .iter()
                    .map(|a| self.lower_expr(a))
                    .collect::<Result<Vec<_>, _>>()?;
                let param_kinds: Vec<ValueKind> = lowered_args
                    .iter()
                    .map(|a| infer_kind(a, &self.locals, &self.functions))
                    .collect();
                // Filter user fns by `param_kinds` only and pick the
                // first match's `ret_kinds` as the canonical
                // signature. Multi-assign callers re-filter with the
                // multi-position ret_kinds; single-value callers
                // truncate to the first result.
                let param_only_matches: Vec<&HirFunction> = self
                    .functions
                    .iter()
                    .filter(|f| {
                        let f_param_kinds: Vec<ValueKind> =
                            f.params.iter().map(|p| p.kind).collect();
                        f_param_kinds == param_kinds
                    })
                    .collect();
                if param_only_matches.is_empty() {
                    return Err(HirError::IndirectCallNoCandidates {
                        local_name: name.clone(),
                        param_kinds: param_kinds.clone(),
                        ret_kinds: vec![ValueKind::Number],
                        offset: whole.span.start,
                    });
                }
                // Canonical ret_kinds: take the first match's. The
                // dispatch chain's `result_types` follow this. If
                // other param-matching user fns have a different
                // ret_kinds vector, they're filtered OUT here — the
                // contract is that all candidates share signature.
                let canonical_ret_kinds = param_only_matches[0].ret_kinds.clone();
                let sig = IndirectSig {
                    param_kinds: param_kinds.clone(),
                    ret_kinds: canonical_ret_kinds,
                };
                let candidates = compatible_user_functions(&sig, &self.functions);
                return Ok(HirExprKind::Call {
                    callee: Callee::IndirectDispatch {
                        local_id,
                        sig,
                        candidates,
                    },
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
            // Phase 2.5c-full Commit 2a-fix (ADR 0083 / ADR 0075
            // amend): same ret_kind restriction as the local-
            // Function-kind path — Function-kind args must come
            // from a Number-returning user fn.
            check_function_arg_ret_kinds(
                &param_kinds,
                &lowered_args,
                &self.locals,
                &self.functions,
            )?;
            // Phase 2.5b.2/2.5e: each arg's kind must match the
            // corresponding param's kind (Number ↔ Number, Bool ↔
            // Bool, Nil ↔ Nil, Function(arity) ↔ Function(arity)).
            for (i, arg) in lowered_args.iter().enumerate() {
                let arg_kind = infer_kind(arg, &self.locals, &self.functions);
                let expected_kind = param_kinds[i];
                let compatible = match (expected_kind, arg_kind) {
                    (ValueKind::Number, ValueKind::Number) => true,
                    (ValueKind::Bool, ValueKind::Bool) => true,
                    (ValueKind::Nil, ValueKind::Nil) => true,
                    (ValueKind::String, ValueKind::String) => true,
                    (ValueKind::Table, ValueKind::Table) => true,
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
                // Phase 2.5c-full Commit 3c (ADR 0083 supersede 0044):
                // closure-with-upvalues args no longer escape-reject
                // on the function_names path either — see local-resolve
                // path above for the reasoning.
            }
            // Phase 2.5c-min (ADR 0037): if the callee captured
            // upvalues, append them as extra arguments. The
            // captured value is reloaded at each call site by
            // referencing the outer `LocalId` recorded during the
            // closure's lowering — equivalent to a snapshot taken
            // when the closure expression was evaluated, since the
            // outer slot is what was current at that moment and
            // `lower_call` runs after FunctionExpr lowering for
            // sibling closures. (Commit 3b will retire this in
            // favour of cell-ptr-first ABI.)
            //
            // Phase 2.5c-full Commit 3b prep (ADR 0083): record
            // `holding_local` on the Callee::User so the upcoming
            // codegen cutover can locate the binding's slot. After
            // the prep-fix (synthetic FunctionDef-locals), the
            // function_names fallback only fires when the name
            // isn't visible in any outer scope as a local —
            // typically self-recursion (Function-kind upvalue
            // rejection forces fall-through here for the
            // capturing fn's own body) or a sibling/forward call
            // that crossed a fn boundary.
            //
            // Phase 2.5c-full Commit 3b prep fix (ADR 0083): the
            // mutual-capturing-recursion check happens in a
            // post-pass after every body is lowered (because the
            // target's `upvalues` field isn't fully populated at
            // call-site lowering time when the target's body is
            // processed later in source order). Rejection lives
            // in `lower()` post-pass; here we just record
            // `holding_local` for codegen consumption.
            // Phase 2.5c-full Commit 3b body atomic Step 2 (ADR 0083):
            // the legacy ADR 0037 trailing-upvalue ABI is gone.
            // codegen prepends the closure cell ptr (loaded via
            // `holding_local` or the recursion shortcut) and the
            // body unpacks `cell.upvalue_box[i]` at entry.
            let holding_local = self.resolve(name);
            return Ok(HirExprKind::Call {
                callee: Callee::User { fid, holding_local },
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
        // Phase 2.8b (ADR 0032): `print` is the one variadic builtin
        // — accepts any arity ≥ 0. Phase 2.7m (ADR 0051): `assert`
        // takes 1 *or* 2 args (the optional second is a String
        // failure-message). Every other builtin keeps its fixed
        // arity from `Builtin::arity()`.
        if matches!(builtin, Builtin::Assert) {
            if args.is_empty() || args.len() > 2 {
                return Err(HirError::ArityMismatch {
                    builtin: name.clone(),
                    expected: 1,
                    actual: args.len(),
                    offset: whole.span.start,
                });
            }
        } else if !matches!(builtin, Builtin::Print) {
            let arity = builtin.arity();
            if args.len() != arity {
                return Err(HirError::ArityMismatch {
                    builtin: name.clone(),
                    expected: arity,
                    actual: args.len(),
                    offset: whole.span.start,
                });
            }
        }
        let lowered_args = args
            .iter()
            .map(|a| self.lower_expr(a))
            .collect::<Result<Vec<_>, _>>()?;
        // Phase 2.5b: builtin args may be Number/Bool/Nil but never a
        // Function value (function values cannot be printed or otherwise
        // observed as values yet). Reject explicitly.
        for arg in &lowered_args {
            let k = infer_kind(arg, &self.locals, &self.functions);
            // Phase 2.7f (ADR 0029) / 2.7n (ADR 0052): `type(f)`
            // and `tostring(f)` both legitimately accept a Function
            // value. Phase 2.8e-iter-next (ADR 0081): `next(t, fn)`
            // joins the allow-list because Function values are
            // valid hash keys (Lua spec §3.4.5). Every other call
            // site keeps treating Function-as-value as a hard
            // error.
            if let ValueKind::Function(_) = k
                && !matches!(builtin, Builtin::Type | Builtin::ToString | Builtin::Next)
            {
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
            // Phase 2.7e (ADR 0028): `tonumber(x)` only accepts
            // Number or String. Other kinds reject as TypeMismatch.
            if matches!(builtin, Builtin::ToNumber)
                && !matches!(k, ValueKind::Number | ValueKind::String)
            {
                return Err(HirError::TypeMismatch {
                    op: "tonumber".to_owned(),
                    lhs_kind: "number or string".to_owned(),
                    rhs_kind: k.name().to_owned(),
                    offset: arg.span.start,
                });
            }
            // Phase 2.7g (ADR 0030): `assert(cond, [msg])` — first
            // arg must be Bool. Phase 2.7m (ADR 0051): the optional
            // second arg, when present, must be String. Use the
            // arg's index in `lowered_args` to dispatch.
            if matches!(builtin, Builtin::Assert) {
                let arg_idx = lowered_args
                    .iter()
                    .position(|a| std::ptr::eq(a as *const _, arg as *const _))
                    .unwrap_or(0);
                let expected = if arg_idx == 0 {
                    ValueKind::Bool
                } else {
                    ValueKind::String
                };
                if k != expected {
                    return Err(HirError::TypeMismatch {
                        op: "assert".to_owned(),
                        lhs_kind: expected.name().to_owned(),
                        rhs_kind: k.name().to_owned(),
                        offset: arg.span.start,
                    });
                }
            }
            // Phase 2.7h (ADR 0033): `error(msg)` requires a String
            // operand. Lua's table-as-message form is deferred
            // until tables exist.
            if matches!(builtin, Builtin::Error) && k != ValueKind::String {
                return Err(HirError::TypeMismatch {
                    op: "error".to_owned(),
                    lhs_kind: "string".to_owned(),
                    rhs_kind: k.name().to_owned(),
                    offset: arg.span.start,
                });
            }
            // Phase 2.8e-iter-next (ADR 0081): `next(t, k)` requires
            // arg 0 = Table, arg 1 = TaggedValue / Nil / Number /
            // Bool / String / Function / Table. Anything that can
            // legally be a hash key qualifies as `prev_k`. Function
            // values pass the FunctionUsedAsValue gate above only
            // for Type/ToString — `next` joins that allow-list.
            if matches!(builtin, Builtin::Next) {
                let arg_idx = lowered_args
                    .iter()
                    .position(|a| std::ptr::eq(a as *const _, arg as *const _))
                    .unwrap_or(0);
                if arg_idx == 0 && k != ValueKind::Table {
                    return Err(HirError::TypeMismatch {
                        op: "next".to_owned(),
                        lhs_kind: "table".to_owned(),
                        rhs_kind: k.name().to_owned(),
                        offset: arg.span.start,
                    });
                }
            }
        }
        Ok(HirExprKind::Call {
            callee: Callee::Builtin(builtin),
            args: lowered_args,
        })
    }

    /// Phase 2.6+-methods (ADR 0092): MethodDef chokepoint desugar.
    /// Builds the effective params (prepending `self` when `is_colon`),
    /// seeds `external_kinds[0] = TaggedValue` so the body lowers
    /// `self.field` reads through the TaggedValue Index path, and
    /// registers the anon function via the FunctionExpr-style flow.
    /// Emits `IndexAssign(target=Ident(receiver), key=Str(method),
    /// value=FunctionRef(synth_fid))` — reusing the existing
    /// LIC-2.6c-tag-locals-fn-indirect-1 check at
    /// `IndexAssign`'s function-value branch (hetero-return methods
    /// trip that, surfacing as `TypeMismatch`; documented as ADR 0092
    /// non-goal carry-over).
    fn lower_method_def(
        &mut self,
        receiver: &str,
        method: &str,
        is_colon: bool,
        params: &[String],
        body: &[Stmt],
        stmt_span: Span,
    ) -> Result<HirStmt, HirError> {
        let effective_params: Vec<String> = if is_colon {
            let mut p = Vec::with_capacity(params.len() + 1);
            p.push("self".to_owned());
            p.extend_from_slice(params);
            p
        } else {
            params.to_vec()
        };
        // Phase 2.6+-method-arg-refine (ADR 0093): the FuncId was
        // pre-allocated in Pass 1 via `register_method_signature` and
        // the placeholder is already in `self.functions`. Look it up
        // by `(receiver, method)` so Pass 1.5's refinement of param
        // kinds carries forward into `external_kinds` below.
        let id = self.method_funcs[&(receiver.to_owned(), method.to_owned())];
        let mut external_kinds: Vec<ValueKind> =
            self.functions[id.0].params.iter().map(|p| p.kind).collect();
        if is_colon {
            // ADR 0092 MVP: `self` kind = Table so the dispatch
            // arg-kind matching (ADR 0082 strict-equal) succeeds for
            // the typical receiver `obj` where `local obj = {}` makes
            // `obj` a Table-kind local. ADR 0093: refinement may have
            // pushed self toward another kind, but the policy seeds
            // Table here unconditionally to maintain ADR 0092's
            // contract. Future ADR (metatables / __index) will widen
            // `self` to TaggedValue once dispatch permits arg widening.
            external_kinds[0] = ValueKind::Table;
        }
        let outer_visible = self.outer_visible_snapshot();
        let mut fn_ctx = LowerCtx::for_function(
            &self.function_names,
            &self.method_funcs,
            &self.functions,
            &effective_params,
            body,
            &external_kinds,
            outer_visible,
            id,
        );
        let body_hir = fn_ctx.lower_function_body(body)?;
        let ret_kinds = fn_ctx.in_function_ret_kinds.unwrap_or_default();
        self.functions[id.0].params = fn_ctx.locals[..effective_params.len()].to_vec();
        self.functions[id.0].upvalues = fn_ctx.upvalues;
        self.functions[id.0].locals = fn_ctx.locals;
        self.functions[id.0].body = body_hir;
        self.functions[id.0].ret_kinds = ret_kinds;
        // Build the IndexAssign at HIR level. Target is the receiver
        // local (must be Table-kind; the existing IndexAssign target
        // check at lower_stmt_match_arms enforces this if we route
        // through synthetic AST, so we synthesize and re-lower).
        let target_ast = Expr::new(ExprKind::Ident(receiver.to_owned()), stmt_span);
        let target_hir = self.lower_expr(&target_ast)?;
        let target_kind = infer_kind(&target_hir, &self.locals, &self.functions);
        if target_kind != ValueKind::Table {
            return Err(HirError::TypeMismatch {
                op: "method-def receiver".to_owned(),
                lhs_kind: "table".to_owned(),
                rhs_kind: target_kind.name().to_owned(),
                offset: stmt_span.start,
            });
        }
        let key_hir = HirExpr {
            kind: HirExprKind::Str(method.to_owned()),
            span: stmt_span,
        };
        let value_hir = HirExpr {
            kind: HirExprKind::FunctionRef(id),
            span: stmt_span,
        };
        // Reuse the LIC-2.6c-tag-locals-fn-indirect-1 check at
        // IndexAssign — hetero-return methods (ret_kinds widening to
        // TaggedValue) are rejected here, surfacing as TypeMismatch.
        if self.functions[id.0]
            .ret_kinds
            .iter()
            .any(|k| matches!(k, ValueKind::TaggedValue))
        {
            return Err(HirError::TypeMismatch {
                op: "method-def value (function with tagged return)".to_owned(),
                lhs_kind: "function with non-tagged return".to_owned(),
                rhs_kind: "function returning TaggedValue".to_owned(),
                offset: stmt_span.start,
            });
        }
        Ok(HirStmt {
            kind: HirStmtKind::IndexAssign {
                target: target_hir,
                key: key_hir,
                value: value_hir,
            },
            span: stmt_span,
        })
    }

    /// Phase 2.6+-callee-norm (ADR 0091 v2, renamed by ADR 0092):
    /// effectful executor that lowers an arbitrary expression once
    /// and binds the result to a synthetic `__callee_<N>` TaggedValue
    /// local. Pushes a `LocalInit` pre-stmt into `pending_pre_stmts`;
    /// the surrounding `lower_stmt` drains it at the stmt boundary.
    /// Returns the synthetic LocalId so the caller can recurse
    /// through `lower_call`'s DirectIdent → IndirectDispatch path
    /// (callee materialisation) or thread the local as a method
    /// receiver argument (ADR 0092 MethodCall desugar).
    ///
    /// ADR 0063's `widen_index_for_local_init` is applied so an
    /// Index value rewrites to `IndexTagged` and the synthetic local
    /// widens to TaggedValue. The helper is idempotent on every
    /// other shape, so non-Index expressions (e.g. an `Ident` cell
    /// already widened upstream) pass through unchanged.
    fn materialize_to_synth_local(
        &mut self,
        expr: &Expr,
        synth_span: Span,
    ) -> Result<LocalId, HirError> {
        let value_hir = self.lower_expr(expr)?;
        let widened = widen_index_for_local_init(value_hir);
        let seq = self.callee_seq;
        self.callee_seq += 1;
        let synth_name = format!("__callee_{seq}");
        let synth_id = self.declare_local(synth_name, ValueKind::TaggedValue);
        self.pending_pre_stmts.push(HirStmt {
            kind: HirStmtKind::LocalInit {
                id: synth_id,
                value: widened,
            },
            span: synth_span,
        });
        Ok(synth_id)
    }
}

/// Phase 2.6+-callee-norm (ADR 0091 v2): pure classifier for the
/// callee position of a `Call` AST node. Decides which lowering path
/// applies — the existing Ident-callee path or the new Index-callee
/// pre-bind path. Anything else surfaces as the existing
/// `HirError::UnsupportedCall` (future ADRs can broaden coverage,
/// e.g. `(expr_returning_fn)()`, but ADR 0091 v2 keeps the MVP tight
/// per codex post-abort guideline #2).
enum CalleeForm<'a> {
    /// `name(args)` — Ident callee. Routed through the existing
    /// Builtin / User / Indirect / IndirectDispatch resolution.
    DirectIdent,
    /// `target[key](args)` or `target.field(args)` — Index callee.
    /// HIR pre-binds the Index result to a synthetic local and
    /// recurses with that local as the new callee.
    IndexCallee { target: &'a Expr, key: &'a Expr },
}

/// Phase 2.6+-methods (ADR 0092): pure receiver-shape walker.
/// Methods are sugar at the call site but the receiver-once
/// evaluation invariant restricts which receiver shapes the MVP
/// accepts. The walker descends recursively into `Index { target, key }`
/// and rejects any node carrying side-effects or complex value
/// production (`Call`, `MethodCall`, `FunctionExpr`, `BinOp`,
/// `UnaryOp`). Permitted: `Ident`, `Number`, `Str`, `Bool`, `Nil`,
/// `TableCtor`, plus chained `[]` / `.` over the above. A future
/// ADR can broaden coverage once side-effect ordering of compound
/// receivers is reconciled with `pending_pre_stmts`.
fn check_method_receiver_shape(expr: &Expr) -> Result<(), HirError> {
    match &expr.kind {
        ExprKind::Call { .. }
        | ExprKind::MethodCall { .. }
        | ExprKind::FunctionExpr { .. }
        | ExprKind::BinOp { .. }
        | ExprKind::UnaryOp { .. } => Err(HirError::ComplexMethodReceiver {
            offset: expr.span.start,
        }),
        ExprKind::Index { target, key } => {
            check_method_receiver_shape(target)?;
            check_method_receiver_shape(key)
        }
        ExprKind::Number(_)
        | ExprKind::Str(_)
        | ExprKind::Bool(_)
        | ExprKind::Nil
        | ExprKind::Ident(_)
        | ExprKind::Table(_) => Ok(()),
    }
}

fn classify_callee_form(callee: &Expr) -> Result<CalleeForm<'_>, HirError> {
    match &callee.kind {
        ExprKind::Ident(_) => Ok(CalleeForm::DirectIdent),
        ExprKind::Index { target, key } => Ok(CalleeForm::IndexCallee {
            target: target.as_ref(),
            key: key.as_ref(),
        }),
        _ => Err(HirError::UnsupportedCall {
            offset: callee.span.start,
        }),
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
    fn lower_assign_to_undefined_name_inside_function_errors() {
        // Phase 2.0a (ADR 0048): top-level `y = 1` now auto-declares.
        // Inside a function body the unresolved-name path still
        // errors — auto-declare is restricted to chunk top level.
        let err = lower_src(
            "local function f()
  y = 1
end
f()",
        )
        .expect_err("assign-to-undef inside fn must fail");
        match err {
            HirError::UndefinedName { name, .. } => assert_eq!(name, "y"),
            other => panic!("expected UndefinedName, got {other:?}"),
        }
    }

    #[test]
    fn lower_top_level_bare_assign_now_auto_declares_after_2_0a() {
        // Boundary: `y = 1` at chunk scope is now legal — it
        // declares `y` as a chunk-level local.
        let hir = lower_src("y = 1\nprint(y)").expect("bare assign must lower in 2.0a");
        assert_eq!(hir.locals.len(), 1);
        assert_eq!(hir.locals[0].name, "y");
        assert_eq!(hir.locals[0].kind, ValueKind::Number);
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
    fn lower_print_zero_arg_lowers_after_2_8b() {
        // Phase 2.8b (ADR 0032): `print()` is now legal — outputs
        // a bare newline. Pre-2.8b this surfaced as `ArityMismatch`.
        assert!(lower_src("print()").is_ok());
    }

    // -----------------------------------------------------------
    // Phase 2.7h (ADR 0033) — error(msg) builtin.
    // -----------------------------------------------------------

    #[test]
    fn lower_error_with_string_arg_resolves_to_error_builtin() {
        let hir = lower_src("error(\"oops\")").expect("error(string) must lower");
        let HirStmtKind::ExprStmt(call) = &hir.stmts[0].kind else {
            panic!("expected ExprStmt");
        };
        let HirExprKind::Call { callee, args } = &call.kind else {
            panic!("expected Call");
        };
        assert!(matches!(callee, Callee::Builtin(Builtin::Error)));
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn lower_error_with_number_arg_is_static_error() {
        let err = lower_src("error(42)").expect_err("error(number) must reject");
        assert!(matches!(err, HirError::TypeMismatch { .. }));
    }

    #[test]
    fn lower_error_with_bool_arg_is_static_error() {
        let err = lower_src("error(true)").expect_err("error(bool) must reject");
        assert!(matches!(err, HirError::TypeMismatch { .. }));
    }

    #[test]
    fn lower_error_with_zero_args_is_arity_mismatch() {
        let err = lower_src("error()").expect_err("error() must reject");
        assert!(matches!(err, HirError::ArityMismatch { .. }));
    }

    // -----------------------------------------------------------
    // Phase 2.4b — `repeat ... until cond` (ADR 0035).
    // -----------------------------------------------------------

    #[test]
    fn lower_repeat_until_lowers_to_repeat_variant() {
        let hir =
            lower_src("local i = 0\nrepeat i = i + 1 until i == 3").expect("repeat must lower");
        // The repeat lowers to a top-level Repeat (no break flag).
        let HirStmtKind::Repeat { body, break_id, .. } = &hir.stmts[1].kind else {
            panic!("expected Repeat at stmts[1], got {:?}", hir.stmts[1].kind);
        };
        assert!(break_id.is_none(), "no break in body → no flag");
        assert!(!body.is_empty());
    }

    #[test]
    fn lower_repeat_with_break_allocates_break_id() {
        let src = "repeat if true then break end until false";
        let hir = lower_src(src).expect("repeat+break must lower");
        // With a break, Repeat is wrapped in an enclosing Block
        // that initialises the `_broken` flag, mirroring the
        // existing While/ForNumeric pattern.
        let HirStmtKind::Block { stmts } = &hir.stmts[0].kind else {
            panic!("expected enclosing Block for repeat+break");
        };
        assert!(matches!(stmts[0].kind, HirStmtKind::LocalInit { .. }));
        let HirStmtKind::Repeat { break_id, .. } = &stmts[1].kind else {
            panic!("expected inner Repeat");
        };
        assert!(break_id.is_some());
    }

    #[test]
    fn lower_repeat_cond_sees_body_local() {
        // Lua 5.4 §3.3.4: the until-cond is evaluated inside the
        // body's scope and may reference body-introduced locals.
        let src = "repeat local x = 5 until x == 5";
        let hir = lower_src(src).expect("until-cond must see body local");
        let HirStmtKind::Repeat { cond, .. } = &hir.stmts[0].kind else {
            panic!("expected Repeat");
        };
        // The cond resolves `x` to the body's local id (LocalId(0)).
        let HirExprKind::BinOp { lhs, .. } = &cond.kind else {
            panic!("expected BinOp in cond");
        };
        assert!(matches!(lhs.kind, HirExprKind::Local(LocalId(0))));
    }

    #[test]
    fn lower_repeat_cond_referencing_undefined_name_errors() {
        let err =
            lower_src("repeat until missing_var").expect_err("undefined cond name must reject");
        assert!(matches!(err, HirError::UndefinedName { .. }));
    }

    // -----------------------------------------------------------
    // Phase 2.5f — nested `local function` definitions (ADR 0036).
    // -----------------------------------------------------------

    #[test]
    fn lower_nested_local_function_does_not_panic() {
        // Pre-2.5f this hit `unimplemented!`. The nested function
        // is hoisted into the chunk's `functions` table just like
        // a top-level one.
        let src = "local function outer()
  local function inner()
    return 7
  end
  return inner()
end";
        let hir = lower_src(src).expect("nested fn def must lower");
        // Two functions registered: outer + inner.
        assert_eq!(hir.functions.len(), 2);
        let names: Vec<&str> = hir.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"outer"));
        assert!(names.contains(&"inner"));
    }

    #[test]
    fn lower_nested_local_function_supports_recursion() {
        // The nested function's name is in scope inside its own body.
        let src = "local function outer(x)
  local function fib(n)
    if n < 2 then return n end
    return fib(n - 1) + fib(n - 2)
  end
  return fib(x)
end";
        assert!(lower_src(src).is_ok(), "fib recursion must lower");
    }

    #[test]
    fn lower_nested_function_referencing_outer_local_errors() {
        // Phase 2.5c-min lifts this for `local function`. A nested
        // `local function` body that references an outer Number
        // local is now valid (capture-by-value).
        let src = "local function outer(x)
  local function inner()
    return x
  end
  return inner()
end";
        assert!(lower_src(src).is_ok(), "Phase 2.5c-min: capture must lower");
    }

    // -----------------------------------------------------------
    // Phase 2.5c-min — capture-by-value closures (ADR 0037).
    // -----------------------------------------------------------

    #[test]
    fn lower_anonymous_closure_records_upvalue() {
        let src = "local x = 5
local f = function() return x end";
        let hir = lower_src(src).expect("anon closure must lower");
        // The anon function (single FuncId 0) should record one
        // upvalue referencing the outer chunk's `x`.
        assert_eq!(hir.functions.len(), 1);
        let f = &hir.functions[0];
        assert_eq!(f.upvalues.len(), 1);
        assert_eq!(f.upvalues[0].name, "x");
        assert_eq!(f.upvalues[0].kind, ValueKind::Number);
    }

    #[test]
    fn lower_closure_with_no_outer_reference_has_no_upvalues() {
        let src = "local f = function() return 42 end";
        let hir = lower_src(src).expect("must lower");
        assert!(hir.functions[0].upvalues.is_empty());
    }

    #[test]
    fn lower_closure_capturing_function_is_static_error() {
        // Phase 2.5c.2 (ADR 0043) opened captures for Number/Bool/
        // Nil/String. Function-kind captures still reject — codegen
        // has no path to wire a function value through the
        // alloca-backed inner slot.
        let src = "local g = function(x) return x + 1 end
local f = function() return g end";
        let err = lower_src(src).expect_err("Function capture must reject");
        assert!(matches!(err, HirError::TypeMismatch { .. }));
    }

    #[test]
    fn lower_closure_capturing_bool_now_succeeds_after_2_5c2() {
        // Boundary documentation: Bool was rejected in 2.5c-min and
        // is allowed in 2.5c.2. The captured local appears in the
        // closure's upvalue list.
        let src = "local b = true
local f = function() return b end";
        let hir = lower_src(src).expect("Bool capture must lower in 2.5c.2");
        assert_eq!(hir.functions[0].upvalues.len(), 1);
        assert_eq!(hir.functions[0].upvalues[0].kind, ValueKind::Bool);
    }

    #[test]
    fn lower_closure_capturing_unknown_name_errors() {
        // No outer scope match → UndefinedName, same as before.
        let src = "local f = function() return missing end";
        let err = lower_src(src).expect_err("undefined upvalue must reject");
        assert!(matches!(err, HirError::UndefinedName { .. }));
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
        assert!(hir.functions[0].ret_kinds.is_empty());
    }

    #[test]
    fn lower_chunk_top_level_call_carries_holding_local_some() {
        // Phase 2.5c-full Commit 3b prep fix (ADR 0083):
        // `local function f` declares a synthetic chunk local for `f`,
        // so the chunk-level `print(f())` call resolves through the
        // local-resolve path with `holding_local = Some(_)`.
        let hir = lower_src("local function f() return 1 end\nprint(f())").expect("must lower");
        // stmts[0] = synthetic LocalInit for f
        // stmts[1] = print(f()) ExprStmt
        let HirStmtKind::ExprStmt(call) = &hir.stmts[1].kind else {
            panic!("expected ExprStmt at idx 1, got {:?}", &hir.stmts[1].kind);
        };
        let HirExprKind::Call { args, .. } = &call.kind else {
            panic!("expected Call");
        };
        let HirExprKind::Call { callee: inner, .. } = &args[0].kind else {
            panic!("expected nested Call");
        };
        match inner {
            Callee::User {
                fid: FuncId(0),
                holding_local,
            } => {
                assert!(
                    holding_local.is_some(),
                    "chunk top-level call to local function should carry holding_local Some, \
                     got None"
                );
            }
            other => panic!("expected Callee::User, got {other:?}"),
        }
    }

    #[test]
    fn lower_self_recursion_carries_holding_local_none() {
        // Phase 2.5c-full Commit 3b prep fix (ADR 0083): inside the
        // capturing fn's own body, `self.resolve(name)` returns None
        // (the body's locals don't include the fn's own name) and
        // the call falls through to `function_names`, yielding
        // `holding_local = None`. Codegen's `emit_call_user_with_cell`
        // recognises this as the recursion shortcut.
        let hir = lower_src(
            "local function fact(n) if n == 0 then return 1 end
return fact(n - 1) end
print(fact(3))",
        )
        .expect("must lower");
        // Walk fact's body to find the self-call.
        let fact = &hir.functions[0];
        let mut found = false;
        fn scan(stmts: &[HirStmt], found: &mut bool) {
            for stmt in stmts {
                match &stmt.kind {
                    HirStmtKind::Assign { value, .. } | HirStmtKind::LocalInit { value, .. } => {
                        scan_expr(value, found);
                    }
                    HirStmtKind::ExprStmt(e) => scan_expr(e, found),
                    HirStmtKind::Block { stmts } => scan(stmts, found),
                    HirStmtKind::If {
                        then_body,
                        elifs,
                        else_body,
                        ..
                    } => {
                        scan(then_body, found);
                        for (_, b) in elifs {
                            scan(b, found);
                        }
                        if let Some(eb) = else_body {
                            scan(eb, found);
                        }
                    }
                    _ => {}
                }
            }
        }
        fn scan_expr(e: &HirExpr, found: &mut bool) {
            if let HirExprKind::Call { callee, args } = &e.kind {
                if let Callee::User {
                    fid: FuncId(0),
                    holding_local,
                } = callee
                    && holding_local.is_none()
                {
                    *found = true;
                }
                for a in args {
                    scan_expr(a, found);
                }
            }
            if let HirExprKind::BinOp { lhs, rhs, .. } = &e.kind {
                scan_expr(lhs, found);
                scan_expr(rhs, found);
            }
        }
        scan(&fact.body, &mut found);
        assert!(
            found,
            "self-recursion `fact(n - 1)` inside fact's body should carry \
             holding_local = None (function_names fallback for the fn's own name)"
        );
    }

    #[test]
    fn lower_function_call_resolves_to_user_func_id() {
        let hir = lower_src("local function f() return 1 end\nprint(f())").expect("must lower");
        // Phase 2.5c-full Commit 3b prep fix (ADR 0083): chunk now
        // begins with a synthetic `LocalInit { id, FunctionRef(0) }`
        // for `local function f` (idx 0), so the print ExprStmt is
        // at idx 1.
        let HirStmtKind::ExprStmt(call) = &hir.stmts[1].kind else {
            panic!("expected ExprStmt at idx 1");
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
        assert!(matches!(inner, Callee::User { fid: FuncId(0), .. }));
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
        // inspect the function's ret_kinds here.
        assert_eq!(hir.functions[0].ret_kinds, vec![ValueKind::Number]);
        assert!(!body.is_empty());
    }

    #[test]
    fn lower_function_body_locals_are_independent_of_outer() {
        // Outer `x` is a Number; the function body's `x` parameter is
        // its own slot, not the outer one. The function shouldn't see
        // the outer x at all.
        let hir = lower_src("local x = 1\nlocal function f(x) return x end\nprint(f(2))")
            .expect("must lower");
        // Phase 2.5c-full Commit 3b prep fix (ADR 0083): chunk now
        // declares a synthetic local "f" alongside the explicit `x`,
        // so the chunk has two locals. Synthetic FunctionDef locals
        // are declared in pass 1.5 (before pass 2 lowering of
        // explicit `local x`), so "f" lands at idx 0 and "x" at
        // idx 1. Local IDs are scope-relative slot indices; the
        // numeric order is irrelevant for correctness as long as
        // pass 2's HirStmt construction references the right ID.
        assert_eq!(hir.locals.len(), 2);
        let names: Vec<&str> = hir.locals.iter().map(|l| l.name.as_str()).collect();
        assert!(names.contains(&"x"));
        assert!(names.contains(&"f"));
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
        assert!(matches!(inner, Callee::User { fid: FuncId(0), .. }));
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

    // -----------------------------------------------------------
    // Phase 2.5b.3 — functions as return values (ADR 0019).
    // -----------------------------------------------------------

    #[test]
    fn lower_function_returning_function_has_function_ret_kind() {
        let src = "local function f(x) return x end\nlocal function get_f() return f end";
        let hir = lower_src(src).expect("Phase 2.5b.3: returning a function must lower");
        let get_f = hir
            .functions
            .iter()
            .find(|fn_| fn_.name == "get_f")
            .expect("get_f present");
        assert_eq!(get_f.ret_kinds, vec![ValueKind::Function(1)]);
    }

    #[test]
    fn lower_local_bound_to_call_returning_function_has_function_kind() {
        let src = concat!(
            "local function f(x) return x * 2 end\n",
            "local function get_f() return f end\n",
            "local g = get_f()\n",
        );
        let hir = lower_src(src).expect("Phase 2.5b.3: local from function-returning call");
        let g_local = hir
            .locals
            .iter()
            .find(|l| l.name == "g")
            .expect("g local present");
        assert_eq!(g_local.kind, ValueKind::Function(1));
        assert_eq!(g_local.func_id, None, "call result has no static FuncId");
    }

    #[test]
    fn lower_anon_function_directly_returned() {
        let src = "local function make() return function(x) return x + 1 end end";
        let hir = lower_src(src).expect("Phase 2.5b.3: returning anon fn must lower");
        let make_fn = hir
            .functions
            .iter()
            .find(|fn_| fn_.name == "make")
            .expect("make present");
        assert_eq!(make_fn.ret_kinds, vec![ValueKind::Function(1)]);
        // The anon function must also have been hoisted into the
        // chunk's function table so codegen can emit it.
        assert!(
            hir.functions.iter().any(|fn_| fn_.name.is_empty()),
            "anon function must be hoisted into chunk.functions, got names: {:?}",
            hir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn lower_arity_mismatch_on_returned_function_errors() {
        // get_f returns a 1-arg function; calling g with 2 args must reject.
        let src = concat!(
            "local function f(x) return x end\n",
            "local function get_f() return f end\n",
            "local g = get_f()\n",
            "print(g(1, 2))\n",
        );
        let result = lower_src(src);
        assert!(
            result.is_err(),
            "arity mismatch on returned function must reject"
        );
    }

    // -----------------------------------------------------------
    // Phase 2.5e — Bool/Nil params/returns (ADR 0020).
    // -----------------------------------------------------------

    #[test]
    fn lower_function_returning_bool_has_bool_ret_kind() {
        let src = "local function pos(x) return x > 0 end";
        let hir = lower_src(src).expect("Phase 2.5e: Bool return must lower");
        let pos = &hir.functions[0];
        assert_eq!(pos.ret_kinds, vec![ValueKind::Bool]);
    }

    #[test]
    fn lower_function_returning_nil_has_nil_ret_kind() {
        let src = "local function n() return nil end";
        let hir = lower_src(src).expect("Phase 2.5e: Nil return must lower");
        let n = &hir.functions[0];
        assert_eq!(n.ret_kinds, vec![ValueKind::Nil]);
    }

    #[test]
    fn lower_inconsistent_bool_number_returns_widens_to_tagged() {
        // Phase 2.6c-tag-locals-fn (ADR 0074): inconsistent kinds
        // at the same return position used to reject; now they
        // widen to TaggedValue so the function value can flow
        // into a heterogeneous local at the call site.
        let src = concat!(
            "local function widened(x)\n",
            "  if x > 0 then return true end\n",
            "  return 42\n",
            "end\n",
        );
        let hir =
            lower_src(src).expect("Phase 2.6c-tag-locals-fn: heterogeneous returns must widen");
        let f = &hir.functions[0];
        assert_eq!(f.ret_kinds, vec![ValueKind::TaggedValue]);
    }

    #[test]
    fn lower_call_site_infers_bool_param() {
        // negate(true) at the call site marks `b` as Bool.
        let src = "local function negate(b) return not b end\nprint(negate(true))";
        let hir = lower_src(src).expect("Phase 2.5e: Bool param inference must lower");
        let neg = &hir.functions[0];
        assert_eq!(neg.params[0].kind, ValueKind::Bool);
        assert_eq!(neg.ret_kinds, vec![ValueKind::Bool]);
    }

    #[test]
    fn lower_call_site_infers_nil_param() {
        // is_nil(nil): the call site infers x as Nil. Body's `x == nil`
        // folds to a constant Bool(true) at HIR-time.
        let src = "local function is_nil(x) return x == nil end\nprint(is_nil(nil))";
        let hir = lower_src(src).expect("Phase 2.5e: Nil param inference must lower");
        let f = &hir.functions[0];
        assert_eq!(f.params[0].kind, ValueKind::Nil);
    }

    // -----------------------------------------------------------
    // Phase 2.5d — multi-return / multi-binding (ADR 0021).
    // -----------------------------------------------------------

    #[test]
    fn lower_multi_return_collects_all_kinds() {
        let src = "local function pair() return 1, 2 end";
        let hir = lower_src(src).expect("Phase 2.5d: multi-return must lower");
        let pair = &hir.functions[0];
        assert_eq!(pair.ret_kinds, vec![ValueKind::Number, ValueKind::Number]);
    }

    #[test]
    fn lower_local_multi_from_call_emits_multi_assign() {
        let src = "local function pair() return 1, 2 end\nlocal a, b = pair()";
        let hir = lower_src(src).expect("Phase 2.5d: local a,b = pair() must lower");
        // Phase 2.5c-full Commit 3b prep fix (ADR 0083): chunk now
        // begins with a synthetic `LocalInit` for `local function pair`
        // (idx 0); the `local a, b = pair()` MultiAssignFromCall
        // shifts to idx 1.
        match &hir.stmts[1].kind {
            HirStmtKind::MultiAssignFromCall { dst_ids, .. } => {
                assert_eq!(dst_ids.len(), 2);
            }
            other => panic!("expected MultiAssignFromCall at idx 1, got {other:?}"),
        }
    }

    #[test]
    fn lower_local_multi_from_values_emits_block_of_inits() {
        let src = "local a, b = 1, 2";
        let hir = lower_src(src).expect("Phase 2.5d: parallel binding must lower");
        match &hir.stmts[0].kind {
            HirStmtKind::Block { stmts } => {
                assert_eq!(stmts.len(), 2);
                assert!(matches!(stmts[0].kind, HirStmtKind::LocalInit { .. }));
                assert!(matches!(stmts[1].kind, HirStmtKind::LocalInit { .. }));
            }
            other => panic!("expected Block of LocalInits, got {other:?}"),
        }
    }

    #[test]
    fn lower_arity_mismatch_in_multi_call_errors() {
        // 1 LHS, 2-result call.
        let src = "local function pair() return 1, 2 end\nlocal a = pair()";
        // This actually parses as `Local { name: "a", value: Call }` —
        // single name, single value, the call's first result is taken.
        // To exercise the multi-call arity check, we need ≥2 LHS.
        let _ok = lower_src(src).expect("1-binding from multi-call truncates per Lua");

        // 3 LHS, 2-result call → arity mismatch.
        let bad = "local function pair() return 1, 2 end\nlocal a, b, c = pair()";
        assert!(lower_src(bad).is_err());
    }

    #[test]
    fn lower_inconsistent_return_arity_errors() {
        let src = concat!(
            "local function bad(x)\n",
            "  if x > 0 then return 1, 2 end\n",
            "  return 3\n",
            "end\n",
        );
        assert!(lower_src(src).is_err());
    }
}
