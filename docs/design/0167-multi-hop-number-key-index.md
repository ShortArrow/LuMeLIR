# 0167. Multi-Hop Number-Key `__index` (Table-Form Chain)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-02
- **Deciders:** ShortArrow

## Context

[ADR 0165](0165-number-key-index-array-oob-fallback.md) closed the single-hop Number-key `__index` Table-form gap; [ADR 0166](0166-index-function-form-number-key.md) added Function-form. Both were single-hop only — a chained Table form like `a → b → c` (each linked via `mt.__index`) still falls through to Nil when the key isn't found at hop 1.

ADR 0134 already established the depth-counter recursion pattern (`emit_check_metatable_index_with_depth`, `METATABLE_INDEX_MAX_HOPS`) for hash-key chains. This ADR mirrors that pattern for the Number-key path.

## Scope (literal)

- ✅ Multi-hop Number-key `__index` with Table-form links. Each hop's `__index_table[key_i]` is bounds-checked; OOB recurses into that table's own metatable.
- ✅ Reuses `METATABLE_INDEX_MAX_HOPS` (same chain budget as hash-key chains).
- ✅ Function-form arm (ADR 0166) still fires at any hop — once a hop resolves `__index` to a Function, dispatch there and stop.
- ❌ Non-Number return from Function-form (ADR 0166 limitation persists).
- ❌ Flat-f64 Number-key `Index` codegen at `emit_expr` (still traps on OOB; same as ADR 0165).
- ❌ TaggedValue runtime key dispatch.
- ❌ Cycle detection beyond static unroll budget (matches ADR 0134 — depth budget exhaustion falls through to Nil rather than trapping, since this is the *Number-key* path and OOB-falls-to-Nil is the contract).

## Decision

### Codegen

`emit_number_key_metatable_index_fallback` gains a `remaining_hops: u32` parameter. The public entry (single existing call site in `emit_local_init_tagged`) passes `METATABLE_INDEX_MAX_HOPS`.

Recursion structure:

1. `remaining_hops == 0` → noop (dst stays Nil). Chain budget exhausted; consistent with the Number-key path's "OOB falls to Nil" contract.
2. Load `mt_ptr`; if null → noop.
3. Probe `mt["__index"]` into tmp slot.
4. If probe tag is `TAG_TABLE`:
   - Bounds-check `__index_table[key_i]`. In-range → copy slot into `dst_slot` (existing C+P semantics).
   - **NEW: OOB → recurse `emit_number_key_metatable_index_fallback(index_table, key_i, dst_slot, remaining_hops - 1, ...)`**.
5. Else if probe tag is `TAG_FUNCTION` AND `(Table, Number) → Number` candidate exists (ADR 0166): sitofp + dispatch + Number-store.
6. Else: noop.

### Naming

The depth-aware helper keeps the existing name `emit_number_key_metatable_index_fallback` (signature change is the new param). A thin Rust shim is unnecessary — the single call site passes the constant explicitly, mirroring how ADR 0134 ended up with `emit_check_metatable_index_with_depth` as the primary helper.

## Alternatives considered

- **Runtime depth counter instead of static unroll**. Rejected — matches ADR 0134 deferred concern; same future ADR can lift both.
- **Trap on budget exhaustion**. Rejected — Number-key semantics is "missing → Nil"; trapping changes the contract.
- **Function-form recursion (function returns missing → consult mt of returned table)**. Rejected — Lua does not recurse through Function-form `__index` returns; once Function fires, the result *is* the value. Only Table-form links are chained.

## Consequences

**Positive**
- Chained Table-form Number-key `__index` now resolves (up to `METATABLE_INDEX_MAX_HOPS`).
- Closes one more bullet on ADR 0165's future-work.
- Pattern symmetry with ADR 0134 (hash-key chain).

**Negative**
- Each call site sees the full static unroll (n × scf.if). At `METATABLE_INDEX_MAX_HOPS = 8`, code size grows modestly; only `emit_local_init_tagged` Number-key OOB path is affected.

**Locked in until superseded**
- Static unroll depth = `METATABLE_INDEX_MAX_HOPS`.
- Table-form only; Function-form is terminal (no chain through Function result).

## Documentation updates

- [x] §4 LIC — extend `LIC-number-key-index-array-oob-fallback-1` note: now multi-hop.
- [x] §8 — adds 0167.
- [x] ADR 0165 future-work — multi-hop bullet RESOLVED.

## Test count delta

```
Step 0:   1372 (after ADR 0166)
C2 (2 e2e Red Day 0):  1372 → 1372
C3 (impl): 1372 → 1374
```

## Critical files

- `src/codegen/emit.rs`:
  - `emit_number_key_metatable_index_fallback` gains `remaining_hops: u32`; recurses in the Table-form OOB arm; budget-exhaustion early-return.
  - Caller in `emit_local_init_tagged` passes `METATABLE_INDEX_MAX_HOPS`.
- `tests/phase2_6plus_multi_hop_number_key_index.rs` (NEW) — 2 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Single-hop (ADR 0165/0166) regresses | Recursion is additive in the OOB arm. In-range path unchanged. Existing 0165/0166 tests pin. |
| Budget exhaustion silently leaks to Nil | Documented; matches Number-key "missing → Nil" contract. |
| Code-size explosion at depth 8 | Single call site; modest. |

## Future work

- Number-key `__newindex` Function form (ADR 0166 mirror for write side).
- Multi-hop Number-key `__newindex`.
- Runtime depth counter (also covers ADR 0134's deferral).
- Unified `__index` runtime tag dispatch (per ADR 0166 future-work).

## References

- [ADR 0134](0134-metatables-index-read.md) — hash-key chain depth-recursion pattern (model for this ADR).
- [ADR 0165](0165-number-key-index-array-oob-fallback.md) — single-hop Table form (helper this ADR extends).
- [ADR 0166](0166-index-function-form-number-key.md) — Function form (preserved at every hop).
- Lua 5.4 reference manual §3.4.10 — `__index` semantics.
