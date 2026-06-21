# 0238. `__gc` Field — Surface Acceptance Pin + Runtime Deferral

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M10 sub-ADR. [ADR 0163](0163-gc-finaliser.md) decided the long-term `__gc` finalizer strategy (two-pass sweep: collect WHITE objects with `__gc`, run all finalizers, then free). The implementation is gated on two preconditions that the current M3 close does NOT satisfy:

1. **Tables in `g_gc_head`.** ADR 0218 / 0219 / 0220's chunk-safe predicate excludes any chunk with Tables in its slots — those programs run in v1 safety mode (mark-all-BLACK, sweep frees nothing). Without Tables ever reaching WHITE status, `__gc` has no objects to fire on.
2. **Table DFS through array + hash parts.** Even when Tables become tracked, the mark phase needs to walk their child references to avoid spurious frees of live elements. ADR 0156's roadmap parks DFS at the same M3-extended slot as `__gc`.

This ADR ships the narrow piece available today: pin that `__gc` as a metatable field works through the existing Table machinery — declarable, retrievable, callable, shareable across tables. The runtime auto-dispatch on unreachable lives in a future M10-stretch sub-ADR alongside the M3-extended Table DFS.

## Scope (literal)

- ✅ `__gc` Function value can be stored on a metatable: `mt.__gc = function(t) ... end`.
- ✅ `setmetatable(t, mt)` succeeds without error when `mt.__gc` is a Function value.
- ✅ The Function value round-trips: `mt.__gc` after assignment reads back as `function` (via `type()`).
- ✅ A single metatable carrying `__gc` may be attached to multiple Tables; the field persists.
- ✅ Existing Table value-kind validation (`type(mt.__gc) == "function"`) holds today.
- ❌ Runtime auto-dispatch on unreachable. M10-stretch — paired with M3-extended Table DFS.
- ❌ Two-pass sweep (collect-then-fire-then-free) per ADR 0163. M10-stretch.
- ❌ Finalizer resurrection-during-finalizer safety (Lua spec §2.5.3 deferral). M10-stretch.
- ❌ Special-purpose `Builtin::Finalize` to fire finalizers from user code. Not required by Lua spec; out of scope.
- ❌ Metatable-field signature validation. `__gc = "string"` should be a no-op per Lua spec; currently the value is just stored without spec-conformance verification — acceptable until the runtime dispatch lands.

## Decision

No new code. The `__gc` name has no special meaning at parser / HIR / codegen layer today; it flows through the existing Table machinery (ADR 0134 `__index`, ADR 0135 `__newindex`, ADR 0136 raw set/get) as a plain string key.

Future M10-stretch will:
- Add a runtime registration list at `setmetatable` time when the mt has a Function-valued `__gc` field.
- Hook the sweep loop (ADR 0163 §"Hook point") to scan the list and dispatch.
- Lift the M3 chunk-safe predicate's Table exclusion (ADR 0220 §M3-stretch) so Tables actually reach WHITE.

## Tests

`tests/phase4_m10_gc_field_pin.rs` (NEW, 3 e2e):

1. `mt = { __gc = function(t) return 1 end }; type(mt.__gc)` → `"function"`.
2. `setmetatable({}, mt)` with `mt.__gc` Function succeeds → `"ok"`.
3. Same metatable attached to two Tables; `mt.__gc` field persists → `"function"`.

## Test count delta

```
Step 0:  1571 (after ADR 0237)
C3 (3 e2e): 1571 → 1574
```

## References

- [ADR 0156](0156-gc-architecture-v1.md) — GC architecture roadmap.
- [ADR 0163](0163-gc-finaliser.md) — `__gc` finalizer strategy.
- [ADR 0134](0134-metatables-index-read.md) / [ADR 0135](0135-metatables-newindex-write.md) — metatable machinery the field rides on.
- [ADR 0218](0218-gc-chunk-safe-real-freeing.md) — chunk-safe predicate that excludes Tables.
- [ADR 0220](0220-gc-tagged-value-roots.md) — M3 closing; documents Tables-in-chunk-roots as deferred.
- [Lua 5.4 §2.5.3](https://www.lua.org/manual/5.4/manual.html#2.5.3) — finalizers spec.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M10 milestone.
