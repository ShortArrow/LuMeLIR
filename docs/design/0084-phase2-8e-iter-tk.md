# 0084. Phase 2.8e-iter-tk: TaggedValue-Key IndexAssign + Index Read

- **Status:** Accepted (read-side arms partially superseded by ADR 0087 / 0088)
- **Date:** 2026-05-06
- **Deciders:** ShortArrow

> **ADR 0087 / 0088 supersede note (2026-05-10):** the inline
> nil/missing-key traps that this ADR introduced in the Index
> TaggedValue arm have been restructured. ADR 0087 (`cc231a2`)
> moved the nil/NaN tag-validity gate to the probe chokepoint.
> ADR 0088 reified missing-key reads as a Nil-tagged TaggedValue
> slot (consumer chains its own contract downstream); the Index
> arm's diagnostic on missing-key arith now surfaces as
> `s_table_type_mismatch` ("table value type mismatch") instead
> of `s_table_missing_key`. The IndexAssign arm and the
> `pairs`-body idiom (`t[k] = v + 100`) are unchanged.

## Context

ADR 0080 (`pairs(t)`) shipped with `tests/phase2_8e_pairs.rs::
pairs_body_writes_separate_table_safely`, which used a workaround
pattern (aggregate into a side table) because the natural

```lua
for k, v in pairs(t) do
  t[k] = v + 100
end
```

was rejected at HIR by `is_hash_key_eligible`. The reason: `k` is the
iterator-bound TaggedValue local (its slot stores `{i64 tag, i64
payload}` where the tag is determined at runtime by the `next`
walker), and the IndexAssign / Index codegen had no runtime tag
dispatch on the key.

ADR 0080 logged this as `LIC-2.8e-pairs-tagged-key-write-1`
(pending). ADR 0084 resolves it by routing TaggedValue keys through
the same hash path that ADR 0079's tag-dispatched probe already
supports, with a runtime tag check for `nil` at the boundary.

## Decision

### HIR relaxation (1 line)

```rust
// src/hir/mod.rs
fn is_hash_key_eligible(k: ValueKind) -> bool {
    matches!(k,
        ValueKind::Number | ValueKind::String | ValueKind::Bool
        | ValueKind::Function(_) | ValueKind::Table
        | ValueKind::TaggedValue)              // <- new
}
```

Both `IndexAssign` and `Index` (read) use this guard, so the single
edit unblocks both writes and reads.

### Codegen runtime tag dispatch

Two new arms (one per IndexAssign and Index read) handle
`ValueKind::TaggedValue`:

1. **Slot pinning**: when the key expression is `Local(idx)` the
   TaggedValue local's existing slot (`slots[idx]`) is *already* a
   16-byte tagged search-key slot (tag at +0, payload at +8). It is
   handed directly to `emit_hash_probe_for_insert` /
   `emit_hash_probe_lookup`; no fresh `emit_build_search_key_slot`
   tmp is needed. Other expression shapes (e.g. `IndexTagged`) are
   out of scope and reject with `CodegenError::UnsupportedExpr`.
2. **Tag check**: load `tag = slot+0`. If `tag == TAG_NIL`, exit
   with the new `s_table_index_nil` global ("table index is nil",
   Lua spec §3.4.5). The check runs **before** the payload is
   reinterpreted as anything (Codex pre-ADR-0082 §4
   forward-edge-integrity discipline carried forward).
3. **Hash probe**: ADR 0079's `emit_hash_key_hash_dispatched` and
   `emit_hash_key_eq_dispatched` already pick up the tag from
   slot+0 and dispatch FNV / strcmp / cmpf / cmpi accordingly, so
   the existing probe loop walks the search slot correctly without
   per-tag specialisation at the call site.
4. **New-key commit (write side)**: when the bucket was empty, the
   16-byte search slot is copied raw into `entry+0` (tag word +
   payload word, two i64 stores) — `emit_value_slot_store_dispatched`
   has no `TaggedValue` arm, and the slot's tag/payload pair is
   already in slot-compatible form, so the raw copy is the simplest
   correct path. `count++` mirrors the static-key arm.
5. **Value store / delete**: identical to ADR 0079's static-key arm
   — `emit_value_slot_store_dispatched` for live values; TAG_DELETED
   tombstone for `t[k] = nil`.

Read path (Index): tag check + null-buf check + probe lookup +
`emit_value_slot_check_number` trapping value load. Returns f64 as
before; TaggedValue values would need `IndexTagged`, which keeps
its existing scope (out of ADR 0084).

