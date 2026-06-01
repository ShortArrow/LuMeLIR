# 0166. `__index = Function` Form, Number Key

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-02
- **Deciders:** ShortArrow

## Context

[ADR 0150](0150-index-function-form.md) wired the Function-form `__index` chokepoint for **String** keys. [ADR 0165](0165-number-key-index-array-oob-fallback.md) closed the Number-key Table-form gap. This ADR closes the third combination — **Function-form, Number key** — completing the Function-form `__index` matrix for both static key kinds.

Mirror of ADR 0150, sibling of ADR 0165.

## Scope (literal)

- ✅ `t[i]` (Number key) where array-OOB → `mt.__index = function(t, k) ... end` dispatched with `(t, k_f64)` and returning Number, stored Number-tagged into `dst_slot`.
- ✅ Single-hop only.
- ❌ Multi-hop Function form (deferred to a future ADR alongside Number-key multi-hop Table form).
- ❌ Non-Number return.
- ❌ Flat-f64 Number-key `Index` codegen at `emit_expr` (still traps on OOB; only tagged-consumer path gets the fallback — same as ADR 0165).
- ❌ TaggedValue runtime key dispatch.

## Decision

### Codegen

`emit_number_key_metatable_index_fallback` (ADR 0165) extended with a Function-tag branch:

After the existing Table-tag arm (recurse into `__index_table[key_i]`), add:

1. If probe tag is `TAG_FUNCTION` AND a `(Table, Number) → Number` candidate exists in the module:
   - Convert `key_i` (i64) → `key_f` (f64) via `arith.sitofp`.
   - Dispatch via `emit_dispatch_chain_from_slot_ptr` with sig `(Table, Number) → Number`, args `[target_ptr, key_f]`.
   - Store the f64 result into `dst_slot` via `emit_value_slot_store_number`.
2. Empty candidate set → no-op (dst stays Nil).
3. Other probe tags → no-op (existing behaviour).

### HIR metamethod-aware refinement

ADR 0150's refinement walk already forces `__index` with `params.len() == 2` to `[Table, String] → Number`. This ADR's Number-key form needs `[Table, Number] → Number`.

Two `__index` Function-form shapes now coexist:

| key kind | refined `(params, ret)` |
|---|---|
| static-String (`t.k`) | `(Table, String) → Number` (ADR 0150) |
| static-Number (`t[i]`) | `(Table, Number) → Number` (this ADR) |

The HIR walker can't distinguish them from the FunctionExpr alone — the call site key kind is what disambiguates. To support both:

- The refinement walker forces the second param to a special "either-kind" sentinel? No — kinds are not optional.
- Alternative: the **codegen-side candidate filter** at each chokepoint already selects by sig; we leave the HIR refinement as `params[1] = String` (ADR 0150's default) and **let codegen accept (Table, Number) → Number user fns regardless of the slot's refinement**.

Concretely: ADR 0150's refinement still fires for `__index` slot. If the user wrote a `(Table, Number) → Number` function, the codegen filter in this ADR's new arm finds it; ADR 0150's String-key filter doesn't. No conflict — different filters at different chokepoints.

The user-facing implication: if both `t.k` (String) and `t[i]` (Number) consumers want `__index` Function form, they need TWO user functions — one with each signature. Documented limitation; future ADR may unify via a runtime tag dispatch.

## Alternatives considered

- **HIR refinement of `__index` to `(Table, TaggedValue) → TaggedValue`**. Rejected — the existing TaggedValue ABI for ret_kinds would ripple through call-site widening; out of scope here.
- **Skip Function-form Number-key entirely**. Rejected — was on 0165's future-work bullet; closing it now removes a documented gap.
- **Bundle with multi-hop**. Rejected per ADR-per-decision; multi-hop is its own concern.

## Consequences

**Positive**
- `t[i]` with `mt.__index = function(t, k) return computation(k) end` now works.
- Sibling parity with ADR 0150 (String-key Function form).
- One bullet closed on ADR 0165's future-work.

**Negative**
- Mixed String/Number `__index` consumers need two user fns. Documented.

**Locked in until superseded**
- Single-hop only.
- `(Table, Number) → Number` candidate filter.
- Coexists with ADR 0150 String-key filter without overlap.

## Documentation updates

- [x] §4 LIC — new `LIC-index-function-form-number-key-1`.
- [x] §8 — adds 0166.
- [x] ADR 0165 future-work — Function-form Number-key bullet marked RESOLVED.

## Test count delta

```
Step 0:   1370 (after ADR 0165)
C2 (2 e2e Red Day 0):  1370 → 1370
C3 (impl): 1370 → 1372
```

## Critical files

- `src/codegen/emit.rs`:
  - `emit_number_key_metatable_index_fallback` else-arm extended with Function-tag check + dispatch + Number-tagged store.
- `tests/phase2_6plus_index_function_form_number_key.rs` (NEW) — 2 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Table-form (ADR 0165) regresses | Function-tag branch added *after* Table-tag arm; Table-form path unchanged. |
| Mixed String/Number `__index` user accidentally hits wrong filter | Compile-time filters are signature-strict; mismatches don't dispatch. |
| Empty candidate set hangs / corrupts | Empty filter → no-op (dst stays Nil). Test pins. |

## Future work

- Multi-hop Number-key `__index` (Table + Function chains).
- Unified `__index` via runtime tag dispatch (allows one user fn for both String and Number consumers).
- Flat-f64 Number-key Index OOB widening (still traps in non-tagged consumers).
- Number-key `__newindex` Function form (ADR 0151 sibling).

## References

- [ADR 0150](0150-index-function-form.md) — String-key Function form (sibling).
- [ADR 0165](0165-number-key-index-array-oob-fallback.md) — Number-key Table-form fallback (the helper this ADR extends).
- [ADR 0142](0142-tostring-metamethod.md) — `emit_dispatch_chain_from_slot_ptr` reused.
- Lua 5.4 reference manual §3.4.10 — `__index` semantics.
