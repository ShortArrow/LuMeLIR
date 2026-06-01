# 0157. Phase 3 GC step 1 — Allocator Wrapper + String-Object Migration + `collectgarbage("count")`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

First implementation step from [ADR 0156](0156-gc-architecture-v1.md)'s roadmap. Introduces the `emit_gc_alloc` allocator wrapper and migrates **one** allocator type (ADR 0112 string-objects) so the design has a concrete first consumer. Mark / sweep / stack walk all stay deferred (ADRs 0159 / 0160 / 0161).

## Scope (literal)

- ✅ `g_gc_head` mutable ptr global (init null).
- ✅ `g_gc_total_bytes` mutable i64 global (init 0).
- ✅ `s_gc_oom` trap message global.
- ✅ `emit_gc_alloc(payload_size, type_tag) -> Value<ptr>` helper (returns `raw + 16`).
- ✅ Migrate `emit_string_obj_alloc` (single chokepoint for all string-object allocations) to use the new helper with `type_tag = GC_TYPE_STRING_OBJ`.
- ✅ `Builtin::CollectGarbage` HIR variant + Lua-builtin dispatch.
- ✅ `collectgarbage("count")` returns the tracked byte count / 1024 as Number.
- ✅ `collectgarbage()` returns 0 (no-op stub) — full GC arrives in ADR 0159 + 0161.

Out of scope:

- ❌ Mark phase (ADR 0159).
- ❌ Sweep phase (ADR 0161).
- ❌ Stack walk for alloca-slot roots (ADR 0160).
- ❌ Other allocator types (Table / HashBuf / ArrayBuf / ClosureCell / UpvalueBox / ScratchBuf) — ADR 0158.
- ❌ Auto trigger threshold (ADR 0162).
- ❌ `__gc` / weak tables (ADRs 0163 / 0164).
- ❌ Other `collectgarbage` options (`"stop"` / `"restart"` / `"step"` / `"setpause"` / `"setstepmul"`) — reject at HIR.

## Decision

### New module globals (registered in `emit_module_init`)

```
g_gc_head:        !llvm.ptr (mutable; init = null)
g_gc_total_bytes: i64       (mutable; init = 0)
s_gc_oom:         i8 array  (constant; "out of memory: GC alloc failed\0")
```

GC type-tag constants live in `tagged.rs` alongside `TAG_*`:

```rust
pub(crate) const GC_TYPE_TABLE:         u8 = 1;
pub(crate) const GC_TYPE_HASH_BUF:      u8 = 2;
pub(crate) const GC_TYPE_ARRAY_BUF:     u8 = 3;
pub(crate) const GC_TYPE_STRING_OBJ:    u8 = 4;
pub(crate) const GC_TYPE_CLOSURE_CELL:  u8 = 5;
pub(crate) const GC_TYPE_UPVALUE_BOX:   u8 = 6;
pub(crate) const GC_TYPE_SCRATCH_BUF:   u8 = 7;
pub(crate) const GC_HEADER_SIZE:        i64 = 16;
```

### `emit_gc_alloc` helper

In `src/codegen/primitive.rs`:

```rust
pub(crate) fn emit_gc_alloc<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    payload_size: Value<'c, 'a>,  // i64
    type_tag: u8,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a>
```

Body:
1. `total = payload_size + GC_HEADER_SIZE`.
2. `raw = malloc(total)`; null-check trap with `s_gc_oom`.
3. Store header bytes:
   - `*(raw + 0) = 0_u8` (mark = WHITE).
   - `*(raw + 1) = type_tag_u8`.
   - `*(raw + 2) = 0_u16` (padding).
   - `*(raw + 4) = trunc(payload_size, u32)`.
   - `*(raw + 8) = load(g_gc_head)`.
4. `store(raw → g_gc_head)`.
5. `g_gc_total_bytes += total`.
6. Return `raw + GC_HEADER_SIZE`.

### `emit_string_obj_alloc` migration

The current implementation calls `emit_alloc_with_oom_check(total_size, "s_alloc_oom")`. Replace with `emit_gc_alloc(total_size, GC_TYPE_STRING_OBJ)`.

The user-visible ptr stays at the same offset relative to the boxed object payload (`{i64 len, i8 data[len + 1]}`); the only change is that 16 bytes of header now live BEFORE the payload (invisible to callers).

### HIR

New `Builtin::CollectGarbage` variant in `src/hir/ir.rs`:
- `from_name("collectgarbage")` → `CollectGarbage`.
- `arity = (0, 1)`.
- `ret_kinds = [Number]`.
- `name = "collectgarbage"`.
- `param_kinds_for_arity` → `&[]` (per-arg check inline).

`lower_builtin_call` adds a `CollectGarbage` arm:
- 0 args: OK.
- 1 arg: must be `Str("count")`; other Str values → `TypeMismatch` ("unsupported collectgarbage option, expected \"count\"").

