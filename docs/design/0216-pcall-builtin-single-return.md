# 0216. `Builtin::Pcall` Single-Return (Bool)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-20
- **Deciders:** ShortArrow

## Context

Second M2 sub-ADR. [ADR 0215](0215-pcall-setjmp-infrastructure.md) landed the setjmp/longjmp infrastructure: chunk-level landing pad, `g_jmpbuf` / `g_error_value` globals, and `error(msg) → longjmp` rewrite. This ADR adds `Builtin::Pcall` as the consumer — Lua's `pcall(f)` non-fatally catches `error()` calls inside `f`.

Scope is intentionally narrow to land Green in one session. The follow-up ADR 0217 widens the ABI to multi-return `(ok, err)` (composes with ADR 0021) and the `pcall(f, arg1, arg2, ...)` arg-forwarding shape.

## Scope (literal)

- ✅ HIR `Builtin::Pcall` variant. Name `"pcall"`, arity `(1, 1)`, ret_kinds `[Bool]`, included in `lower_builtin_call` per-arg validation block.
- ✅ HIR arg-kind check: `args[0]` must lower to `ValueKind::Function(0)` (no-arg callable). All other kinds reject as `TypeMismatch { op: "pcall" }`.
- ✅ HIR `FunctionUsedAsValue` allow-list extended: `Builtin::Pcall` joins `Type / ToString / Next` as a builtin permitted to receive a `ValueKind::Function(_)` arg.
- ✅ Codegen emit arm: stack-allocate a 512-byte save buffer, `memcpy(save_buf, g_jmpbuf, 512)`, nested `_setjmp(g_jmpbuf)`, branch on result; the success path calls `fn_ptr(cell_ptr) -> f64` (return value discarded), restores `g_jmpbuf` via reverse memcpy, yields `i1 true`; the landing pad restores `g_jmpbuf` and yields `i1 false`.
- ✅ Static-FuncId fast path: when `args[0]` is a Local with `LocalInfo::func_id == Some(fid)` and the target user fn is non-capturing (`functions[fid].upvalues.is_empty()`), cell_ptr is `addressof @<mangled>_closure` instead of a slot load. Required because the LocalInit alias-skip rule (`emit.rs:3418`) never stores into such slots.
- ❌ Multi-return `(ok, err)` ABI. ADR 0217 scope (composes with ADR 0021).
- ❌ `pcall(f, arg1, arg2, ...)` arg forwarding to `f`. Function(0) restriction only.
- ❌ Inline lambda arg (`pcall(function() ... end)`). User wraps in `local f = function() ... end` first. HIR layer rejection — the FunctionRef arg shape is reachable but the codegen requires a Local source for the static-FuncId fast path.
- ❌ Captured-upvalue Function arg (`local function outer() pcall(inner) end` where `inner` is a free name in outer's scope captured as an upvalue). HIR lowers the inner reference through a different shape; future ADR.
- ❌ `xpcall(f, msgh, ...)`. Sibling, future M2 sub-ADR.
- ❌ Diverting internal traps through longjmp (per ADR 0215 §Scope).

## Decision

### HIR (`src/hir/ir.rs`)

- `Builtin::Pcall` variant added.
- `name_from_global`: `"pcall" → Some(Builtin::Pcall)`.
- `arity`: `(1, 1)`.
- `name`: `"pcall"`.
- `ret_kinds`: `&[ValueKind::Bool]`.
- `param_kinds_for_arity`: empty (per-arg validation in `lower_builtin_call`).

### HIR validation (`src/hir/mod.rs`)

```rust
// FunctionUsedAsValue allow-list (line ~5260)
if let ValueKind::Function(_) = k
    && !matches!(builtin, Builtin::Type | Builtin::ToString | Builtin::Next | Builtin::Pcall)
{ … reject … }

// Per-arg kind check (line ~5310)
if matches!(builtin, Builtin::Pcall) && !matches!(k, ValueKind::Function(0)) {
    return Err(HirError::TypeMismatch { op: "pcall", … });
}
```

`infer_kind` for `Callee::Builtin(Builtin::Pcall)` returns `ValueKind::Bool`.

### Codegen emit arm (`src/codegen/emit.rs`)

```mlir
// args[0] = Local(LocalId(idx))
%cell_ptr = if non-capturing static FuncId: addressof @<mangled>_closure
            else if idx < params_len:       slots[idx]                   (param)
            else:                           load slots[idx] : !llvm.ptr  (capturing slot)
%fn_ptr   = emit_load_closure_fn_ptr(cell_ptr)
%save_buf = llvm.alloca i32 512 x i8
call @memcpy(%save_buf, @g_jmpbuf, 512)
%ret      = call @_setjmp(@g_jmpbuf) : i32
%is_init  = arith.cmpi eq, %ret, 0_i32
%result   = scf.if %is_init -> i1 {
  call %fn_ptr(%cell_ptr)             // discard f64 result
  call @memcpy(@g_jmpbuf, %save_buf, 512)
  scf.yield true
} else {
  call @memcpy(@g_jmpbuf, %save_buf, 512)
  scf.yield false
}
```

The nested setjmp overwrites the chunk-level state in `g_jmpbuf` for the duration of `f`'s execution — a longjmp from inside `f` lands in the nested pad. After pcall finishes, the chunk-level state is restored, so any subsequent uncaught `error()` returns to the chunk-level pad as expected.

## Tests

`tests/phase4_pcall_basic.rs` (NEW, 4 e2e):

1. `pcall(passing_fn)` returns `true`.
2. `pcall(erroring_fn)` returns `false`.
3. Caught error does not propagate: subsequent `print("after")` runs.
4. Pcall'd fn with internal Local (`local x = 1 + 2 return x`) returns `true` — proves the setjmp dance does not corrupt locals on the success path.

## Test count delta

```
Step 0:  1478 (after ADR 0215)
C3 (impl + 4 e2e): 1478 → 1482
```

## References

- [Lua 5.4 §6.1 `pcall`](https://www.lua.org/manual/5.4/manual.html#pdf-pcall)
- [ADR 0153](0153-pcall-error-strategy.md) — strategy.
- [ADR 0215](0215-pcall-setjmp-infrastructure.md) — setjmp/longjmp foundation.
- [ADR 0083](0083-phase2-5c-full-closures.md) — closure cell + LocalInit alias-skip rule reused by the static-FuncId fast path.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M2 milestone.
