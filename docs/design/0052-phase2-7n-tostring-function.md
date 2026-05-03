# 0052. Phase 2.7n: `tostring(f)` for Function Values

- **Status:** Accepted
- **Date:** 2026-05-03
- **Deciders:** ShortArrow

## Context

Phase 2.7c (ADR 0026) shipped `tostring(x)` covering Number,
Bool, Nil, and String. Phase 2.7f (ADR 0029) lifted the
`FunctionUsedAsValue` ban specifically for `type(f)` so users
could query the type of a function value. The natural sibling
— `tostring(f)` returning the literal "function" — was left
out, matching Lua's reference-implementation form which
prints `function: 0x<addr>`.

We don't expose addresses today (and probably shouldn't —
`tostring` shouldn't leak code-pointer ABI), but Lua-side
code routinely calls `tostring(f)` for diagnostic prints.
Returning a stable literal `"function"` is enough for those
diagnostics.

## Decision

### Extend the HIR exception list

```rust
if let ValueKind::Function(_) = k
    && !matches!(builtin, Builtin::Type | Builtin::ToString)
{
    return Err(HirError::FunctionUsedAsValue { … });
}
```

Adds `ToString` next to `Type` in the "Function-as-value
permitted" list. The check still fires for every other
builtin (`Print`, `ToNumber`, `Assert`, `Error`, `Print`'s
elements, etc.) — closure-as-print-target remains a hard
error until the (fn_ptr, env_ptr) closure-value rework
arrives.

### Codegen: reuse `s_typename_function`

```rust
ValueKind::Function(_) => {
    emit_addressof(context, block, "s_typename_function", types, loc)
}
```

`s_typename_function` was registered for `type(f)` (ADR
0029) and contains `"function\0"`. `tostring(f)` returns
the same string — sharing the global avoids a near-
duplicate. Both Lua semantics deliver the same payload, so
the sharing is honest.

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | None                                                |
| Parser   | None                                                |
| AST      | None                                                |
| HIR      | One predicate widened to include `ToString` next to `Type` |
| Codegen  | `emit_tostring` Function arm switches from `unreachable!()` to `emit_addressof("s_typename_function", …)` |

## TDD Process

1. **Red.** 6 e2e tests covering anonymous-function arg,
   local-`function` arg, `local function` arg, the concat
   path through `tostring`, plus Number/`type(f)` regressions.
   4 failed (the new path); 2 passed (regressions).
2. **Green.** Predicate widened; codegen Function arm wired
   to `s_typename_function`. The pre-existing
   `tostring_of_function_value_is_static_error` test
   reframed as `…now_succeeds_after_2_7n` (boundary
   documentation).
3. **Refactor.** None warranted.

## Alternatives Considered

- **Synthesize `function: 0xADDR`** by reading the function
  pointer's bits via libc `snprintf`. Real address leakage
  is a no-go security-wise; faking it would diverge from any
  observable Lua behaviour. Reject.
- **Return `"function: <name>"`** so users can distinguish
  function values. Adds plumbing (the function name lives in
  HirFunction; codegen would need a per-FuncId const string
  pool). Defer until a real use case arrives.
- **Allow the wider `FunctionUsedAsValue` lift** (open all
  builtins to Function args). Would also unlock
  `print(f)` etc., which currently has no sensible
  semantic. Rejected — keep the rule "Function values flow
  only through the call/return ABI and the named
  introspection builtins."

## Consequences

- HIR adds one identifier to a `matches!`.
- Codegen replaces an `unreachable!()` with a real arm
  (~3 lines).
- 6 new e2e tests; 1 reframed test. Total green at 663.
- Concat-with-function-via-tostring (`"got: " ..
  tostring(f)`) now produces `"got: function"` end-to-end.

## Out of Scope

- **Per-function names** in the rendered string.
- **Address leakage** (Lua's `function: 0x…` form).
- **Closure-aware rendering** (e.g. `function (closure)`
  for closures with upvalues). The same literal applies to
  every Function-kind value today.
- **`tostring` accepting Table values** — pending tables.
