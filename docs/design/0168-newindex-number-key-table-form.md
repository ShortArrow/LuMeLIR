# 0168. `__newindex = Table` Form, Number Key (key > length)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-02
- **Deciders:** ShortArrow

## Context

[ADR 0135](0135-metatables-newindex-write.md) wired hash-key `__newindex` Table form and explicitly deferred the Number-key (array) write path, citing a prerequisite "missing key in array part" ABI decision. [ADR 0165](0165-number-key-index-array-oob-fallback.md) and [ADR 0167](0167-multi-hop-number-key-index.md) closed the read side; this ADR closes the **simplest correct scope** of the write side.

## Scope (literal)

- ✅ `t[i] = v` (Number key, any tagged value kind) where **`i > current length`** AND `setmetatable(t, mt)` AND `mt.__newindex` is `Table`. Redirects the write to `mt.__newindex` via the array-extending write semantics (grow + gap-fill + length update + store) on the inner table. The outer `t`'s array is left untouched (NO growth, NO write at outer).
- ✅ Single-hop only. The inner write to `mt.__newindex` does **not** consult its own metatable's `__newindex`; it always lands in the inner table's array part.
- ❌ `i in [1, length] && existing slot tag == TAG_NIL` (mid-array nil case). Per Lua §2.4, this is "rawnil" and should fire `__newindex` too. Deferred — requires a runtime slot-tag load *before* the store and is not exercised by current corpus.
- ❌ Function form (`mt.__newindex = function(t, k, v) ... end`). Deferred to a sibling ADR (mirror of ADR 0151 for Number key).
- ❌ Multi-hop chains (`mt.__newindex` has its own `__newindex`).
- ❌ `i < 1` — pre-existing `s_table_oob` trap fires at the outer scope before any routing, matching ADR 0057 behaviour.

## Decision

### Codegen

The existing Number-key `IndexAssign` arm (`HirStmtKind::IndexAssign` in `emit_stmt`, currently inline in `src/codegen/emit.rs` at ~3651) is **Tidy First refactored** into a small helper:

```rust
fn emit_array_index_assign_at(
    context, block,
    target_ptr,   // table descriptor ptr to write into
    key_i,        // i64, already ≥ 1 (caller traps too_small)
    value_v,      // f64 / ptr / i1 already lowered
    value_kind,
    types, loc,
)
```

Body = existing grow + gap-fill + length-update + store sequence verbatim. No semantic change.

The Number-key arm then becomes:

```
trap_if(key < 1, "s_table_oob")
length = load len(target_ptr)
key_high = key > length
mt = load metatable_ptr(target_ptr)
mt_present = mt != null

// Compute "should redirect to inner" + materialise inner_target_ptr.
inner_target_alloca = alloca ptr
store null into inner_target_alloca

if key_high && mt_present:
    probe = hash_lookup mt["__newindex"] into tmp tagged slot
    if probe tag == TAG_TABLE:
        store probe.payload (ptr) into inner_target_alloca

inner_target = load inner_target_alloca
should_route = ptrtoint(inner_target) != 0

if should_route:
    emit_array_index_assign_at(inner_target, key, value, value_kind)
else:
    emit_array_index_assign_at(target_ptr, key, value, value_kind)
```

Two call sites of the helper (one in each scf.if branch) — the duplication is the cost of branching on a runtime check; an alternative phi-style structure would cost more MLIR.

### Why "key > length" only (not also tag == Nil mid-array)

Mid-array Nil writes (`t[2] = nil` then `t[2] = "x"`) are rare in the corpus and require an extra runtime tag-load+check before the store. Deferring keeps this ADR's literal scope tight; a follow-up ADR can extend `should_route` with `OR (tag_at(elem) == TAG_NIL)`.

## Alternatives considered

- **Full Lua-spec rawnil semantics (key > length OR slot tag == Nil)**. Rejected — see above; deferral is documented.
- **Recursive `__newindex` chain through inner table**. Rejected — Multi-hop scope ceiling, mirror of ADR 0167's stance.
- **Reuse `emit_hash_indexassign_with_newindex` (ADR 0135)**. Rejected — that helper is hash-key-shaped (uses a tagged search-key slot); the array path is structurally different.
- **No helper extraction; inline the routing**. Rejected — two structurally identical write paths (outer vs inner) demand a helper; Tidy First.

## Consequences

**Positive**
- ADR 0135's Number-key deferral closed for the simplest case.
- Helper extraction makes future Function-form / multi-hop / mid-array-Nil ADRs cheap to layer.
- Read side (ADR 0165/0167) + write side (this) now have matching single-hop Table-form coverage for Number key.

**Negative**
- Each Number-key `IndexAssign` site now emits a metatable probe even when no `__newindex` is set (1 ptr load + 1 cmp + 1 scf.if guard). Negligible.
- Mid-array Nil case stays "raw write to outer" until a follow-up ADR.

**Locked in until superseded**
- `key > length` trigger only.
- Single-hop Table form only.
- Inner write is "raw" (no recursive metatable consult).

## Documentation updates

- [x] §8 — adds 0168 (when SoT next refresh).
- [x] ADR 0135 future-work — Number-key Table form RESOLVED by this ADR (for the `key > length` case).
- [x] ADR 0165 future-work — IndexAssign analog Table form RESOLVED.

## Test count delta

```
Step 0: 1374 (after ADR 0167)
C2 (2 e2e Red Day 0): 1374 → 1374 (1 Red + 1 regression pin)
C3 (impl): 1374 → 1376
```

## Critical files

- `src/codegen/emit.rs`:
  - NEW helper `emit_array_index_assign_at` — verbatim move of existing grow + fill + length-update + store sequence.
  - `IndexAssign` Number-key arm rewires to: trap + length-load + mt-probe + conditional route → helper call.
- `tests/phase2_6plus_newindex_number_key_table_form.rs` (NEW) — 2 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Existing array-write tests regress | Helper extraction is verbatim move; default route (no mt or non-Table `__newindex`) calls helper on outer target — identical MLIR to before. Pinned by full corpus regression. |
| Routing leaks into outer table | Routed branch returns without touching outer; non-routed branch leaves inner alloca null, never reads `inner_target`. |
| `__newindex` mid-array Nil still raw-writes | Documented; deferred. |
| Inner write itself triggering `__newindex` chain | Helper does raw array write, no further metatable consult. Single-hop scope literal. |

## Future work

- Mid-array `TAG_NIL` slot triggers `__newindex` (extends `should_route`).
- Function-form Number-key `__newindex` (ADR 0151 mirror, ADR 0166 sibling).
- Multi-hop Number-key `__newindex` (ADR 0167 write-side mirror).
- `rawset(t, n, v)` for Number `n` (ADR 0136 deferral pair).

## References

- [ADR 0057](0057-phase2-6c-array-grow.md) — array grow + gap-fill semantics this ADR preserves.
- [ADR 0135](0135-metatables-newindex-write.md) — hash-key `__newindex` Table form (deferral source).
- [ADR 0151](0151-newindex-function-form.md) — hash-key `__newindex` Function form (sibling for future Number-key Function form).
- [ADR 0165](0165-number-key-index-array-oob-fallback.md) — read-side single-hop Table form (this ADR's mirror).
- [ADR 0167](0167-multi-hop-number-key-index.md) — read-side multi-hop (future write-side mirror).
- Lua 5.4 reference manual §2.4 / §3.4.10 — `__newindex` semantics.
