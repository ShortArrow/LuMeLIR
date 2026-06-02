# 0169. `__newindex = Function` Form, Number Key

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-02
- **Deciders:** ShortArrow

## Context

[ADR 0151](0151-newindex-function-form.md) wired Function-form `__newindex` for **String** keys; [ADR 0168](0168-newindex-number-key-table-form.md) closed Table-form Number key (`key > length`). This ADR closes the third combination — **Function-form, Number key** — completing the Number-key `__newindex` matrix for static keys (alongside Table form).

Sibling of ADR 0166 (read-side Number-key Function form). Mirror of ADR 0151 (write-side String-key Function form).

## Scope (literal)

- ✅ `t[i] = v` (Number key, Number value) where `i > current length` AND `mt.__newindex` is `Function` AND a `(Table, Number, Number) → ()` candidate exists in the module. Dispatches `fn(t, i_f64, v)`, suppresses any write to `t`.
- ✅ Single-hop. The candidate body is responsible for any further routing.
- ❌ Non-Number value (matches ADR 0151's Number-value restriction for the Function-form `__newindex` scope).
- ❌ `i in [1, length] && existing slot tag == TAG_NIL` (mid-array nil). Deferred per ADR 0168.
- ❌ Multi-hop chains.
- ❌ TaggedValue runtime key dispatch.

## Decision

### Codegen

ADR 0168's routing already allocates `route_iv_slot` (i64 sentinel; non-zero = "route to inner Table"). This ADR adds a parallel `handled_by_fn_slot` (i1 sentinel; true = "Function form already executed the call").

Inside the existing `mt_then_blk` (probe of `mt["__newindex"]`):

1. Existing TAG_TABLE arm — unchanged (stores inner ptr's i64 into `route_iv_slot`).
2. **NEW** TAG_FUNCTION arm — filter `functions` for `(Table, Number, Number) → ()` candidates. When non-empty AND probe tag == TAG_FUNCTION AND value is Number-kind:
   - sitofp `key_i` (i64) → `key_f` (f64).
   - Dispatch via `emit_dispatch_chain_from_slot_ptr` with sig `(Table, Number, Number) → ()`, args `[target_ptr, key_f, value_v]`.
   - Store `true` into `handled_by_fn_slot`.

Final dispatch (replacing the existing `scf.if(should_route, ...)`):

```
if should_route:
    emit_array_index_assign_at(inner_target, ...)
else:
    if handled_by_fn:
        noop
    else:
        emit_array_index_assign_at(target_ptr, ...)
```

The Function-form check is Rust-time gated on `value_kind == Number` AND non-empty candidate set; empty candidates simply leave `handled_by_fn` false → outer write proceeds (matches existing behaviour).

### Coexistence with ADR 0166

ADR 0166's read-side filter expects `(Table, Number) → Number` user functions. This ADR's write-side filter expects `(Table, Number, Number) → ()`. Different signatures → different filters → no conflict.

User-facing limitation: an `__index` reader and an `__newindex` writer require TWO user functions of different signatures; documented.

## Alternatives considered

- **Single fused `handled` flag for both Table and Function cases**. Rejected — would require restructuring the existing ADR 0168 routing alloca; cleaner to add the second sentinel additively.
- **Non-Number value support**. Rejected — matches ADR 0151's Number-only scope literal; full TaggedValue ABI for the value slot is its own decision.
- **Bundle with multi-hop**. Rejected per ADR-per-decision.

## Consequences

**Positive**
- `t[i] = v` with `mt.__newindex = function(t, k, v) ... end` now works for `i > length`, Number `v`.
- Symmetry with ADR 0166 (read-side Function form Number key).
- ADR 0168's "Function form deferred" future-work closed.

**Negative**
- Two parallel flag allocas in the routing path. Acceptable; modest LOC growth.
- User code mixing String and Number `__newindex` needs two user fns. Documented.

**Locked in until superseded**
- Number-value only.
- Single-hop.
- `key > length` trigger only.

## Documentation updates

- [x] §8 — adds 0169 (when SoT next refresh).
- [x] ADR 0168 future-work — Function form RESOLVED.

## Test count delta

```
Step 0: 1376 (after ADR 0168)
C2 (2 e2e Red Day 0): 1376 → 1376
C3 (impl): 1376 → 1378
```

## Critical files

- `src/codegen/emit.rs`:
  - Number-key IndexAssign arm: add `handled_by_fn_slot: i1` alloca, TAG_FUNCTION sub-arm in the `mt_then` block, final dispatch becomes nested scf.if.
- `tests/phase2_6plus_newindex_function_form_number_key.rs` (NEW) — 2 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| ADR 0168 Table form regresses | Function arm is added in parallel; Table arm logic unchanged. Both arms set independent sentinels. Pinned by ADR 0168 tests. |
| Empty candidate set hangs | Empty filter → no-op (handled_by_fn stays false) → outer write proceeds. |
| Function call with wrong sig | Compile-time filter rejects mismatches. |

## Future work

- Non-Number value (full TaggedValue ABI).
- Multi-hop chain (recursion through Function-form return is not Lua semantics, but Table-form chain inside Function-form callee is).
- Mid-array TAG_NIL trigger (extends ADR 0168 + this ADR).
- Unified runtime tag dispatch (per ADR 0166 future-work).

## References

- [ADR 0151](0151-newindex-function-form.md) — String-key Function form (mirror).
- [ADR 0166](0166-index-function-form-number-key.md) — read-side Number-key Function form (sibling).
- [ADR 0168](0168-newindex-number-key-table-form.md) — Number-key Table form (helper extension this ADR builds on).
- Lua 5.4 reference manual §2.4 / §3.4.10 — `__newindex` semantics.
