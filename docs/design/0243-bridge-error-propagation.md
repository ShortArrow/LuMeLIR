# 0243. Bridge Error Propagation — `rust.fail(msg)` via setjmp/longjmp

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M12 sub-ADR. ADR 0191 §"Error propagation" deferred Rust → Lua error surfacing as a Bridge sub-piece blocked on `pcall`. M2 (ADRs 0215-0217) shipped the setjmp/longjmp infrastructure + `pcall(f)` single/multi-return. With both pieces in place, Rust → Lua error propagation is now buildable: a Rust extern can raise a Lua-side error that the chunk-level pad catches (uncaught → exit 1 + msg) or `pcall` catches (returns `(false, msg)`).

This ADR adds `rust.fail(msg)` as the canonical hook. The Rust side delegates to a codegen-emitted `lumelir_raise_error(msg_ptr)` wrapper rather than touching `g_error_value` / `g_jmpbuf` directly — those globals have Internal linkage in the lumelir module and aren't visible to the bridge object at link time.

## Scope (literal)

- ✅ New `Builtin::RustFail` HIR variant. `rust_from_method("fail") → Some(RustFail)`. Name `"rust.fail"`, arity `(1, 1)`, `ret_kinds = [Number]` (placeholder; the call is noreturn), `param_kinds = [String]`, `infer_kind → Number`.
- ✅ Bridge runtime: `extern "C" fn rust_fail(msg_ptr: *const u8) -> !` delegates to `lumelir_raise_error(msg_ptr)` via a Rust `extern { fn ... }` block.
- ✅ Codegen emits `lumelir_raise_error(msg_ptr: ptr)` with External linkage. Body stashes `msg_ptr` into `g_error_value` + longjmps to `g_jmpbuf` + emits `llvm.unreachable`.
- ✅ Codegen extern declaration for `rust_fail(ptr) -> void`.
- ✅ Emit arm: lower the msg arg, call `rust_fail`, yield placeholder `f64 0.0` (LLVM eliminates as dead after the noreturn call).
- ✅ Composes with `pcall` (ADRs 0216 / 0217): a caught `rust.fail` returns `(false, msg)` through the multi-return path.
- ❌ Rust panic → Lua error. Bridge runtime is `#![no_std]` with a `loop {}` panic handler; integrating a real panic-to-Lua-error path requires the bridge to opt into a slightly fatter runtime (or a `catch_unwind` wrapper). Future sub-ADR.
- ❌ Error-value layout for Table or other tagged kinds. `g_error_value` is still a single ptr (currently boxed-string). Widening to a TaggedValue 16-byte slot is future M2-stretch.
- ❌ `panic_handler` integration that calls `rust_fail`. The current handler is `loop {}`; switching to `rust_fail` would emit panic messages but tangles the static no_std story. Out of scope.
- ❌ Aborting from inside `pcall` (raise a non-catchable error). Future micro-extension.

## Decision

### Why route through `lumelir_raise_error` not direct symbol access

`g_error_value` (i64 mutable global) and `g_jmpbuf` (512-byte mutable array) both use `Linkage::Internal`. Direct Rust extern access fails at link time:

```
undefined reference to `g_error_value'
undefined reference to `g_jmpbuf'
```

Two clean options:

1. Switch the globals to External linkage and expose them to the bridge.
2. Emit a wrapper function with External linkage that the bridge calls.

Option 2 wins: keeps the global-shape choices internal to lumelir codegen (we can swap `g_error_value` for a TaggedValue slot later without touching the bridge ABI), and matches how libc `setjmp` / `longjmp` already route through a helper boundary.

### Wrapper function shape

```mlir
llvm.func @lumelir_raise_error(%arg0: !llvm.ptr) {
  %err_addr = llvm.mlir.addressof @g_error_value
  llvm.store %arg0, %err_addr
  %jmpbuf = llvm.mlir.addressof @g_jmpbuf
  llvm.call @longjmp(%jmpbuf, 1)
  llvm.unreachable
}
```

`llvm.unreachable` after the noreturn `longjmp` tells LLVM the function does not return.

### Why `rust_fail` accepts the boxed-string ptr directly

The user-visible String value already IS a `*const u8` pointing at the boxed-string-object header (ADR 0112). `error(msg)`'s codegen stores the same ptr into `g_error_value`. The chunk-level landing pad (ADR 0215 `emit_main`) loads `g_error_value` as `!llvm.ptr` and passes it to `emit_exit_with_message`, which interprets it as a boxed string. No format conversion needed.

## Tests

`tests/phase4_m12a_bridge_fail.rs` (NEW, 4 e2e):

1. `rust.fail("boom-from-rust")` uncaught → exit 1 + stdout contains the message.
2. `pcall(f)` where `f` calls `rust.fail("caught-from-rust")` → `(false, "caught-from-rust")`.
3. `if not ok then print("handled") end` over the caught case → `"handled"`.
4. After a caught fail, subsequent `rust.add(1, 2)` still returns `3` (state is clean).

## Test count delta

```
Step 0:  1599 (after ADR 0242)
C3 (impl + 4 e2e): 1599 → 1603
```

## References

- [ADR 0191](0191-rust-lua-bridge-mvp.md) — Bridge MVP; error propagation as Future work.
- [ADR 0215](0215-pcall-setjmp-infrastructure.md) — `g_error_value` / `g_jmpbuf` foundation.
- [ADR 0216](0216-pcall-builtin-single-return.md) — `pcall(f)` single-return.
- [ADR 0217](0217-pcall-multireturn-abi.md) — `pcall(f)` multi-return `(ok, err)`.
- [ADR 0112](0112-string-abi-refactor.md) — boxed-string-object layout the msg ptr carries.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M12 milestone.
