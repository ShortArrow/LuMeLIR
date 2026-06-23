# 0259. `__gc` Finalizer Runtime Dispatch (N3-A)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Context

Second N3 sub-ADR (sibling of [ADR 0258](0258-close-runtime-dispatch.md) N3-D). ADR 0238 pinned `__gc` as a metatable field that round-trips through normal Table machinery; the runtime "fire on unreachable" dispatch was explicitly deferred. ADR 0163 specified the long-term two-pass sweep design. This ADR lands the minimal runtime dispatch: at sweep time, every WHITE Table about to be freed has its metatable probed for `__gc`; if it's a Function it is invoked before `free`.

The preconditions are now satisfied: ADR 0254-0257 (N2-A/B/C/D) made Tables reachable as WHITE in sweep. ADR 0258 (N3-D) established the placeholder-arg metamethod ABI strategy that bypasses the `compatible_user_functions` exact-match block — we reuse the same approach here with sig `(Number) → ()` instead of `(Number, Number) → ()`.

## Scope (literal)

- ✅ At sweep time, before unlink + free, each WHITE GC node has its type tag probed; if `GC_TYPE_TABLE`, the table payload's `metatable_ptr` is loaded.
- ✅ When `metatable_ptr != null`, `mt["__gc"]` is probed via `emit_hash_lookup_into_tagged_slot` (ADR 0088 chokepoint).
- ✅ When the probe slot's tag is `TAG_FUNCTION`, the finalizer dispatches via `emit_dispatch_chain_from_slot_ptr` (ADR 0146 / 0258 precedent) with sig `(Number) → ()` and arg `0.0` (matches user fn default param ABI per ADR 0258 §"Why this ABI"; the canonical metamethod body `function(t) ... end` matches the sig by construction).
- ✅ `gc_candidates` is computed once at codegen time from `chunk.functions`, filtered to user fns whose declared params are exactly `[Number]` and `ret_kinds` is empty.
- ✅ Single-pass: probe → call → unlink → free. Lua spec's two-pass collect-then-fire-then-free (ADR 0163) is not yet implemented — see below.
- ✅ Skip semantics (no trap on any of these):
  - Non-Table GC nodes (`GC_TYPE_HASH_BUF`, `GC_TYPE_ARRAY_BUF`, `GC_TYPE_SCRATCH_BUF`).
  - Tables with no metatable.
  - Metatables with no `__gc` field.
  - Metatables with non-Function `__gc` (e.g. string, number — per Lua spec lenient rule).
- ✅ Lifted ADR 0238 §Scope's "runtime dispatch deferred" caveat; the pin remains valid for the field-handling surface.
- ❌ **Real `(Table)` arg pass-through** — same dynamic-typing question as ADR 0258 (placeholder `0.0` instead of the real table ptr). Finalizers that access `t` would see f64 zero. Spec-strict applications must wait for the umbrella ABI ADR.
- ❌ **Two-pass sweep (resurrection safety)** — ADR 0163's two-pass algorithm: collect all WHITE+`__gc` pairs, run all finalizers, then free WHITE. Today's single-pass would re-finalize a resurrected object next cycle (the object remains in `g_gc_head` unless the finalizer stores it somewhere reachable, which is the whole point of resurrection). Documented deviation. ADR 0163 §Resurrection retains the long-term plan.
- ❌ **`__gc` on userdata** — N3-C precondition. Today only Tables can carry a metatable in `g_gc_head`.
- ❌ **Finalizer that re-allocates** — if `__gc` calls `collectgarbage()` recursively or allocates more than threshold, the sweep re-enters. Today's code does not guard against this; the test surface does not exercise it. Document as a known foot-gun.
- ❌ **Re-finalization prevention** — ADR 0163 §Resurrection-during-finaliser. Out of scope.

## Decision

### Why the ADR 0258 ABI strategy fits

