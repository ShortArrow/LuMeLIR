# 0019. Phase 2.5b.3: Functions as Return Values

- **Status:** Accepted
- **Date:** 2026-04-29
- **Deciders:** ShortArrow

## Context

Phase 2.5b.2 (ADR 0018) made it possible to **pass** functions as
arguments through `func.call_indirect`. The natural symmetric
operation — **returning** a function from a function — was deferred
because it requires extending `ret_kind` past the Phase 2.5a "Number
only" rule. That extension is the topic of this phase:

```lua
local function d(x) return x * 2 end
local function get_doubler() return d end
local f = get_doubler()
print(f(5))                         -- 10
```

Together with 2.5b.2 this completes the I/O symmetry of first-class
functions in our subset: any function value (literal, alias,
parameter, or returned-from-call) can flow into another call site.

## Decision

### 1. `ret_kind` admits `Function(arity)`

`HirFunction::ret_kind` widens its accepted value-set:

```rust
pub ret_kind: Option<ValueKind>,    // Some(Number) or Some(Function(N))
```

The HIR value/return type-check in `lower_stmt::Return`:

- Accepts `Number` and `Function(arity)` values; rejects `Bool` and
  `Nil` with `TypeMismatch` (their full integration lands in 2.5e).
- Cross-checks every `return` in a body — all returns must agree on
  a single kind, otherwise `TypeMismatch`.
- Upgrades the `_ret_value` synthetic slot's `LocalInfo.kind` to the
  inferred kind on the first value-returning return seen, so codegen
  picks the right slot type.

### 2. Call result kind propagation

`infer_kind`'s existing `Callee::User(fid) ⇒ functions[fid].ret_kind`
arm now naturally returns `Function(arity)` when the callee returns a
function, so `local f = get_doubler()` lowers `f` with kind
`Function(1)` and `func_id = None`. Subsequent `f(5)` resolves to
`Callee::Indirect(f)` via the existing 2.5b.2 path.

### 3. No special handling for `Callee::Indirect` results

Phase 2.5b.3 keeps the rule that a `Callee::Indirect` call result is
always `Number`. Returning a Function from a Function-typed parameter
needs an arity-tracked indirect signature and is deferred. With the
parameter-as-callee path Number-only, we never form
`indirect-call-returning-function` HIR.

### 4. Anonymous functions hoisted from inner bodies

A `local function get() return function(x) return x + 1 end end`
registers the inner anonymous function inside the inner
`LowerCtx::for_function`'s `functions` clone. To make codegen emit
that inner symbol, `lower()` now hoists the new entries (indices `>=
pre_count`) into the outer chunk's `functions` table after each
top-level `FunctionDef` finishes lowering. `FuncId`s remain stable
because the inner table is a clone and indices only grow.

### 5. Codegen — Function-kind locals get a `!llvm.ptr` slot

Phase 2.5b.2 carried Function-kind values purely as SSA. Returning a
function across the body-guard `_ret_value` indirection demands real
storage so multiple return paths can write the slot. The decision:

- `emit_alloca_slot_for_kind(Function(_)) → llvm.alloca <ptr>` (one
  pointer per slot).
- Stores into Function-kind slots (`local g = some_call()`,
  `_ret_value = ...`) bridge the `!func.func<...>` value via
  `builtin.unrealized_conversion_cast` to `!llvm.ptr` before
  `llvm.store`.
- Loads (`Local(idx)`, trailing `func.return`) load the `ptr` and
  bridge back to the function type via the same
  `unrealized_conversion_cast`.
- Subsequent `--convert-func-to-llvm` lowers `!func.func<...>` to
  `!llvm.ptr`, turning the casts into ptr → ptr identities, and
  `--reconcile-unrealized-casts` erases them.

### 6. Three-bucket `Function`-kind locals at codegen

Function-kind locals fall into three buckets, distinguished at every
read/write site:

1. **Function parameter** (`idx < params_len`): the slot stores the
   block argument value directly, not an alloca pointer. No load /
   store / cast.
2. **Known FuncId** (`info.func_id = Some(_)`): the value is
   reproducible via `func.constant @<mangled>`; we skip the slot
   write as an optimisation and re-emit the constant at every read.
3. **Otherwise** (e.g. `local g = get_f()`, `_ret_value` for
   Function returns): the slot is a `!llvm.ptr` alloca that
   round-trips the function value via `unrealized_conversion_cast`.

`Callee::Indirect` reads the callee's value through the same buckets.

### 7. Synthetic `_ret_value` init is conditional

The body-guard pattern previously started every body with
`_ret_value = 0.0` (Number) so an early-exit path would still load a
sane value. For Function-kind ret slots there is no equivalent
default, and storing Number(0.0) into a `ptr` slot would type-error.
HIR now defers building the prelude until **after** body lowering and
omits the `_ret_value = 0.0` init when the slot's final kind is not
Number. Bodies that never reach a return on some control-flow path
get UB on the trailing load — acceptable for the current subset, and
no worse than the existing Number-return behaviour for hand-written
non-terminating bodies.

### 8. Function signature emission

`emit_function`'s result-type list now uses a shared helper:

```rust
fn ret_mlir_types(ret_kind: Option<ValueKind>, types: &Types) -> Vec<Type>
```

which maps `Number → [f64]`, `Function(arity) → [func.func<(f64×N) →
f64>]`, `None → []`. The same helper is used by `Callee::User` so the
call-site result type matches the declaration.

## Alternatives Considered

- **Don't extend `ret_kind`; encode function ID as `f64`**. Recovers
  the function via a runtime lookup. Tagged-union territory — defeats
  the static type model. Rejected.
- **All functions return Function(arity)** (uniform calling
  convention). Trivial to implement but kills Number returns. Rejected.
- **Use scf-yield to thread the return value instead of a slot**.
  Would avoid the ptr alloca but requires reshaping the body-guard
  pattern into expression-form. Larger change for the same
  observable outcome. Rejected.
- **Always store every Function-kind local through a slot** (drop
  the FuncId-known fast path). Simpler code but a regression for
  every existing 2.5b.2 use site. Rejected.

## Consequences

- HIR `lower_stmt::Return` accepts Number/Function values; multi-
  return cross-check tightens the type system without churn elsewhere.
- HIR `lower()` hoists anonymous functions registered inside
  `FunctionDef` bodies into the outer chunk's `functions` table.
- HIR `lower_function_body` reorders prelude generation to follow
  body lowering, conditionally emitting the `_ret_value = 0.0` init.
- Codegen `emit_alloca_slot_for_kind` now uses `ptr` for Function-
  kind slots; the new shared `ret_mlir_types` helper keeps decl /
  call-site types in lock-step.
- Codegen LocalInit/Assign, `Local(idx)`, `Callee::Indirect`, and
  `FunctionRef` expression all gain three-bucket logic for Function-
  kind locals (param / known-FuncId / alloca'd).
- `params_len` is threaded through `emit_stmts` / `emit_stmt` /
  `emit_expr` and helpers so codegen can tell a parameter slot from a
  body-local slot at every store/read site.

## Out of Scope

- **Closure (upvalue capture)** — `local function get() return outer
  end` for a `local outer = ...`. Needs heap-allocated environments;
  Phase 2.5c.
- **Indirect call returning Function** — passing a function-returning
  function as a parameter. Needs ret-arity tracking on Function
  values; Phase 2.5c+.
- **Multiple return values / varargs** — Phase 2.5d.
- **Non-Number param/return kinds** (Bool, Nil) — Phase 2.5e.
- **Function-typed table fields, methods, metatables** — Phase 2.6+.
