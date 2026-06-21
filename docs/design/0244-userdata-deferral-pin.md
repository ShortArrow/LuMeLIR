# 0244. userdata — Opaque-Number Shim Pin + Real-Type Deferral

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second (closing) M12 sub-ADR. Lua 5.4 §2.1 lists `userdata` as a primary value type — opaque blocks of memory owned by C (or in our case Rust) code, with optional metatables, GC tracking via `__gc`, and tag-distinct equality. Implementing a real `userdata` value kind requires:

1. New `ValueKind::Userdata` (or a TaggedValue tag) propagated through HIR + codegen.
2. GC tracking — userdata participates in the mark + sweep just like Tables. **Blocked on M3-extended Table DFS.**
3. `__gc` dispatch — userdata's primary metamethod use case. **Blocked on M10-stretch finalizer dispatch.**
4. Metatables-for-userdata. The existing Table metatable machinery (ADRs 0134-0140) doesn't extend trivially to a non-Table receiver.

Items 2-4 are all structurally gated by M3-extended. This ADR documents the deferral and pins the existing "opaque-Number" workaround that Bridge consumers can use today.

## Scope (literal)

- ✅ Pin: Bridge functions returning Number can be used as opaque handles. The user code holds the Number in a Lua Local, passes it back through Bridge calls.
- ✅ Pin: handle arithmetic works for values within the Phase B f64 precision band (±2^53).
- ✅ Pin: composed Bridge calls (`rust.add(rust.add(a, b), c)`) chain correctly.
- ✅ Documents the deferral list: `ValueKind::Userdata`, GC tracking, `__gc` dispatch, metatables.
- ❌ Real userdata `ValueKind`. Future ADR after M3-extended.
- ❌ Bridge GC interaction (Rust-owned Lua refs tracked by GC). Future ADR after M3-extended.
- ❌ Userdata metatables. Future ADR after the userdata `ValueKind` lands.
- ❌ Light userdata (Lua's distinction between `userdata` and `lightuserdata`). Future micro-ADR if demand arises.

## Decision

No new code. The Bridge already supports Number return shapes (ADR 0191 / 0224 / 0226). User code that needs an opaque handle today can return a 53-bit pointer / index / token as a Lua Number. Programs that work within the ±2^53 precision band (the vast majority of real handles — indices into a free-list, hash bucket ids, file descriptors, etc.) work correctly.

When M3-extended lands and Tables become GC-tracked, a future ADR can add `ValueKind::Userdata` riding on the same DFS + sweep infrastructure. Until then this surface-pin avoids regressing the workaround.

### Why ship this as a pin

Two reasons to commit pin tests rather than just deferring silently:

1. The workaround pattern is documented and stable. Future code-changes that accidentally widen the Bridge return type away from Number would silently break user programs that depend on this shape; the pin tests catch that.
2. The deferral list is recorded in the ADR so future readers (including future agents) know exactly what blocks the real userdata implementation.

## Tests

`tests/phase4_m12b_userdata_opaque.rs` (NEW, 3 e2e):

1. `local h = rust.add(3, 4); print(h)` → `"7"` — Number returns round-trip through a Local.
2. `print(rust.add(rust.add(1, 2), 3))` → `"6"` — chained Bridge calls compose.
3. Bridge handle arithmetic: `local h = rust.add(42, 0); print(h * 2)` → `"84"`.

## M12 milestone close

ADRs 0243 + 0244 land M12 at 2/2-3 minimum-viable close.

M12-stretch (deferred to M3-extended landing):
- Real `ValueKind::Userdata` with tag-distinct equality.
- GC interaction: userdata participates in mark + sweep.
- `__gc` dispatch on userdata.
- Metatables for userdata.
- `lightuserdata` (Lua's non-tracked opaque ptr).
- Bridge value-kind widening to Table (DFS-aware bridge marshaling).
- Rust panic → Lua error (no_std panic_handler integration, sibling of ADR 0243's error path).
- User-defined bridge crates (drop-in extensibility per ADR 0191 §Future).

## Test count delta

```
Step 0:  1603 (after ADR 0243)
C3 (3 e2e): 1603 → 1606
```

## References

- [Lua 5.4 §2.1](https://www.lua.org/manual/5.4/manual.html#2.1) — userdata type spec.
- [Lua 5.4 §2.5.3](https://www.lua.org/manual/5.4/manual.html#2.5.3) — `__gc` for userdata.
- [ADR 0191](0191-rust-lua-bridge-mvp.md) — Bridge MVP.
- [ADR 0220](0220-gc-tagged-value-roots.md) — M3 close documenting Tables-as-chunk-roots deferral.
- [ADR 0238](0238-gc-finalizer-field-pin.md) — sibling pin pattern (`__gc` field deferral).
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M12 milestone close.
