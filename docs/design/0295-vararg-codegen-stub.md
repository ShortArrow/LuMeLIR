# 0295. Vararg `...` codegen stub (F1-C-step1)

- **Status:** Accepted (stub only — documented Lua-spec deviation)
- **Kind:** Architecture Decision
- **Date:** 2026-07-05
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ Codegen for `HirExprKind::Vararg` emits an `i1 0` (nil placeholder), matching the existing `HirExprKind::Nil` stub shape.
- ✅ `infer_kind(HirExprKind::Vararg)` downgraded from `TaggedValue` (per ADR 0294) to `Nil` so downstream consumers see the actual codegen shape.
- ✅ HIR arity check at Ident-callee call sites (`Callee::User` / `Callee::Indirect` via known local) accepts extra positional args when the target `HirFunction.is_vararg`. Extras are truncated before HIR lowering / type-checking / codegen so the direct-call ABI is undisturbed.
- ❌ **Real pack materialisation** — `...` still evaluates to `nil` regardless of what was passed. Call-site extras are silently dropped. This is F1-C-step2 (Table-pack ABI) and F1-C-step3 (call-arg spread).
- ❌ Other arity check sites (Function-kind Local without known FuncId, IndirectDispatch, methods) not yet loosened for vararg targets. Follow-up.
- ❌ Chunk-level implicit-vararg — deferred.

## Why a stub

F1-C proper (Table-based pack + hidden param + call-site spread) is `abi-new` layer per [roadmap 2026-07-02](../notes/roadmap-2026-07-02-rebuild.md) — 3+ sessions. Landing a stub now unblocks:

- Every parser test that constructs vararg fns (F1-A) can now run compiled binaries.
- Every HIR test that lowers `...` (F1-B) has a working codegen sink.
- Follow-up work (`select`, `table.pack`, `table.unpack`, `xpcall`, `string.format` extras) can be attempted incrementally without stumbling into "codegen error, cannot even compile".

The deviation is documented so that the next arc (F1-C-step2/3) picks up in exactly the right place: replace the `i1 0` emit with a load from the pack, and add the pack parameter + spread ABI.

## Behavior differences from Lua 5.4

| Program shape | Lua 5.4 | LuMeLIR (this ADR) |
|---|---|---|
| `local function f(...) local t = ... end; f(42)` | `t = 42` | `t = nil` |
| `local function f(...) return ... end; print(f(1, 2))` | `1  2` | (multi-return spread also not wired — `nil` on first, arity error on second) |
| `local function f(...) return #{...} end; print(f(1,2,3))` | `3` | `0` (table `{...}` today has zero elements — spread in table ctor not wired) |

All three become correct in F1-C-step2 (pack materialisation) + F1-C-step3 (spread contexts).

## Tests

4 e2e (`tests/phase4_f1c_vararg_codegen_stub.rs`): bare `...` in body compiles + runs; `type(...)` returns `"nil"`; extra call args accepted without arity error; nested vararg fn compiles. 1800 → 1804.

## References

- ADR 0293 — F1-A parser + AST shape.
- ADR 0294 — F1-B HIR representation.
- Roadmap 2026-07-03 — full completion path (F1 chain identified as foundation blocker).
- Lua 5.4 §3.4.11 — `...`.
