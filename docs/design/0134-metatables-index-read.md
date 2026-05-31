# 0134. Metatables `__index` Read-Path Only (Table-form)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

[ADR 0048](0048-phase2-0a-auto-declare-globals.md:23-24) flagged metatables as a Phase 2.6+ prerequisite for true `_ENV` and dynamic typing. [ADR 0058](0058-phase2-6b-hash-keys.md:217) and [ADR 0060](0060-phase2-6c-tag-hash.md:228) deferred `__index`/`__newindex` as "far out of scope". With Phase 2 winding down (per [ADR 0133](0133-phase2-completion-criteria.md)), the read-side metamethod is the smallest unit that can be carved out without dragging in the call ABI (Function-form `__index`), the write side (`__newindex`), or arithmetic / comparison metamethods.

`src/hir/mod.rs:44, 400, 1731, 4804` and `src/codegen/emit.rs:4804` carry comments anticipating metatable dispatch. This ADR turns those anticipations into a concrete read-path contract.

## Decision

### Scope (literal)

**`__index` read-path only**, **Table form only**. Everything else is explicitly out of scope and lands as a separate ADR per [ADR 0133](0133-phase2-completion-criteria.md):

- ❌ `__newindex` (write-path).
- ❌ `__index = Function` (call ABI integration deferred).
- ❌ `__add` / `__sub` / `__mul` / `__div` / `__mod` / `__pow` / `__unm` / `__idiv` / bitwise.
- ❌ `__eq` / `__lt` / `__le`.
- ❌ `__tostring` / `__concat`.
- ❌ `__call`.
- ❌ `setmetatable(t, nil)` clear semantics — HIR rejects `nil` second arg.
- ❌ `__metatable` field hiding.
- ❌ Shared metatable across multiple tables (GC interaction).

### Table header ABI

`TABLE_HEADER_SIZE` grows **32 → 40 bytes**:

| Offset | Field | Notes |
|---|---|---|
| 0 | `length: i64` | unchanged |
| 8 | `capacity: i64` | unchanged |
| 16 | `array_buf: !llvm.ptr` | unchanged |
| 24 | `hash_buf: !llvm.ptr` | unchanged |
| **32** | **`metatable_ptr: !llvm.ptr`** | **NEW; null on construct** |

All table constructor sites (`{}`, `{e1, e2, ...}`, `{k = v, ...}`) allocate 40 bytes and initialize offset 32 to null.

### New HIR `Builtin` variants

- `Builtin::SetMetatable` — arity 2, `param_kinds_for_arity = [Table, Table]`, ret = `Table` (returns `t` per Lua §6.1).
- `Builtin::GetMetatable` — arity 1, `[Table]`, ret = `TaggedValue` (Table or Nil).

Both are global builtins (not `metatable.*` namespace); `lower_call` dispatches on bare-Ident `setmetatable` / `getmetatable` when not shadowed.

### Runtime helpers (codegen)

- `emit_setmetatable_runtime(t, mt) -> t_ptr` — stores `mt` at offset 32 of `t`. Returns the original `t` per Lua §6.1.
- `emit_getmetatable_runtime(t) -> tagged_slot` — loads offset 32; null → Nil-tagged; non-null → Table-tagged.
- **`emit_check_metatable_index(t, key, out_slot, depth)` (the chokepoint)**:
  1. Load `mt = *(t + 32)`.
  2. If `mt` is null → write Nil to `out_slot`.
  3. Else: hash-lookup `mt["__index"]` via `emit_hash_lookup_into_tagged_slot` (ADR 0088 reuse).
  4. If the slot tag is `TAG_NIL` → write Nil to `out_slot`.
  5. If the slot tag is `TAG_TABLE` → recurse on the new table for the same `key` with `depth + 1`.
  6. Any other tag (Function, Number, etc.) → trap `s_metatable_index_unsupported_kind`.
  7. If `depth > 2000` → trap `s_metatable_chain_too_deep` (Lua reference limit).

### Index miss-path integration

The helper is called from every site that previously wrote a Nil-tagged result on miss:

- **Hash miss path**: `emit_hash_lookup_into_tagged_slot` `HashLookupOutcome::NilOnMissing` branch.
- **Array OOB path**: `src/codegen/emit.rs:7117+` bounds-check fall-through.
- **TaggedValue Index widening**: `emit_local_init_tagged` inherits the dispatch via the hash chokepoint; no per-site change.

`HashLookupOutcome::TrapMissing` branch is **unchanged** — that's the arith / cmp context from ADR 0088, where missing key is a type-mismatch.

## Alternatives considered

- **`__index = Function` in the same ADR.** Rejected — call ABI integration is a separate decision (varargs, return-kind widening, recursion budget). Belongs in its own ADR per [ADR 0133](0133-phase2-completion-criteria.md).
- **Both read and write paths in one ADR.** Rejected — write path needs `__newindex` semantics, rawset hatch, and decides whether tables become aliased (GC pressure). Independent design surface.
- **Lazy table-header migration (some tables 32-byte, some 40-byte).** Rejected — branchy header reads, cache-hostile, fragile under `emit_table_grow_if_needed`. Atomic ABI shift is cheaper.
- **Static `__index` resolution at HIR time when both literals are known.** Rejected — most metatable patterns build the metatable at runtime; static dispatch saves nothing in real code.
- **Depth cap lower than 2000.** Rejected — Lua reference uses 2000; deviating breaks legitimate deep delegation chains.

## Consequences

