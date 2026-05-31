# 0136. `rawset` and `rawget` Builtins (hash key only)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

[ADR 0134](0134-metatables-index-read.md) and [ADR 0135](0135-metatables-newindex-write.md) wired `__index` / `__newindex` metamethod dispatch into the Index / IndexAssign chokepoints. Lua spec §6.1 documents two builtins — `rawset(t, k, v)` and `rawget(t, k)` — as the canonical *escape hatch* from the metamethod chain: they perform a direct hash operation on `t` regardless of any metatable.

Per [ADR 0133](0133-phase2-completion-criteria.md)'s deferral table, the `raw*` builtin family is the natural follow-up after the read+write metamethod pair landed. Codex post-`43b881c` review flagged it as the smallest principled cut and the natural sibling.

## Scope (literal)

**`rawset` and `rawget`, hash key only.** Number-key (array) forms are HIR-rejected; TaggedValue-key (`pairs`-body) variants are deferred. Everything else is explicitly out of scope:

- ❌ `rawset(t, n, v)` / `rawget(t, n)` with `n: Number` (array path) — the Number-key Index/IndexAssign ABI returns `f64` directly, which can't express the nil-on-miss semantics rawget needs. Deferred to a follow-up ADR alongside the array `__newindex` ABI decision.
- ❌ TaggedValue-key forms — same scope restriction as ADR 0084 / 0135.
- ❌ `rawequal(t1, t2)` and `rawlen(t)` — separate builtins, separate ADRs.
- ❌ `rawset(t, k, nil)` for hash delete — the existing nil-value path (ADR 0062 tombstone) already bypasses metatables, so the existing `t[k] = nil` syntax is enough. `rawset` is reserved for non-Nil writes that need to skip `__newindex`.

## Decision

### HIR

Two new `Builtin` variants in `src/hir/ir.rs`:

- `Builtin::RawSet` — arity `(3, 3)`, returns `Table` (per Lua spec `rawset` returns `t`).
- `Builtin::RawGet` — arity `(2, 2)`, returns `TaggedValue` (the value at `t[k]` or nil-tagged on miss).

`Builtin::from_name` maps bare identifiers `rawset` and `rawget` to the variants (same pattern as `setmetatable` / `getmetatable` from ADR 0134).

`lower_builtin_call` enforces kind constraints alongside the Assert / SetMetatable / Next checks:

- `rawset`: arg 0 = `Table`; arg 1 = hash-eligible key (String / Bool / Function / Table); arg 2 = non-Nil value (Number / String / Bool / Function / Table).
- `rawget`: arg 0 = `Table`; arg 1 = hash-eligible key.

A Number-key argument trips `HirError::TypeMismatch` with a "hash-eligible key (string, bool, function, or table)" message that surfaces the deferral scope clearly. A Nil value at `rawset`'s arg 2 trips a separate "non-nil value" message — Nil-clear is `t[k] = nil` territory.

### Codegen

New emit arms in `src/codegen/emit.rs` under the `Callee::Builtin(b)` match in `emit_expr`:

- `Builtin::RawSet`:
  1. Lower the three args to `target_ptr`, `key_value`, `value_v`.
  2. Build the search-key slot via `emit_build_search_key_slot`.
  3. Call the existing `emit_hash_indexassign_with_newindex` helper with **`remaining_hops = 0`** would trap — instead, the helper gains a `skip_metatable: bool` parameter. When `true`, the helper skips the `__newindex` probe in the `was_empty` branch and goes directly to the raw commit+write path. `rawset` passes `true`.
  4. Return `target_ptr` (Lua spec — `rawset` returns its first argument).

- `Builtin::RawGet`:
  1. Lower the two args to `target_ptr`, `key_value`.
  2. Build the search-key slot.
  3. Allocate a TaggedValue tmp slot, initialize Nil.
  4. Call `emit_hash_lookup_into_tagged_slot` with `HashLookupOutcome::NilOnMissing` directly (no fallback wrapper, no metatable chain).
  5. Return the tmp slot pointer (the `TaggedValue` value).

### Helper change

`emit_hash_indexassign_with_newindex` (ADR 0135) gains a leading `skip_metatable: bool` parameter:

```rust
fn emit_hash_indexassign_with_newindex(
    context, block,
    target_ptr,
    key_slot, key_kind, key_value,
    value_v, value_kind,
    remaining_hops,
    skip_metatable: bool,   // NEW
    types, loc,
)
```

When `skip_metatable == true`, the `was_empty == true` branch skips the metatable probe entirely and emits only the raw commit+write. The existing IndexAssign call site passes `false`; the new `rawset` emit arm passes `true`.

## Alternatives considered

- **Separate raw helper (duplication)** — emit `rawset` via its own inline hash-write copy. Rejected: that's exactly the duplication the ADR 0135 extract was supposed to prevent.
- **Single-ADR `raw*` family** (bundle `rawset` + `rawget` + `rawequal` + `rawlen`) — rejected: `rawequal` needs equality dispatch (separate concern) and `rawlen` reads array length (no metatable interaction at all). One ADR per family member keeps each decision reviewable.
- **Function-form `rawset` / `rawget`** — N/A; these are not metamethods, they're plain builtins.
- **Allow Number-key forms now** — would force the same array-OOB-widening decision deferred in ADR 0134 test 7. Out of scope.
- **`rawset(t, k, nil)` as hash delete** — covered by existing `t[k] = nil` syntax (ADR 0062). Adding rawset-nil duplicates the surface.

