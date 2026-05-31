# 0151. `__newindex = Function` Form (Static-String Key, Number Value)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

Mirror of [ADR 0150](0150-index-function-form.md) for the write side. [ADR 0135](0135-metatables-newindex-write.md) wired the `__newindex` Table form; the Function form was deferred ("Function-form `__newindex` deferred per ADR 0133").

Today `emit_hash_indexassign_with_newindex` (ADR 0135) traps with `s_metatable_newindex_unsupported_kind` when the probed `mt["__newindex"]` tag isn't Table. Lua spec §3.4.10: when `__newindex` is a Function, call it as `mt.__newindex(t, k, v)` — three args, no return.

With the metamethod dispatch ABI mature, the chokepoint extension drops in cleanly.

## Scope (literal)

**Static-String-key IndexAssign**, Function `__newindex` with signature `(Table, String, Number) → ()` (void). Out of scope:

- ❌ Number-key Function form.
- ❌ Non-Number value (`(Table, String, String) → ()`, etc.). Single-typed value only.
- ❌ TaggedValue-key Function form.
- ❌ `__newindex` returning a value (Lua spec says void; we restrict to void user fns).
- ❌ Multi-hop chain through Function result.

## Decision

### HIR

Metamethod-aware refinement walk extended for `__newindex` slot: when the user wrote `mt.__newindex = function(t, k, v) ... end` (Function form, `params.len() == 3`), force `params = [Table, String, Number]` and `ret_kinds = []`.

The Table form (`mt.__newindex = some_table`) doesn't match — its RHS is an Ident chain, not a FunctionExpr.

### Codegen

`emit_hash_indexassign_with_newindex` (ADR 0135 chokepoint): the existing trap arm for non-Table `__newindex` gains a Function-tag check before the trap.

When probed `mt["__newindex"]` tag is `TAG_FUNCTION` AND a `(Table, String, Number) → ()` candidate exists in the module:

1. Load String key ptr from `key_slot`'s payload (offset 8).
2. Dispatch via `emit_dispatch_chain_from_slot_ptr` with `args = [target_ptr, key_ptr, value_v]`, sig `(Table, String, Number) → ()`.
3. No result to store. Set `handled_by_metatable = true` so the raw commit+write path is skipped.

Empty candidate set / Nil tag / Table tag fall through to existing arms (trap / noop / recurse).

### Function threading

The chokepoint helper `emit_hash_indexassign_with_newindex` gains a `functions: &[HirFunction]` parameter, threaded through:

- 2 call sites in `IndexAssign` codegen (static-String/Number arm + TaggedValue-key arm).
- 1 recursive self-call inside the helper (Table form recursion).

## Alternatives considered

- **Widen to (Table, String, Any-Kind) → Any.** Rejected — value-side dispatch adds another tag branch per call site; defer until needed.
- **Non-void return.** Rejected — Lua spec is explicit; restricting to void matches.
- **Bundle Number-key variant.** Rejected — Number-key has its own ABI question (Number key as f64 vs i64 vs tagged), defer.

## Consequences

**Positive**
- Computed-write idiom works: `mt.__newindex = function(t, k, v) storage[k] = v end`.
- Chokepoint extension is ~70 LOC.
- Mirror parity with ADR 0150.

**Negative**
- Helper signature grows by 1 param (functions). Three call-site updates.

**Locked in until superseded**
- Static-String key only.
- `(Table, String, Number) → ()` only.
- Single-hop (no recurse-through-Function).

## Documentation updates

- [x] §4 LIC — new `LIC-newindex-function-form-1`.
- [x] §7 — closes `__newindex = Function` open item.
- [x] §8 — adds 0151.

## Test count delta

```
Step 0:   1356 (after ADR 0150)
C2 (4 e2e Red Day 0):  1356 → 1356
C3 (impl): 1356 → 1360
```

## Critical files

- `src/codegen/emit.rs`:
  - `emit_hash_indexassign_with_newindex` signature: `+functions: &[HirFunction]`.
  - Three call-site updates pass `functions`.
  - Else-arm restructure: Function-tag branch + dispatch.
- `src/hir/mod.rs`:
  - Metamethod-aware refinement walk: `__newindex` Function-form arm.
- `tests/phase2_6plus_newindex_function_form.rs` (NEW) — 4 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Table-form `__newindex` regresses | Function check fires only when probed tag is Function; Table arm unchanged. ADR 0135's 12 green tests are the regression net. |
| Number-key path accidentally routes through Function form | The IndexAssign Number-key arm uses a different entry (no `__newindex` probe at all per ADR 0135). |
| Empty candidate set silently dispatches | Compile-time filter explicit; no candidates → fall through to existing trap. |
| Function returning a value corrupts the abstract state | Compile-time filter restricts to `ret_kinds == []`. |
| ADR 0139 TaggedValue-key IndexAssign Function form not handled | TaggedValue-key arm also calls the same helper, so the new path is reachable via both arms. |

## Future work

- Number-key `__newindex = Function`.
- Non-Number value (`(Table, String, String) → ()` etc.).
- TaggedValue-key Function form.
- Non-void return.
- Multi-hop chain through Function result.

## References

- [ADR 0135](0135-metatables-newindex-write.md) — `__newindex` Table form; chokepoint extended here.
- [ADR 0142](0142-tostring-metamethod.md) — `emit_dispatch_chain_from_slot_ptr` helper reuse.
- [ADR 0150](0150-index-function-form.md) — mirror for `__index`.
- Lua 5.4 reference manual §3.4.10 — `__newindex` semantics.
