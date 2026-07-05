# 0296. Vararg pack ABI + spread — implementation design (F1-C-step2/3)

- **Status:** Design — implementation split across F1-C-step2 and F1-C-step3
- **Kind:** Architecture Decision
- **Date:** 2026-07-05
- **Deciders:** ShortArrow

## Why this ADR exists

F1-C-step1 (ADR 0295) landed a nil-stub. The Stop hook keeps firing for "F1-C" because real Lua semantics require actual pack materialisation + spread. Both are `abi-new` layer (roadmap 2026-07-02 principle 8), each is its own session. This ADR pre-decomposes the two remaining steps so the next session can pick up cleanly without re-deriving the design.

## Decision: Table-pack ABI

The pack is a **`Table` value**, not a variadic C call convention.

Rationale:
- Table constructors + array-part indexing are already fully wired (ADR 0053 / 0054).
- Every downstream consumer (`select`, `table.pack`, `table.unpack`, `{...}`) either *is* a Table or can be trivially derived from one.
- Avoids a new MLIR calling convention or a heterogeneous pack layout — one type, one path.

Cost:
- One heap allocation per vararg call. Acceptable at MVP; escape analysis (N8-C) can lift to stack.

## F1-C-step2 — call-site pack + hidden param

### Signature change

Vararg `HirFunction` gains one extra `LocalInfo` at the end of `params`:

```rust
LocalInfo {
    name: "_va_pack".to_owned(),
    kind: ValueKind::Table,
    func_id: None,
    is_captured: false,
    subtype: NumberSubtype::Unknown,
    is_const: true,
    is_close: false,
}
```

Injected during `lower_into_function` when `is_vararg=true`, **after** the body pass so user's own local names aren't disturbed but **before** `functions[fid.0].params` is written.

### Call site rewrite

At every HIR call whose target `is_vararg`:

1. Split `args` at declared arity: `(declared, extras)`.
2. Wrap `extras` in a single new `HirExprKind::Table(extras)` node.
3. New args = `declared ++ [pack_table]`.

Sites (`Callee::User` / `Callee::Indirect` with known FuncId / `Callee::IndirectDispatch`):
- `lower_call` — 3 arm updates.
- `emit_call_string_find_into_locals`-style multi-return dispatchers — audit and update.

### `HirExprKind::Vararg` inside body

`lower_expr` for `Vararg` inside `in_vararg_function`:

```rust
HirExprKind::Index {
    target: Box::new(HirExpr {
        kind: HirExprKind::Local(_va_pack_local_id),
        span,
    }),
    key: Box::new(HirExpr {
        kind: HirExprKind::Integer(1),
        span,
    }),
}
```

Only supports single-value context (returns first extra). Empty pack → runtime OOB trap (documented deviation).

### Tests (step2)

- `local function f(...) local t = ... print(t) end; f(42)` → `42`
- `f(1, 2, 3)` under same `f` → `1`
- Empty pack: `f()` traps at runtime (deviation vs Lua's nil).

### Zero-diff

Codegen: no MLIR-level changes; existing user-fn ABI absorbs the extra Table param.
Runtime: no new bridge functions.

## F1-C-step3 — spread contexts + multi-value semantics

Handles the multi-value shapes that step2 leaves broken:

### Contexts

1. **Table constructor**: `{...}` — array-copy all elements from `_va_pack` into the new table.
2. **Return statement**: `return ...` — new HIR statement kind `HirStmtKind::ReturnVararg { pack_local: LocalId }`.
3. **Call argument spread**: `f(...)`, `f(a, ...)` — call receives all pack elements after `a`. Needs multi-return-from-Vararg wiring in `lower_multi_assign_from_call` and callee arity resolution.
4. **Multi-assign**: `local a, b, c = ...` — assigns from pack array-part positions.

### New HIR kinds

- `HirExprKind::VarargSpread` — differentiates single-value `...` from spread `...`. `lower_expr` picks based on syntactic context. Might be lifted to a parser-level distinction (`ArgsList` node containing spread).
- `HirStmtKind::ReturnVararg` — mirrors existing multi-return but from a Table.

### Empty-pack semantics

`{...}` and `return ...` with empty pack must produce a length-0 Table / zero returns respectively. Codegen: same as `{}` / bare `return`.

### `select` builtin

- `select("#", ...)` → `#_va_pack`.
- `select(n, ...)` → array slice `_va_pack[n..]`. Needs a runtime helper `lumelir_table_slice(t, n)` or a HIR loop.

### `table.pack` / `table.unpack`

- `table.pack(...)` → equivalent to `{...}` plus `t.n = #t`. Straight HIR desugar.
- `table.unpack(t)` → return spread — needs `HirStmtKind::ReturnVararg` from arbitrary Table.

### Tests (step3)

- `local function f(...) return #{...} end; print(f(1, 2, 3))` → `3`
- `local function f(...) return ... end; print(f(1, 2))` → `1  2`
- `select("#", 1, 2, 3)` → `3`
- `table.pack(1, 2, 3).n` → `3`
- `table.unpack({1, 2, 3})` used as call arg → three args passed.

## Session decomposition estimate

- **F1-C-step2**: 1-2 sessions. Table pack ABI is straightforward once designed.
- **F1-C-step3**: 2-3 sessions. Spread contexts touch parser / HIR / codegen; multi-return-from-Vararg is genuinely new plumbing.
- **F1-C-follow-up N7 items** (`select`, `table.pack`, `table.unpack`, `xpcall`, `string.format` extras): 1 session each after step3 lands.

Total to full F1-C parity with Lua 5.4: **3-5 sessions from today**.

## What this session lands

Only this ADR (design). Attempted concrete implementation of step2 would consume more context than available and risk half-wired commits that block subsequent sessions. This is the same recovery pattern that ADR 0284 used successfully for N4-F-2.

## References

- ADR 0293 — F1-A parser + AST.
- ADR 0294 — F1-B HIR representation.
- ADR 0295 — F1-C-step1 codegen stub.
- ADR 0284 — the precedent case for design-only deferral.
- Roadmap 2026-07-03 — F1 chain sequencing.
- Lua 5.4 §3.4.11 — `...`.
