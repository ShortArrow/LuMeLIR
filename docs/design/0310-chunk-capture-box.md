# 0310. Chunk-level captured locals get heap boxes (N9-E)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-07-09
- **Deciders:** ShortArrow

## The bug (first catch of the ADR 0308 arm64 CI lane)

`tests/phase2_5c2_kind_captures.rs` — `bool_upvalue_*` / `nil_upvalue_*` crashed (exit ≠ 0) on aarch64 only.

Root cause was **not** an aarch64 ABI subtlety but a latent stack-buffer overflow visible in the LLVM IR:

```llvm
%2 = alloca i1, align 1        ; captured Bool local `b`
...
store i64 1, ptr %2            ; 8-byte box-store into a 1-byte slot
```

ADR 0083's fn-body Step 6 replaces a captured local's alloca slot with a heap upvalue box ptr — but the **chunk main's slot map never did the equivalent**. Chunk-level captured locals kept their plain kind-typed alloca while:

- `LocalInit`/`Assign` stores routed through `emit_store_upvalue_box_value` (widened 8-byte write), and
- the closure-cell population passed the raw alloca ptr as if it were a box ptr (the inner fn then read 8 bytes through it).

For String/Table/Number captures the alloca is already 8 bytes, so the write "fit" and everything appeared to work. For Bool/Nil (1-byte `i1` alloca) the store smashed 7 bytes of adjacent stack — silently tolerated by the x86-64 frame layout, fatal on aarch64. A latent memory-safety bug on **both** architectures; arm64 merely surfaced it.

## Fix

The chunk slot map (`emit.rs`) mirrors fn Step 6: locals with `is_captured` (non-Function / non-TaggedValue kinds) allocate a heap upvalue box via `emit_allocate_upvalue_box` instead of a stack alloca. All existing consumers already treat captured slots as box ptrs, so the one-site change re-establishes the invariant everywhere.

## Verification

- LLVM IR after fix: the captured Bool's storage is a `gc_alloc`'d 8-byte box; the widened `store i64` targets the box.
- Suite green locally (x86-64); arm64 CI lane verification pending the next push (this commit).

## Follow-up

- Once the arm64 lane is green: flip `continue-on-error: false` in `ci.yml` → closes N9-B.

## References

- ADR 0308 — the arm64 lane that caught this.
- ADR 0083 — fn-body Step 6 (the invariant the chunk was missing).
- ADR 0043 — Bool/Nil/String upvalue support (where the latent bug was introduced).
