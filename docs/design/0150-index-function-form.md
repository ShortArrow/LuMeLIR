# 0150. `__index = Function` Form (Static-String Key, Number Return)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

[ADR 0134](0134-metatables-index-read.md) wired the `__index` Table form: `mt.__index = some_table` recurses up to depth 2000. The Function form (`mt.__index = function(t, k) ... end`) was deferred — it requires call-ABI dispatch from the chokepoint helper `emit_check_metatable_index_with_depth`, which today traps on non-Table `__index`.

With ADRs 0142–0149 having proven the `emit_dispatch_chain_from_slot_ptr` pattern, the Function form drops in cleanly: when the probed `mt["__index"]` tag is `TAG_FUNCTION`, dispatch `(t, key) → Number` instead of trapping.

## Scope (literal)

**Static-String-key Index only**, Function `__index` with signature `(Table, String) → Number`. The metamethod result is stored Number-tagged into the consumer's `out_slot`. Out of scope:

- ❌ Number-key Index (`t[i]`) — the chokepoint receives a NumberKind search key, but the metamethod expects String. Number-key Function-form deferred.
- ❌ Non-Number return (String / Bool / Nil / Table / Function). Function returning TaggedValue requires consumer-side widening, deferred.
- ❌ Multi-hop chain through Function (Function returns a Table, recurse). Lua spec allows it; this ADR keeps the single-hop case.
- ❌ TaggedValue runtime-key dispatch.
- ❌ Mixed `__index` with both Table and Function (Function takes precedence in spec; this ADR's chokepoint chooses Function only when the probed tag is Function, otherwise the existing Table arm fires).

## Decision

### HIR

No HIR change. The Index path already flows through the codegen chokepoint. Metamethod-aware refinement walk extended for `__index` slot: when the user wrote `mt.__index = function(t, k) ... end`, force `params = [Table, String]` and `ret_kinds = [Number]`.

### Codegen

`emit_check_metatable_index_with_depth` (chokepoint at `src/codegen/emit.rs:~12209`): the existing "is_table == false → if is_nil noop else trap" else arm gains a Function-tag check before the trap. When `probe_tag == TAG_FUNCTION`:

1. Compile-time candidate filter: user fns with sig `(Table, String) → Number`.
2. Empty candidate set → fall through to existing trap.
3. Extract closure ptr from probe slot's payload; load the user-supplied String key from `user_search_key_slot`'s payload.
4. Dispatch via `emit_dispatch_chain_from_slot_ptr` with `args = [target_ptr, key_ptr]`, sig `(Table, String) → Number`.
5. Store the Number result into `dst_slot` via `emit_value_slot_store_number`.

The Table-form arm is unchanged. The Nil-tag and unsupported-kind paths are unchanged.

### New module globals

None — `s_metatable_index_field_name` already exists (ADR 0134); the dispatch reuses `s_call_non_function` for the empty-candidate trap (existing).

## Alternatives considered

- **Widen `__index` Function return to TaggedValue.** Rejected for first cut — consumer-side widening adds tag-dispatch at every `t.k` consumer; defer until a use case demands it.
- **Support Number-key in the same ADR.** Rejected — the chokepoint's `user_search_key_slot` holds a Number-tagged f64; passing that as a Number arg to the metamethod is a different sig (`(Table, Number) → Number`), and changing the chokepoint to dual-key is its own design surface.
- **Trap on empty candidate set silently.** Rejected — falls back to the existing `s_metatable_index_unsupported_kind` trap for clarity at the runtime.

## Consequences

**Positive**
- The canonical lazy / computed lookup idiom works: `mt.__index = function(t, k) return some_computation(k) end`.
- Reuses `emit_dispatch_chain_from_slot_ptr` — the chokepoint diff is ~50 LOC.

**Negative**
- Chokepoint helper grows ~50 LOC; no new globals.
- Number-key Function form remains rejected. Documented limitation.

**Locked in until superseded**
- Static-String key only.
- `(Table, String) → Number` only.
- Single-hop (no recursing through a Function result).

## Documentation updates

- [x] §4 LIC — new `LIC-index-function-form-1`.
- [x] §7 — closes `__index = Function` open item.
- [x] §8 — adds 0150.

## Test count delta

```
Step 0:   1352 (after ADR 0149)
C2 (4 e2e Red Day 0):  1352 → 1352
C3 (impl): 1352 → 1356
```

## Critical files

- `src/codegen/emit.rs`:
  - `emit_check_metatable_index_with_depth` else arm: add Function-tag branch before the trap.
  - Restructured: candidate filter + dispatch + Number-tagged store.
- `src/hir/mod.rs`:
  - Metamethod-aware refinement walk: `__index` Function form when `params.len() == 2`.
- `tests/phase2_6plus_index_function_form.rs` (NEW) — 4 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Table-form `__index` regresses | Function check fires only when probed tag is Function; Table arm unchanged. ADR 0134's 11 green tests are the regression net. |
| Number-key `t[i]` accidentally routes through Function form | The Index chokepoint is per-key-kind; Number-key path uses a different entry. Test pins. |
| Empty candidate set silently dispatches | Compile-time filter explicit; no candidates → fall through to existing trap. |
| Function returning non-Number causes corrupt slot | Compile-time candidate filter restricts to `(Table, String) → Number`. |

## Future work

- Number-key `__index = Function`.
- Non-Number return (TaggedValue widening).
- Multi-hop through Function result.
- `__newindex = Function` form (ADR 0151 candidate).
- TaggedValue runtime-key dispatch.

## References

- [ADR 0134](0134-metatables-index-read.md) — `__index` Table form; chokepoint extended here.
- [ADR 0142](0142-tostring-metamethod.md) — `emit_dispatch_chain_from_slot_ptr` helper reuse.
- [ADR 0141](0141-anon-fn-indexassign-param-refine.md) — anon-fn param refinement.
- Lua 5.4 reference manual §3.4.10 — `__index` semantics.