**Positive**
- The biggest single Phase 2 dependency hub closes. `_ENV` (ADR 0048), `__call` (`hir/mod.rs:1731`), `__tostring` (`hir/mod.rs:399`), and downstream metamethod ADRs all build on this ABI.
- Single chokepoint (`emit_check_metatable_index`) keeps the dispatch logic auditable.
- ABI break is one-shot and protected by the existing 1260-test corpus.

**Negative**
- Every table now carries an extra 8 bytes regardless of whether it has a metatable. Acceptable: real Lua tables average ≥ 64 bytes; +8 is ≤ 12% overhead.
- `__index` chain walking adds a hot-path branch on every miss. For tables without a metatable (the common case), the cost is one null-check load on `(t + 32)`.
- The Function-form rejection at runtime is a hard trap rather than HIR error — non-Function/non-Table `__index` storage can't be caught statically without typed table values.

**Locked in until superseded**
- Table header is 40 bytes; `metatable_ptr` is at offset 32. Future ADRs (e.g. weak-key flags, GC mark bits) take offset 40+.
- `__index` chain depth cap = 2000.

## Documentation updates

[`docs/design/tagged-semantics.md`](tagged-semantics.md) is the SoT for the Phase 2.6c TaggedValue runtime model (ADR 0068). This ADR touches:

- [x] §1 slot layout — **no change** (16-byte TaggedValue slot unchanged; metatable lives in the table header, not in a payload).
- [x] §2 producer / source taxonomy — adds `metatable.__index` chain lookup as a new TaggedValue producer.
- [x] §3 consumer coverage matrix — Index consumer rows gain an `__index dispatch` annotation.
- [x] §4 LIC consolidation — new resolved entry `LIC-metatable-index-read-1`.
- [x] §5 runtime tag invariants — **no change**.
- [x] §7 open questions — moves `__index Table dispatch` from open to resolved; adds `__index = Function`, `__newindex`, arith/cmp metamethods as new open items.
- [x] §8 ADR index — adds 0133, 0134 to the chronological table.

## Test count delta

```
Step 0:    1260 + 4 audit = 1264 (現状)
Commit C3 (header 32 → 40 ABI): 1264 → 1264 (behaviour unchanged)
Commit C4 (12 new e2e Red Day 0): 1264 → 1264 (existing green; 12 new red)
Commit C5 (setmetatable / getmetatable builtins): 4 of 12 go Green
Commit C6 (__index dispatch impl): all 12 Green → 1276
```

## Critical files

- `src/codegen/emit.rs` — `TABLE_HEADER_SIZE`, constructor sites, helpers, miss-path call-outs.
- `src/hir/mod.rs` — `Builtin::SetMetatable` / `GetMetatable` variants + dispatch.
- `tests/phase2_6plus_metatables_index_read.rs` (NEW) — 12 e2e tests.
- `docs/design/tagged-semantics.md` — §2 / §3 / §4 / §6 / §7 / §8 updates.

## Risks

| Risk | Mitigation |
|---|---|
| Header 32→40 silently breaks the 25 `phase2_6*` test files | Commit C3 lands the ABI alone with no impl; all 1264 must stay green before C4. |
| `__index` chain cycle → infinite recursion | Depth 2000 cap + dedicated Red Day 0 test. |
| Arithmetic / comparison metamethods scope-creep into this ADR | Title says "read-path only"; Non-goals enumerated explicitly. |
| `emit_check_metatable_index` melior eager-region build issue | Precedent: `emit_hash_probe_loop` / `emit_hash_grow_if_needed` use the same `scf.while` nested in `scf.if` pattern successfully. |
| Array path OOB fallback regresses existing nil-on-OOB tests | Existing OOB is nil-on-OOB (not trap); helper preserves nil result when `metatable_ptr` is null. |

## Future work

- ADR for `__newindex` write-path (next). **RESOLVED by [ADR 0135](0135-metatables-newindex-write.md) (2026-05-31)** for the hash-key Table form. Function-form `__newindex`, Number-key (array) `__newindex`, and `rawset` builtin remain deferred per ADR 0133.
- ADR for `__index = Function` form (call-ABI integration).
- Per-op ADRs for arithmetic / comparison / `__tostring` / `__concat` / `__call` metamethods.
- `setmetatable(t, nil)` clear semantics.
- `__metatable` field hiding.
- Shared metatable + GC interaction (ties into the GC strategy ADR per [ADR 0133](0133-phase2-completion-criteria.md)).

## References

- [ADR 0048](0048-phase2-0a-auto-declare-globals.md) — original metatable prerequisite note (this ADR partially resolves the read direction).
- [ADR 0058](0058-phase2-6b-hash-keys.md), [ADR 0060](0060-phase2-6c-tag-hash.md) — earlier `__index` deferrals (this ADR closes the read side).
- [ADR 0068](0068-phase2-6c-tag-doc-consolidate.md) — TaggedValue SoT updated per §Documentation updates above.
- [ADR 0073](0073-phase2-6c-tag-rs-split.md) — module layering (helper stays in `emit.rs`, not `tagged.rs`).
- [ADR 0088](0088-phase2-6b-hash-lookup-miss.md) — `HashLookupOutcome::NilOnMissing` chokepoint reused as the `__index` hook.
- [ADR 0114](0114-phase2-emit-f2i-gate-sweep.md) — `emit_trap_if` pattern reused for depth / unsupported-kind traps.
- [ADR 0129](0129-phase-tag-convention.md) — this ADR carries phase tag `2.6+-metatables-index-read`.
- [ADR 0133](0133-phase2-completion-criteria.md) — scope freeze that motivates the literal "read-path only" boundary.
