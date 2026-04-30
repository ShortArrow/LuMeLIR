# 0017. Phase 2.5b: Anonymous Function Expressions + First-Class Function Values (HIR-Time Resolution)

- **Status:** Accepted
- **Date:** 2026-04-30
- **Deciders:** ShortArrow

## Context

Phase 2.5a (ADR 0016) introduced top-level `local function` definitions
with `return` and recursion. The next Lua-shaped capability is
**anonymous function expressions** and **storing functions in
variables**:

```lua
local f = function(x) return x * 2 end
print(f(7))                          -- 14

local g = f
print(g(5))                          -- 10
```

A *true* first-class function model — passing functions as arguments,
returning them from functions, runtime dispatch on a function value —
requires `func.call_indirect` plus a uniform signature, and motivates
its own ADR (Phase 2.5b.2). Phase 2.5b ships only the slice that does
**not** need indirect calls: every callable identifier can be resolved
to a single `FuncId` at HIR time.

## Decision

### 1. Parser: anonymous function expression

```rust
ExprKind::FunctionExpr { params: Vec<String>, body: Chunk }
```

`parse_primary` recognises `Keyword::Function` and shares the existing
`parse_function_def`'s body-and-params helper. The expression form
mirrors the statement form except for the absence of a name.

### 2. HIR: ValueKind extension and FunctionRef

```rust
pub enum ValueKind {
    Number, Bool, Nil,
    Function(FuncId),     // NEW
}

pub enum HirExprKind {
    ..., FunctionRef(FuncId),
}

pub enum HirError {
    ..., FunctionUsedAsValue { name: String, offset: usize },
}
```

Anonymous functions registered during lowering get the mangled name
`user_anon_<idx>`. The HIR for `local f = function() ... end` is:

- A new `HirFunction` entry at index `idx`.
- The local `f` is declared with `LocalInfo.kind = Function(FuncId(idx))`.
- The `LocalInit` stores a placeholder `i1 0` into the slot — the
  actual function is resolved by name at every call site, so the slot
  value is irrelevant.

`local g = f` triggers the same path: lowering finds `f` is a
`Function`-kind local, copies the kind to `g`. No runtime data flows.

### 3. Call resolution

`lower_call` checks the callee identifier's kind first:

- `Function(fid)` → `Callee::User(fid)` direct dispatch.
- Otherwise → existing path (named user function via
  `function_names`, then builtin).

Calls into a function value are therefore static. No `func.call_indirect`,
no signature unification.

### 4. Restrictions (the entire reason 2.5b stops short of true first-class)

A `Function`-kind local may appear **only** as a call's callee. Any
other position — `print(f)`, `f + 1`, `apply(f, 5)`,
`local b = f` (where the rhs is a Function-kind local being stored
into a non-Function slot) — emits
`HirError::FunctionUsedAsValue`.

Concretely, `lower_expr` walks each expression's kind and rejects a
`Local(_)` whose kind is `Function(_)` unless it is the immediate
callee of a `Call`. The callee path bypasses this check.

Reassigning a `Function`-kind local to a different `FuncId` reuses
the existing `TypeMismatch` machinery (the kinds differ —
`Function(0) != Function(1)`).

### 5. Codegen

`emit_alloca_slot_for_kind` returns `i1` for `Function(_)` (the slot
exists but is never read). `HirExprKind::FunctionRef(_)` lowers to
`arith.constant 0 : i1` — also a placeholder; it is only ever stored
into a Function slot and never read back, since the slot value isn't
consulted by call resolution.

Anonymous functions are emitted as ordinary `func.func @user_anon_<idx>`
symbols by `emit_function`, identical to named functions.

## Alternatives Considered

- **`func.call_indirect` from the start.** Would unblock argument
  passing and function-typed returns, but requires a uniform
  signature. The signature constraints alone (1-arity? variadic?
  signature in `ValueKind`?) deserve a phase of their own. Defer to
  2.5b.2.
- **HIR-only `LocalInfo.func_id: Option<FuncId>`** (no new
  `ValueKind` variant). Keeps the type system narrower but every
  type query has to special-case the side-channel. The
  `Function(FuncId)` variant is uniform and clean.
- **Skip slot allocation for Function-kind locals.** The slot's
  `LocalId` and the `slots: Vec<Value>` index would diverge.
  Allocating an unused `i1` slot keeps the invariants simple.
- **Allow `print(f)` to print the function name or address.** No
  obvious correct semantics — functions don't have a Lua-visible
  string form here. Reject in 2.5b; revisit if needed.

## Consequences

- `ExprKind` +1 (`FunctionExpr`).
- `HirExprKind` +1 (`FunctionRef`).
- `ValueKind` +1 (`Function(FuncId)`).
- `HirError` +1 (`FunctionUsedAsValue`).
- `infer_kind` and the related codegen dispatch handle the new
  variant; otherwise the structure of Phase 2.5a is preserved.
- Runtime overhead: zero. The Function slot's `i1 0` constant fold
  away at LLVM optimisation.

## Out of Scope (deferred)

- Functions as arguments (`apply(f, x)`) → Phase 2.5b.2.
- Functions as return values → Phase 2.5b.2.
- Reassigning a function-kind local to a different function → Phase 2.5b.2.
- Function values flowing through `print`, comparisons, arithmetic →
  permanent restriction or revisited with table support.
- Closures (capturing outer locals) → Phase 2.5c.
- Multiple return values → Phase 2.5d.
- Bool/Nil parameters or returns → Phase 2.5e.
