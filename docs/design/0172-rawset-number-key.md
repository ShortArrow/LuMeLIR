# 0172. `rawset(t, n, v)` — Number Key

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-03
- **Deciders:** ShortArrow

## Context

[ADR 0136](0136-raw-set-get-builtins.md) landed `rawset` / `rawget` for hash-eligible keys and explicitly deferred Number-key forms "alongside the array `__newindex` ABI decision". ADRs 0168–0171 settled that ABI. With `emit_array_index_assign_at` already in hand (extracted in ADR 0168), Number-key `rawset` collapses to one direct helper call — no metatable consult, no helper recursion.

This ADR closes only the **write side**. `rawget(t, n: Number)` still needs a Number-key Index ABI shift (the existing f64 ABI can't carry "nil on missing"); that's its own ADR.

## Scope (literal)

- ✅ `rawset(t, n, v)` with `n: Number` (literal or computed), `v: non-Nil`. Direct array write — bypasses `mt.__newindex` and all multi-hop chains. Returns `t`.
- ✅ Bounds: `n >= 1` trap (`s_table_oob`) — matches IndexAssign Number-key behaviour.
- ✅ NaN guard via the existing `s_table_index_nan` trap.
- ❌ `rawget(t, n: Number)` — ABI shift deferred.
- ❌ `v: Nil` — already rejected by HIR (`t[k] = nil` is the clear path; ADR 0136 stance preserved).
- ❌ TaggedValue runtime-key dispatch.

## Decision

### HIR validation (`src/hir/mod.rs`)

The existing `arg_idx == 1` type check rejects every non-hash-eligible kind. Loosen it for `RawSet` only:

```rust
if arg_idx == 1 {
    let allowed_for_set = matches!(builtin, Builtin::RawSet)
        && matches!(k, ValueKind::Number);
    let allowed_for_hash = matches!(
        k,
        ValueKind::String | ValueKind::Bool
            | ValueKind::Function(_) | ValueKind::Table
    );
    if !(allowed_for_hash || allowed_for_set) { return Err(TypeMismatch ...) }
}
```

`RawGet` keeps the strict "hash-eligible only" message.

### Codegen (`src/codegen/emit.rs`)

`Callee::Builtin(Builtin::RawSet)` arm gets a Number-key sub-arm before the hash path:

```rust
if matches!(key_kind, ValueKind::Number) {
    // key_value is f64 — NaN guard, f2i, key >= 1 trap, then
    // delegate to emit_array_index_assign_at(t_ptr, key_i, value_v, value_kind).
    // Return t_ptr.
}
// Existing hash-key path falls through unchanged.
```

The existing String/Bool/Function/Table arm calls `emit_hash_indexassign_with_newindex` with `skip_metatable = true`. The Number-key arm uses `emit_array_index_assign_at` directly (the raw-write primitive — by construction it doesn't consult `__newindex`).

## Alternatives considered

- **Wire rawset through `emit_number_key_indexassign_routed` with `remaining_hops = 0`**. Rejected — that helper still allocates `route_iv_slot` / `handled_by_fn_slot` machinery; calling the raw-write primitive directly is cleaner and matches `rawset`'s intent (escape hatch from metatable).
- **Bundle with Number-key `rawget`**. Rejected — `rawget` needs an ABI shift; `rawset` doesn't. ADR-per-decision.
- **Keep deferring**. Rejected — closes a 7-ADR-old deferral cheaply.

## Consequences

**Positive**
- ADR 0136 Number-key deferral closed for `rawset`.
- One-call delegation to an existing primitive (`emit_array_index_assign_at`); minimal LOC.

**Negative**
- `rawset` / `rawget` lose key-kind symmetry: `rawset` accepts Number, `rawget` does not. Documented; the asymmetry is rooted in the Number-key Index ABI returning f64.

**Locked in until superseded**
- Number-key arm uses the raw array-write primitive.
- Non-Nil value restriction inherited from ADR 0136.

## Documentation updates

- [x] §8 — adds 0172 (when SoT next refresh).
- [x] ADR 0136 future-work — `rawset` Number-key RESOLVED; `rawget` Number-key remains.

## Test count delta

```
Step 0: 1382 (after ADR 0171)
C2 (3 e2e Red Day 0): 1382 → 1382
C3 (impl): 1382 → 1385
```

## Critical files

- `src/hir/mod.rs`: `arg_idx == 1` type-check arm gains a `RawSet`-only Number bypass.
- `src/codegen/emit.rs`: `Callee::Builtin(Builtin::RawSet)` arm gets a Number-key sub-arm dispatching to `emit_array_index_assign_at`.
- `tests/phase2_6plus_rawset_number_key.rs` (NEW) — 3 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Hash-key rawset regresses | Number sub-arm is a typed dispatch; hash path stays identical. |
| `rawget(t, 1.0)` regression (still rejected) | Test pins rejection. |
| Number rawset bypasses `__newindex` correctly | Helper is the raw-write primitive — has no metatable code-path by construction. Test pins. |

## Future work

- `rawget(t, n: Number)` with TaggedValue return ABI.
- TaggedValue runtime key for both `rawset` / `rawget`.

## References

- [ADR 0057](0057-phase2-6c-array-grow.md) — array grow + gap-fill (the raw write semantics).
- [ADR 0136](0136-raw-set-get-builtins.md) — original `rawset` / `rawget` ADR (deferral source).
- [ADR 0168](0168-newindex-number-key-table-form.md) — `emit_array_index_assign_at` extraction (the primitive this ADR delegates to).
- Lua 5.4 reference manual §6.1 — `rawset` / `rawget` spec.