## Consequences

**Positive**
- Phase 2 metatables surface gains the canonical Lua spec escape hatch — code that needs to inspect raw storage (debug helpers, serialization, deep-copy, custom iterators built on top of `pairs`) can now bypass `__index` / `__newindex` without disabling them.
- Existing hash machinery (`emit_hash_lookup_into_tagged_slot`, `emit_hash_indexassign_with_newindex`) is reused; the only architectural change is the `skip_metatable` flag on the write helper.
- The helper signature change is local (1 new bool param, default-false at the existing call site).

**Negative**
- `rawset` and `rawget` are restricted to hash-eligible keys, which is a divergence from full Lua spec (the reference accepts any key including Number). The restriction matches the existing project-wide hash/array split — a future ABI ADR resolves both at once.
- Adding a `skip_metatable` flag to the helper is mild interface bloat; if more flags accumulate, a `MetatableMode` enum may be warranted.

**Locked in until superseded**
- Hash key only for both builtins. Number-key support is gated on the array-OOB-widening ADR.
- `rawset` returns `Table` (first arg, per Lua spec). `rawget` returns `TaggedValue` (the value at key or nil).

## Documentation updates

[`docs/design/tagged-semantics.md`](tagged-semantics.md) is the SoT for the TaggedValue runtime model (ADR 0068). This ADR touches:

- [x] §1 slot layout — **no change**.
- [x] §2 producer / source taxonomy — `rawget` joins as a TaggedValue producer (the lookup result tmp slot).
- [x] §3 consumer coverage matrix — IndexAssign hash-key consumer row gains a `rawset` annotation (skip-metatable variant of the write helper).
- [x] §4 LIC consolidation — new resolved entry `LIC-raw-builtins-1`.
- [x] §5 runtime tag invariants — **no change**.
- [x] §7 open questions — closes the `rawset` / `rawget` open items; adds `rawequal`, `rawlen`, Number-key `raw*`, TaggedValue-key `raw*` as new open items.
- [x] §8 ADR index — adds 0136 to the chronological table.

## Test count delta

```
Step 0:   1287 (1276 + 4 audit + 11 metatables read + 11 metatables write — 2 deferred)
Commit C2 (8 new e2e Red Day 0): 1287 → 1287 (existing green; 8 new red)
Commit C3 (HIR + codegen impl):  1287 → 1295 (all green)
```

## Critical files

- `src/hir/ir.rs` — 2 new `Builtin` variants + `from_name` + `arity` + `name` + `ret_kinds` + `param_kinds_for_arity` arms.
- `src/hir/mod.rs` — `infer_kind` arm + per-builtin kind checks in `lower_builtin_call`.
- `src/codegen/emit.rs` — `Callee::Builtin(RawSet | RawGet)` emit arms, `skip_metatable` parameter on `emit_hash_indexassign_with_newindex`.
- `tests/phase2_6plus_raw_builtins.rs` (NEW) — 8 e2e tests.
- `docs/design/tagged-semantics.md` — §2 / §3 / §4 / §7 / §8 updates.

## Risks

| Risk | Mitigation |
|---|---|
| `skip_metatable` flag on the existing write helper introduces subtle behaviour change at the existing call site | The default-false at the IndexAssign call site means existing semantics is preserved; the 11/12 metatables-write tests stay green. |
| `rawset(t, k, v)` with `k` that already exists (was_empty == false) accidentally fires `__newindex` | The was_empty branch is the ONLY place the metatable probe lives; skip_metatable just guards that branch. Existing-key overwrite was already direct per Lua §2.4. |
| HIR's Number-key rejection breaks user code that relied on Number-key rawset | Number-key rawset has never been supported (it predates this ADR). The diagnostic surfaces the deferral. |
| `rawget` returning `TaggedValue` while users expect Number / String / etc. | Same widening contract as `getmetatable` / `table.remove` / `io.read` — consumers see a TaggedValue and dispatch via tag. |

## Future work

- ADR for `rawequal(t1, t2)` and `rawlen(t)` (separate, smaller).
- Number-key `raw*` forms — bundled with the array-OOB-widening ADR (which also unblocks 0134 test 7 and 0135 test 9).
- TaggedValue-key `raw*` — bundled with the TaggedValue-key IndexAssign arm wiring (0135 test 9 follow-up).
- Same `skip_metatable` pattern for `__index` chokepoint if a `rawindex`-style escape needs it (no canonical Lua builtin, so probably not).

## References

- [ADR 0084](0084-phase2-8e-iter-tk.md) — TaggedValue-key restriction inherited.
- [ADR 0088](0088-phase2-6b-hash-lookup-miss.md) — `emit_hash_lookup_into_tagged_slot` chokepoint reused for `rawget`.
- [ADR 0133](0133-phase2-completion-criteria.md) — deferral table sequenced this ADR after 0135.
- [ADR 0134](0134-metatables-index-read.md) — `__index` chokepoint that `rawget` bypasses.
- [ADR 0135](0135-metatables-newindex-write.md) — `__newindex` chokepoint that `rawset` bypasses via the `skip_metatable` flag.
- Lua 5.4 reference manual §6.1 — `rawset` / `rawget` semantics.
