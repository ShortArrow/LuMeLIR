# 0217. `pcall(f)` Multi-Return `(ok, err)` ABI

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-20
- **Deciders:** ShortArrow

## Context

Third (closing) M2 sub-ADR. [ADR 0216](0216-pcall-builtin-single-return.md) landed `pcall(f)` as a single-return Bool. Lua 5.4 §6.1 mandates the multi-return shape `local ok, err = pcall(f)` — the second result carries the caught error value (the `msg` passed to `error()`). Without this, the canonical idiom

```lua
local ok, err = pcall(f)
if not ok then
  print(err)
end
```

does not work. This ADR widens the Pcall ret_kinds to `[Bool, TaggedValue]` and adds a `emit_call_pcall_into_locals` sibling for the multi-assign path, composing with ADR 0021 / 0076.

## Scope (literal)

- ✅ `Builtin::Pcall::ret_kinds` widened to `&[ValueKind::Bool, ValueKind::TaggedValue]`. Position 0 = ok flag; position 1 = err value.
- ✅ `emit_multi_assign_from_builtin` dispatch arm for `Builtin::Pcall` → new `emit_call_pcall_into_locals` helper. Sibling of `emit_call_next_into_locals` (ADR 0081 precedent).
- ✅ Multi-return helper emits the same setjmp dance as the single-return arm; on success stores `(true_i1, Nil-tag)`; on caught error stores `(false_i1, String(g_error_value))`.
- ✅ Single-assign `local ok = pcall(f)` still works via the existing `Callee::Builtin(Pcall)` emit arm (ADR 0216) — truncates to position 0, identical to how `local k = next(t)` truncates from `(k, v)` (ADR 0081 precedent).
- ✅ Err value carries the actual `msg` string-object pointer captured from `error("msg")`; `print(err)` produces the original string.
- ❌ `error({code=N, msg="..."})` Table-form error values. `g_error_value` is still ptr-to-string-object only; Table-form requires the slot widening to 16-byte tagged value + `__tostring` chain integration. Future M-stretch sub-ADR.
- ❌ `pcall(f, arg1, arg2, ...)` arg forwarding to `f`. Function(0) restriction inherited from ADR 0216.
- ❌ `xpcall(f, msgh, ...)`. Sibling, future M-stretch.
- ❌ Diverting internal traps (tag-mismatch, OOB) through longjmp. Per ADR 0215.

## Decision

### HIR

```rust
// ir.rs
Builtin::Pcall => &[ValueKind::Bool, ValueKind::TaggedValue],
```

`infer_kind` for `Callee::Builtin(Pcall)` stays `ValueKind::Bool` (returns `ret_kinds.first()`-equivalent at single-result position).

### Codegen multi-assign dispatch

```rust
// emit_multi_assign_from_builtin
Builtin::Pcall => emit_call_pcall_into_locals(...)
```

### `emit_call_pcall_into_locals` shape

```mlir
// dst_ids = [ok_local, err_local]; args = [Function-Local]
%cell_ptr   = (same fast path as ADR 0216 — addressof @<mangled>_closure
              for non-capturing static FuncId; slot load otherwise)
%fn_ptr     = emit_load_closure_fn_ptr(cell_ptr)
%save_buf   = llvm.alloca i32 512 x i8
call @memcpy(%save_buf, @g_jmpbuf, 512)
%setjmp_ret = call @_setjmp(@g_jmpbuf) : i32
%is_init    = arith.cmpi eq, %setjmp_ret, 0
scf.if %is_init {
  call %fn_ptr(%cell_ptr)           // discard f64
  call @memcpy(@g_jmpbuf, %save_buf, 512)
  store true_i1, ok_slot
  emit_value_slot_store_nil(err_slot)
} else {
  call @memcpy(@g_jmpbuf, %save_buf, 512)
  store false_i1, ok_slot
  %err_ptr = llvm.load @g_error_value : !llvm.ptr
  emit_value_slot_store_string(err_slot, %err_ptr)
}
```

Both branches restore `g_jmpbuf` from the save buffer so any subsequent uncaught `error()` after pcall lands in the chunk-level pad (ADR 0215).

## Tests

`tests/phase4_pcall_multireturn.rs` (NEW, 3 e2e):

1. Success path: `pcall(ok_fn)` → `(true, nil)`. `print(ok); print(err)` → `"true\nnil"`.
2. Caught path: `pcall(bad)` where bad calls `error("oops")` → `(false, "oops")`. `print(ok); print(err)` → `"false\noops"`.
3. Idiom chain: `local ok, msg = pcall(bad); if not ok then print(msg) end` → `"hello"` — proves the err slot carries a properly boxed string-object that flows through into print.

The 4 e2e from ADR 0216 continue to pass through the single-return arm (truncation to position 0).

## Test count delta

```
Step 0:  1482 (after ADR 0216)
C3 (impl + 3 e2e): 1482 → 1485
```

## References

- [Lua 5.4 §6.1 `pcall`](https://www.lua.org/manual/5.4/manual.html#pdf-pcall)
- [ADR 0153](0153-pcall-error-strategy.md) — strategy.
- [ADR 0215](0215-pcall-setjmp-infrastructure.md) — setjmp/longjmp foundation.
- [ADR 0216](0216-pcall-builtin-single-return.md) — single-return Pcall.
- [ADR 0021](0021-phase2-5d-multi-return.md) — multi-return ABI.
- [ADR 0081](0081-phase2-8e-iter-next.md) — `next(t)` multi-return precedent reused by `emit_multi_assign_from_builtin` dispatch.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M2 milestone.
