# 0223. `local _ENV = sandbox` Rebind

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Third (closing) M4 sub-ADR. [ADR 0221](0221-env-globals-chunk-table.md) synthesises `local _ENV = _G` at chunk start when the source references `_ENV`. The canonical Lua sandboxing pattern — `local _ENV = {...}` — was not yet validated against this synthesiser. Lua spec §3.5 lets `_ENV` be rebound in any scope; the rebound table is the lookup target for free names in that scope.

This ADR verifies and pins that user-level `local _ENV = ...` correctly shadows the synthesised alias via standard Lua lexical scoping rules — no synthesiser-suppression logic is needed. `_G` retains the original chunk-level binding throughout, so a sandbox can write its own state via `_ENV` while still reaching `_G` for shared globals.

## Scope (literal)

- ✅ Verify: `local _ENV = {}` shadows the synthesised `local _ENV = _G` for the remainder of its lexical scope.
- ✅ Verify: After rebind, `_ENV.x = ...` writes to the sandbox table; `_G.x = ...` writes still target the original chunk-level `_G` table.
- ✅ Verify: Multiple `_ENV` keys round-trip through the sandbox table.
- ❌ Free-name fallback through the rebound `_ENV`. `x = 1` still resolves via ADR 0048 auto-declared locals, not `_ENV.x = 1`. Migration to free-name → `_ENV[name]` is the long-term ADR 0154 trajectory and remains deferred.
- ❌ `_ENV` rebind inside a function body. The synthesised binding is chunk-scoped; user fns capture `_G` as an upvalue per ADR 0083. Rebind inside a fn body works for direct `_ENV.x = ...` accesses but does not change free-name resolution.
- ❌ Functions stored inside a sandbox table called via `_ENV.f()`. Requires runtime function-value dispatch through a Table cell; out of scope.

## Decision

No new code — the existing Lua shadowing rules deliver the sandbox behaviour without intervention. The HIR pre-pass from ADR 0221 always prepends `local _ENV = _G` when `_ENV` is referenced; the user's later `local _ENV = ...` is a standard shadowing declaration and the rest of the chunk's `_ENV` accesses resolve to the shadowed binding via scope resolution.

This ADR pins the behaviour with 3 e2e tests so that future refactors of the pre-pass or the closure system cannot regress sandboxing without explicit acknowledgement.

### M4 milestone close

With ADRs 0221 + 0222 + 0223, M4 (`_ENV` / true globals) reaches its 3/3-5 sub-ADR minimum-viable close. Remaining M4-stretch work (deferred):

- Full builtin migration into `_ENV` entries (`_G.print`, `_G.string.format`, etc.) per ADR 0154 §"What Phase 3 ADR owns".
- Free-name → `_ENV[name]` resolution for the ADR 0048 backstop, with sandbox-aware lookup.
- Multi-hop nested closure capture of `_G` / `_ENV` (lifts the ADR 0222 §Scope limitation; pairs with M3-extended Table DFS).
- Bool / Nil values in `_G[k]` slots (lifts the homogeneous-Table value-kind constraint).

## Tests

`tests/phase4_env_sandbox.rs` (NEW, 3 e2e):

1. `local _ENV = {}; _ENV.x = 42; print(_ENV.x)` → `42` (sandbox rebind works).
2. `_G.outer = 1; local _ENV = {}; _ENV.inner = 2; _G.via_g = 3; print(_G.outer); print(_G.via_g); print(_ENV.inner)` → `1\n3\n2` (isolation between sandbox and `_G`).
3. Sandbox table carries multiple distinct keys.

## Test count delta

```
Step 0:  1503 (after ADR 0222)
C3 (3 e2e): 1503 → 1506
```

## References

- [Lua 5.4 §3.5](https://www.lua.org/manual/5.4/manual.html#3.5) — visibility / `_ENV` rebind semantics.
- [ADR 0154](0154-env-true-globals-strategy.md) — long-term `_ENV` strategy.
- [ADR 0221](0221-env-globals-chunk-table.md) — `_G` / `_ENV` chunk-level Table foundation.
- [ADR 0222](0222-env-globals-extended-surface.md) — `_G` extended surface pin.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M4 milestone close.
