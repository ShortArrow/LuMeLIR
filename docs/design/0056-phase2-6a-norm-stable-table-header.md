# 0056. Phase 2.6a-norm: Stable Table Header (Tidy First)

- **Status:** Accepted
- **Kind:** Refactor Memo
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

The Phase 2.6a-min/arr/wr table representation made the table
value a raw heap pointer that pointed directly at
`[i64 length][f64 elem₀]…[f64 elem_{N-1}]`. ADR 0053 chose
this minimal "header-inline-with-elements" layout (option 1)
on the assumption that hash part and capacity tracking would
restructure things later anyway.

A `codex review` strategic pass on this session's HEAD flagged
a concrete failure mode the inline layout cannot survive:
**alias resize**.

```lua
local a = {1, 2, 3}
local b = a            -- copies the heap ptr into b's slot
a[#a + 1] = 99         -- 2.6a-grow would need to realloc/relocate
print(b[1])            -- … and now b's ptr is stale → use-after-free / wrong data
```

The same problem appears the moment the hash part gets a
separate region: any layout that fuses the outer pointer with
the element storage forces relocation when the element store
needs more space.

The fix is the canonical one: make the table value a **stable
header pointer**, with the element / hash storage living in
separate, independently-resizable buffers reached via fields
*inside* the header. Resize swaps the inner pointer; the outer
header pointer never moves; aliases stay valid for free.

We ship this as a Tidy First refactor *before* 2.6a-grow
lands — fixing the ABI now is much cheaper than fixing it
after grow has been implemented against the old shape.

## Decision

### 32-byte stable header layout

```text
table value (!llvm.ptr)
  → header [32 bytes, malloc'd, never moves]
       offset 0  : i64 length          ← `#t` reads here
       offset 8  : i64 capacity        ← grow updates (2.6a-grow)
       offset 16 : !llvm.ptr array_buf ← f64 elem buffer (sep. malloc)
       offset 24 : !llvm.ptr hash_buf  ← null today; 2.6b lights it up

  array_buf (!llvm.ptr) → [f64 elem₀][f64 elem₁]…[f64 elem_{cap-1}]
```

### Frozen vs mutable offsets

To make this layout safe to extend, two offsets are
**frozen** as part of the public-by-construction codegen
contract:

- `TABLE_OFF_LEN = 0` — `#t` and bounds checks load here;
- `TABLE_OFF_ARRAY_BUF = 16` — index read/write fetch the
  buffer pointer here.

The other two header slots are **mutable** in the sense that
their offsets may be relocated as the layout grows
(metatable ptr addition, NaN-boxing tag fields, etc.):

- `TABLE_OFF_CAP = 8` — used only by future grow logic;
- `TABLE_OFF_HASH_BUF = 24` — used only by future hash part.

Module-level constants in `src/codegen/emit.rs` make these
offsets the single source of truth:

```rust
const TABLE_OFF_LEN: i64 = 0;
const TABLE_OFF_CAP: i64 = 8;
const TABLE_OFF_ARRAY_BUF: i64 = 16;
const TABLE_OFF_HASH_BUF: i64 = 24;
const TABLE_HEADER_SIZE: i64 = 32;
```

### Empty tables: `array_buf = null`

For `{}` (zero elements), `malloc(0)` is implementation-
defined per POSIX. Instead of relying on it we explicitly
store a null `array_buf`. The bounds check rejects every
index for a length-0 table, so the null is never
dereferenced.

### Two new helpers

- **`emit_null_ptr`** — `llvm.mlir.zero -> !llvm.ptr`. Used
  for the empty-array-buf case and for `hash_buf`
  initialisation.
- **`emit_table_array_buf(table_ptr) -> !llvm.ptr`** —
  loads the `array_buf` field from the header. Used by both
  index-read and index-write; future grow uses the same
  offset to *store* a new buffer there. Three call sites
  trigger the rule of three; the helper extracts the
  duplicate `emit_byte_offset_ptr` + `emit_load` pair.

### Codegen call-site changes

