# 0057. Phase 2.6a-grow: Table Array Push & Resize

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

Phase 2.6a-norm (ADR 0056) put the table value behind a stable
header pointer with the f64 element storage in a separate
`array_buf` allocation. That refactor was Tidy First — it
preserved every existing behaviour. Its purpose was to enable
this phase: lighting up Lua's most-used array idiom,
push-back via `t[#t+1] = v`, with **alias safety guaranteed
by the stable header**.

```lua
local t = {}
for i = 1, 5 do
  t[i] = i * 10        -- previously trapped (LIC-2.6a-wr-2);
                       -- now grows
end
print(#t)              -- 5
print(t[3])            -- 30
```

Lua's full write semantics also include hole creation
(`t[i] = v` for `i > length + 1` skipping intermediate slots,
which become nil). That branch still requires tagged values
for the nil representation; we keep trapping (LIC-2.6a-wr-1
unchanged).

## Decision

### Three-state write path

`t[i] = v` now decides among:

| Condition | Action |
|---|---|
| `i < 1` or `i > length + 1` | trap `"table index out of bounds"` |
| `i == length + 1` | **push-back**: grow array_buf if needed, then store; length becomes `i` |
| `1 ≤ i ≤ length` | **in-place write** (existing 2.6a-wr) |

read-side bounds check stays at `[1, length]` — pushed slots
become observable on the next iteration's read because `length`
is updated *before* the new element settles.

### Capacity policy: doubling

```text
new_cap = max(cap * 2, key)
```

From `cap = 0`, the first push grows to exactly the key value
(`max(0, 1) = 1`). Subsequent pushes double: 1 → 2 → 4 → 8 → 16
→ 32. **Amortised O(1) push, O(N) total for N pushes.** The
allocator overhead is bounded at 2× the current size.

### Codegen structure (write path)

```text
length    = load i64, header+0
key_i     = fptosi(key)
length+1  = arith.addi length, 1
emit_table_bounds_check(key_i, length+1)              # already-existing helper

emit_table_grow_if_needed(target_ptr, key_i, length)  # new helper
  cap = load i64, header+8
  scf.if key_i > cap:
    new_cap   = max(cap * 2, key_i)
    new_buf   = malloc(new_cap * 8)
    scf.if cap > 0:                                   # null-old guard
      old_buf = load ptr, header+16
      memcpy(new_buf, old_buf, length * 8)
      free(old_buf)                                   # newly declared libc
    store new_cap, header+8
    store new_buf, header+16

scf.if key_i > length:                                # length update
  store key_i, header+0

array_buf = load ptr, header+16                       # post-grow reload
elem_ptr  = gep i8, array_buf, (key_i - 1) * 8
store value_v, elem_ptr
```

The grow scf.if region is empty when `key_i ≤ cap` — there's
already enough slack from a prior allocation. The inner
copy-and-free scf.if region only runs when there *was* a
previous non-null buffer (`cap > 0`), avoiding the C-standard
UB of `memcpy` with a null source even at zero size.

`free` is declared as a fresh libc external (`free(ptr) -> void`)
in `emit_libc_decls`, alongside the existing `malloc`/`memcpy`/
`exit`/`strcmp`/`snprintf`/`sscanf`/`strlen` declarations. A
new `emit_libc_call_void` helper handles the no-result call
shape.

### `emit_table_grow_if_needed` factors the grow path

Putting the conditional realloc behind one helper keeps the
`HirStmtKind::IndexAssign` arm linear and reads top-to-bottom:
bounds check → grow if needed → length update → store. The
helper is a private leaf — no other call site today, but the
shape is reusable for the future hash-buf grow phase.

### `emit_table_array_buf` reload after grow

Because the grow may have just *replaced* the array_buf field,
the write path re-loads `array_buf` from offset 16 *after* the
grow helper returns. This is what `emit_table_array_buf`
already does (read + write reuse it; this phase makes it
genuinely needed by write — rule of three was pre-paid in
2.6a-norm).

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | None                                                |
| Parser   | None                                                |
| AST      | None                                                |
| HIR      | None — `HirStmtKind::IndexAssign` shape unchanged; the value range it accepts widens   |
| Codegen  | `free` libc declaration + `emit_libc_call_void` helper; `emit_table_grow_if_needed` helper; `IndexAssign` arm extended with grow + length-update + reload |

