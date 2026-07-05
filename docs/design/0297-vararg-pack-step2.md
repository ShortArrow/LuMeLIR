# 0297. Vararg Table-pack ABI (F1-C-step2)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-07-05
- **Deciders:** ShortArrow

## Scope (literal)

Implements ADR 0296 §F1-C-step2 exactly as designed:

- ✅ `LowerCtx::for_function` gains an `is_vararg` param; when true it appends a synthetic `_va_pack` local (kind `Table`) after the declared params and sets `in_vararg_function`.
- ✅ `lower_into_function` writes `params[..declared + 1]` (including the pack) into `HirFunction.params` for vararg fns; anonymous `FunctionExpr` path stamps `is_vararg` on its `HirFunction` too.
- ✅ `ValueKind::Function(arity)` for vararg fns now stores the **effective** arity (declared + 1 pack slot) at both declaration sites (chunk-level pass and function-body pass 1.5).
- ✅ Call sites (Ident-callee with known FuncId): split args at declared arity, lower declared args normally, wrap extras in `HirExprKind::Table(extras)` appended as the final arg. Zero extras → empty Table.
- ✅ `ExprKind::Vararg` in a vararg fn body lowers to `Index(Local(_va_pack), Integer(1))` — single-value position reads the first extra.
- ✅ `FunctionExpr` with `...` no longer errors (`VarargUnsupported` removed from that path).
- ❌ Empty-pack read: `f()` then `...` traps at the Table OOB check instead of yielding nil — deviation pinned in the step1 test, resolved by step3's empty-check.
- ❌ Spread contexts (`{...}` full copy, `return ...`, `f(a, ...)` multi-value, multi-assign from `...`) — F1-C-step3 per ADR 0296.
- ❌ `select` / `table.pack` / `table.unpack` — post-step3 N7 items.
- ❌ MethodDef with `...` — still `VarargUnsupported` (colon-method vararg is rare; follow-up).

## What changed vs step1 (ADR 0295)

| Program | step1 result | step2 result (this ADR) |
|---|---|---|
| `local function f(...) local t = ... end; f(42); print(type(t))` | `nil` | `number` ✅ matches Lua |
| nested `outer(...) → inner(...)` first-value pass-through | `nil` | `number` ✅ |
| `f()` then reading `...` | `nil` | OOB trap (step3 fixes to nil) |

Codegen was untouched — the existing user-fn Table-param ABI absorbs `_va_pack` like any other Table param, exactly as ADR 0296 predicted (zero-diff codegen for step2).

## Tests

Updated in place (same 9 files stay green): `tests/phase4_f1c_vararg_codegen_stub.rs` 4 e2e now assert real pack semantics (`type(...)` = `"number"` with extras); `tests/phase4_f1b_vararg_hir.rs` asserts `_va_pack` is the last param. Full suite 1804 green, 0 failed.

## References

- ADR 0295 — F1-C-step1 stub (superseded semantics).
- ADR 0296 — the design this implements.
- Roadmap 2026-07-03 — F1 chain.
- Lua 5.4 §3.4.11 — `...`.
