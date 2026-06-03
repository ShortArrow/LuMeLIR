# 0173. `rawget(t, n)` — Number Key

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-04
- **Deciders:** ShortArrow

## Context

[ADR 0136](0136-raw-set-get-builtins.md) deferred Number-key `rawget` citing the "Number-key Index ABI returns f64, can't express nil-on-miss". [ADR 0172](0172-rawset-number-key.md) closed the write side. This ADR closes the read side by **localising the ABI shift** to the rawget call itself — rawget already returns `TaggedValue` (ADR 0136 §29), so the rawget arm can populate a TaggedValue out-slot directly without changing the general Number-key Index ABI.

## Scope (literal)

- ✅ `rawget(t, n)` with `n: Number` (literal or computed). Returns the slot at `t[n]` as TaggedValue. Out-of-range or `TAG_NIL` slot → Nil-tagged TaggedValue. Never traps on OOB.
- ✅ NaN-key trap (`s_table_index_nan`) — consistent with all Number-key code paths.
- ✅ Bypasses `mt.__index` entirely (matches ADR 0136 raw-builtins contract).
- ❌ TaggedValue runtime-key dispatch.
- ❌ Changing the general Number-key `Index` ABI at `emit_expr` (still returns f64 in flat consumers, still traps on OOB).

## Decision

### HIR validation (`src/hir/mod.rs`)

The `arg_idx == 1` arm for raw builtins (ADR 0172 form) gains a parallel `RawGet` Number-key bypass:

```rust
let rawset_number_ok = matches!(builtin, Builtin::RawSet) && k == Number;
let rawget_number_ok = matches!(builtin, Builtin::RawGet) && k == Number;
if !(hash_ok || rawset_number_ok || rawget_number_ok) { error }
```

### Codegen (`src/codegen/emit.rs`)

`Callee::Builtin(Builtin::RawGet)` arm gets a Number-key sub-arm:

```rust
if matches!(key_kind, ValueKind::Number) {
    let out_slot = alloca TaggedValue, init Nil;
    NaN trap on key_value;
    let key_i = f2i(key_value);
    let length_i = load t_ptr+0 (length field);
    let one = const i64 1;
    let in_range = (key_i >= 1) AND (key_i <= length_i);
    scf.if(in_range) {
        let array_buf = emit_table_array_buf(t_ptr);
        let elem_ptr = emit_array_elem_ptr(array_buf, key_i);
        // raw 16-byte copy: tag word + payload word
        let tag = load elem_ptr;
        store tag → out_slot;
        let payload_src = elem_ptr + 8;
        let payload_dst = out_slot + 8;
        let payload = load payload_src;
        store payload → payload_dst;
    } else {
        noop // stays Nil
    };
    return Ok(out_slot);
}
// hash-key path unchanged
```

The OOB branch is silent — Lua spec: rawget out-of-range returns nil, no trap.

## Alternatives considered

- **Widen general Number-key Index ABI to TaggedValue**. Rejected — ripples through every Number-key Index consumer. Out of scope.
- **Reuse `emit_number_key_metatable_index_fallback`** (ADR 0165 helper). Rejected — that helper consults `__index`; rawget must bypass.
- **Bundle with TaggedValue runtime-key dispatch**. Rejected per ADR-per-decision.

## Consequences

**Positive**
- Closes the last ADR 0136 Number-key deferral; full `rawset`/`rawget` Number-key symmetry.
- The ABI shift is localised — no consumer of `emit_expr` sees a change.
- Read/write parity with ADR 0172.

**Negative**
- Adds a fourth Number-key array-read path (Index trap-OOB at `emit_expr`, tagged-consumer fallback via ADR 0165, multi-hop fallback via ADR 0167, and now rawget no-trap-OOB). Documented; each has distinct semantics required by its caller.

**Locked in until superseded**
- Number-key arm allocates a TaggedValue out_slot and does raw 16-byte slot copy.
- OOB → Nil silently (rawget spec).

## Documentation updates

- [x] §8 — adds 0173 (when SoT next refresh).
- [x] ADR 0136 future-work — Number-key rawget RESOLVED.

## Test count delta

```
Step 0: 1385 (after ADR 0172)
C2 (3 e2e Red Day 0): 1385 → 1385
C3 (impl): 1385 → 1388
```

## Critical files

- `src/hir/mod.rs`: extend the `arg_idx == 1` raw-builtins type check to accept `RawGet + Number`.
- `src/codegen/emit.rs`: `Callee::Builtin(Builtin::RawGet)` arm dispatches on key_kind.
- `tests/phase2_6plus_rawget_number_key.rs` (NEW) — 3 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Hash-key rawget regresses | Number sub-arm is a typed dispatch; hash path is the else-arm. |
| OOB rawget traps | The Number sub-arm uses scf.if guard — no trap. Test 2 pins. |
| Mid-array TAG_NIL leaks via rawget | Raw 16-byte copy preserves TAG_NIL tag → out_slot reads as Nil. Test 3 pins. |
| `__index` consulted accidentally | The arm never calls any `__index` helper. |

## Future work

- TaggedValue runtime-key rawset / rawget.
- Widen general Number-key Index ABI to TaggedValue (optional; only if a non-rawget consumer demands it).

## References

- [ADR 0136](0136-raw-set-get-builtins.md) — `rawset`/`rawget` original ADR.
- [ADR 0165](0165-number-key-index-array-oob-fallback.md) — tagged-consumer Number-key fallback (different semantics: consults `__index`).
- [ADR 0172](0172-rawset-number-key.md) — write-side sibling.
- Lua 5.4 reference manual §6.1 — `rawget` spec.