### Codegen

`Callee::Builtin(CollectGarbage)` emit arm in `src/codegen/emit.rs`:
- `args.len() == 0`: emit `arith.constant 0.0 : f64`.
- `args.len() == 1` (statically Str("count")):
  - Load `g_gc_total_bytes` as i64.
  - sitofp i64 → f64.
  - Divide by `1024.0` (`arith.divf`).
  - Return the f64.

## Alternatives considered

- **Add `type_tag` parameter to `emit_alloc_with_oom_check`** instead of new helper. Rejected — the existing helper is Lua-semantics-agnostic ("plain OOM-checked malloc"); the GC tracking is a separate concern.
- **Track only `total_bytes` (no linked list) in v1.** Rejected — the linked list is the gating data structure for sweep (ADR 0161); without it, future ADRs have no anchor.
- **`collectgarbage("count")` returns bytes (not KB).** Rejected — Lua spec says KB. Match.
- **Skip `collectgarbage` builtin in this ADR; add it with the mark phase.** Rejected — having the builtin (even as a stub) makes the v1 observable from Lua tests; otherwise the migration is invisible.

## Consequences

**Positive**
- Every string-object allocation now has a GC header. Future mark/sweep ADRs can walk the list immediately.
- `collectgarbage("count")` provides a user-visible knob that exercises the tracking machinery.
- ADR 0112's OOM-trap contract is preserved (s_gc_oom is the new trap; same semantics).

**Negative**
- 16-byte header per string object. For tiny strings (`""`, `"a"`), >100% overhead. Acceptable per ADR 0156.
- Header init adds ~6 stores per allocation. Minor.
- Other allocator types (Table / HashBuf / etc.) still leak without tracking until ADR 0158 — `collectgarbage("count")` undercounts the true heap until then.

**Locked in until superseded**
- GC header layout (ADR 0156 v1).
- `emit_gc_alloc` signature.
- `g_gc_head` global linked list.
- `collectgarbage("count")` returns KB.
- `collectgarbage()` no-op stub (until ADR 0159 + 0161 land mark/sweep).

## Documentation updates

- [x] §4 LIC — new `LIC-gc-allocator-wrapper-1`.
- [x] §8 — adds 0157.

## Test count delta

```
Step 0:   1366 (after ADR 0156)
C2 (3 e2e Red Day 0):  1366 → 1366
C3 (impl): 1366 → 1369
```

## Critical files

- `src/codegen/tagged.rs` — 7 GC type-tag constants + `GC_HEADER_SIZE`.
- `src/codegen/emit.rs`:
  - 3 new module globals (`g_gc_head`, `g_gc_total_bytes`, `s_gc_oom`).
  - `Callee::Builtin(CollectGarbage)` emit arm.
- `src/codegen/primitive.rs`:
  - `emit_gc_alloc` helper.
  - `emit_string_obj_alloc` migrated to call it.
- `src/hir/ir.rs` — `Builtin::CollectGarbage` variant + `from_name` / `arity` / `name` / `ret_kinds` / `param_kinds_for_arity` arms.
- `src/hir/mod.rs` — `CollectGarbage` HIR check + `infer_kind` arm.
- `tests/phase3_gc_allocator.rs` (NEW) — 3 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| 16-byte header offset corrupts existing string-object layout | The header lives BEFORE the user-visible ptr; payload offsets (`STRING_LEN = 0`, `STRING_DATA = 8`) stay correct. |
| `g_gc_head` mutable global break MLIR verification | Mutable globals work in `llvm.mlir.global` without the `constant` flag; precedent: existing init code patterns. |
| `collectgarbage("count")` returns 0 in trivial programs | Any program that allocates a single string (e.g. `tostring(1)` → boxed string) produces `count > 0`. Test pins. |
| `collectgarbage()` looks broken because nothing happens | Documented: stub until ADRs 0159 + 0161. Test 3 explicitly pins the 0-return contract. |

## Future work

- ADR 0158 — migrate remaining allocator types.
- ADR 0159 — mark phase.
- ADR 0160 — stack walk for alloca roots.
- ADR 0161 — sweep phase.
- ADR 0162 — automatic 1 MB trigger.
- `collectgarbage("stop")` / `"restart"` / `"step"` / `"setpause"` / `"setstepmul"` — separate ADR after the core GC lands.

## References

- [ADR 0112](0112-phase2-string-abi-refactor.md) — string-object allocation chokepoint (`emit_string_obj_alloc`) being migrated here.
- [ADR 0145](0145-gc-strategy.md) — high-level GC strategy.
- [ADR 0156](0156-gc-architecture-v1.md) — concrete data structures elaborated here.
- Lua 5.4 reference manual §6.1 — `collectgarbage`.
