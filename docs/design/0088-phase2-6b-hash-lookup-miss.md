# 0088. Phase 2.6b-hash-lookup-miss: Hash Read Lookup Miss Reified as Nil-tagged TaggedValue

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-10
- **Deciders:** ShortArrow

## Context

ADR 0084 introduced `Local(TaggedValue)`-key Index read with an inline
`emit_hash_probe_lookup` trap on missing — a Lua §3.4.5 violation
discovered during the ADR 0087 review. The same buggy pattern existed
in the static-key Index arm (String/Bool/Function/Table) at
`emit.rs:6589-6720`, where a missing-key read fired `s_table_missing_key`
exit instead of returning nil.

Both arms duplicated the
`null_buf check → trap + emit_hash_probe_lookup → trap on missing`
shape. The proven mirror lived at `emit.rs:4426-4604`
(`emit_local_init_tagged` String/Bool/Function/Table arms) which uses
`emit_hash_probe_for_insert + null_buf → store Nil + key_at_null →
store Nil`. ADR 0088 generalises the proven shape into a single
chokepoint and retires `emit_hash_probe_lookup`.

`codex review v3` flagged Strong Go on the LIC and Refactor on plan
v1; v2 reflected codex feedback by:
- Moving the new `HashLookupOutcome` enum from `tagged.rs` (where it
  would have been a "decision-less pure abstraction") to `emit.rs`
  private (where it is honestly a consumer-contract config).
- Splitting the test set into Red 2 / regression-pin 2 to match the
  actual TDD trajectory.
- Reframing the ADR title from "missing-key trap message fix" to
  "hash read lookup miss reified as Nil-tagged TaggedValue".

## Decision

### `emit.rs` private `enum HashLookupOutcome`

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // TrapMissing reserved for next() / strict-key consumers
enum HashLookupOutcome {
    NilOnMissing,
    TrapMissing,
}
```

Lives in `emit.rs` because it is a consumer-contract config of the
lookup helper, not a tag-layer concept. `tagged.rs` stays focused on
slot layout / tag constants / pure store-check helpers. No
`policy_for_<context>`-style decision function — the variant *is* the
decision; call sites pick directly.

### `emit.rs` chokepoint helper `emit_hash_lookup_into_tagged_slot`

```rust
fn emit_hash_lookup_into_tagged_slot(
    context, block,
    target_ptr, search_key_slot, dst_slot,
    outcome: HashLookupOutcome,
    types, loc,
);
```

Encapsulates `null_buf check → emit_hash_probe_for_insert →
key_at_null check → outcome dispatch`. The dst slot is always a
16-byte TaggedValue layout; the helper is value-kind agnostic.

- **Found**: raw 16-byte tagged copy `{tag, payload}` from the entry's
  value slot into `dst_slot`.
- **Missing** (either `hash_buf == null` OR `key_at_null == true`):
  dispatch on `outcome`:
  - `NilOnMissing` → `emit_value_slot_store_nil(dst_slot)`.
  - `TrapMissing` → `printf` + `exit(1)` with `s_table_missing_key`
    (reserved; no caller in this ADR).

ADR 0087's runtime-validity gate (`emit_hash_key_runtime_validity_gate`)
fires inside `emit_hash_probe_for_insert` regardless of
`HashLookupOutcome`, so nil/NaN keys still trap with their dedicated
diagnostics before reaching the materialization step.

### Migration: 9 call sites converge on the helper

| Site                                                     | Before                                                     | After                                                         |
|----------------------------------------------------------|------------------------------------------------------------|---------------------------------------------------------------|
| `emit_local_init_tagged` 4 hash arms (~120 LOC inline)   | `null_buf` + `for_insert` + `key_at_null` + `store_nil/copy` | `emit_hash_lookup_into_tagged_slot(NilOnMissing)`            |
| Index static-key arm (String/Bool/Function/Table)        | `null_buf` + `lookup` (trap on missing) + `check_number`    | tmp slot + helper(NilOnMissing) + `check_number` + load f64   |
| Index TaggedValue arm                                    | same as static-key arm                                     | same as static-key arm                                        |

### Retire `emit_hash_probe_lookup` and `trap_on_null` parameter

`emit_hash_probe_lookup` and the `trap_on_null: bool` parameter on
`emit_hash_probe_loop` are deleted entirely (codex v3 §non-ad-hoc:
the bool was a "粗い abstraction" coupling search-and-trap). The
probe-loop body's `if trap_on_null { trap }` block goes; the
terminate condition simplifies to `is_null OR eq` unconditionally.
`emit_hash_probe_for_insert` remains the sole probe wrapper.

The retirement is safe: only 2 callers existed
(Index static-key + TaggedValue arms), both migrated in Step 4
through the new helper.

### User-visible diagnostic shift (narrow scope)

In **arith / cmp** contexts that bypass `widen_index_for_local_init`,
the trap message changes:

```lua
local t = {}
print(t["x"] + 1)
-- Before: "table missing key"
-- After:  "table value type mismatch"
```

Both still trap. The new message is more accurate: the lookup
returns Nil-tagged into the tmp slot; `emit_value_slot_check_number`
then traps because the consumer expected Number. Crucially:

- **LocalInit / Assign / print contexts already returned nil** via
  `widen_index_for_local_init` → `IndexTagged` →
  `emit_local_init_tagged`; ADR 0088 preserves that path (the
  Step 3 helper migration is behaviour-equivalent).
- **No existing test asserted the old message** (verified via Grep
  during ADR 0087 follow-up exploration).

## Alternatives Considered

- **Keep `emit_hash_probe_lookup` and add a third
  `emit_hash_probe_lookup_or_nil`.** Three wrappers for one knob.
  Rejected — the bool→enum lift is the abstraction win.
- **Promote `HashLookupOutcome` to `tagged.rs` as a "pure decision".**
  v1 of the plan did this; codex v3 critical issue #1 flagged it as
  "abstraction without owner" (no `policy_for_*` selector — just
  variants). Lookup miss policy is a consumer contract, not a tag
  policy. Moved to `emit.rs` private.
- **Widen Index value-side to TaggedValue everywhere.** Would fully
  spec-align (return nil natively instead of trap-on-non-Number),
  but is a structural restructure of all consumers. Future ADR.
- **Extract `tmp slot → check_number → load` into a micro-helper.**
  6 sites share the post-helper pattern (4 static-key Index + 1
  TaggedValue Index in this ADR + 1 `Local(TaggedValue)` Number
  extract elsewhere). Worth extracting once stabilised; ADR 0088
  does not extract to keep scope sharp.

## TDD Process

1. **Red.** 4 e2e in `tests/phase2_6b_hash_missing_key_read.rs`
   split per codex v3 critical #2:
   - **Step 0a — Red 2** (assert NEW diagnostic): static String
     key in arith → traps with `s_table_type_mismatch`; TaggedValue
     key in arith → same.
   - **Step 0b — Regression 2** (Day 0 green, must stay green):
     static String key in `local v = t["x"]; print(type(v))` →
     `nil` (this exercises the **`hash_buf == null` branch**
     explicitly, codex v3 nice-to-have #1); valid Number lookup
     via arith returns the correct value (happy-path pin).
2. **Green.**
   - Step 1: `enum HashLookupOutcome` (variants only).
   - Step 2: `emit_hash_lookup_into_tagged_slot` helper.
   - Step 3: `emit_local_init_tagged` 4 arms migrated (regression
     unchanged; existing ADR 0084 / 0079 e2e pin).
   - Step 4: Index 5 arms restructured → Red 2 flip Green.
     **995 → 999.**
   - Step 5: `emit_hash_probe_lookup` + `trap_on_null` parameter
     retired (refactor only).
3. **Refactor.** Step 5's wrapper retirement *is* the refactor —
   the bool→enum lift removed an awkward search-and-trap coupling
   in `emit_hash_probe_loop`.

## Consequences

- **`LIC-2.6b-hash-missing-key-read-1` → resolved.**
- **Test totals: 995 → 999 green** (4 new e2e; 0 unit since the
  enum has no decision logic).
- **LIC totals: 27 / 0 / 2 → 28 / 0 / 1** (resolved / partial /
  pending). Only `LIC-2.7p-arith-coerce-tagged-1` remains pending.
- **Source LOC delta** in `emit.rs`:
  - **Removed**: `emit_hash_probe_lookup` wrapper (-12 LOC),
    `trap_on_null` parameter + `if trap_on_null { trap }` block
    (-25 LOC), inline null_buf trap × 2 in Index arms (-~50 LOC),
    inline null_buf + null_check + dispatch in `emit_local_init_tagged`
    String/Bool/Function/Table arm (-~125 LOC).
  - **Added**: `enum HashLookupOutcome` (~8 LOC),
    `emit_hash_lookup_into_tagged_slot` (~190 LOC),
    `emit_lookup_miss_dispatch` (~20 LOC), tmp-slot + helper
    + check_number patterns in 5 Index arms (~75 LOC).
  - **Net**: ~80 LOC reduction, 9 sites → 1 chokepoint.
- **`tagged.rs` is unchanged**, preserving the ADR 0073 module
  boundary (slot layout / tag constants / pure store-check helpers).

### Carry-overs

- **`HashLookupOutcome::TrapMissing` is reserved-but-unused** — no
  caller in this ADR. Documented as future `next()`-strict-key
  consumer; clippy `#[allow(dead_code)]` keeps lint clean.
