# 0139. TaggedValue-key + TaggedValue-value IndexAssign `__newindex` Wiring

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

[ADR 0135](0135-metatables-newindex-write.md) wired `__newindex` into the hash-key static-kind `IndexAssign` arm via `emit_hash_indexassign_with_newindex`, but explicitly **deferred** the TaggedValue-key arm (test 9, `taggedvalue_key_newindex_redirects_through_metatable`). The `pairs`-body idiom `for k, v in pairs(src) do dst[k] = v end` — with `dst` carrying an `__newindex` metatable — therefore did not fire the metamethod even though all the supporting machinery existed.

The deferral was a scope decision, not a design gap. With [ADR 0136](0136-raw-set-get-builtins.md) and [ADR 0137](0137-raw-equal-len-builtins.md) landed, this is Tier 1 Step M on `plans/zany-giggling-cat.md` — completing the metatables write-path's coverage by routing the TaggedValue-key arm through the shared helper.

The wiring is non-trivial because two new gaps surface:

1. **TaggedValue value commit.** The helper's existing key/value commit uses `emit_value_slot_store_dispatched`, which has no TaggedValue arm (would `unreachable!`). The TaggedValue source carries the discriminator at runtime — a 16-byte raw slot copy (tag at +0, payload at +8) preserves it.
2. **`emit_expr` on a TaggedValue Local traps on non-Number.** The IndexAssign codegen entry lowers `value` via `emit_expr` before the key-kind branch. For a TaggedValue Local value (`v` from `pairs`), `emit_expr` enforces TAG_NUMBER and loads f64 — wrong for the helper, which needs the slot ptr.

## Scope (literal)

**TaggedValue-key IndexAssign with non-Nil value routes through `emit_hash_indexassign_with_newindex`.** TaggedValue value, like the existing TaggedValue key (ADR 0084), is restricted to **Local source** at codegen — other shapes (Index / Call results) materialise through a tmp slot only when the broader TaggedValue-IndexAssign ABI ADR lands.

Out of scope:

- ❌ TaggedValue-key IndexAssign with non-Local value source. Materialising into a tmp slot is straightforward but premature; no current `pairs`-body idiom needs it.
- ❌ Number-key (array) `__newindex` — separate ABI ADR.
- ❌ TaggedValue value in the static-key arm — no observed call shape; deferred.

## Decision

### HIR

`src/hir/mod.rs` `IndexAssign` lower widens the value-kind matrix: `ValueKind::TaggedValue` is now accepted when the key is a non-Number (hash path). The Number-key (array) arm still rejects TaggedValue — array slots need a concrete kind for grow-extend.

### Codegen

`src/codegen/emit.rs`:

- New helper `emit_copy_tagged_slot_16b(src_slot, dst_slot)` — raw 16-byte tagged-slot copy (load tag at +0 / store, load payload at +8 / store). Used at the key and value commit points inside the helper below for TaggedValue sources.
- `emit_hash_indexassign_with_newindex` extended:
  - Key commit: if `key_kind == TaggedValue` → `emit_copy_tagged_slot_16b(key_slot, entry_ptr)`; else existing `emit_value_slot_store_dispatched(entry_ptr, key_value, key_kind)`.
  - Value commit: if `value_kind == TaggedValue` → `emit_copy_tagged_slot_16b(value_v, value_slot_ptr)` (`value_v` is the source slot ptr in this case); else existing dispatched store.
