# 0284. `string.gmatch` closure form — structural blocker + decomposition (N4-F-2 design)

- **Status:** Design — implementation deferred
- **Kind:** Architecture Decision
- **Date:** 2026-06-30
- **Deciders:** ShortArrow

## Why this ADR exists

The `/goal N4-F done` hook keeps firing because N4-F-1 (`string.find` 3-arg form, ADR 0283) only covers half of the roadmap entry. The other half — spec-named `string.gmatch(s, pat)` returning a closure-iterator — has two genuinely-multi-session blockers. This ADR records them honestly so future sessions hit the right decomposition.

## Lua spec recap

```lua
for word in string.gmatch("the quick brown fox", "%a+") do
    print(word)
end
```

`string.gmatch(s, pat)` must return an *iterator function* `f` such that each call `f()` returns the next match (or `nil` when exhausted). The `for ... in` form binds the iterator's result to `word` each iteration.

## Blocker 1 — builtin-returned closure synthesis

A builtin today cannot construct a closure value. Closure cells (ADR 0083) are only allocated by codegen for user `function () ... end` literals. To return a closure from `Builtin::StringGmatch`, the compiler needs one of:

- **(a)** HIR-level synthetic-function infrastructure — when lowering `string.gmatch(s, pat)`, generate a fresh `HirFunction` whose body is the iterator step + register its captures.
- **(b)** Codegen-level closure synthesis — bypass HIR and emit the closure cell + a hand-written MLIR function body directly.
- **(c)** A Table-with-`__call` workaround — return a Table that's callable via `__call`. Today's `__call` dispatch (ADR 0146) still requires a `Function` value in `__call`, so this doesn't fully sidestep blocker 1.

Each is its own session. **(a)** is the cleanest because it reuses every existing closure pipeline (capture analysis, cell allocation, call-site dispatch, GC root walk) and is the smallest *architectural* delta — the closure mechanism stays unique and the synthesis becomes one more code path into it.

### Sub-blocker — mutable upvalue (`pos`)

ADR 0083 captures by value (snapshot). The gmatch iterator needs a mutable position to advance per call. The standard workaround is a single-slot Table held as the upvalue: the closure captures the *pointer* by value, but mutating Table fields through it persists. So a synthesized gmatch closure captures one Table (`{s, pat, pos}`) and reads/writes it on each call.

## Blocker 2 — generic-for 1-name form

The current parser (`src/parser/mod.rs:262-271`) rejects the 1-name generic-for `for x in iter do` and only accepts `for x, y in ipairs(...) | pairs(...) | iter, state, ctl do`. Per the existing comment, "1-name is parser-rejected; the user can pad with `_` to force the 2-name form" — but the 2-name form *also* rejects an arbitrary `iter` expression because it goes through `unwrap_named_unary_call(..., "ipairs"/"pairs")`.

A real `for word in string.gmatch(s, pat) do ... end` therefore needs:

- Parser: accept the 1-name form with arbitrary iter expression.
- HIR: lower 1-name generic-for as a `while`-style loop calling `iter()` each iteration, binding result to the single name, breaking when result is `nil`.
- Codegen: nothing new — the lowered loop reuses existing while + call infrastructure.

## Decomposition (each = one session goal)

| sub | scope |
|---|---|
| N4-F-2a | Generic-for 1-name form accepting arbitrary callable. Parser + HIR lowering + 4-6 e2e (using any existing iter, e.g. ipairs already done, just shape-test the new path). |
| N4-F-2b | HIR-level synthetic-function infrastructure. Add `lower_synthetic_iter_closure(...)` returning a `(HirFunction, captured_table_local)`. Test by synthesizing a trivial counter iterator end-to-end. |
| N4-F-2c | `Builtin::StringGmatch(s, pat)` using N4-F-2a + N4-F-2b. Synthesize closure that reads from a captured `{s, pat, pos}` Table and calls the existing `lumelir_string_find_init` runtime. 6+ e2e covering Lua-spec for-in form. |

## What this session lands

- This ADR (design only).
- N4-F-1 = ADR 0283 already in main.
- Implementation of N4-F-2 not attempted further this turn because each sub-task above is genuinely larger than the remaining context budget, and partially-wired infrastructure (Builtin enum variant + ret_kinds + half a codegen arm) would block subsequent sessions until reverted.

## References

- ADR 0083 — closure cells, by-value capture.
- ADR 0146 — `__call` metamethod.
- ADR 0283 — N4-F-1 `string.find(s, pat, init)`.
- Roadmap 2026-06-27 N4-F.
- Lua 5.4 §3.3.5 — generic for, §6.4 — `string.gmatch`.
