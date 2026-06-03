# 0174. `rawget(t, k)` — TaggedValue Local Key

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-04
- **Deciders:** ShortArrow

## Context

[ADR 0173](0173-rawget-number-key.md) wired `rawget(t, n: Number)`. The remaining `rawget` deferral row — runtime-tag dispatch when the key is a TaggedValue Local (typical `pairs`-body shape `rawget(t, k)` where `k` was iterator-bound) — is closed here.

Scope is deliberately narrow: **Local(TaggedValue) only**. Non-Local TaggedValue expressions (e.g. function returns flowing directly into rawget) stay rejected — they would need a tmp-slot materialisation pass that's its own ADR.

## Scope (literal)

- ✅ `rawget(t, k)` where `k` is `HirExprKind::Local(LocalId)` with `kind == TaggedValue`. Dispatches at runtime on the slot tag:
  - `TAG_NUMBER` → bitcast payload to f64, run the Number-key sub-arm (ADR 0173).
  - `TAG_STRING / TAG_BOOL / TAG_FUNCTION / TAG_TABLE` → use the source slot directly as `search_key_slot`, call `emit_hash_lookup_into_tagged_slot` (ADR 0088).
  - `TAG_NIL` → result is Nil (matches the "table indexing with nil → nil for rawget" pragma; no trap).
- ✅ Bypasses `__index` entirely.
- ❌ Non-Local TaggedValue source (e.g. `rawget(t, f())` where `f` returns TaggedValue).
- ❌ `rawset` TaggedValue-key — sibling ADR.

## Decision

### HIR validation (`src/hir/mod.rs`)

The `arg_idx == 1` raw-builtins check gains a parallel arm:

```rust
let rawget_tagged_local_ok = matches!(builtin, Builtin::RawGet)
    && matches!(k, ValueKind::TaggedValue)
    && matches!(arg.kind, HirExprKind::Local(_));
if !(hash_ok || rawset_number_ok || rawget_number_ok || rawget_tagged_local_ok) { error }
```

`RawSet` TaggedValue-key stays rejected (deferred to a sibling ADR).

### Codegen (`src/codegen/emit.rs`)

`Callee::Builtin(Builtin::RawGet)` arm grows a TaggedValue-key sub-arm before the existing Number / hash dispatch. Detection: `key_kind == TaggedValue` AND `args[1].kind == Local(idx)`.

```rust
let source_slot = slots[idx];
let tag = load source_slot, i64;
scf.if(tag == TAG_NUMBER) {
    let pay_ptr = source_slot + ARRAY_ELEM_OFF_VALUE;
    let pay_i64 = load pay_ptr;
    let pay_f64 = arith.bitcast pay_i64 → f64;
    // inline the ADR 0173 Number-key in-range + raw 16-byte copy:
    //   NaN trap, f2i, length load, in-range scf.if, copy or noop.
} else {
    emit_hash_lookup_into_tagged_slot(t_ptr, source_slot, out_slot, NilOnMissing);
}
return Ok(out_slot);
```

`out_slot` is pre-initialised to Nil at the top of the rawget arm (existing behaviour), so the TAG_NIL case is a no-op.

The Number sub-arm logic is duplicated structurally with ADR 0173 — both run the same algorithm, only the `pay_f64` source differs (bitcast from i64 payload vs. `key_value` f64 directly). A future cleanup ADR may factor this into a shared helper; for now, two short blocks keep the diff narrow.

## Alternatives considered

- **Factor the Number-key in-range copy into a helper now**. Rejected — second use isn't yet third use; the duplication is small (~30 LOC) and live for one ADR.
- **Materialise non-Local TaggedValue into a tmp slot inside this ADR**. Rejected — the materialisation pattern (`emit_extract_prev_k` precedent) needs its own scope decision per ADR-per-decision.
- **Use `emit_extract_prev_k` directly**. Rejected — it returns `(tag, payload)` tuples; we need the slot ptr to pass into the hash lookup helper.

## Consequences

**Positive**
- `pairs`-body idiom `for k, v in pairs(t) do print(rawget(t, k)) end` now compiles end-to-end.
- Reuses ADR 0088 (`emit_hash_lookup_into_tagged_slot`) which already does runtime-tag-aware hash probing — no new helper.

**Negative**
- Number-key sub-arm code duplicated structurally with ADR 0173 (different f64 source). Live cost: ~30 LOC.
- `rawset` still rejects TaggedValue keys; user-facing asymmetry until the sibling ADR lands.

**Locked in until superseded**
- Local restriction. Non-Local TaggedValue source still HIR-rejected.

## Documentation updates

- [x] §8 — adds 0174 (when SoT next refresh).
- [x] ADR 0173 future-work — TaggedValue runtime-key bullet RESOLVED for rawget (Local restriction).

## Test count delta

```
Step 0: 1388 (after ADR 0173)
C2 (3 e2e Red Day 0): 1388 → 1388
C3 (impl): 1388 → 1391
```

## Critical files

- `src/hir/mod.rs`: extend `arg_idx == 1` raw-builtins type check.
- `src/codegen/emit.rs`: `Callee::Builtin(Builtin::RawGet)` arm gains a TaggedValue Local sub-arm.
- `tests/phase2_6plus_rawget_tagged_key.rs` (NEW) — 3 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Hash-key + Number-key paths regress | TaggedValue sub-arm sits before the existing key_kind dispatch; static-kind paths unchanged. |
| Number tag with non-f64 payload | TAG_NUMBER's payload is always a Number f64 (bit-pattern stored as i64). Bitcast is correctness-preserving. |
| TaggedValue Local without an actual local backing | HIR pattern guard ensures `Local(LocalId)` shape; slots[idx] is the bound slot. |
| `__index` consulted | Code never invokes the metatable helpers. |

## Future work

- `rawset(t, k, v)` TaggedValue Local key (sibling ADR).
- Non-Local TaggedValue source (tmp materialisation pass).
- Same dispatch for `Index` / `IndexAssign` chokepoints (each closing a "TaggedValue runtime-key dispatch" deferral row across the ADR catalogue).

## References

- [ADR 0084](0084-phase2-6plus-taggedvalue-key.md) — TaggedValue Local-key restriction precedent.
- [ADR 0088](0088-phase2-6b-hash-lookup-miss.md) — `emit_hash_lookup_into_tagged_slot` (the hash arm reuses this).
- [ADR 0136](0136-raw-set-get-builtins.md) — `rawset` / `rawget` builtins.
- [ADR 0173](0173-rawget-number-key.md) — Number-key sub-arm (the algorithm this ADR's Number branch mirrors).
- Lua 5.4 reference manual §6.1 — `rawget` spec.