- IndexAssign entry: when `key_kind == TaggedValue && value_kind == TaggedValue`, **skip** the early `emit_expr(value)` and substitute a placeholder f64 — the TaggedValue-key arm then overrides `value_v` with `slots[idx]` (the value Local's slot ptr) before calling the helper. This avoids the `emit_expr`-traps-on-non-Number path that would otherwise fire for a TaggedValue Local whose runtime tag isn't TAG_NUMBER.
- The TaggedValue-key arm dispatches: non-Nil value → call helper; Nil value → existing inline soft-delete (Lua spec — `t[k] = nil` does not consult `__newindex`).

## Alternatives considered

- **Materialise TaggedValue value through a tmp slot when source isn't a Local.** Rejected for this ADR — extends scope to the full TaggedValue-IndexAssign ABI. Tracked as a future-work bullet here so the next ADR that needs it inherits the requirement.
- **Inline the metatable probe in the TaggedValue-key arm (no helper).** Rejected — duplicates ADR 0135's chokepoint and re-creates the maintenance burden the helper was extracted to remove.
- **Pass the TaggedValue Local's `LocalId` down to the helper.** Rejected — the helper is codegen-layer; routing HIR LocalIds through it would break the abstraction. The slot ptr is the natural interface.

## Consequences

**Positive**
- The `pairs`-body idiom `dst[k] = v` correctly fires `__newindex` when `dst` has a metatable. The last deferred test from ADR 0135 (test 9) goes Green.
- The metatables write-path coverage matrix is now complete for hash-key static + hash-key TaggedValue. The remaining red — ADR 0134 test 7 (array OOB) — is the only deferred test on Phase 2.6+.
- `emit_copy_tagged_slot_16b` is a clean reusable primitive — future ADRs (TaggedValue rehash migration, TaggedValue table-element copy) inherit it.

**Negative**
- The IndexAssign entry now has a peek-ahead value-kind check before lowering `value`. One extra `infer_kind` call per IndexAssign — negligible.
- TaggedValue-value source restricted to Local at codegen. Non-Local sources surface as `UnsupportedExpr` until the broader TaggedValue-IndexAssign ABI ADR lands.

**Locked in until superseded**
- TaggedValue-value Local-only restriction. The full ABI ADR may relax this.
- `emit_copy_tagged_slot_16b` is the canonical TaggedValue → TaggedValue slot copy primitive.

## Documentation updates

- [x] §1–§3 — **no change** (slot layout, producer, consumer matrices unchanged; this ADR uses existing TaggedValue surfaces).
- [x] §4 LIC consolidation — new resolved entry `LIC-taggedvalue-key-newindex-wiring-1`.
- [x] §7 open questions — closes the ADR 0135 test 9 deferral; adds non-Local TaggedValue-value as a new open item.
- [x] §8 ADR index — adds 0139.

## Test count delta

```
Step 0:   1304 (1301 + 3 setmetatable-nil) — ADR 0135 test 9 still red
Commit C2 (HIR + codegen impl):  1304 → 1305 (test 9 flips Green)
```

## Critical files

- `src/hir/mod.rs` — `IndexAssign` value-kind matrix widens TaggedValue accepted for non-Number keys.
- `src/codegen/emit.rs`:
  - IndexAssign entry: peek value_kind; skip early `emit_expr` when both kinds are TaggedValue.
  - TaggedValue-key arm: non-Nil branch routes through helper, with TaggedValue-value Local restriction.
  - `emit_copy_tagged_slot_16b` (NEW, ~15 LOC).
  - `emit_hash_indexassign_with_newindex` key + value commit branches on TaggedValue.
- `docs/design/tagged-semantics.md` — §4 / §7 / §8 updates.

## Risks

| Risk | Mitigation |
|---|---|
| Non-Local TaggedValue value silently lowers wrong | Codegen returns `UnsupportedExpr` for non-Local TaggedValue value. Test 9 only exercises the Local source. |
| Existing static-key IndexAssign regresses | The static-key arm is untouched; only the TaggedValue-key arm and the IndexAssign entry's value lowering changed. All ADR 0135 tests 1-8 + 10-12 stay green. |
| Soft-delete (Nil value) regresses for TaggedValue key | Nil-value branch is left inline (unchanged). |
| `emit_copy_tagged_slot_16b` produces wrong tag/payload offsets | Mirrors the existing inline raw copy in the TaggedValue-key arm (lines ~3863-3884 before this ADR). |

## Future work

- TaggedValue-value IndexAssign with non-Local source — folds into the next IndexAssign ABI ADR.
- TaggedValue value in the static-key arm — same.
- Array OOB widening (ADR 0134 test 7 / 0136 Number-key raw*) — the last Phase 2.6+ deferred test.

## References

- [ADR 0084](0084-phase2-8e-iter-tk.md) — TaggedValue-key IndexAssign Local-source restriction (inherited here for the value side).
- [ADR 0135](0135-metatables-newindex-write.md) — `__newindex` write path; test 9 was the deferred entry this ADR resolves.
- [ADR 0136](0136-raw-set-get-builtins.md) — `skip_metatable` flag on the helper used here.
- [ADR 0138](0138-setmetatable-nil-clear.md) — sibling Tier 1 step (B).
- Lua 5.4 reference manual §2.4 — `__newindex` semantics.
