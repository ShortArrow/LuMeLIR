# 0021. Phase 2.5d: Multi-Value `return` and Multi-Binding `local`

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-04-29
- **Deciders:** ShortArrow

## Context

Through Phase 2.5e, every user-defined function had a single result
(or void). Lua's actual return-value model is N-ary:

```lua
local function divmod(n, d) return n / d, n - d end
local q, r = divmod(7, 2)         -- q=3.5, r=5
```

This phase generalises both sides of the function-value boundary:

- `return EXPR (, EXPR)*` produces N values in source order.
- `local NAME (, NAME)+ = ...` binds N names from either parallel
  values (`local a, b = 1, 2`) or a single multi-result call
  (`local a, b = pair()`).

MLIR's `func.func` and `func.call` already support N-ary results, so
this is a HIR / codegen plumbing change rather than a target-level
limitation lift.

## Decision

### 1. AST: parallel `Vec` variants

The parser keeps the historical single-name / single-value paths
unchanged and adds two new statement shapes:

```rust
StmtKind::LocalMulti  { names: Vec<String>, values: Vec<Expr> }
StmtKind::ReturnMulti { values: Vec<Expr> }
```

`Local`/`Return` are still emitted for the 1-1 case so existing AST
consumers and tests are undisturbed; `*Multi` shows up only when
either side has more than one element.

### 2. HIR: `ret_kinds: Vec<ValueKind>` replaces `ret_kind`

`HirFunction.ret_kind: Option<ValueKind>` becomes
`ret_kinds: Vec<ValueKind>`. The mapping is straightforward:

| Old `ret_kind` | New `ret_kinds`     |
|----------------|---------------------|
| `None`         | `vec![]`            |
| `Some(k)`      | `vec![k]`           |
| (new) multi    | `vec![k1, k2, ...]` |

`infer_kind` for `Callee::User` returns `ret_kinds.first()` (Lua
truncates a multi-result call to its first value when used in
expression position).

### 3. HIR: per-position `_ret_value_N` slots

The body-guard pattern still routes returns through hidden slots:
`_returned: Bool` plus one `_ret_value_N` per return position.
`lower_function_body` pre-scans the AST for the maximum return
arity (`ast_max_return_arity`) and allocates that many slots up
front, each getting a kind-appropriate default (`0.0`, `false`,
`nil`, or no init for Function — same rules as 2.5e/2.5b.3).

`in_function_ret_kinds: Option<Vec<ValueKind>>` tracks the inferred
shape across multiple `return`s in the same body. The first return
sets it; later returns must agree on **arity** and **per-position
kind**, otherwise `ArityMismatch` / `TypeMismatch`. Mixed `return`
(void) and `return X` (value) reject for the same reason.

### 4. HIR: `MultiAssignFromCall` for `local a, b = call()`

Parallel binding `local a, b = 1, 2` lowers to a `Block` of
ordinary `LocalInit` statements — no new shape needed.

Multi-binding from a single call needs an atomic statement so codegen
emits the call once and reads multiple results:

```rust
HirStmtKind::MultiAssignFromCall {
    dst_ids: Vec<LocalId>,
    callee:  Callee,
    args:    Vec<HirExpr>,
}
```

`lower_local_multi` validates the call's static `ret_kinds` length
matches `dst_ids` and propagates each per-position kind into the
declared `LocalInfo`.

`Callee::Indirect` and `Callee::Builtin` are not allowed as the call
in `MultiAssignFromCall` — the former lacks statically-tracked ret
arity, the latter has fixed shapes. They reject as `ArityMismatch`.

### 5. Codegen: N-ary results everywhere

`ret_mlir_types(ret_kinds: &[ValueKind])` returns a `Vec<Type>` —
one MLIR type per position. Function declaration, call-site result
types, and `func.constant` all consume it.

`emit_function`'s trailing return loops over `hir_fn.ret_kinds`,
loading each `_ret_value_N` slot at index `params.len() + 1 + i` and
collecting the values into a single `func.return %v0, %v1, ...`.

`emit_multi_assign_from_call` emits the call once, walks
`op_ref.result(i)` for each destination, and routes each value
through the existing per-kind store path (Function-kind uses the
`unrealized_conversion_cast` bridge introduced in 2.5b.3).

### 6. Truncation rule for expression-position calls

Lua truncates a multi-result call used in expression context to its
first value. `infer_kind` for `Callee::User` already does this, and
codegen consumes only `op_ref.result(0)` from the existing `Call`
path. The `MultiAssignFromCall` path is the only place that consumes
multiple results.

## Alternatives Considered

- **Encode multi-return as a tuple type in `ValueKind`** — viable
  but pushes complexity into every kind-based switch. Multi-ret is
  only meaningful at the function boundary, not as a value-kind.
  Rejected.
- **Reuse `Block { stmts: [LocalInit, LocalInit, ...] }` for multi-
  binding from a call** — would require a temporary slot per result
  and double evaluation guard. `MultiAssignFromCall` evaluates the
  call once, atomically. Adopted.
- **Keep `ret_kind: Option<ValueKind>` and add a parallel
  `extra_ret_kinds: Vec<ValueKind>`** — strictly additive but every
  consumer must remember to look at both fields. Rejected in favour
  of one canonical `Vec`.
- **Allow `local a, b = f(), g()` (mixed parallel + last-call
  expansion)** — Lua's full rule. Defer; the simpler "exactly one
  call OR exactly N values" coverage handles the practical patterns
  while keeping the HIR check trivial.

## Consequences

- AST gains `LocalMulti` and `ReturnMulti` variants; existing
  `Local` / `Return` shapes stay for 1-1 cases.
- `HirFunction.ret_kind` → `ret_kinds: Vec<ValueKind>`; every
  consumer (HIR `infer_kind`, codegen `ret_mlir_types`, trailing
  return load) updated to walk the Vec.
- `LowerCtx::in_function` carries `Vec<LocalId>` of `_ret_value_N`
  slots; `in_function_ret_kinds` tracks the body-wide kinds shape.
- New `HirStmtKind::MultiAssignFromCall` and codegen helper
  `emit_multi_assign_from_call`.
- New AST helper `ast_max_return_arity` pre-scans bodies for slot
  allocation.
- `infer_param_kinds` and `infer_user_function_param_kinds`
  (Phase 2.5b.2 / 2.5e) gain trivial `LocalMulti` / `ReturnMulti`
  arms — they recurse into the contained expressions.

## Out of Scope

- **Last-call expansion in mixed RHS** (`local a, b = 1, f()` where
  `f()` returns 2+ values) — needs HIR-time runtime-vs-static
  arity dispatch.
- **`select(n, ...)` builtin / varargs** — needs a variadic ABI.
  Defer.
- **Multi-value print** (`print(f())` printing N values) — print is
  fixed at one arg.
- **Multi-value parameter expansion** (passing a function call's
  results as multiple args).
- **Bool/Nil/Function multi-return positions** — the type system
  already supports them in single-return; extending the per-position
  kind table is mechanical and lands when a test demands it.
- **Closures, tables** — Phase 2.5c, 2.6.
