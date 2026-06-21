# 0245. `coroutine.isyieldable` + `coroutine.running` — Main-Thread Spec Conformance

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M13 sub-ADR. The full Lua 5.4 §6.2 `coroutine` library has 7 functions: `create`, `resume`, `yield`, `wrap`, `status`, `isyieldable`, `running`. The first five need a coroutine runtime (stack switching via `ucontext` / `setjmp`+manual stack / LLVM coroutine intrinsics) — substantial multi-session work. The last two have spec-conformant behavior on the main thread without any runtime support:

- `coroutine.isyieldable()` returns `true` only when called from inside a coroutine that can yield. From main-thread code the answer is unconditionally `false`.
- `coroutine.running()` returns `(thread, is_main)` where `thread` is the current coroutine or `nil` on the main thread, and `is_main` is the boolean. From main thread the answer is `(nil, true)`.

This ADR ships those two functions. Code that uses them today gets correct answers; code that needs `create`/`resume`/`yield` continues to defer to M13-stretch.

## Scope (literal)

- ✅ New `coroutine` namespace dispatched by `Builtin::from_namespace_method`.
- ✅ `Builtin::CoroutineIsYieldable` HIR variant; `arity (0, 0)`; `ret_kinds [Bool]`; `infer_kind → Bool`; codegen emits `arith.constant false`.
- ✅ `Builtin::CoroutineRunning` HIR variant; `arity (0, 0)`; `ret_kinds [TaggedValue, Bool]`; `infer_kind → TaggedValue` (single-result truncation).
- ✅ Multi-assign dispatch via `emit_call_*_into_locals` precedent (Next / Pcall / StringFind). The body stores Nil into the first slot and `true` into the second.
- ✅ Single-assign `local th = coroutine.running()` truncates to position 0 (Nil) per ADR 0081.
- ✅ Composes with the ADR 0228 non-trapping `if Local(TaggedValue)` cond path (`if th then ... end` over the `nil` result reads as `false` via `tag == TAG_NIL`).
- ❌ `coroutine.create(f)`. M13-stretch — needs runtime to spawn a coroutine.
- ❌ `coroutine.resume(co, ...)`. M13-stretch — needs stack switching.
- ❌ `coroutine.yield(...)`. M13-stretch.
- ❌ `coroutine.wrap(f)`. M13-stretch.
- ❌ `coroutine.status(co)`. M13-stretch.
- ❌ `coroutine.close(co)` (Lua 5.4 add). M13-stretch.

## Decision

The behaviors of `isyieldable` and `running` from main thread are unconditionally fixed — there's no other answer that can ever be correct. Implementing them ahead of the runtime is therefore not a deviation from spec; it's a complete implementation for the main-thread case. When the runtime lands, these same functions will need to be updated to check the active-coroutine state, but the main-thread fallback path will remain.

### Why the two-return shape for `running`

Lua 5.4 returns the boolean as the second value so callers can check `if is_main then ...` directly. Our HIR `ret_kinds = [TaggedValue, Bool]` matches. The multi-assign dispatch follows the existing Next / Pcall / StringFind precedent.

## Tests

`tests/phase4_m13_coroutine_main_thread.rs` (NEW, 5 e2e):

1. `coroutine.isyieldable()` → `false`.
2. `local th, main = coroutine.running()` → `(nil, true)`.
3. Single-assign `local th = coroutine.running()` → `nil` (truncation).
4. `if coroutine.isyieldable() then ... else "main" end` → `"main"`.
5. `local th, is_main = coroutine.running(); if is_main then ...` → `"main thread"`.

## Test count delta

```
Step 0:  1606 (after ADR 0244)
C3 (impl + 5 e2e): 1606 → 1611
```

## References

- [Lua 5.4 §6.2](https://www.lua.org/manual/5.4/manual.html#6.2) — coroutine library spec.
- [ADR 0081](0081-phase2-8e-iter-next.md) — multi-return single-result truncation precedent.
- [ADR 0228](0228-string-find-plain.md) — non-trapping `if Local(TaggedValue)` cond.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M13 milestone.