### Array path bypass

TaggedValue keys *always* route to the hash path, even when the
runtime tag is `TAG_NUMBER` and the value happens to be an integer
in `[1, len]`. ADR 0079's hash dispatch handles `TAG_NUMBER` keys
via `cmpf Oeq` so this is correct semantically. The array part is
never updated through this path. `taggedvalue_key_write_number_keys`
was originally planned but excluded from the test surface — see
the **Limitations** section below.

## Alternatives Considered

- **Build a fresh tmp slot via a TaggedValue-aware
  `emit_build_search_key_slot`**: would require adding a TaggedValue
  arm to `emit_value_slot_store_dispatched` for raw 16-byte copy.
  Rejected as gratuitous: when the key is a Local, its slot is
  already a valid search-key slot.
- **Promote the static-key paths to runtime dispatch and remove the
  duplication**: would unify IndexAssign / Index into one tag-
  dispatched path. Rejected as scope creep — the static paths are
  hot, and fold optimally because the tag is a compile-time
  constant.

## Consequences

- **`LIC-2.8e-pairs-tagged-key-write-1` → resolved.**
- **Test totals: 944 → 951 green.** 7 new e2e in
  `tests/phase2_8e_tagged_key_indexassign.rs` (TaggedValue write
  for string / bool / function keys, read-side dispatch, write-then-
  read roundtrip, multi-iteration aggregation, ADR 0080 reframe).
  ADR 0080's `pairs_body_writes_separate_table_safely` is reframed
  to `pairs_body_mutates_existing_value_safely` using the natural
  in-place `t[k] = v + 100`.
- **LIC totals: 23 / 0 / 4 → 24 / 0 / 3** (resolved / partial /
  pending).
- **Source LOC**: HIR `+1`, codegen `+260` (`s_table_index_nil`
  global + IndexAssign TaggedValue arm + Index read TaggedValue
  arm). Tests `+150`. ADR 0084 + SoT updates `+250`.

### Limitations carried forward

- **`IndexTagged` with TaggedValue key**: still HIR-rejected (the
  `_ => unreachable!("IndexTagged key must be Number or String")`
  fall-through in `emit_local_init_tagged`). The non-trapping
  read-with-widening codepath needs a parallel TaggedValue-key arm.
  Tracked as a follow-up; existing tests don't hit it.
- **TaggedValue key with non-Local source**: e.g. `t[some_call()]
  = v` where the call returns TaggedValue. Rejected at codegen
  with `CodegenError::UnsupportedExpr`. Materialising a tmp slot
  via `emit_local_init_tagged`-style logic is the natural fix and
  fits a future Tidy First.
- **Number-tagged key write meets array-path read**: TaggedValue
  writes always go to hash; Number-keyed *reads* (`print(t[1])`)
  always go to array. After `for k, v in pairs(t) do t[k] = v +
  100 end` over an array-only table, the array still holds the
  original values, and the hash mirror holds the bumped ones. A
  future ADR (LIC pending) can unify the read-side dispatch so
  Number-keyed reads check the hash mirror after the array slot;
  out of ADR 0084's scope.
- **Runtime NaN as TaggedValue Number key**: `cmpf Oeq` excludes
  NaN, so probe always misses. Currently surfaces as the generic
  `s_table_missing_key` trap. Refining to a `s_table_index_nan`
  diagnostic is `LIC-2.6b-hash-key-nan-runtime-1` (pending,
  separate ADR candidate).

## Documentation updates

- [x] §1 slot layout — n/a (no new tag).
- [x] §2 producer / source taxonomy — n/a.
- [x] §3 consumer matrix — n/a.
- [x] §4 LIC consolidation — `pairs-tagged-key-write-1` moved to
      Resolved. Totals: **26 entries — 24 resolved / 0 partial /
      3 pending**.
- [x] §5 runtime tag invariants — n/a.
- [x] §6 cross-reference — new "TaggedValue-key IndexAssign /
      Index" subsection sketching the slot-pin + tag-check + probe
      dispatch.
- [x] §7 open questions — `pairs-tagged-key-write-1` removed. ADR
      0083 (Full closures, deferred) remains #1; hash-key
      runtime diagnostics promoted accordingly.
- [x] §8 ADR index — ADR 0084 row added; "Last updated" stamp
      bumped.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
