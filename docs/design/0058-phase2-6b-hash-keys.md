# 0058. Phase 2.6b-hash: String-Keyed Field Access `t.k` / `t["k"]`

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

After 2.6a-norm/grow finished the array side, the remaining
big gap was the **hash side** — table-as-record /
table-as-config / table-as-module patterns all need string
keys. ADR 0056's stable header reserved offset 24
(`hash_buf`) precisely for this; this ADR lights that field.

```lua
local config = {}
config.host = 1234
config.port = 8080
print(config.host + config.port)   -- 9314
```

The design discipline from previous ADRs carries:
- value kind stays static (Number-only — heterogeneous values
  are pending tagged values, LIC-2.6b-hash-2)
- missing key access still traps (LIC-2.6b-hash-1; Lua spec
  returns nil)
- alias safety remains automatic via the stable header

## Decision

### Open addressing with linear probing

Lua's reference implementation uses open addressing. It's
compact and cache-friendly. Lazy-allocated: an empty table
has `header.hash_buf == null`; the first `t.k = v` allocates
the initial buffer.

```text
hash_buf (!llvm.ptr)
  [0..8)   : i64 hash_cap     (always power-of-two)
  [8..16)  : i64 hash_count
  [16..)   : entry array      (cap × 16-byte entries)
     entry: { ptr key_str, f64 value }   ← key=null marks empty
```

- **Hash function**: FNV-1a 64-bit. Simple, no libc, good
  distribution.
- **Bucket**: `hash & (cap - 1)` (cap is 2^n).
- **Probe**: linear, `(bucket + 1) & mask`.
- **Resize**: when `count * 4 >= cap * 3` (load factor 0.75)
  the cap doubles and all entries rehash into a fresh buffer.
  Old buffer is `free`'d. The header's `hash_buf` field is
  swapped — outer table pointer doesn't move.
- **Initial cap**: 8 entries (= 144 bytes incl. header).

### `t.k` is parser-level sugar for `t["k"]`

Lua 5.4 §3.4.9 specifies this. We implement it at the parse
layer: `parse_call_suffix` adds a `Dot` arm that builds
`ExprKind::Index { target, key: Str("k") }`. **No new AST
variant.** HIR/codegen sees the same `Index` shape it already
processes for `t["k"]`.

### Lexer addition: `Dot` token

A lone `.` was a `LexError::Unexpected` until this phase.
The lexer dispatch arm gains:

```rust
('.', Some('.')) => (TokenKind::DotDot, true),
('.', _)         => (TokenKind::Dot, false),  // NEW
```

`.NUMBER` (`.5`) was already protected — `scan_number`
demands a trailing digit before consuming the `.` for a
fractional part, so genuine `.5` literals never reach the
single-char dispatch.

### HIR widening: 2 predicate flips

`lower_expr::Index` and `lower_stmt::IndexAssign` each had
a `key_kind != Number` check. Both widen to
`!matches!(key_kind, Number | String)`. Value kind for
write stays `Number-only` (LIC tracker entry).

### Codegen dispatch on key kind

Both `HirExprKind::Index` (read) and `HirStmtKind::IndexAssign`
(write) now branch on the static `key_kind`:

- `Number` → existing array path (bounds check + GEP into
  array_buf)
- `String` → hash path (probe + load/store)

### Five new codegen helpers

1. **`emit_string_hash`** — FNV-1a 64-bit. Calls `strlen` for
   length, `scf.while` over the bytes folding into the running
   hash with `xor + multiply`. The offset basis stored as i64
   wraps to its bit pattern — `arith.muli` is wrapping, so
   FNV's unsigned overflow semantics apply identically.
2. **`emit_hash_ensure_buf`** — if `header.hash_buf` is null,
   `malloc(16 + 8*16)` (header + 8 entries), zero-init the
   key fields, write cap=8 / count=0, store buffer pointer
   into the table header.
3. **`emit_hash_grow_if_needed`** — load-factor check
   (`count*4 ≥ cap*3`). On hit: malloc new buf at 2× cap,
   null-init keys, walk the old buf, for each non-null entry
   probe the new buf (via `emit_hash_probe_for_insert`) and
   migrate the (key, value) pair, free the old buf, swap
   `header.hash_buf`.
4. **`emit_hash_probe_lookup`** — read-side probe: trap on
   null bucket (= key not found, missing key trap), `strcmp`
   on non-null. Returns the matching bucket index.
5. **`emit_hash_probe_for_insert`** — write-side probe:
   stops on either null OR matching key. Returns the bucket.
   Uses an inner `scf.if` to short-circuit `strcmp` when the
   bucket is null (calling strcmp on a null pointer would be
   UB).

