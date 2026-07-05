# 0294. Vararg `...` HIR representation (F1-B)

- **Status:** Accepted (HIR-only; codegen expansion deferred to F1-C)
- **Kind:** Architecture Decision
- **Date:** 2026-07-05
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ `HirFunction` gains `is_vararg: bool`.
- ✅ New `HirExprKind::Vararg` for expression-position `...` inside a vararg function body.
- ✅ `LowerCtx::in_vararg_function` flag set/cleared around each function body lowering. Non-vararg functions still raise `HirError::VarargUnsupported` on any inner `...`.
- ✅ Chunk-level `...` intentionally kept as an error (Lua spec makes main chunks implicit-vararg, but that plumbing rides on a real HIR chunk-function representation — deferred).
- ✅ `infer_kind(HirExprKind::Vararg) = TaggedValue` — each element carries a runtime tag; the pack itself is spread at call sites.
- ✅ Codegen emits `CodegenError::UnsupportedExpr("vararg `...` codegen deferred to F1-C")` — F1-C removes it.
- ❌ Chunk-level implicit-vararg — F1-C or later.
- ❌ Pack materialisation choice (Table upvalue vs SSA multi-value) — F1-C picks.
- ❌ `select`, `table.pack`, `table.unpack` builtins — depend on F1-C.

## Design notes

Why the flag on `HirFunction` instead of on `HirFunction::params`?

- Params today are `Vec<LocalInfo>` — bolting a Vararg sentinel into that list would ripple through arity calculations, upvalue projection, direct-call ABI matching, etc.
- A separate `is_vararg: bool` keeps everything downstream honest without touching per-param invariants.
- Codegen (F1-C) will read the flag and *append* a hidden variadic-pack parameter to the MLIR signature; direct callers pass a synthesised Table pointer or the null value when there are no extras.

Why `TaggedValue` for `HirExprKind::Vararg`?

- Each element could be any Lua value at runtime. The static kind of a variadic expression as a whole is not a single position — but consumers (call args, table constructor tail, `select`, unpacking assignment) all treat elements individually. `TaggedValue` propagates through the widening path (ADR 0060/0110) exactly like other multi-kind values.

## Tests

5 HIR-level e2e (`tests/phase4_f1b_vararg_hir.rs`): `is_vararg=true` on `function(...)`; `false` without; body contains `HirExprKind::Vararg`; error on `...` outside vararg function; error on chunk-level `...`. 1795 → 1800.

## References

- ADR 0293 — F1-A parser + AST shape.
- ADR 0083 — `HirFunction` + upvalue mechanism (unchanged).
- Roadmap 2026-07-03 — F1 chain sequencing.
- Lua 5.4 §3.4.11 — `...`.
