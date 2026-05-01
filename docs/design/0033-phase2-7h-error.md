# 0033. Phase 2.7h: `error(msg)` Builtin

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.7g shipped `assert(cond)`, whose failure path is "print
diagnostic + `exit(1)`". `error(msg)` is the same failure path
divorced from a condition — it always fires, lets the user
choose the message, and is the standard Lua idiom for raising
explicit failures (`error("not implemented")`,
`error("expected " .. tostring(want) .. ", got " ..
tostring(got))`).

This phase adds `error(msg)` for `msg : String`. A future variadic
overload `error(msg, level)` and the table-as-message form land
when variadic builtins / tables arrive.

## Decision

### 1. Tidy First — extract `emit_exit_with_message` (no behaviour change)

`emit_assert`'s failure arm previously inlined the
`printf("%s\n", msg) + exit(1)` sequence. That sequence is
verbatim what `error(msg)` needs (just unconditional rather than
guarded by `not cond`). Before adding the new builtin, the
sequence is lifted into a pure helper:

```rust
fn emit_exit_with_message<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    msg_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
);
```

The helper depends only on its arguments (FP-pure relative to its
signature; the only side effect is appending ops to the supplied
`block`, the same shape every other `emit_*` helper uses).
`emit_assert` is rewritten to call the helper from inside its
`scf.if` then-region, preserving observable behaviour (verified
by the existing 462-test suite passing unchanged).

### 2. `Builtin::Error` joins the enum (Red-Green-Refactor)

Sixth variant after `Print`, `ToString`, `ToNumber`, `Type`,
`Assert`. `from_name` recognises `"error"`; `arity` is 1;
`infer_kind`'s `Callee::Builtin(Builtin::Error)` returns
`ValueKind::Number` — a placeholder, since `error` never returns
at runtime. Code after `error(...)` is dead, but the static type
still has to flow into the surrounding expression.

`lower_call` adds the per-builtin kind check: arg must be
`String`, otherwise `TypeMismatch`. The check follows the same
pattern as `Builtin::ToNumber` (Number/String) and
`Builtin::Assert` (Bool).

### 3. Codegen reuses the shared helper

The new `Builtin::Error` arm in `emit_expr`:

```rust
Callee::Builtin(Builtin::Error) => {
    let msg_val = emit_expr(...)?;
    emit_exit_with_message(context, block, msg_val, types, loc);
    /* placeholder f64 0.0 satisfies expression-position contract */
    Ok(zero_constant(...))
}
```

`emit_exit_with_message` does the work; the arm itself is
straight-line. This is the FP-style "compose pure functions"
shape — the arm is one function call to a small helper plus a
placeholder result.

### 4. Layering preserved

| Layer    | Change                                                |
|----------|-------------------------------------------------------|
| Lexer    | None                                                  |
| Parser   | None                                                  |
| AST      | None                                                  |
| HIR      | `Builtin::Error` variant + per-call kind check        |
| Codegen  | New arm in `emit_expr` that reuses the shared helper  |

Dependency direction (`codegen → hir → parser → lexer`) is
unchanged. No new modules, no inter-layer leaks.

## TDD Process Notes

The implementation followed Red → Green → Refactor strictly:

1. **Tidy First** — `emit_exit_with_message` extracted from
   `emit_assert`. Tests passed unchanged at 462 → confirms the
   refactor was behaviour-preserving.
2. **Red** — four HIR unit tests + six integration tests added
   referring to the not-yet-existent `Builtin::Error`. `cargo
   test` failed compilation as expected.
3. **Green** — `Builtin::Error` variant added, kind check wired,
   codegen arm added. Tests passed.
4. **Refactor review** — code shape inspected; no further
   duplication beyond what was already factored out in step 1.

## Alternatives Considered

- **Inline the `printf + exit` sequence in the new arm**.
  Equivalent observable behaviour but reintroduces the
  duplication that step 1's Tidy First just removed. Rejected.
- **Make `error` return `ValueKind::Nil`**. Cleaner ("never
  returns" mapped to a dead-code-only kind) but every consumer of
  the call's static kind would need to special-case Nil-from-error
  versus actual-Nil. The Number placeholder is consistent with
  `print`'s placeholder return.
- **Variadic `error(msg, level)` in this phase**. Needs the
  `Arity` enum (currently only `Print` is variadic, with a
  one-line carve-out). Defer — when `string.format` or `error`'s
  level arg first demands it, the enum lands.
- **Special "noreturn" ValueKind** that elides downstream type
  checks. Useful long-term but over-built for the current pair of
  callers (`assert`, `error`). Defer.

## Consequences

- HIR: `Builtin::Error` variant; `infer_kind` arm; per-call kind
  check.
- Codegen: new `Callee::Builtin(Builtin::Error)` arm in
  `emit_expr` that reuses the freshly-extracted
  `emit_exit_with_message` helper. `emit_assert` itself loses ~20
  lines (now delegates to the helper) — net codegen growth is
  modest.
- HIR tests: four unit tests covering the lower path, three
  rejection paths.
- E2E tests: six tests covering the literal-message case, the
  follow-up-statements-skipped case, message via local, message
  via concat, message via user-function call, and the
  pass-then-error sequence.

## Out of Scope

- **Variadic `error(msg, level)`** with the `level` arg
  controlling stack-frame attribution.
- **Table-as-message form** `error({code = 1, msg = "..."})`.
- **`pcall` / `xpcall`** — protected calls require unwinding-style
  control flow we don't have yet.
- **Stderr redirection** — currently emitting to stdout via
  `printf`. A future `stderr` global + `fprintf` extern can swap
  the stream without changing the surface API.
- **Source-location attribution** in the error message
  (`file:line:`).
