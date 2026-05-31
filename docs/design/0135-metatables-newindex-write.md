# 0135. Metatables `__newindex` Write-Path (Table-form, hash key only)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

[ADR 0134](0134-metatables-index-read.md) landed the read direction of metatable dispatch — `__index` Table-form, hash key only. The natural pair is the **write-path** metamethod `__newindex`, which [ADR 0133](0133-phase2-completion-criteria.md)'s deferral table sequences immediately after 0134 ("After 0134 ABI lands" trigger).

Per Codex post-`7a5cf9b` review verdict, `__newindex` Table-form is the smallest principled cut that follows 0134:

- ABI is already paid (40-byte header, `metatable_ptr` at offset 32) — no new allocations.
- 0134's chokepoint (`src/codegen/emit.rs:11569`, `emit_check_metatable_index_with_depth`) is structurally identical to what `__newindex` needs — `dst_slot` swaps to `value_slot`, recursion target shifts from `__index` to `__newindex`, max-hops constant is shared.
- The hash-key `IndexAssign` arm at `src/codegen/emit.rs:3505` already exposes a `was_empty` signal at `:3591` that is the exact "key really missing" branch Lua spec §2.4 wants the metamethod to fire on.

## Scope (literal)

**`__newindex` write-path, Table form only, hash key only.** Everything else is explicitly out of scope and lands as a separate ADR per [ADR 0133](0133-phase2-completion-criteria.md):

- ❌ `__newindex = Function` (call ABI integration deferred).
- ❌ Number-key (array) `__newindex` — existing grow-extend semantics from ADR 0057 preserved. The "missing key" definition for the array part (`key > length` vs `value == Nil`) is its own ABI decision and gets a separate ADR.
- ❌ Existing-key writes — Lua §2.4 explicitly says `__newindex` does **not** fire when the key already has a non-nil value in `t`; the existing-key overwrite path stays raw.
- ❌ `__add` / `__sub` / `__mul` / `__div` / `__mod` / `__pow` / `__unm` / `__idiv` / bitwise / `__eq` / `__lt` / `__le` / `__tostring` / `__concat` / `__call` (per-op ADRs per 0133).
- ❌ `rawset(t, k, v)` builtin (1-commit follow-up after this lands).
- ❌ Chain-depth runtime counter (the 8-hop static unroll inherited from 0134 is correct; the Lua-spec 2000 cap is deferred to a future refactor).
- ❌ `setmetatable(t, nil)` clear / `__metatable` field hiding / shared metatable + GC interaction.

## Decision

### Hook point

The hash-key `IndexAssign` arm probes `mt["__newindex"]` only inside the `was_empty == true` branch at `src/codegen/emit.rs:3591`. The existing `was_empty == false` branch (overwrite an already-present key) is **untouched** so Lua §2.4's spec compliance is preserved.

### Chain walk

The `__newindex` chain is walked via a new chokepoint helper modeled on 0134's read chokepoint:

```rust
fn emit_check_metatable_newindex_with_depth(
    context, block,
    target_ptr,              // table we just probed (key was missing here)
    user_search_key_slot,    // the missing key
    value_v, value_kind,     // the value to assign
    remaining_hops,          // static-unroll budget (starts at METATABLE_INDEX_MAX_HOPS = 8)
    types, loc,
) -> Value<i1>               // `handled_by_metatable` — caller skips raw write when true
```

Per-hop body:

1. Load `mt = *(target_ptr + TABLE_OFF_METATABLE)` (offset 32).
2. If `mt` is null → return `handled = false`. Caller does the raw write to `target_ptr`.
3. Probe `mt["__newindex"]` via `emit_hash_lookup_into_tagged_slot` (raw lookup — Lua spec does **not** chain on the metatable's own metatable when resolving the metamethod field itself).
4. Check the tag of the `__newindex` result slot:
   - `TAG_NIL` → `handled = false` (raw write to `target_ptr`).
   - `TAG_TABLE` → recursively write `__newindex_table[key] = value` via the same hash-write path with `remaining_hops - 1`. Set `handled = true`.
   - Any other tag (Function / Number / Bool / String / Deleted) → trap `s_metatable_newindex_unsupported_kind`. Function-form `__newindex` is out of scope.
5. `remaining_hops == 0` is the base case — trap `s_metatable_chain_too_deep` (shared with ADR 0134). This catches the cycle case `setmetatable(mt, mt); mt.__newindex = mt` where the chain genuinely never terminates.

### Helper extraction

The existing inline `IndexAssign` hash-key body (`src/codegen/emit.rs:3505`–`3760`) is extracted into `emit_hash_indexassign_with_newindex` so that the `__newindex` chain recurses into the *same* code path the user-level write does. The helper signature accepts everything the inline arm took plus `remaining_hops`:

```rust
fn emit_hash_indexassign_with_newindex(
    context, block,
    target_ptr,
    key_slot, key_kind, key_value,
    value_v, value_kind,
    remaining_hops,
    types, loc,
)
```

The two HirStmtKind::IndexAssign arms that need the metatable path — the static-kind String/Bool/Function/Table key arm (`:3505`) and the TaggedValue-key arm (`:3770`) — both replace their existing inline hash-write with a call to this helper. The Number-key array arm (`:3353`) is left as-is.

### Diagnostic + global string

Two new globals registered at module init:

```rust
emit_string_global(ctx, mod, i8, "s_metatable_newindex_field_name",
                   "__newindex\0", loc);
emit_string_global(ctx, mod, i8, "s_metatable_newindex_unsupported_kind",
                   "attempt to assign through non-table '__newindex' metafield\0", loc);
```

The chain-too-deep diagnostic `s_metatable_chain_too_deep` is shared with ADR 0134 (same Lua-spec phrasing).

## Alternatives considered

- **`__newindex` for Number-key array writes** — would force a decision on what "missing key" means for the array part (`key > length` vs `array_buf[key-1].tag == Nil`). Both interpretations break some existing test. Defer to a separate ABI ADR that pins the choice.
- **Fire `__newindex` on existing-key overwrites** — violates Lua §2.4. The whole point of the metamethod is to handle *new* keys; overwrites are direct.
- **Bundle `rawset` builtin** — Codex flagged this as ADR-per-decision violation. `rawset` is a clean 1-commit follow-up once this lands.
- **Function-form `__newindex` in the same ADR** — pulls in the call ABI (varargs / return-kind widening / recursion budget). Sequenced after a separate call-ABI cleanup ADR per ADR 0133.
- **Runtime depth counter (replace static unroll)** — premature; 0134's 8-hop static unroll passes 11/12 tests including the cycle case.
- **Strict §2.4 interpretation extension** (fire on any nil-valued key) — doesn't match real Lua, where only `was_empty` in the hash structure matters.

## Consequences

**Positive**
- Phase 2 metatables read+write pair complete; subsequent ADRs (`_ENV`, `pcall` error-table shape, arith / cmp metamethods) all build on this contract.
- Null-mt fast path (`i64 load + nonzero cmpi` at offset 32) keeps tables-without-metatable at near-zero cost; the existing 1276-test corpus is the regression net.
- The helper extract `emit_hash_indexassign_with_newindex` factors the historical inline hash-write into a callable unit, which any future write-side metamethod (e.g. `__add` on an indexed assignment) can reuse.

**Negative**
- The `IndexAssign` hash arm's MLIR layout changes (was inline, becomes a helper call). The helper body is structurally identical, but `git blame` for the affected lines shifts.
- The chain-too-deep trap fires at `remaining_hops == 0` only when the chain was *still extending* at the last hop. Legitimate 8-deep chains pass; 9+ deep chains and cycles both trap. A future runtime-counter ADR may raise the cap to Lua's spec 2000.

**Locked in until superseded**
- The `was_empty` gate is the chosen Lua §2.4 hook point.
- The static unroll bound is `METATABLE_INDEX_MAX_HOPS = 8` (shared with ADR 0134).
- Array writes do not fire `__newindex` until a successor ADR decides the "missing key" definition.

## Documentation updates

[`docs/design/tagged-semantics.md`](tagged-semantics.md) is the SoT for the TaggedValue runtime model (ADR 0068). This ADR touches:

- [x] §1 slot layout — **no change** (16-byte slot unchanged; metatable_ptr lives in the table header).
- [x] §2 producer / source taxonomy — adds `metatable.__newindex` chain write as a new TaggedValue producer (writes go into the destination table's slots, kind-dispatched).
- [x] §3 consumer coverage matrix — IndexAssign hash-key and TaggedValue-key rows gain a `__newindex dispatch` annotation.
- [x] §4 LIC consolidation — new resolved entry `LIC-metatable-newindex-write-1`.
- [x] §5 runtime tag invariants — **no change**.
- [x] §7 open questions — moves `__newindex Table dispatch` from open to resolved; adds Function-form `__newindex`, Number-key array `__newindex`, and `rawset` as new open items.
- [x] §8 ADR index — adds 0135 to the chronological table.

## Test count delta

```
Step 0:    1276 (current — 1260 + 4 audit + 12 metatables read)
Commit C2 (12 new e2e Red Day 0): 1276 → 1276 (existing green; 12 new red)
Commit C3 (helper + extract + wiring): 1276 → 1288 (all green)
```

## Critical files

- `src/codegen/emit.rs` — `emit_check_metatable_newindex_with_depth` (new ~150 LOC), `emit_hash_indexassign_with_newindex` helper extract (~80 LOC from existing inline), 2 new globals at module init, 2 IndexAssign arm rewires.
- `tests/phase2_6plus_metatables_newindex_write.rs` (NEW) — 12 e2e tests.
- `docs/design/tagged-semantics.md` — §2 / §3 / §4 / §7 / §8 updates.
- `docs/design/0058-phase2-6b-hash-keys.md`, `0060-phase2-6c-tag-hash.md`, `0134-metatables-index-read.md` — future-work annotations.
- `docs/design/0133-phase2-completion-criteria.md` — deferral table row updated.

## Risks

| Risk | Mitigation |
|---|---|
| Existing-key writes accidentally fire `__newindex` (Lua §2.4 violation) | Helper invocation lives only inside the `was_empty == true` branch at `emit.rs:3591`. Test 5 (existing-key overwrite) pins this. |
| Array (Number-key) writes redirect to metatable surprisingly | Number-key arm at `emit.rs:3353` left untouched. Test 8 (Number-key array write) pins this. |
| Helper extraction introduces a subtle hash-write regression | The extraction is a 1:1 code move; the existing 1276 tests are the safety net for Commit C3. |
| Chain cycle `setmetatable(mt, mt) + mt.__newindex = mt` doesn't trap | Test 3 builds the real cycle (matching the lesson from ADR 0134 Test 4); 8-hop static unroll bottoms out into the chain-too-deep trap. |
| `__newindex = Function` runtime trap fires from unexpected callers | Trap message `s_metatable_newindex_unsupported_kind` is explicit; the future Function-form ADR will replace the trap with a call dispatch. |

## Future work

- ADR for `rawset(t, k, v)` (next). **RESOLVED by [ADR 0136](0136-raw-set-get-builtins.md) (2026-05-31)** (bundled with `rawget`, hash key only).
- ADR for `__newindex = Function` form (after call ABI cleanup).
- ADR for Number-key (array) `__newindex` (after the "missing key in array part" ABI decision).
- Promote static unroll to runtime depth counter (Lua spec's 2000 cap) when a real workload needs it.
- Arith / cmp / `__tostring` / `__concat` / `__call` metamethods (per ADR 0133).
- `_ENV` / true globals (downstream of this ADR, supersedes 0048).
- GC strategy decision (still no heap-allocated metatable trigger).

## References

- [ADR 0057](0057-phase2-6a-grow-array-push.md) — array grow-extend semantics that the Non-goals preserve.
- [ADR 0058](0058-phase2-6b-hash-keys.md), [ADR 0060](0060-phase2-6c-tag-hash.md) — earlier `__newindex` deferrals (this ADR closes the Table-form hash side).
- [ADR 0068](0068-phase2-6c-tag-doc-consolidate.md) — TaggedValue SoT updated per §Documentation updates above.
- [ADR 0084](0084-phase2-8e-iter-tk.md) — TaggedValue-key IndexAssign restriction inherited.
- [ADR 0088](0088-phase2-6b-hash-lookup-miss.md) — hash probe helper reused for the `mt["__newindex"]` lookup.
- [ADR 0133](0133-phase2-completion-criteria.md) — scope freeze that motivates the literal "Table-form, hash key only" boundary.
- [ADR 0134](0134-metatables-index-read.md) — read-path mirror; chokepoint helper, max-hops constant, and `s_metatable_chain_too_deep` shared.
