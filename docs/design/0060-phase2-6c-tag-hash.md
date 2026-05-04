# 0060. Phase 2.6c-tag-hash: Tagged Hash Entry Values + `t.k = nil` Soft Delete

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

ADR 0059 (Phase 2.6c-tag-arr) introduced 16-byte tagged slots
for `array_buf` elements and resolved LIC-2.6a-wr-1 (hole
write). This phase mirrors the same pattern on the **hash
side**: each hash entry's value field becomes a 16-byte
`{tag, value}` tagged slot.

The user-visible win is `t.k = nil` (soft delete):

```lua
local t = {}
t.host = 1234
t.host = nil       -- previously: HIR rejected (Number-only value)
t.host = 9999      -- works after delete; no count++ on overwrite
print(t.host)      -- 9999
```

The structural win is **layout unification**: the hash entry's
value slot has identical layout to an array_buf slot, so the
generic `emit_value_slot_*` helpers introduced in 2.6c-tag-arr
(and renamed in this session's Tidy First step) drive both
sides. Future locals widening will lower `Index` reads through
the same extraction path regardless of array vs hash origin.

## Decision

### Hash entry layout: 16 → 24 bytes

```text
hash_buf entry array (offset HASH_OFF_ENTRIES = 16):
  entry[i] (24 bytes):
    [0..8)   : !llvm.ptr key_str          (null = empty bucket)
    [8..24)  : 16-byte tagged value slot
                 [0..8)  : i64 tag         (TAG_NIL=0, TAG_NUMBER=1)
                 [8..16) : f64 value       (zero when tag is Nil)
```

Constants:
```rust
const HASH_ENTRY_SIZE: i64 = 24;          // was 16
const HASH_ENTRY_OFF_KEY: i64 = 0;        // unchanged
const HASH_ENTRY_OFF_VALUE_SLOT: i64 = 8; // renamed from HASH_ENTRY_OFF_VALUE
                                          // (now points at the tagged slot)
```

The retired `HASH_ENTRY_OFF_VALUE` (= 8) carried "the f64 lives
here" semantics; the new `_VALUE_SLOT` (= 8) carries "the
tagged slot starts here". Inside the slot, the f64 is at +8
(the existing `ARRAY_ELEM_OFF_VALUE` constant).

### `t.k = nil` semantics: soft tombstone

- HIR accepts Nil value when key is String (only).
- Codegen detects Nil-write path and calls
  `emit_value_slot_store_nil` instead of `_store_number`.
- The hash key is **left in place** — probing still finds the
  entry; a subsequent `t.k = something` overwrites in place
  (count not incremented since `was_empty` is false).
- Reading `t.k` after delete: tag is Nil, `_check_number`
  traps with `s_table_type_mismatch`. Same as 2.6c-tag-arr
  hole-read behaviour.

This is a **soft tombstone**: no probing skip, no proper
"missing" semantic. A future *hard* tombstone phase would
mark the key with a sentinel and have probing skip past
it, distinguishing "deleted slot to re-use" from
"deleted slot to skip during search". Out of scope here.

### Array path unchanged

Array writes still reject Nil (the upper-bound lift in
ADR 0059 already handles array-side hole creation; explicit
Nil-write to an array slot has no observable use without
locals widening).

The HIR predicate becomes:

```rust
let value_ok = match (key_kind, value_kind) {
    (Number, Number) => true,    // array path
    (String, Number) => true,    // hash insert
    (String, Nil)    => true,    // hash delete (NEW)
    _ => false,
};
```

### Helper consolidation

The Tidy First commit
(`refactor(codegen): rename array helpers to value_slot generic`)
preceded this feature. Three helpers operate uniformly on
both array and hash value slots:

- `emit_value_slot_store_number(slot_ptr, f64)` — write
  `{TAG_NUMBER, value}`
- `emit_value_slot_store_nil(slot_ptr)` — write `{TAG_NIL, 0.0}`
- `emit_value_slot_check_number(slot_ptr)` — load tag, trap on
  mismatch

The slot pointer is computed by callers from their context-
specific layout (array_buf + (key-1)*16 vs entry_ptr + 8).

### Codegen update sites

1. **Hash insert** (`HirStmtKind::IndexAssign` String arm):
   replace the unconditional f64 store with value-kind dispatch
   on the value slot at `entry_ptr + HASH_ENTRY_OFF_VALUE_SLOT`.
2. **Hash lookup** (`HirExprKind::Index` String arm): replace
   the unconditional f64 load with `emit_value_slot_check_number`
   followed by `emit_byte_offset_ptr(+ARRAY_ELEM_OFF_VALUE)` and
   load f64.
3. **Rehash** (`emit_hash_grow_if_needed`): the value migration
   loop now copies the entire 16-byte tagged slot (tag at +0,
   value at +8) so Nil-tagged entries survive the rehash with
   their Nil tag intact. Otherwise rehash would silently
   "resurrect" deleted entries by reading garbage tag bits.

The `emit_hash_probe_lookup` and `emit_hash_probe_for_insert`
helpers only touch the key field — they need no changes (the
24-byte stride takes care of itself via `HASH_ENTRY_SIZE`).

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | None                                                |
| Parser   | None                                                |
| AST      | None                                                |
| HIR      | One predicate matrix in `lower_stmt::IndexAssign` accepts `(String, Nil)` |
| Codegen  | Hash entry constants (size 16→24, value-slot offset rename); insert/lookup/rehash sites refactored to use value-slot helpers; HIR Nil-write dispatches to `_store_nil` |

## TDD Process

1. **Step 1 — Tidy First** (separate commit
   `refactor(codegen): rename array helpers to value_slot generic`):
   `emit_array_elem_store_number/_check_number` →
   `emit_value_slot_store_number/_check_number`. Add
   `emit_value_slot_store_nil`. `emit_array_fill_nil` body
   collapses to one helper call. Tests stay at 730 — pure
   structural refactor.
2. **Step 2 — Red.** 11 e2e tests in
   `tests/phase2_6c_tag_hash_delete.rs`. 7 fail at `cargo test`
   (the new behaviour); 4 already pass (regressions +
   String-value rejection which still holds).
3. **Step 3 — Green.** Constants update (size, value-slot
   offset rename); HIR predicate matrix; codegen
   insert/lookup/rehash. All 11 tests pass at 741 (730 + 11).
4. **Step 4 — ADR + AGENTS + commit.** Single feature commit
   on top of the Tidy First commit.

## Alternatives Considered

- **Keep hash entry at 16 bytes**: `{i64 tag-and-key-marker,
  f64 value}` packing key-pointer-or-tag bits together. Would
  save 8 bytes per entry but force ugly bit-fiddling in probe
  helpers. Rejected.
- **Hard tombstone (key → sentinel + probing skip)**: makes
  delete fully Lua-spec compliant (`t.k` after delete returns
  nil, doesn't re-find the entry). Requires extending probe
  helpers with sentinel-skip logic and managing a separate
  "deleted" path during insert. A reasonable follow-up phase;
  rejected for this round to keep blast radius contained.
- **Allow Nil-write to array slots too**: Number-keyed `t[i] =
  nil` would mark slot Nil (manual hole). No observable use
  yet (hole reads still trap), and the upper-bound lift in
  ADR 0059 already produces holes implicitly. Rejected as
  premature.
- **Heterogeneous hash values (Bool/String) in this phase**:
  blast radius expands to Lookup return type and locals
  widening. Strictly defers behind locals widening.

## Consequences

- ~200 LOC across codegen + HIR.
- Memory: hash entry footprint 16 → 24 bytes per entry
  (50% increase). Acceptable; matches Lua's reference table
  per-entry cost order of magnitude.
- 11 new e2e tests; total green at 741 (730 + 11).
- New diagnostic global `s_table_type_mismatch` is now used
  by both array hole-read and hash-after-delete-read paths
  (rule of three confirmation post hoc).
- Deletion-and-reinsertion cycle (`t.k = 1; t.k = nil; t.k =
  2`) works correctly across rehash boundaries (the 30-key
  stress test exercises this).

## Lua-Incompatibility Tracker (cumulative)

Updated from ADR 0059. New: LIC-2.6c-tag-hash-1.

| ID | Behaviour | Lua spec | Our behaviour | Status |
|----|-----------|----------|---------------|--------|
| LIC-2.6a-arr-1 | OOB read | returns nil | exits(1) | pending tagged values + locals widening |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number-only | pending tagged values + locals widening |
| LIC-2.6a-arr-3 | key kinds | any | Number+String | partial (ADR 0058) |
| LIC-2.6a-wr-1 | hole write | creates a hole | creates a hole (Nil-tagged) | resolved (ADR 0059) |
| LIC-2.6a-wr-2 | grow write | extends length | extends length | resolved (ADR 0057) |
| LIC-2.6a-wr-3 | array value kinds | heterogeneous | Number-only | pending tagged values |
| LIC-2.6b-hash-1 | missing key read | returns nil | exits(1) | pending tagged values + locals widening |
| LIC-2.6b-hash-2 | hash value kinds | heterogeneous | Number+Nil | **partial (this ADR)** |
| LIC-2.6c-tag-hash-1 | `t.k = nil` | physically removes the key | marks Nil tag (key persists) | **new (this ADR)** |

LIC-2.6b-hash-2 transitions from "pending" to "partial" — Nil
is now an accepted value kind for hash writes (delete path),
but Bool/String/Function/Table values still reject. Full
heterogeneous hash values need locals widening so the read
side can extract them coherently.

## Out of Scope

- **Hard tombstone**: would resolve LIC-2.6c-tag-hash-1 by
  making `t.k = nil` semantically equivalent to "key never
  existed". Probing skip + sentinel key.
- **Heterogeneous hash values** (Bool/String/Function/Table
  in hash entries): pending locals widening so reads can
  extract heterogeneous payloads.
- **Bracket-keyed arbitrary-string runtime keys** (`t[expr_str]`):
  works already via the existing String-key path.
- **Iteration** (`pairs(t)`, `ipairs(t)`): pending; would
  surface tag-aware extraction.
- **Metatables / `__index` / `__newindex`**: far out of scope.
- **Hash deletion that rebalances via tombstones**: see hard
  tombstone above.
