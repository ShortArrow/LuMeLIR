# 0030. Phase 2.7g: `assert(cond)` Builtin

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Lua programs use `assert(expr)` extensively for runtime checks
("did `tonumber` parse it?", "is the table non-empty?"). Stock Lua
accepts any value, returns it untouched on truthy, and calls
`error(message or "assertion failed!")` on falsy. The "returns it
untouched" half of that signature is heterogeneous — `assert(7)`
returns a number, `assert("a")` a string, `assert(t)` a table —
which our static type system can't yet model.

This phase ships the smallest viable assert that still lets test
programs assert predicate conditions: a `Bool`-typed input,
`Bool`-typed output, and a libc-mediated abort path on failure.
The "return the value untouched" half can land later when a
heterogeneous return shape arrives.

## Decision

### 1. `Builtin::Assert` joins the enum

Fifth variant after `Print`, `ToString`, `ToNumber`, `Type`.
`from_name` recognises `"assert"`; `arity` is 1; `infer_kind`'s
`Callee::Builtin(Builtin::Assert)` arm returns `ValueKind::Bool`.

### 2. HIR-time arg-kind restriction

`lower_call` adds a per-call check: `assert(x)` requires `x` to be
`ValueKind::Bool`. Any other kind — Number, String, Nil, Function
— is `HirError::TypeMismatch` with message `"assert bool vs
<kind>"`.

The restriction sidesteps the heterogeneous-return question: every
truthy Bool is `true`, every falsy Bool is `false`, so the result
kind is always `Bool`.

User code expresses non-Bool checks as predicates:

```lua
assert(tonumber(s) == 42)   -- explicit comparison
assert(x ~= nil)            -- explicit nil check (when nil is
                            --  reachable; not yet practical
                            --  because `nil` and Number share no
                            --  common == path post-fold)
```

### 3. Codegen: scf.if + libc `exit`

A new libc extern `exit(i32) -> void` joins
`emit_string_runtime_decls`. A new module-top global
`s_assert_failed` holds the diagnostic `"assertion failed!"` (no
trailing `\n` — the existing `printf("%s\n", _)` path adds one).

`emit_assert(cond)`:

```text
not_cond = cond XOR 1                 ; invert i1
scf.if not_cond {
  printf("%s\n", "assertion failed!")
  exit(1)
  scf.yield                            ; structurally required;
                                       ; exit() is noreturn so
                                       ; this is unreachable
} else {
  scf.yield
}
```

The assert call's HIR result kind is Bool; codegen yields the
`cond` value verbatim from the calling arm so user code that does
`local x = assert(cond)` gets `x == cond` (always `true` for the
program paths that survive).

### 4. No printf to stderr in this phase

`printf` writes to stdout; Lua's `error` writes to stderr. The
e2e test harness reads stdout only, so the diagnostic surfaces
there too. A future phase that wires up `stderr` (via libc
`fprintf` + the `stderr` global) can switch the diagnostic without
breaking the existing semantic.

## Alternatives Considered

- **Accept any kind, return same kind**. The Lua-faithful
  signature. Needs a heterogeneous return shape (or per-kind
  monomorphisation at HIR-time). Rejected for this phase.
- **Accept any kind, always return Bool**. Useable but loses the
  Lua-typical `local x = assert(parse(s))` chain. Rejected — Bool
  return + Bool input is a cleaner single-purpose builtin.
- **Use `abort()` instead of `exit(1)`**. Equivalent semantics for
  our subset; `exit(1)` lets the test harness assert a stable
  status code (1) and is the more conventional choice.
- **Synthesise a runtime trap via `llvm.trap`**. Avoids the libc
  dependency but produces an opaque `Aborted` signal; harder to
  test and harder to debug. Rejected.
- **Add a custom-message arg `assert(v, msg)`**. Needs variadic
  builtin support (currently fixed at 1 arg). Defer.

## Consequences

- HIR: `Builtin::Assert` variant; per-call kind check in
  `lower_call`.
- Codegen: `exit(i32) -> void` extern; `s_assert_failed` global;
  `Callee::Builtin(Builtin::Assert)` arm in `emit_expr`; new
  `emit_assert` helper using void `scf.if`.
- Nine integration tests in `phase2_7g_assert.rs` cover the
  three pass paths (literal `true`, `==`, `tonumber()==`),
  failure-with-exit-1 + diagnostic + post-assert code skipped,
  passing assert inside a user function, a chained-assert
  pattern, and three rejection paths (Number, String, Nil).

## Out of Scope

- **Lua-faithful "any kind, return same kind" signature**.
- **Optional message arg `assert(v, msg)`**.
- **Stderr / file-descriptor write target**.
- **Backtrace / source-location reporting on failure**.
- **`error(msg)` standalone** — same diagnostic shape but no
  conditional check. Will land alongside the variadic builtin
  support.
