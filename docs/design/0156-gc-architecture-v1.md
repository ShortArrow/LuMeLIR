# 0156. Phase 3 GC — Architecture v1 (Data Structures + Algorithm)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

[ADR 0145](0145-gc-strategy.md) pinned the high-level GC strategy: non-moving mark-and-sweep, Phase 2 = leak. With Phase 2 closed, Phase 3 implementation work begins. The implementation is M-L sized; this ADR carves the v1 architecture (data structures + algorithm + ABI) so subsequent commits can land as small concrete steps.

This ADR is **decision-only**. Implementation lands in follow-up ADRs (0157+) that each migrate one allocator site or wire one walker.

## Decision

### Object header

Every GC-managed allocation is prefixed with a 16-byte `GcHeader`:

```
offset 0  +-------------------+
          | u8 mark           |   0 = white (unreachable / fresh), 1 = grey (in worklist), 2 = black (reached)
offset 1  +-------------------+
          | u8 type_tag       |   one of GcType (below)
offset 2  +-------------------+
          | u16 padding       |   reserved (future weak-flag bits etc.)
offset 4  +-------------------+
          | u32 size          |   payload bytes (= total alloc - 16, used by sweep + telemetry)
offset 8  +-------------------+
          | ptr  next         |   next allocation in the global linked list
offset 16 +-------------------+
          | payload (varies)  |   the user-visible ptr is &header + 16
          ...
```

`GcType` discriminator values:

| value | name | payload layout owner |
|---|---|---|
| 1 | `Table` | ADR 0134 (40-byte header + array_buf + hash_buf) |
| 2 | `HashBuf` | ADR 0079 (cap + count + entries) |
| 3 | `ArrayBuf` | ADR 0059 (tagged 16-byte slots × cap) |
| 4 | `StringObj` | ADR 0112 (boxed string {len, data}) |
| 5 | `ClosureCell` | ADR 0083 (fn_ptr + upvalues) |
| 6 | `UpvalueBox` | ADR 0083 (kind-typed upvalue slot) |
| 7 | `ScratchBuf` | ADR 0112 string-alloc OOM consolidation buffer |
| ≥ 100 | reserved | future types |

### Global linked list

A single module-level `!llvm.ptr` global `g_gc_head` (initialized to null at process start) stores the head of the all-allocations linked list. Every `gc_alloc` prepends the new header to this list.

The list is **singly-linked, append-at-head, never reordered**. Walk order is reverse-allocation order (newest first), but the sweep phase doesn't depend on order.

### Allocator wrapper

A new helper `emit_gc_alloc(payload_size, type_tag) -> Value<ptr>` replaces every existing `malloc` call site that produces a GC-managed object. The helper inline-emits:

1. `total_size = payload_size + 16` (header size).
2. `raw = malloc(total_size)` — null check trap (`s_gc_oom`, new global).
3. `*(raw + 0) = 0_u8` (mark = white).
4. `*(raw + 1) = type_tag_u8`.
5. `*(raw + 2) = 0_u16` (padding).
6. `*(raw + 4) = payload_size_u32`.
7. `*(raw + 8) = g_gc_head` (next = current head).
8. `g_gc_head = raw` (push).
9. Return `raw + 16` (the payload ptr the existing callers expect).

For sites that today use `emit_alloc_with_oom_check` (ADR 0112), the helper is a thin wrapper that also calls `gc_alloc` internally — keeps the OOM-trap contract.

### Root set

The mark phase needs a complete root set. v1 scope:

| Root source | Mechanism |
|---|---|
| Module-level globals (string literals, fmt strings, typename strings) | Static — never freed; mark on init. |
| Top-level chunk locals (the main fn's alloca slots) | Walked via a `mark_alloca_roots` helper called from `collectgarbage` builtin (Lua-level) or runtime trigger (Phase 3 step 5). |
| Inner-function alloca slots | Each `func.func`'s slots are roots while that frame is live. The mark phase walks the **current call stack** via libc `setjmp` / Rust-side `backtrace` — TBD; deferred to implementation ADR with a documented stub. |
| Closure cell upvalue boxes | Reached via the closure cell's `upvalues[i]` field once the closure cell itself is marked. |

For v1 implementation **`mark_alloca_roots` is a no-op** — only module-level globals are walked. Local alloca slots are effectively immortal in v1 (matches current leak behaviour). v2 wires the stack walk.

### Mark phase

Iterative DFS starting from each root:

```
worklist: Vec<ptr> = roots
while !worklist.empty():
    obj = worklist.pop()
    if obj.mark == BLACK: continue
    obj.mark = BLACK
    match obj.type_tag:
        Table:
            push obj.array_buf
            push obj.hash_buf
            push obj.metatable_ptr (if non-null)
            // array element ptrs walked via array_buf's mark step
        HashBuf:
            for each occupied entry:
                if key.tag is reference kind (String/Function/Table): push key.payload
                if value.tag is reference kind: push value.payload
        ArrayBuf:
            for each non-Nil element:
                if elem.tag is reference kind: push elem.payload
        StringObj / ScratchBuf:
            // no outgoing refs; just mark
        ClosureCell:
            for each upvalue_box: push upvalue_box
        UpvalueBox:
            if box.kind is reference: push box.payload
```

Tag discrimination uses the existing `TAG_*` constants from `tagged.rs`.

### Sweep phase

Walk `g_gc_head` linked list. For each node:

- If `mark == BLACK`: reset to WHITE, keep in list.
- If `mark == WHITE`: unlink from list, `free(node)`.

The unlink mutates the prior node's `next` pointer; standard prev-cursor walk.

### `collectgarbage` builtin

Lua spec §6.1: `collectgarbage([opt [, arg]])`. v1 scope:

- `collectgarbage()` (no args) → run a full collection, return collected-byte count (Number).
- `collectgarbage("count")` → return current heap size in KB (Number).
- Other options reject at HIR (`BuiltinArgKindMismatch`).

HIR: new `Builtin::CollectGarbage` variant, arity `(0, 1)`.

### Trigger

v1: explicit `collectgarbage()` calls only. The 1 MB automatic trigger (ADR 0145) lands as v3 alongside the runtime-counter check on every `gc_alloc`.

## Alternatives considered

- **Inline allocator (no `gc_alloc` wrapper)**. Rejected — would require touching every malloc site individually for header init; helper is the obvious factor.
- **Per-type linked lists** instead of a single global. Rejected — sweep walks more memory but the single list keeps the invariant simple.
- **Mark-bit in the existing TaggedValue tag byte**. Rejected — tag byte is per-slot, not per-object; the object header is the right scope.
- **Stop-the-world only, no incremental.** Accepted for v1 per ADR 0145 ("stop-the-world initially").
- **Generational nursery.** Deferred per ADR 0145 ("may revisit after profiling").
- **Move heap-alloc to a per-stack-frame arena.** Rejected — would conflict with `__index` chain crossing scope boundaries.

## Consequences

**Positive**
- Concrete data structures pin the implementation surface for subsequent ADRs.
- The 16-byte header is small relative to typical Lua object sizes.
- `emit_gc_alloc` is a single chokepoint — easy to instrument with telemetry, easy to swap implementations later.

**Negative**
- 16 bytes per allocation overhead. For tiny strings this is a >100% overhead; acceptable but documented.
- v1 alloca-slot root set is a no-op — local Lua objects effectively leak until v2 wires the stack walk.
- Mark phase doesn't yet handle the call stack — current-frame locals are missed.

**Locked in until superseded**
- 16-byte header layout (mark / type_tag / padding / size / next).
- Single global linked list `g_gc_head`.
- Tri-color mark (WHITE / GREY / BLACK) — even though v1 doesn't use GREY (stop-the-world reaches it transiently in DFS).
- `emit_gc_alloc(size, type_tag) -> ptr` ABI.

## Documentation updates

- [x] §4 LIC — new `LIC-gc-architecture-v1-1`.
- [x] §7 — moves GC implementation from "Phase 3 future work" to "Phase 3 in progress".
- [x] §8 — adds 0156.

## Test count delta

```
Step 0:   1366 (after ADR 0155)
C1 (this ADR + SoT updates):  1366 → 1366 (decision-only)
```

## Critical files (when implementation ADRs land)

- `src/codegen/emit.rs`:
  - New module-level global `g_gc_head` (initialized to null).
  - New trap message `s_gc_oom`.
  - New helper `emit_gc_alloc(size: Value<i64>, type_tag: u8) -> Value<ptr>`.
  - Migration of every `malloc` site that produces a GC-managed object.
  - New `Callee::Builtin(CollectGarbage)` emit arm.
- `src/hir/ir.rs`: `Builtin::CollectGarbage` variant.
- `src/hir/mod.rs`: `from_name("collectgarbage")` dispatch.
- `docs/design/tagged-semantics.md` — §4 / §7 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| 16-byte header breaks the existing 40-byte table header offsets (ADR 0134) | The header lives BEFORE the existing payload; existing offset constants (`TABLE_OFF_LEN = 0` etc.) stay correct relative to the user-visible ptr. |
| Migrating every malloc site at once is high-risk | Split implementation into multiple small ADRs, one allocator type per commit. |
| Stack walk for local alloca slots is non-trivial | v1 explicit deferral; v2 implementation ADR owns the design. |
| `collectgarbage("count")` returns wrong value | v1 returns the running `total_bytes` counter (incremented in `emit_gc_alloc`, decremented in sweep). |

## Implementation roadmap

| ADR | Scope | Size |
|---|---|---|
| 0156 (this) | Architecture decision | XS doc |
| 0157 | `emit_gc_alloc` helper + `g_gc_head` global + migrate string-object allocs (ADR 0112 sites) + `collectgarbage("count")` builtin | M |
| 0158 | Migrate remaining allocator types (Table / HashBuf / ArrayBuf / ClosureCell / UpvalueBox / ScratchBuf) | M |
| 0159 | Mark phase (no stack walk yet — module-globals only) + `collectgarbage()` full call | M |
| 0160 | Stack walk for alloca-slot roots (call-stack mark) | L |
| 0161 | Sweep phase (free unreached) | M |
| 0162 | 1 MB automatic trigger + per-allocation counter | S |
| 0163 | `__gc` finaliser support | M |
| 0164 | Weak tables (`__mode`) | M |

## Future work

- v2 (call-stack root walk via libc `unw_*` or hand-coded backtrace).
- v3 (1 MB automatic trigger).
- v4 (`__gc` finaliser).
- v5 (weak tables).
- Generational nursery (only if profiling demands).
- Incremental marking (only if pause times demand).

## References

- [ADR 0083](0083-phase2-5c-full-closures.md) — closure cell + upvalue box layout.
- [ADR 0112](0112-phase2-string-abi-refactor.md) — boxed String object + OOM check.
- [ADR 0145](0145-gc-strategy.md) — high-level GC strategy decision this ADR elaborates.
- [ADR 0134](0134-metatables-index-read.md) — Table header layout (offsets relative to user-visible ptr).
- Lua 5.4 reference manual §2.5 — garbage collection.
