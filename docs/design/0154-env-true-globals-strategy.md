# 0154. `_ENV` / True Globals — Phase 3 Trigger

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

[ADR 0048](0048-phase2-0a-auto-declare-globals.md) installed "auto-declared globals" as the Phase 2 stand-in: any bare `Ident` that misses local resolution gets a synthetic top-level Local. ADR 0048 explicitly notes this is a placeholder for true `_ENV` semantics deferred to Phase 2.6+.

With metatables fully landed (ADRs 0134 – 0151), the precondition for true `_ENV` is satisfied — `_ENV` is itself a table whose `__index` / `__newindex` direct lookup at function scope. Per ADR 0133's contract, this ADR records the strategy decision and pins implementation to Phase 3.

## Decision

**Phase 2 freezes ADR 0048's auto-declared-locals strategy. Phase 3 introduces a true `_ENV` table that every function captures by upvalue.**

Concretely:

1. **Phase 2 = ADR 0048 preserved.**
   - Bare-Ident references that miss local resolution stay as synthetic top-level Locals.
   - No `_ENV` table; no global runtime state machinery.
   - User code that depends on the auto-declared semantics keeps working.

2. **Phase 3 = `_ENV` table + chunk-level capture.**
   - Top-level chunk is wrapped in an implicit `function(_ENV) ... end` that captures `_ENV`.
   - Every bare Ident `x` lowers to `_ENV.x` (read) or `_ENV.x = v` (write).
   - `_ENV` starts as a built-in table populated with `print`, `string`, `math`, etc. — the existing builtins move from per-name HIR dispatch into entries of this table.
   - Nested functions inherit `_ENV` via the existing closure-cell upvalue mechanism (ADR 0083 / ADR 0043).
   - `local _ENV = something` rebinds the implicit upvalue for the lexical scope — matches Lua spec §3.5.

3. **Builtin migration path.** Each existing `Builtin::*` (`Print`, `ToString`, etc.) gets a dual face: the HIR-direct path stays for early-bind performance, and a `_ENV.print` lookup path lands for spec-compliant shadowing. The implementation ADR will land both arms together.

### What Phase 3 ADR owns

- New `Builtin::Env`-style scope-init entry that materialises the implicit `_ENV` table at chunk start.
- Per-function `_ENV` upvalue threading via ADR 0083's closure mechanism.
- HIR resolution change: bare Ident that doesn't resolve to a local / param / upvalue lowers to `_ENV[<name>]` via ADR 0134's `__index` chain.
- Migration of every builtin (`print`, `tostring`, ..., `string.*`, `math.*`, etc.) into `_ENV` entries — chunk init populates them.
- Per-chunk `_ENV` rebind (`local _ENV = empty_env` ⇒ sandbox).

### Trigger to implement

Any of:

- (a) User code that rebinds `_ENV` (sandboxing pattern).
- (b) `pcall` / `error` (ADR 0153) lands and needs `_ENV` for error context.
- (c) `string` patterns (ADR 0155) wants `string.format` / `string.find` to be replaceable via `_ENV`.
- (d) `setfenv` / `getfenv` requested.

## Alternatives considered

- **Implement now in Phase 2.** Rejected — the builtin migration alone is L sized; ADR 0048's stand-in covers the dominant case without the heavy lifting.
- **Keep ADR 0048 forever.** Rejected — silent Lua-spec deviation; rebinding `_ENV` is a real language feature.
- **`_G` magic global without a real `_ENV` upvalue.** Rejected — Lua 5.4 mandates `_ENV` upvalue per function; `_G` is just `_ENV[<chunk-level>]`. A half-implementation drifts.
- **Roll into the GC ADR** (since `_ENV` is heap-allocated). Rejected — GC strategy (ADR 0145) decided the high-level approach; the `_ENV` implementation owns the concrete table allocation.

## Consequences

**Positive**
- ADR 0133's `_ENV` row is satisfied; Phase 2 close criterion advances.
- Future ADR has a clear plan to cite.
- ADR 0048's auto-declared-locals continues to work for existing test code.

**Negative**
- User-visible `_ENV` rebind / `getfenv` / `setfenv` feature gap persists in Phase 2.
- Phase 3 builtin migration is L sized.

**Locked in until superseded**
- Chunk-level `_ENV` upvalue (matches Lua 5.4).
- Builtin-name early bind preserved (with `_ENV.print`-style fallback).
- ADR 0048 stand-in stays the Phase 2 default.

## Documentation updates

- [x] §4 LIC — new `LIC-env-true-globals-strategy-1`.
- [x] §7 — closes `_ENV` open item with Phase 3 trigger.
- [x] §8 — adds 0154.
- [x] ADR 0133 deferral table — `_ENV` row gets `RESOLVED by ADR 0154 (Phase 3 trigger)` annotation.

## Test count delta

```
Step 0:   1366 (after ADR 0152)
C1 (this ADR + SoT updates): 1366 → 1366 (docs only)
```

Decision-only ADR.

## Critical files

- `docs/design/0154-env-true-globals-strategy.md` (this file).
- `docs/design/tagged-semantics.md` — §4 / §7 / §8.
- `docs/design/0133-phase2-completion-criteria.md` — deferral row annotation.
- `docs/design/0048-phase2-0a-auto-declare-globals.md` — `RESOLVED by ADR 0154 (Phase 3 supersede)` annotation.
- `docs/design/README.md` — index entry.

## Risks

| Risk | Mitigation |
|---|---|
| Builtin migration breaks existing call sites | Dual-arm approach: HIR early-bind stays for known cases; `_ENV` lookup falls back. |
| Sandbox tests force the `_ENV` rebind before the Phase 3 ADR | Sandbox test code can be deferred until the Phase 3 ADR lands. |
| Closure-cell ABI changes (ADR 0083) interact with `_ENV` upvalue | Phase 3 ADR will audit ADR 0083 and reuse the existing upvalue box mechanism. |

## Future work

- Phase 3 implementation ADR (per the bullets above).
- `setfenv` / `getfenv` Lua 5.1-style (Phase 3 likely rejects; Lua 5.4 spec dropped them).
- `package` / `require` system — builds on top of `_ENV`.
- Per-chunk sandboxing API.

## References

- [ADR 0048](0048-phase2-0a-auto-declare-globals.md) — current auto-declared-globals stand-in.
- [ADR 0083](0083-phase2-5c-full-closures.md) — closure cell mechanism that `_ENV` upvalue rides on.
- [ADR 0133](0133-phase2-completion-criteria.md) — deferral table row this ADR closes.
- [ADR 0134](0134-metatables-index-read.md) — `__index` chain that `_ENV` lookup uses.
- [ADR 0145](0145-gc-strategy.md) — GC strategy that `_ENV` heap allocation inherits.
- Lua 5.4 reference manual §3.5 — visibility / `_ENV` semantics.
