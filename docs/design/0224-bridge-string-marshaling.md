# 0224. Bridge String → Number Marshaling — `rust.strlen`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M5 sub-ADR. [ADR 0191](0191-rust-lua-bridge-mvp.md) landed the Rust-Lua Bridge MVP with `rust.add(a: Number, b: Number) -> Number` as the only signature. M5 widens the marshaling envelope. The natural next direction is String — Lua's most common non-Number value type and the only one whose layout (ADR 0112 boxed-string-object) can cross the C ABI as a single pointer without allocator coordination.

This ADR adds `rust.strlen(s: String) -> Number`. The Rust side reads the i64 length header at offset 0 of the boxed-string-object payload — the same byte the existing `string.len` builtin reads — and returns it as f64. No new allocation, no ownership transfer, no lifetime question.

## Scope (literal)

- ✅ New `Builtin::RustStrlen` HIR variant. `rust_from_method("strlen") → Some(RustStrlen)`. Name `"rust.strlen"`, arity `(1, 1)`, `ret_kinds = [Number]`, `param_kinds_for_arity = [String]`. `infer_kind → Number`.
- ✅ Bridge runtime: `extern "C" fn rust_strlen(s_ptr: *const u8) -> f64`. Reads `core::ptr::read_unaligned(s_ptr as *const i64)`, casts to f64.
- ✅ Codegen extern declaration alongside `rust_add`: `llvm.func @rust_strlen(!llvm.ptr) -> f64`.
- ✅ `Callee::Builtin(Builtin::RustStrlen)` emit arm: emits the String arg expression (which lowers to the boxed-object ptr per ADR 0112) and calls via `emit_libc_call_f64`.
- ❌ String → String marshaling (Rust function returning a Lua string). Allocator coordination — the Rust side would need to call `emit_string_obj_alloc` equivalents from no_std. Future ADR.
- ❌ Bool / Nil arg or return. Future M5 sub-ADR.
- ❌ Multiple String args (e.g. `rust.compare(a, b) -> Number`). Mechanical extension; future ADR.
- ❌ Error propagation across the boundary. ADR 0191 §"Error propagation" deferred contract still active.
- ❌ Non-`#![no_std]` Rust dependencies in the bridge runtime. Bridge stays leaf-call-only.
- ❌ User-defined bridge crates. Surface remains `src/bridge_runtime.rs` only.

## Decision

### Layout invariant

The boxed-string-object layout (ADR 0112): `[i64 len_le | data[len] | i8 NUL]`. The user-visible pointer points at `len_le`. Both the existing `string.len` builtin and `rust_strlen` read the same 8 bytes.

### Rust side

```rust
#[unsafe(no_mangle)]
pub extern "C" fn rust_strlen(s_ptr: *const u8) -> f64 {
    let len_i64 = unsafe { core::ptr::read_unaligned(s_ptr.cast::<i64>()) };
    len_i64 as f64
}
```

`read_unaligned` is defensive — the boxed-string-object's i64 header is in practice 8-byte aligned, but the no_std bridge cannot rely on platform-specific alignment guarantees. Cost difference is negligible.

### Marshaling shape

The Lua value layer holds Strings as `!llvm.ptr` (ADR 0112), and `param_mlir_type(ValueKind::String) = types.ptr`. The MLIR emit arm passes that ptr directly to `rust_strlen` — no payload transformation. The Rust ABI accepts the ptr, reads the length byte-aligned, returns f64. The Lua side receives an f64 — the same as any other Number return.

This is the cleanest possible String marshaling shape because the ADR 0112 layout already exposes the i64 length at a fixed offset; the Rust function reads the byte the Lua type would have read anyway.

## Tests

`tests/phase4_bridge_strlen.rs` (NEW, 4 e2e):

1. Basic: `print(rust.strlen("hello"))` → `5`.
2. Empty string: `print(rust.strlen(""))` → `0`.
3. Cross-check against `string.len`: same answer for the same input.
4. Concat result: `local s = "abc" .. "defgh"; print(rust.strlen(s))` → `8`. Proves heap-allocated strings (via the GC allocator) work the same as static literals.

## Test count delta

```
Step 0:  1506 (after ADR 0223)
C3 (impl + 4 e2e): 1506 → 1510
```

## References

- [ADR 0191](0191-rust-lua-bridge-mvp.md) — Bridge MVP with `rust.add` only; this ADR extends.
- [ADR 0112](0112-string-abi-refactor.md) — boxed-string-object layout (the `i64 len_le` header that `rust_strlen` reads).
- [ADR 0103](0103-phase2-7q-stdlib-string.md) — `string.len` precedent reading the same byte.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M5 milestone.
