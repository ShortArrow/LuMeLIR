# 0142. `__tostring` Metamethod for `tostring(t)`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

The first Tier 2 metamethod ADR. With [ADR 0141](0141-anon-fn-indexassign-param-refine.md) landing param-kind refinement for the natural `mt.__tostring = function(t) return "..." end` idiom, the call ABI prerequisite is satisfied. This ADR wires `tostring(t)` to consult `t`'s metatable.

Today `tostring(t)` panics at codegen (`src/codegen/emit.rs:13229`) for `ValueKind::Table`. Lua spec §6.1: `tostring` checks the metatable's `__tostring` field — if present and a Function, calls it with `t` as the arg and expects a String back; otherwise returns a default representation (we use the literal "table", matching `type(t)`).

## Scope (literal)

**`tostring(t)` builtin only, Table operand.** Out of scope:

- ❌ `print(t)` for Table operand — separate consumer with its own dispatch (uses `emit_print_value_raw` / `emit_print_tagged_local` Table arms today, prints "table" literal). Future ADR.
- ❌ `..` concat with Table operand — HIR `coerce_to_string` (`src/hir/mod.rs:414`) rejects today. Future ADR.
- ❌ Non-Function `__tostring` value (e.g. `mt.__tostring = "custom"` — Lua spec says String OK there). Function-form only.
- ❌ `__tostring` returning non-String (Lua spec says it's an error). We don't enforce — the existing IndirectDispatch chain's compile-time candidate filter restricts to `(Table) → String` user fns, so non-String returns are not a callable candidate.
- ❌ TaggedValue operand whose runtime tag is `TAG_TABLE` — only the static-Table operand path. TaggedValue dispatch via the existing `emit_tostring(value, TaggedValue)` path goes through `Number` as before.

## Decision

### HIR

`tostring(t)` for `ValueKind::Table` is now accepted (no kind rejection). The `Builtin::ToString` signature is unchanged (`(any-printable) → String`); the per-arg loop in `lower_builtin_call` already permits Table for `ToString` implicitly (no explicit rejection — the panic is at codegen).

### Codegen

`src/codegen/emit.rs::emit_tostring` Table arm replaced:

1. Get `t_ptr` (the Table operand).
2. Load `mt_ptr` at `t_ptr + TABLE_OFF_METATABLE` (offset 32, ADR 0134).
3. If `mt_ptr` is null → yield `emit_addressof("s_typename_table")` (existing literal).
4. Else: probe `mt_ptr["__tostring"]` via `emit_hash_lookup_into_tagged_slot(NilOnMissing)` (ADR 0088) into a fresh TaggedValue tmp slot.
5. Load the probe-result tag. If `tag != TAG_FUNCTION` → yield the literal (matches Lua behaviour when `__tostring` is absent or wrong-typed).
6. Else: extract the closure cell ptr from the tmp slot's payload (offset 8), then call a new helper `emit_dispatch_chain_from_slot_ptr` (Tidy First extract of the slot-load-then-dispatch portion of `emit_indirect_dispatch_chain_in_block`) with:
   - `slot_ptr = tmp_slot`
   - `sig = IndirectSig { param_kinds: [Table], ret_kinds: [String] }`
   - `candidates = all user fns in the module with that signature` (computed via `compatible_user_functions`-equivalent inline filter, accessible from codegen via the `functions` param)
   - `args = [t_ptr]`
7. The dispatch chain yields a `ptr` result (the String). The `scf.if` wrapping the whole flow yields the same.

### New helper `emit_dispatch_chain_from_slot_ptr`

Tidy First extract of `emit_indirect_dispatch_chain_in_block`'s body, decomposing it so the slot source is a `Value<ptr>` (not `slot_idx: usize` into `slots: &[Value]`) and the args are pre-lowered `Vec<Value>` (not `&[HirExpr]` to be lowered).

Signature:

```rust
fn emit_dispatch_chain_from_slot_ptr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    sig: &IndirectSig,
    candidates: &[FuncId],
    arg_vals: &[Value<'c, 'a>],
    functions: &[HirFunction],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<Vec<Value<'c, 'a>>, CodegenError>
```

Existing `emit_indirect_dispatch_chain_in_block` delegates to this helper after computing `slot_ptr = slots[slot_idx]` and pre-lowering args.

## Alternatives considered

- **HIR-level rewrite of `tostring(t)` into `getmetatable(t).__tostring(t)`**. Rejected — getmetatable returns TaggedValue and dispatch needs runtime null-mt + missing-field handling; codegen is the natural layer for that.
- **Cache `__tostring` resolution at `setmetatable` time** (write a header field with the resolved fn ptr). Rejected — `mt.__tostring` can be mutated post-install; cache invalidation is worse than probing.
- **Trap on non-Function `__tostring`** (instead of falling back to "table"). Rejected — Lua spec doesn't trap; it silently uses the default representation. We match.
- **Bundle `print(t)` consumer in this ADR**. Rejected per ADR-per-decision; the print path uses a different code path (`emit_print_value_raw`) with its own tag-dispatch surface.

## Consequences

**Positive**
- The canonical Lua object-printing idiom works: `setmetatable(t, mt); mt.__tostring = function(self) return "..." end; print(tostring(t))` produces the user's string.
- Codegen helper `emit_dispatch_chain_from_slot_ptr` is reusable for future `__call` / `__concat` / `__index = Function` ADRs — they all need the same slot-ptr-to-direct-call shape.

**Negative**
- Every `tostring(t)` call site now pays a metatable null-check + (when mt present) a hash probe for "__tostring". Negligible — single cache line in the common case.
- Candidate set is the union of all `(Table) → String` user fns in the module. Code size grows with the dispatch chain length. Acceptable at current scale.

**Locked in until superseded**
- Function-form `__tostring` only. String-form (Lua spec allows non-Function `__tostring` to be returned directly) is deferred.
- Returns "table" literal on missing / non-Function `__tostring`. The richer Lua representation (`table: 0x<addr>`) is out of scope.

## Documentation updates

- [x] §1–§3 — **no change**.
- [x] §4 LIC consolidation — new resolved entry `LIC-tostring-metamethod-1`.
- [x] §7 open questions — closes `__tostring` open item; opens `__tostring` for `print(t)` / `..` concat as new follow-ups.
- [x] §8 ADR index — adds 0142.

## Test count delta

```
Step 0:   1315 (after ADR 0141)
C2 (5 e2e Red Day 0):  1315 → 1315
C3 (impl): 1315 → 1320
```

## Critical files

- `src/hir/mod.rs` — `coerce_to_string` arm note (Table now flows through `tostring` builtin, but `..` still rejects until a future ADR; no functional HIR change required since `Builtin::ToString` already permits any kind).
- `src/codegen/emit.rs`:
  - `emit_tostring` Table arm replaced (~80 LOC).
  - `emit_dispatch_chain_from_slot_ptr` new helper extracted from `emit_indirect_dispatch_chain_in_block` (~60 LOC moved, ~10 LOC delta).
- `tests/phase2_6plus_tostring_metamethod.rs` (NEW) — 5 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Empty candidate set (no `(Table) → String` user fns) crashes the dispatch chain | Skip the dispatch entirely when candidates empty — fall back to "table" literal. Test 5 pins. |
| `emit_dispatch_chain_from_slot_ptr` extract regresses ADR 0082 `Callee::IndirectDispatch` callers | 1:1 code move; ADR 0082's 15 tests + ADR 0091's 6 tests + ADR 0092's 7 tests stay green as the regression net. |
| `__tostring` returns a non-String result at runtime | Compile-time candidate filter `(Table) → String` restricts; runtime payload returning non-String is structurally impossible (the called fn returns a String ptr by construction). |
| Empty `mt["__tostring"]` field (Nil) silently traps via TAG_NIL ≠ TAG_FUNCTION fallback to "table" | Intended behaviour. Test 4 pins. |
| Metatable cycle through `__tostring` (recursion) | Lua spec doesn't require detection. Our compile-time candidate set means recursion is just normal function-call recursion at runtime. Stack overflow on infinite recursion — same as ordinary user code. |

## Future work

- `print(t)` Table arm should consult `__tostring` the same way (likely shares the helper).
- `..` concat Table operand — HIR `coerce_to_string` widens via the new path.
- Non-Function `__tostring` value (Lua spec String-form). Trivial extension once needed.
- TaggedValue runtime-tag Table dispatch for `tostring`.
- Richer default representation `table: 0x<addr>`.

## References

- [ADR 0026](0026-phase2-7c-tostring.md) — original `tostring` builtin.
- [ADR 0067](0067-phase2-6c-tag-consumers.md) — TaggedValue consumer dispatch surface.
- [ADR 0082](0082-phase2-5x-callee-dispatch.md) — `Callee::IndirectDispatch` chain; helper extract source.
- [ADR 0088](0088-phase2-6b-hash-lookup-miss.md) — `emit_hash_lookup_into_tagged_slot` reused.
- [ADR 0134](0134-metatables-index-read.md) — `TABLE_OFF_METATABLE` offset and metatable null fast-path precedent.
- [ADR 0141](0141-anon-fn-indexassign-param-refine.md) — anonymous FunctionExpr param-kind refinement (enables the natural idiom that this ADR consumes).
- Lua 5.4 reference manual §6.1 — `tostring` / `__tostring` semantics.