## TDD Process

1. **Red.** 11 e2e tests added in
   `tests/phase2_6a_grow_array_push.rs` covering push from
   empty, length growth, multi-push, push to existing array,
   loop push, stress doubling at 30 elements, the
   alias-visibility-under-grow keystone, the still-trapping
   hole case, and read/in-place regressions. 8 failed (the
   new behaviour); 3 already passed (regressions + the
   hole-trap which is unchanged).
2. **Green.** Three pieces in order:
   - declare `free` + `emit_libc_call_void`
   - implement `emit_table_grow_if_needed`
   - extend `HirStmtKind::IndexAssign` (bounds + grow +
     length-update + post-grow reload)
   All 11 grow tests pass.
3. **Stale-test rotation.** The 2.6a-wr test
   `grow_write_at_length_plus_one_traps` had pinned the
   trap as the expected behaviour for LIC-2.6a-wr-2. After
   resolution it becomes
   `…now_works_after_2_6a_grow`, asserting `t[#t+1] = v`
   succeeds and updates length. Total green at 706 (695 + 11).

## Alternatives Considered

- **Linear growth (`new_cap = cap + 1`)**. Worst-case
  `O(N²)` for N pushes due to memcpy on every step.
  Rejected.
- **`realloc(ptr, n)` instead of malloc + memcpy + free**.
  Saves one helper call but introduces the realloc-failure
  edge case (where `ptr` may or may not have been freed).
  Manual is more transparent for a small constant cost.
  Rejected.
- **Minimum-capacity floor (`new_cap = max(cap*2, key, 4)`)**
  to amortise tiny tables. Wastes slots on `t = {x}` style
  one-element tables that never grow. Rejected for now;
  add later if profiling shows a problem.
- **Lua-compatible hole creation in this phase.** Requires
  a nil tag in the array_buf payload, which means tagged
  values. That's a separate ABI change with its own
  blast-radius cost. Defer.

## Consequences

- ~150 LOC added to `src/codegen/emit.rs`.
- 11 new e2e tests; 1 reframed e2e test. Total green at 706
  (695 + 11).
- `free` is now part of our libc surface — the first step
  toward a proper memory lifecycle (no GC yet, but free
  on grow stops the doubling pattern from leaking).
- Phase 2.6a-norm's stable-header contract is now empirically
  validated by the alias-visibility-under-grow test.

## Lua-Incompatibility Tracker (cumulative)

| ID | Behaviour | Lua spec | Our behaviour | Status |
|----|-----------|----------|---------------|--------|
| LIC-2.6a-arr-1 | OOB read | returns nil | exits(1) | pending tagged values |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number-only | pending tagged values |
| LIC-2.6a-arr-3 | key kinds | any | Number-only | pending hash part (2.6b) |
| LIC-2.6a-wr-1 | hole write `t[#t+2]=v` | creates a hole (nil-fill) | exits(1) | pending tagged values |
| LIC-2.6a-wr-2 | grow write `t[#t+1]=v` | extends length | extends length (doubling) | **resolved (this ADR)** |
| LIC-2.6a-wr-3 | value kinds | heterogeneous | Number-only | pending tagged values |

## Out of Scope

- **Hole creation** (`t[i] = v` for `i > length + 1`) —
  pending tagged values.
- **Shrink semantics** (`t[#t] = nil` reducing length) —
  pending tagged values + likely a separate Lua-spec
  compliance phase.
- **Realloc-time GC mark / metatable update** — pending GC
  / metatable infrastructure.
- **Initial capacity tuning** (e.g. minimum cap of 4) —
  premature optimisation.
- **Hash-part grow** — uses the same `emit_table_grow_if_needed`
  shape but on `hash_buf` (offset 24). Lands with 2.6b.