| Site | Old | New |
|---|---|---|
| `HirExprKind::Table` construction | one `malloc(8 + N*8)`, length at offset 0, elements at 8+i*8 | `malloc(32)` for header, store length+capacity, second `malloc(N*8)` (or null) for array_buf, store array_buf+hash_buf in header, elements at array_buf+i*8 |
| `UnaryOp::Len` for Table | `load i64 from ptr+0` | unchanged (frozen contract) |
| `HirExprKind::Index` read | length load + GEP `8+(key-1)*8` from ptr | length load + array_buf load + GEP `(key-1)*8` from array_buf |
| `HirStmtKind::IndexAssign` write | mirror of read | mirror of read |

Bounds-check logic (`emit_table_bounds_check`) is unchanged
— `length` is still loaded from the same offset (0) on the
table pointer, the only difference is what comes after.

### Reference semantics fall out

`local b = a` continues to copy the (header) pointer through
`emit_load`/`emit_store` exactly as before. Both slots now
reference the same stable header. Future write-side phases
that mutate inner buffers will be visible through every
alias because the header `array_buf` slot is shared.

The existing test
`alias_write_is_visible_through_original`
(`tests/phase2_6a_wr_array_write.rs`) is a fixed point of
this property and continues to pass unchanged.

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | None                                                |
| Parser   | None                                                |
| AST      | None                                                |
| HIR      | None — `HirExprKind::Table`, `HirExprKind::Index`, `HirStmtKind::IndexAssign` shapes unchanged |
| Codegen  | Layout constants module-level; construction widens to two mallocs + four field stores; index read/write add an `array_buf` load between bounds-check and element GEP; two new helpers (`emit_null_ptr`, `emit_table_array_buf`) |

## TDD Process

1. **Step 0 — baseline.** `cargo test` reports 695 green
   pre-refactor.
2. **Step 1 — refactor.** Module constants added; `Table`
   construction split into header + array_buf; index
   read/write thread `array_buf` load through. After each
   sub-edit `cargo test` continues to pass at exactly 695.
   No tests added, none removed.
3. **Manual smoke.** Run two representative programs
   (single-table `t[2]=99; print(sum)` and alias-write
   `local b=a; b[1]=99; print(a[1])`) — both produce the
   expected 103 and 99.
4. **Refactor pass.** None warranted — the `array_buf`
   load extracted as `emit_table_array_buf` covers
   read/write/grow (rule of three pre-paid).

## Alternatives Considered

- **Skip 2.6a-norm; do the layout change inside 2.6a-grow.**
  Loses the "tests stay green throughout" Tidy First
  property — grow + layout change + new tests all land in
  one diff. Higher review burden, harder bisect, larger
  blast radius. Rejected.
- **Tagged values + stable header in one shot.** Codex
  flagged this explicitly: combining structural ABI rework
  (LuaValue payload) with a separate ABI rework (stable
  header) is two changes pretending to be one. Rejected —
  defer tagged values to a focused phase.
- **Inline header + `realloc` for grow with relocation
  notification.** No portable mechanism for "tell every
  alias the pointer moved" in C ABI. Rejected.

## Consequences

- ~80 LOC net change in `src/codegen/emit.rs`.
- Test count unchanged at 695. **Behaviour preserved
  exactly.**
- Two new codegen helpers; five new module-level constants.
- ADR 0053's "starter layout (option 1)" supersession is
  partial: the i64-length-at-offset-0 contract from 0053
  carries over verbatim. The element-storage layout is the
  part that changes.
- 2.6a-grow can now ship as `realloc(array_buf)` + update
  `array_buf` field + update length/capacity in header.
  Outer header pointer never moves; aliases stay valid;
  no new alias machinery needed.

## Out of Scope

- **Capacity-aware grow (`t[#t+1] = v`)** — 2.6a-grow.
- **Hash part / `t.k`** — 2.6b.
- **Tagged element values (Lua-spec compliance)** —
  follow-up phase. The `array_buf` element type is still
  `f64`; widening it to `LuaValue` is orthogonal to this
  refactor.
- **Metatable pointer / GC mark byte** — would extend the
  header beyond 32 bytes; the contract pins offsets 0 and
  16, so adding offsets 32+ is fine.

## ADR 0053 Cross-Reference

ADR 0053 chose option 1 (length-only inline) "because
options 3 and 4 can be done later". This phase does that
"later" — option 3 (header + separate buffer), preserving
the offset-0-is-length contract.
