# 0059. Phase 2.6c-tag-arr: Tagged Array Slots + Hole Write

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

Lua's array semantics include **hole creation**: writing
`t[#t + 2] = v` (or further) on a length-N table fills the
intermediate slots with `nil` and extends `length` to the
written index. Until this phase, `t[i] = v` for `i > length+1`
trapped (LIC-2.6a-wr-1) because there was no nil
representation in the f64-only element layout.

```lua
local t = {1, 2}
t[5] = 99       -- 2.6a-wr would have trapped; now indices 3 and 4
                -- become Nil-tagged holes and length = 5.
print(#t)       -- 5
print(t[5])     -- 99
```

Codex's strategic review carved out a path: introduce tagged
values **table-locally**, *not* universally. This phase
delivers the smallest segment of that path — array-side only,
Number/Nil tags only, construction kind constraint preserved.
Hash buffers stay untagged for now (see 2.6c-tag-hash); locals
stay statically typed. The win: LIC-2.6a-wr-1 resolves and
the layout discipline for future tagged-values phases is
established.

## Decision

### 16-byte tagged slot

Each `array_buf` element is now a 16-byte struct:

```text
slot[i] {
    offset 0  : i64 tag      (TAG_NIL=0, TAG_NUMBER=1; 2..=5 reserved)
    offset 8  : f64 value    (zero when tag is Nil)
}
```

i64 tag (rather than i8 + padding) keeps offsets simple and
the f64 naturally 8-byte aligned. Doubles the memory
footprint of array_buf compared to pre-tagging. Acceptable
for the Lua 5.4 reference cost model (Lua's own tagged
values are 16 bytes too).

Constants:
```rust
const ARRAY_ELEM_SIZE: i64 = 16;
const ARRAY_ELEM_OFF_VALUE: i64 = 8;
const TAG_NIL: i64 = 0;
const TAG_NUMBER: i64 = 1;
```

`ARRAY_ELEM_OFF_TAG` = 0 is implicit in the `emit_load(elem_ptr,
i64, …)` call that loads from the slot's start.

### Hole-write semantics

`t[i] = v` for any `i ≥ 1`:
1. **Lower-bound check**: `key < 1` traps (unchanged).
2. **Grow**: `emit_table_grow_if_needed` reallocates `array_buf`
   to fit `key`. Memory sizes use `key * ARRAY_ELEM_SIZE`
   (was `key * 8`).
3. **Gap fill**: `emit_array_fill_nil` walks the half-open
   interval `[length+1, key)` (1-based) and stores
   `{TAG_NIL, 0.0}` to each slot.
4. **Length update**: if `key > length`, `header.length = key`.
5. **Final store**: `{TAG_NUMBER, value}` at `slot[key]`.

The previous `key > length+1 → trap` rule is dropped. The
`grow_if_needed` helper from ADR 0057 already handles
arbitrary `key > cap` by doubling — this phase just lifts the
upper-bound write check that gated it.

### Read with tag check

`t[i]`:
1. **Bounds**: `key in [1, length]` (unchanged).
2. **Slot ptr**: `array_buf + (key-1)*16`.
3. **Tag check**: load `i64` tag at offset 0; trap with
   `s_table_type_mismatch` if not `TAG_NUMBER`.
4. **Value**: load `f64` at offset 8.

A hole within `[1, length]` thus traps on read — a Lua
spec divergence for now (Lua: `t[hole] == nil`). Resolving
this requires either heterogeneous-typed Index expressions
or locals that can hold Nil; both belong to later sub-phases.

### Four new codegen helpers

1. **`emit_array_elem_ptr`** — `array_buf + (key-1)*16`
2. **`emit_array_elem_store_number`** — write `{TAG_NUMBER, v}`
3. **`emit_array_elem_check_number`** — load tag, trap if not Number
4. **`emit_array_fill_nil`** — `scf.while [from..to)` storing
   `{TAG_NIL, 0.0}`

### One new diagnostic global

`s_table_type_mismatch = "table value type mismatch\0"` — joins
`s_table_oob` and `s_table_missing_key` in the diagnostic
pool.

### Memory size adjustments in `emit_table_grow_if_needed`

`new_size = new_cap * 8` → `new_cap * ARRAY_ELEM_SIZE`
`copy_size = length * 8` → `length * ARRAY_ELEM_SIZE`

Logic is otherwise unchanged: doubling cap, conditional
memcpy + free of old buffer, header swap.

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | None                                                |
| Parser   | None                                                |
| AST      | None                                                |
| HIR      | None — `Table` / `Index` / `IndexAssign` shapes and the Number-only construction restriction unchanged |
| Codegen  | New constants, four helpers, one diagnostic global, write path lifts upper bound + adds gap-fill, read path adds tag check, grow path uses `*16` sizes |