### `collect_string_pool` widened

Pre-2.6b the string-pool walker didn't recurse into
`HirExprKind::Index` or `HirExprKind::Table`. Now it does.
`t.k` lowers to `Index { …, Str("k") }` — without the
recursion, the `"k"` literal wasn't seeded into the global
pool and codegen panicked with
`collect_string_pool seeds every literal before codegen`.
Fixing the visitor to walk the new variants resolves the
panic. (Found via the missing-key e2e test; small bug, big
lesson on visitor completeness when adding ExprKind variants.)

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | `TokenKind::Dot` variant + dispatch arm |
| Parser   | `parse_call_suffix` Dot arm building sugar |
| AST      | **None** (sugar reuses `Index { target, key: Str(...) }`) |
| HIR      | 2 predicate flips; visitor recurses into Index/Table for string-pool collection |
| Codegen  | hash path: 5 helpers + dispatch in 2 sites + 1 global (`s_table_missing_key`) |

## TDD Process

1. **Red.** 13 e2e tests covering single-key, dot read/write,
   multi-key, overwrite, 30-key rehash stress, array+hash
   coexistence, `#t` hash-aware semantics, missing-key trap,
   alias-write visibility, regressions.
2. **Green.** Lexer → parser → HIR → codegen helpers → codegen
   dispatch in that order. After codegen 11 of 13 passed; the
   missing-key and rehash-stress tests panicked at codegen
   because `collect_string_pool` skipped Index/Table nodes —
   fixing the visitor closes both. Two stale `non_number_key`
   / `write_key_must_be_number` tests in 2.6a-arr/2.6a-wr
   reframed to `…_after_2_6b` documenting the boundary
   (Bool/Nil/Function/Table keys still reject).
3. **Refactor.** None warranted — five helpers per the plan,
   no further duplication. Three call sites of
   `emit_hash_probe_for_insert` (insert + rehash internal,
   counted as 2 — third would come with `t.k = nil` deletion
   which is deferred).

## Alternatives Considered

- **Chaining**: pointer chase per probe, more allocations.
  Open addressing is canonical for Lua-style tables.
- **Sorted array + binary search**: O(N) insert via memmove.
  Rejected.
- **Robin Hood / quadratic probing**: better on
  collision-heavy workloads but more complex; linear probing
  with load factor 0.75 is fine for the small tables our
  test set exercises.
- **Defer rehash to a future phase**: would force the 30-key
  stress test to skip rehash via a giant initial cap (hacky)
  or fail outright. Rejected.

## Consequences

- ~600 LOC added to `src/codegen/emit.rs` (the bulk being the
  rehash path's nested `scf.while`s).
- 13 e2e tests; 2 stale tests reframed. Total green at 719
  (706 + 13).
- `t.k` and `t["k"]` patterns work for config / record /
  module-style use cases at the boundaries of Number-only
  values.

## Lua-Incompatibility Tracker (cumulative)

Updated from ADR 0057. New: LIC-2.6b-hash-1, LIC-2.6b-hash-2.
Modified: LIC-2.6a-arr-3 partially resolves.

| ID | Behaviour | Lua spec | Our behaviour | Status |
|----|-----------|----------|---------------|--------|
| LIC-2.6a-arr-1 | OOB read | returns nil | exits(1) | pending tagged values |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number-only | pending tagged values |
| LIC-2.6a-arr-3 | key kinds | any | Number+String | **partial (this ADR)** |
| LIC-2.6a-wr-1 | hole write | creates a hole | exits(1) | pending tagged values |
| LIC-2.6a-wr-2 | grow write | extends length | extends length | resolved (ADR 0057) |
| LIC-2.6a-wr-3 | array value kinds | heterogeneous | Number-only | pending tagged values |
| LIC-2.6b-hash-1 | missing key read | returns nil | exits(1) | **new (this ADR)** |
| LIC-2.6b-hash-2 | hash value kinds | heterogeneous | Number-only | **new (this ADR)** |

Bool/Nil/Function/Table keys remain rejected — extending to
those needs tagged values for keys *too*, which is downstream
of LIC-2.6a-arr-2.

## Out of Scope

- **Hash deletion** (`t.k = nil`) — needs nil tag in
  hash_buf entry; pending tagged values.
- **Heterogeneous hash values** — same dependency.
- **Method syntax** `t:m()` — sugar to `t.m(t, ...)` plus
  `self` parameter handling. A separate phase that needs
  Function-kind values in tables, which is downstream of
  tagged values.
- **`__index` / `__newindex` metatables** — far out of scope.
- **Resize-down on heavy delete** — pending delete itself.
- **Initial cap tuning** — premature.