- **Index Number-key array arm** (`emit.rs:6510-6585`) untouched —
  array path with bounds-check trap, separate code path, separate
  LIC (resolved at user-reachable surfaces via widening).
- **`tmp slot → check_number → load` micro-helper** noted as future
  Tidy First once the chokepoint stabilises (codex v3 nice-to-have #3).

## Sentinel-bucket invariant (codex v3 §security)

The probe-loop body's `is_skip = is_null OR is_sentinel` continues to
walk past tombstones — preserved by construction since
`emit_hash_probe_loop`'s body was edited only to drop the
`trap_on_null` block and simplify `terminate`. The `is_skip`
construction at line 5491 is unchanged. Verified by green
`tests/phase2_6c_tag_hash.rs::*` (tombstone-respecting hash tests).

## Documentation updates

- [x] `docs/design/tagged-semantics.md` §6 new "Hash read lookup
      miss" subsection — chokepoint helper, `HashLookupOutcome`
      placement rationale, ownership boundary.
- [x] `docs/design/tagged-semantics.md` §4 — `LIC-2.6b-hash-
      missing-key-read-1` moved to Resolved. Totals: **28 / 0 / 1**.
- [x] `docs/design/0084-phase2-8e-iter-tk.md` — partial supersede
      note (read-side arms restructured; write-side IndexAssign
      arm unchanged).
- [x] `AGENTS.md` — Phase 2.6+ row added.
- [x] Cross-reference cleanup with semantic preservation: in 5
      files, replace stale `ADR 0088` (claimed for closure /
      arith-coerce work in `cc231a2`) with `future ADR
      (Function-kind upvalue / TaggedValue arith coerce)`.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative list
(ADR 0068).
