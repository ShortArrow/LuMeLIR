# 0029. Phase 2.7f: `type(x)` Builtin

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Lua's `type(x)` returns a string naming the runtime type of its
argument. It's the standard way to write type-aware library code
and crops up in nearly every non-trivial Lua program. With our
static type system every value's kind is known at compile time, so
`type(x)` becomes a pure compile-time dispatch — no runtime
introspection required.

## Decision

### 1. `Builtin::Type` joins the enum

Fourth variant after `Print`, `ToString`, `ToNumber`. `from_name`
recognises `"type"`; `arity` is 1; `infer_kind` for
`Callee::Builtin(Builtin::Type)` returns `ValueKind::String`.

### 2. Function values are admissible

Every other builtin rejects Function-kind args as
`FunctionUsedAsValue`. `type(f)` is the one legitimate place to
*observe* a function value without calling it (Lua surfaces the
string `"function"`), so `lower_call` carves out a
`!matches!(builtin, Builtin::Type)` guard before the rejection
fires.

### 3. Codegen: pure addressof of a per-kind global

Five new module-top globals hold the Lua type names:

```
@s_typename_number    "number\0"
@s_typename_string    "string\0"
@s_typename_boolean   "boolean\0"
@s_typename_nil       "nil\0"
@s_typename_function  "function\0"
```

`emit_expr`'s `Callee::Builtin(Builtin::Type)` arm picks the global
by static `kind` and emits `llvm.mlir.addressof @s_typename_<k>`.
The arg's value is irrelevant to the result, but Lua semantics
still require the arg expression to evaluate (for side effects in
the user's call sites). The arm therefore does emit the arg —
unless it's a pure `HirExprKind::Local` or `FunctionRef`, both of
which have no observable side effect, in which case it skips the
materialisation as a small optimisation.

The result is a `ptr` flowing into the existing String-kind use
sites (`print(...)`, `==`, `..`).

## Alternatives Considered

- **Inline the type-name string literal at every call site**
  through the existing `string_pool`. Equivalent observable
  behaviour but multiplies the dedup logic for what is a tiny,
  closed set of payloads. Per-kind globals with stable symbols are
  cleaner.
- **Materialise a runtime type tag** (e.g. an i32 enum) and call a
  `lookup_type_name(tag)` helper. Necessary in a dynamically-typed
  IR; superfluous in our static-kind world.
- **Reject `type(f)`** like every other builtin. Diverges from Lua
  in a trivial, frequently-used path. Rejected.

## Consequences

- HIR: `Builtin::Type` variant; `infer_kind` arm; one-line guard
  in `lower_call`'s Function-rejection check.
- Codegen: five new module-top globals; new arm in `emit_expr`'s
  builtin dispatch.
- Ten integration tests in `phase2_7f_type.rs` cover the five
  kinds (number, string, true, false, nil), Function-kind
  (anonymous and named), and result usage in `==`, `..`, and
  `if` predicates.

## Out of Scope

- **`type` of `table` and `userdata` / `thread`** — pending those
  value kinds.
- **Compile-time elision** of `type(x) == "kind_string"` chains
  (every comparison is statically resolvable since the kind is
  known). The current path lowers to `addressof + strcmp`; an
  optimisation pass could fold to a constant `Bool`. Defer.