## TDD Process

1. **Red.** 11 e2e tests in
   `tests/phase2_6c_tag_arr_holes.rs` covering basic hole
   write, length growth, hole read trap, large hole, fill-
   then-read, alias-under-hole-write (the keystone for
   stable-header alias safety under tag-aware writes), 30-
   step sparse stress, lower-bound trap regression, OOB read
   trap regression, construction regression, hash regression.
   6 failed (the new behaviour); 5 already passed (regressions
   + the trap cases that survive the new layout).
2. **Green.** Constants + 4 helpers + 1 diagnostic global +
   construction `*16` + read tag-check + write hole-fill +
   grow `*16`. All 11 tests pass; full suite at 730 (719+11).
3. **Stale-test rotation.** Three pre-existing tests pinned
   the trap behaviour for hole/grow:
   - 2.6a-wr's `out_of_bounds_write_traps` →
     `…now_creates_hole_after_2_6c_tag_arr`
   - 2.6a-grow's `hole_creation_still_traps` →
     `…now_works_after_2_6c_tag_arr`
   - The 2.6a-wr `grow_write_at_length_plus_one_traps` reframe
     from ADR 0057 stays in its updated form.

## Alternatives Considered

- **NaN-boxing on f64**: encode tag in NaN bit patterns of
  the f64 value. Single-word slots (8 bytes/elem instead of
  16). Bit-fiddling overhead, debug pain, and the nil
  encoding would collide with naturally produced NaN. Rejected.
- **i8 tag + 7-byte padding + f64**: cleaner conceptually,
  but the padding makes static offset calculation more error-
  prone. i64 tag is the same memory and avoids alignment
  fuss.
- **Tag both array and hash in one phase**: blast radius
  doubles; hash entry layout would need to grow from 16 to
  24 bytes with offset rewrites at every probe site.
  Rejected — discipline is to ship the smallest meaningful
  unit. 2.6c-tag-hash will follow.
- **Lift HIR's Number-only construction at the same time**:
  would let users write `{1, "two", true}` but reading any
  non-Number element would trap (Index returns Number
  statically). The "construct heterogeneously, read into
  typed slot" mismatch is awkward; defer until locals
  widening can complete the picture.

## Consequences

- ~250 LOC added to `src/codegen/emit.rs`.
- Memory: array_buf doubles per slot (8 → 16 bytes).
- `s_table_type_mismatch` joins the diagnostic pool.
- 11 new e2e tests; 3 stale tests reframed. Total green at 730.

## Lua-Incompatibility Tracker (cumulative)

Updated from ADR 0058. **LIC-2.6a-wr-1 resolved.**

| ID | Behaviour | Lua spec | Our behaviour | Status |
|----|-----------|----------|---------------|--------|
| LIC-2.6a-arr-1 | OOB read | returns nil | exits(1) | pending tagged values + locals widening |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number-only | pending tagged values + locals widening |
| LIC-2.6a-arr-3 | key kinds | any | Number+String | partial (ADR 0058) |
| LIC-2.6a-wr-1 | hole write | creates a hole | creates a hole (Nil-tagged) | **resolved (this ADR)** |
| LIC-2.6a-wr-2 | grow write | extends length | extends length | resolved (ADR 0057) |
| LIC-2.6a-wr-3 | array value kinds | heterogeneous | Number-only | pending tagged values |
| LIC-2.6b-hash-1 | missing key read | returns nil | exits(1) | pending tagged values |
| LIC-2.6b-hash-2 | hash value kinds | heterogeneous | Number-only | pending tagged values |

## Out of Scope

- **Hash entry value tagging** — Phase 2.6c-tag-hash. Same
  pattern but on hash_buf entry values.
- **Heterogeneous array construction** (`{1, "two", true}`)
  — Phase 2.6c-tag-arr-string and follow-ups; needs HIR to
  accept multiple kinds per Table element list.
- **Lua-compatible hole read returning nil** — needs Index
  expressions to be heterogeneously typed or locals to widen
  to LuaValue. Phase 2.6c-tag-locals or similar.
- **Function/Table-tagged elements** — would unblock `{f1,
  f2}` function arrays (string library tables, dispatch
  tables). Pending Function-kind value lifecycle (closure
  escape, etc.).
- **Hash deletion** (`t.k = nil`) — pending nil-tagged hash
  values.
- **Metatables** — far out of scope.
- **Initial tag-byte cleanup** (e.g. `calloc` over `malloc` +
  manual init) — current code initialises new buffers
  explicitly per slot via `emit_array_fill_nil` for grow gaps;
  fresh allocation in construction stores Number tags directly.
  Both are correct.