`__gc` always takes exactly one argument (the value being finalized). User fn default param kind is `Number` and default `ret_kinds` is empty. A user fn declared `function(t) print("done") end` therefore has signature `(Number) → ()` at the HIR level — matching `compatible_user_functions`. Dispatching with arg `(0.0,)` lets the finalizer body run; metamethods that ignore `t` (the common case for resource release / counter increment / logging) work correctly. The same trade-off documented in ADR 0258 §"Why this ABI" applies.

### Why single-pass works for landing

The Lua-spec two-pass sweep is needed for finalizer **resurrection** correctness — if finaliser A resurrects object B, the spec requires that B's finaliser still runs in the same cycle. Today's mark phase (post-N2) doesn't track WHITE+finalizable objects separately; lifting that requires extending the chunk-root walk to skip-but-record-finalizable objects, then a second sweep pass after all finalizers ran. That's a meaningful chunk of work and not necessary for the canonical "release a handle on unreachable" idiom. Single-pass lands the user-visible behavior; ADR 0163's two-pass design remains the long-term plan.

### Why arg pass-through is shared with ADR 0258

The dynamic-typing mismatch between metamethod call shape and user fn declared signature is identical to ADR 0258's. Resolving it for `__close` resolves it for `__gc`, `__index Function form`, `__call`, and any future Function-valued metamethod. A single follow-up ADR will land the umbrella resolution rather than per-metamethod fixes.

## Tests

`tests/phase4_n3a_gc_finalizer.rs` (NEW, 3 e2e, all Green):

1. **Finalizer fires on fn-frame Table** — Table allocated inside a user fn's frame becomes unreachable when the fn returns (per ADR 0257 N2-D frame pop). Subsequent `collectgarbage()` sweeps it WHITE and fires `__gc` before free. Test asserts `"finalized"` appears before the trailing `"after"`.
2. **Finalizer does NOT fire for rooted Table** — A chunk-level Table stays rooted across `collectgarbage()`. `__gc` never fires. Lua-spec-correct: finalizers fire on unreachable, not on every collect.
3. **No `__gc` field does not trap** — A Table with a metatable that lacks `__gc` collects normally; no trap, no spurious dispatch.

`tests/phase4_m10_gc_field_pin.rs` (the ADR 0238 pin tests) stays Green — the field-handling surface is unchanged.

## Test count delta

```
Step 0:  1646 (after ADR 0258)
N3-A (impl + 3 new e2e):  1646 → 1649
```

## What this unblocks

- The canonical "release a resource on table unreachability" idiom (file handles, mutex tokens, OS-resource wrappers) works for the common case.
- N3-B (`__mode` weak tables) can reuse the same `cur_ptr → metatable_ptr` probe path in sweep.
- N3-C (userdata) can hook the same dispatch once `GC_TYPE_USERDATA` lands.

Still gated:
- Real `(Table)` arg pass-through (umbrella ABI ADR).
- Two-pass resurrection safety (ADR 0163 §Resurrection).
- Finalizer-during-finalizer reentrancy.

## References

- [ADR 0163](0163-gc-finaliser.md) — long-term two-pass strategy that this ADR partially lands.
- [ADR 0238](0238-gc-finalizer-field-pin.md) — predecessor pin; runtime deferral now lifted.
- [ADR 0258](0258-close-runtime-dispatch.md) — N3-D sibling; same ABI strategy reused.
- [ADR 0088](0088-table-hash-lookup-chokepoint.md) — hash-lookup chokepoint reused for the probe.
- [ADR 0146](0146-call-metamethod.md) — closure-cell call ABI; `emit_dispatch_chain_from_slot_ptr` reused.
- [ADR 0257](0257-gc-frame-root-walk.md) — N2-D frame walk; enables a Table to reach WHITE on fn return.
- [Lua 5.4 §2.5.3](https://www.lua.org/manual/5.4/manual.html#2.5.3) — finalizers spec.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N3-A in the N1-N10 path.
