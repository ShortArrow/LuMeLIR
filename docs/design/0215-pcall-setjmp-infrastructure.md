# 0215. `pcall` / `error` setjmp/longjmp Infrastructure

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-20
- **Deciders:** ShortArrow

## Context

First M2 sub-ADR (per [roadmap-status-2026-06-20.md](../notes/roadmap-status-2026-06-20.md) milestone M2 = pcall / error propagation). [ADR 0153](0153-pcall-error-strategy.md) decided the strategy: Phase 3 wires setjmp/longjmp as the runtime mechanism. This ADR lands the **infrastructure half** (libc externs, jmpbuf storage, error-value slot, chunk-level landing pad) and rewires `error(msg)` to route through it. The follow-up ADR 0216 adds `Builtin::Pcall` consuming this foundation.

Splitting infrastructure from `pcall` keeps both deltas reviewable and lets the chunk-level landing pad land green against the existing 6 `error()` tests (zero regression net) before `pcall`'s multi-return ABI complexity is introduced.

## Scope (literal)

- ✅ libc externs `_setjmp(ptr) -> i32` and `longjmp(ptr, i32) -> void` declared in `emit_module_unverified`.
- ✅ Mutable globals: `g_jmpbuf` (512-byte zero-initialised byte array — cross-libc-safe for glibc / musl / macOS jmp_buf sizes), `g_error_value` (i64 holding the boxed-string-object ptr, default 0).
- ✅ `emit_main` wraps the chunk body in a `_setjmp(g_jmpbuf)` landing pad. setjmp result == 0 → run body; nonzero → load `g_error_value`, call `emit_exit_with_message` (preserves ADR 0033 user-visible `print msg + exit 1` contract).
- ✅ `Builtin::Error` emit arm: store msg-ptr to `g_error_value`, `longjmp(g_jmpbuf, 1)`. Placeholder f64 0.0 satisfies expression-position contract — LLVM eliminates it as dead (longjmp is noreturn).
- ✅ Helper `emit_mutable_byte_array_global` for the jmpbuf storage; sibling of `emit_mutable_i64_global`.
- ❌ `Builtin::Pcall` HIR variant. ADR 0216 scope.
- ❌ Multi-return `pcall` ABI (`local ok, err = pcall(f)`). ADR 0216 scope (composes with ADR 0021).
- ❌ `xpcall(f, msgh, ...)`. Future M2 sub-ADR.
- ❌ Table-form error values (`error({code=42, msg="..."})`). Future — depends on `__tostring` chain (ADR 0142) integration.
- ❌ Diverting internal traps (tag-mismatch, OOB, hash-miss) through longjmp. Per ADR 0153 §"String only at Phase 3 first cut": only user-level `error(msg)` routes through. The 17 `emit_exit_with_message` trap call sites stay direct-exit (Lua spec: pcall catches all errors; this is a documented gap to be lifted by a future ADR if real demand emerges).
- ❌ Coroutine TLS jmpbuf. Process-global single slot per ADR 0153 §"Locked in until superseded".
- ❌ `returns_twice` attribute on `_setjmp`. At O0 codegen LLVM is conservative enough; future ADR can attach it when optimisation level rises.

## Decision

### Libc symbols

`_setjmp` (BSD-derived; on glibc + musl it skips signal-mask save vs the `setjmp` macro which calls `__sigsetjmp(env, 1)`) gives a smaller jmpbuf and matches Lua's no-signal-mask semantics. Paired with `longjmp` (POSIX-mandated real symbol). Both are exposed as functions, not macros, on every target libc — direct `llvm.call` resolution works at link time.

`g_jmpbuf` sized at 512 bytes: glibc x86_64 `jmp_buf` is ~200B, musl 304B, macOS 192B. 512B is safely larger than all known libc layouts, with negligible BSS cost.

### Error-value layout (Phase 3 first cut)

`g_error_value` is a mutable `i64` slot. `error(msg)` stores the boxed string-object pointer via `llvm.store`; the landing pad loads it as `!llvm.ptr` (opaque-pointer mode: 8-byte load via either type works). Future Table-form expansion swaps this for a 16-byte TaggedValue slot.

### Chunk-level landing pad shape

```mlir
llvm.func @main() -> i64 {
  // slot allocas first (stack offsets fixed)
  %slot_0 = llvm.alloca ...
  ...
  // setjmp captures SP / regs AFTER allocas
  %jmpbuf = llvm.mlir.addressof @g_jmpbuf
  %ret = llvm.call @_setjmp(%jmpbuf) : (!llvm.ptr) -> i32
  %is_err = arith.cmpi ne, %ret, %zero_i32
  scf.if %is_err {
    %err_addr = llvm.mlir.addressof @g_error_value
    %err_ptr = llvm.load %err_addr : !llvm.ptr
    // prints msg + exit 1 (same as ADR 0033)
    call emit_exit_with_message(%err_ptr)
  }
  // normal chunk body (only reached when setjmp returned 0)
  ...
  llvm.return %zero_i64
}
```

Slot allocas precede setjmp so they are part of the captured SP frame; on longjmp SP rewinds to the post-prolog state and the allocas remain addressable. The chunk body's allocas (e.g. inline tagged tmps inside scopes) are lower than SP-at-setjmp; longjmp invalidates them, which is correct — execution does not reach them on the error path.

## Tests

Zero new tests. The existing `tests/phase2_7h_error.rs` (6 e2e: literal / skip-statements / Local-var msg / concat-expr msg / nested-frame propagation / after-assert) is the regression net — each test compiles a program that calls `error(...)` and asserts exit code 1 + stdout contains the message. All 6 pass through the new longjmp path verbatim, proving:

- setjmp landing pad fires on every `error()` call site.
- Stashed `g_error_value` reaches the pad unchanged.
- Multi-frame error propagation (`error()` inside a user fn called by main) works — longjmp unwinds Lua call-stack frames implicitly.
- Stmts after `error()` are unreachable (verified by `error_skips_following_statements`).

ADR 0216 will add Pcall-specific e2e (catching the longjmp non-fatally) — the green-on-existing state here is the precondition.

## Test count delta

```
Step 0:  1478 (after ADR 0214)
C1 (plumbing-only refactor): 1478 → 1478 (zero regression)
```

## References

- [ADR 0033](0033-phase2-7h-error.md) — current `error(msg)` direct-exit; behaviour preserved by the landing pad.
- [ADR 0153](0153-pcall-error-strategy.md) — strategy decision; this ADR is Phase 3 first cut.
- [ADR 0021](0021-phase2-5d-multi-return.md) — multi-return ABI (used by ADR 0216 Pcall).
- [Lua 5.4 §6.1 `pcall` / `error`](https://www.lua.org/manual/5.4/manual.html#pdf-pcall)
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M2 milestone.
