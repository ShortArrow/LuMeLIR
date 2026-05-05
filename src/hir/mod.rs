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
    LocalId, LocalInfo, UpvalueInfo,
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
            // User function: look up its declared return kind. Phase
            // 2.5a forces this to Number when present; void calls
            // never appear in expression position legally.
            // For multi-return callees in expression position, Lua
            // truncates to the first result. Phase 2.5d (ADR 0021).
            Callee::User(FuncId(id)) => functions[*id]
                .ret_kinds
                .first()
                .copied()
                .unwrap_or(ValueKind::Number),
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
        StmtKind::While { .. } | StmtKind::ForNumeric { .. } | StmtKind::Repeat { .. } => false,
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
            StmtKind::While { body, .. } | StmtKind::ForNumeric { body, .. } => {
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
        kinds: &mut Vec<Vec<ValueKind>>,
        seen: &mut Vec<bool>,
    ) {
        match &s.kind {
            StmtKind::Local { value, .. } | StmtKind::Assign { value, .. } => {
                visit_expr(value, names, kinds, seen);
            }
            StmtKind::ExprStmt(e) => visit_expr(e, names, kinds, seen),
            StmtKind::Return { value: Some(e) } => visit_expr(e, names, kinds, seen),
            StmtKind::Return { value: None } => {}
            StmtKind::Block(b) => {
                for st in b {
                    visit_stmt(st, names, kinds, seen);
                }
            }
            StmtKind::If {
                cond,
                then_body,
                elifs,
                else_body,
            } => {
                visit_expr(cond, names, kinds, seen);
                for st in then_body {
                    visit_stmt(st, names, kinds, seen);
                }
                for (c, b) in elifs {
                    visit_expr(c, names, kinds, seen);
                    for st in b {
                        visit_stmt(st, names, kinds, seen);
                    }
                }
                if let Some(b) = else_body {
                    for st in b {
                        visit_stmt(st, names, kinds, seen);
                    }
                }
            }
            StmtKind::While { cond, body } => {
                visit_expr(cond, names, kinds, seen);
                for st in body {
                    visit_stmt(st, names, kinds, seen);
                }
            }
            StmtKind::ForNumeric {
                start,
                stop,
                step,
                body,
                ..
            } => {
                visit_expr(start, names, kinds, seen);
                visit_expr(stop, names, kinds, seen);
                if let Some(s) = step {
                    visit_expr(s, names, kinds, seen);
                }
                for st in body {
                    visit_stmt(st, names, kinds, seen);
                }
            }
            StmtKind::Repeat { body, cond } => {
                for st in body {
                    visit_stmt(st, names, kinds, seen);
                }
                visit_expr(cond, names, kinds, seen);
            }
            // FunctionDef bodies are also walked — recursive calls and
            // calls into sibling top-level functions count.
            StmtKind::FunctionDef { body, .. } => {
                for st in body {
                    visit_stmt(st, names, kinds, seen);
                }
            }
            StmtKind::LocalMulti { values, .. } | StmtKind::AssignMulti { values, .. } => {
                for v in values {
                    visit_expr(v, names, kinds, seen);
                }
            }
            StmtKind::ReturnMulti { values } => {
                for v in values {
                    visit_expr(v, names, kinds, seen);
                }
            }
            StmtKind::IndexAssign { target, key, value } => {
                visit_expr(target, names, kinds, seen);
                visit_expr(key, names, kinds, seen);
                visit_expr(value, names, kinds, seen);
            }
            StmtKind::Break => {}
        }
    }

    fn visit_expr(
        e: &Expr,
        names: &HashMap<String, FuncId>,
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
                visit_expr(callee, names, kinds, seen);
                for a in args {
                    visit_expr(a, names, kinds, seen);
                }
            }
            ExprKind::BinOp { lhs, rhs, .. } => {
                visit_expr(lhs, names, kinds, seen);
                visit_expr(rhs, names, kinds, seen);
            }
            ExprKind::UnaryOp { operand, .. } => {
                visit_expr(operand, names, kinds, seen);
            }
            ExprKind::FunctionExpr { body, .. } => {
                for st in body {
                    visit_stmt(st, names, kinds, seen);
                }
            }
            _ => {}
        }
    }

    for s in chunk {
        visit_stmt(s, function_names, &mut kinds, &mut seen);
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
            StmtKind::Break => {}
            StmtKind::FunctionDef { .. } => {} // nested fn defs not in 2.5b.2
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
            })
            .collect(),
        upvalues: Vec::new(),
        locals: Vec::new(),
        body: Vec::new(),
        ret_kinds: Vec::new(),
    });
    function_names.insert(name.to_owned(), id);
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
    functions: &mut Vec<HirFunction>,
    outer_visible: HashMap<String, (LocalId, ValueKind)>,
) -> Result<(), HirError> {
    let pre_count = functions.len();
    let external_kinds: Vec<ValueKind> = functions[fid.0].params.iter().map(|p| p.kind).collect();
    let mut sub_ctx = LowerCtx::for_function(
        function_names,
        functions,
        params,
        body,
        &external_kinds,
        outer_visible,
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
            register_function_signature(name, params, &mut function_names, &mut functions);
        }
    }
    // Phase 2.5e (ADR 0020): pre-scan all call sites for top-level
    // function names, refining each function's param kinds from
    // literal arg kinds at the first observed call. Without this,
    // every param defaults to Number and Bool/Nil call args get
    // rejected by `lower_call`'s kind check.
    let arities: Vec<usize> = functions.iter().map(|f| f.params.len()).collect();
    let inferred = infer_user_function_param_kinds(chunk, &function_names, &arities);
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
    let mut ctx = LowerCtx::new(function_names.clone(), functions);
    let mut stmts = Vec::new();
    let mut funcdef_seq: usize = 0;
    for s in chunk {
        if let StmtKind::FunctionDef { params, body, .. } = &s.kind {
            let fid = FuncId(funcdef_seq);
            funcdef_seq += 1;
            let outer_visible = ctx.outer_visible_snapshot();
            lower_into_function(
                fid,
                params,
                body,
                &ctx.function_names,
                &mut ctx.functions,
                outer_visible,
            )?;
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
}

impl LowerCtx {
    fn new(function_names: HashMap<String, FuncId>, functions: Vec<HirFunction>) -> Self {
        Self {
            locals: Vec::new(),
            outer_visible: HashMap::new(),
            upvalues: Vec::new(),
            scopes: vec![HashMap::new()],
            readonly_locals: HashSet::new(),
            loop_break_targets: Vec::new(),
            function_names,
            functions,
            in_function: None,
            in_function_ret_kinds: None,
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
    fn for_function(
        function_names: &HashMap<String, FuncId>,
        functions: &[HirFunction],
        params: &[String],
        body: &[Stmt],
        external_kinds: &[ValueKind],
        outer_visible: HashMap<String, (LocalId, ValueKind)>,
    ) -> Self {
        let mut ctx = Self::new(function_names.clone(), functions.to_vec());
        ctx.outer_visible = outer_visible;
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

    /// Phase 2.5c.3 (ADR 0044): does this lowered HIR expression
    /// evaluate to a closure carrying upvalues? Returns the closure's
    /// `FuncId` on hit so callers can incorporate it into diagnostics
    /// or future analysis. A "closure with upvalues" is a Function
    /// value whose target FuncId records a non-empty `upvalues` list;
    /// such values can only be reached via direct calls
    /// (`Callee::User`) since the call-site arg-extension that
    /// threads upvalues lives only on that path.
    fn closure_with_upvalues(&self, expr: &HirExpr) -> Option<FuncId> {
        let fid = match &expr.kind {
            HirExprKind::FunctionRef(fid) => *fid,
            HirExprKind::Local(LocalId(idx)) => self.locals[*idx].func_id?,
            _ => return None,
        };
        if self.functions[fid.0].upvalues.is_empty() {
            None
        } else {
            Some(fid)
        }
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
        for s in stmts {
            if let StmtKind::FunctionDef { name, params, .. } = &s.kind {
                register_function_signature(
                    name,
                    params,
                    &mut self.function_names,
                    &mut self.functions,
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
                // Phase 2.6c-tag-locals (ADR 0063): TaggedValue
                // is only ever produced by `lower_stmt` rewriting a
                // table read into `IndexTagged`; ret-value slots
                // never carry it (function returns are still
                // Number-only at this sub-phase).
                ValueKind::TaggedValue => None,
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
                    &mut self.functions,
                    outer_visible,
                )?;
                Ok(HirStmt {
                    kind: HirStmtKind::Block { stmts: Vec::new() },
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
                if !matches!(key_kind, ValueKind::Number | ValueKind::String) {
                    return Err(HirError::TypeMismatch {
                        op: "[]=".to_owned(),
                        lhs_kind: "number or string".to_owned(),
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
                let value_ok = matches!(
                    (key_kind, value_kind),
                    (ValueKind::Number, ValueKind::Number)
                        | (ValueKind::Number, ValueKind::Bool)
                        | (ValueKind::Number, ValueKind::String)
                        | (ValueKind::Number, ValueKind::Function(_))
                        | (ValueKind::Number, ValueKind::Table)
                        | (ValueKind::String, ValueKind::Number)
                        | (ValueKind::String, ValueKind::Bool)
                        | (ValueKind::String, ValueKind::String)
                        | (ValueKind::String, ValueKind::Nil)
                        | (ValueKind::String, ValueKind::Function(_))
                        | (ValueKind::String, ValueKind::Table)
                );
                if !value_ok {
                    return Err(HirError::TypeMismatch {
                        op: "[]=".to_owned(),
                        lhs_kind: "number".to_owned(),
                        rhs_kind: value_kind.name().to_owned(),
                        offset: value.span.start,
                    });
                }
                // Phase 2.6c-tag-fn-tbl (ADR 0071): closure with
                // upvalues escapes through the table — reject
                // the same way ADR 0044 already rejects argument
                // / return escape. Plain `function() ... end`
                // (no upvalues) and top-level `local function f`
                // references pass through.
                if let Some(fid) = function_ref_id(&value_hir, &self.locals) {
                    if !self.functions[fid.0].upvalues.is_empty() {
                        return Err(HirError::ClosureEscapes {
                            position: "table value".to_owned(),
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
            if let HirExprKind::Call { callee, args } = lowered.kind {
                let ret_kinds: Vec<ValueKind> = match callee {
                    Callee::User(FuncId(fid)) => self.functions[fid].ret_kinds.clone(),
                    _ => {
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
        // Phase 2.5c.3 (ADR 0044): a closure carrying upvalues
        // returned from its creation scope would outlive the slots
        // it observes. Reject statically.
        for value in &lowered {
            if self.closure_with_upvalues(value).is_some() {
                return Err(HirError::ClosureEscapes {
                    position: "return value".to_owned(),
                    offset: value.span.start,
                });
            }
        }
        let kinds: Vec<ValueKind> = lowered
            .iter()
            .map(|e| infer_kind(e, &self.locals, &self.functions))
            .collect();
        if let Some(prev) = &self.in_function_ret_kinds {
            if prev.len() != kinds.len() {
                return Err(HirError::ArityMismatch {
                    builtin: "return".to_owned(),
                    expected: prev.len(),
                    actual: kinds.len(),
                    offset: span.start,
                });
            }
            for (i, (p, k)) in prev.iter().zip(kinds.iter()).enumerate() {
                if p != k {
                    return Err(HirError::TypeMismatch {
                        op: format!("return position {i}"),
                        lhs_kind: p.name().to_owned(),
                        rhs_kind: k.name().to_owned(),
                        offset: span.start,
                    });
                }
            }
        } else {
            // First return seen — upgrade each `_ret_value_N` slot kind.
            for (slot_id, k) in ret_value_ids.iter().zip(kinds.iter()) {
                self.locals[slot_id.0].kind = *k;
            }
            self.in_function_ret_kinds = Some(kinds.clone());
        }
        let mut block_stmts: Vec<HirStmt> = Vec::with_capacity(lowered.len() + 1);
        for (slot_id, v) in ret_value_ids.iter().zip(lowered.into_iter()) {
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
            for (n, v) in names.iter().zip(lowered.into_iter()) {
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
            // dispatch (Builtin/Indirect ret arities aren't tracked
            // statically), and its ret arity must equal `names.len()`.
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
                Callee::User(FuncId(fid)) => self.functions[fid].ret_kinds.clone(),
                _ => {
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
                    upvalues: Vec::new(),
                    locals: Vec::new(),
                    body: Vec::new(),
                    ret_kinds: Vec::new(),
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
                    &self.functions,
                    params,
                    body,
                    &external_kinds,
                    outer_visible,
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
                    // Phase 2.6c-tag-fn-tbl (ADR 0071): closure
                    // with upvalues escapes through the table.
                    if let Some(fid) = function_ref_id(elem, &self.locals) {
                        if !self.functions[fid.0].upvalues.is_empty() {
                            return Err(HirError::ClosureEscapes {
                                position: "table element".to_owned(),
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
                if !matches!(key_kind, ValueKind::Number | ValueKind::String) {
                    return Err(HirError::TypeMismatch {
                        op: "[]".to_owned(),
                        lhs_kind: "number or string".to_owned(),
                        rhs_kind: key_kind.name().to_owned(),
                        offset: key.span.start,
                    });
                }
                HirExprKind::Index {
                    target: Box::new(target_hir),
                    key: Box::new(key_hir),
                }
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
                    // Phase 2.5c.3 (ADR 0044): a closure carrying
                    // upvalues passed as a value reaches its eventual
                    // call site via Callee::Indirect, which has no
                    // path to thread upvalues. Reject statically.
                    if self.closure_with_upvalues(arg).is_some() {
                        return Err(HirError::ClosureEscapes {
                            position: "call argument".to_owned(),
                            offset: arg.span.start,
                        });
                    }
                }
                let callee = match self.locals[local_id.0].func_id {
                    Some(fid) => Callee::User(fid),
                    None => Callee::Indirect(local_id),
                };
                // Phase 2.5c-min (ADR 0037): direct calls to a
                // closure with upvalues append the captured values
                // as extra arguments, mirroring the matching code
                // in the function_names dispatch path below.
                let mut all_args = lowered_args;
                if let Callee::User(fid) = callee {
                    let upvalue_args: Vec<HirExpr> = self.functions[fid.0]
                        .upvalues
                        .iter()
                        .map(|uv| HirExpr {
                            kind: HirExprKind::Local(uv.outer_local_id),
                            span: whole.span,
                        })
                        .collect();
                    all_args.extend(upvalue_args);
                }
                return Ok(HirExprKind::Call {
                    callee,
                    args: all_args,
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
                // Phase 2.5c.3 (ADR 0044): same escape check as the
                // local-Function-kind path — a closure carrying
                // upvalues cannot survive routing through Indirect.
                if self.closure_with_upvalues(arg).is_some() {
                    return Err(HirError::ClosureEscapes {
                        position: "call argument".to_owned(),
                        offset: arg.span.start,
                    });
                }
            }
            // Phase 2.5c-min (ADR 0037): if the callee captured
            // upvalues, append them as extra arguments. The
            // captured value is reloaded at each call site by
            // referencing the outer `LocalId` recorded during the
            // closure's lowering — equivalent to a snapshot taken
            // when the closure expression was evaluated, since the
            // outer slot is what was current at that moment and
            // `lower_call` runs after FunctionExpr lowering for
            // sibling closures.
            let upvalue_args: Vec<HirExpr> = self.functions[fid.0]
                .upvalues
                .iter()
                .map(|uv| HirExpr {
                    kind: HirExprKind::Local(uv.outer_local_id),
                    span: whole.span,
                })
                .collect();
            let mut all_args = lowered_args;
            all_args.extend(upvalue_args);
            return Ok(HirExprKind::Call {
                callee: Callee::User(fid),
                args: all_args,
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
            // value — `type` returns the typename string,
            // `tostring` returns the literal "function". Every
            // other call site keeps treating Function-as-value
            // as a hard error.
            if let ValueKind::Function(_) = k
                && !matches!(builtin, Builtin::Type | Builtin::ToString)
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
    fn lower_inconsistent_bool_number_returns_error() {
        // Body returns Bool in one branch, Number in another.
        let src = concat!(
            "local function bad(x)\n",
            "  if x > 0 then return true end\n",
            "  return 42\n",
            "end\n",
        );
        let result = lower_src(src);
        assert!(
            result.is_err(),
            "inconsistent Bool/Number returns must reject"
        );
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
        // The multi-assign statement is the second top-level stmt.
        match &hir.stmts[0].kind {
            HirStmtKind::MultiAssignFromCall { dst_ids, .. } => {
                assert_eq!(dst_ids.len(), 2);
            }
            other => panic!("expected MultiAssignFromCall, got {other:?}"),
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
