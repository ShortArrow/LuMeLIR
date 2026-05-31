# 0149. `__len` Metamethod for `#t`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

[ADR 0137](0137-raw-equal-len-builtins.md) landed `rawlen(t)` as the explicit raw-length builtin, with the long-term assumption that `#t` would later consult `mt.__len`. With the metamethod ABI mature (ADRs 0142 – 0148 all share `emit_dispatch_chain_from_slot_ptr`), `__len` is the smallest remaining Lua spec metamethod for the Table operand. Lua spec §3.4.7: `#t` on a Table consults `__len`; absent → returns the raw length.

## Scope (literal)

**`#t` for Table operand**, Function-form `__len`, returns Number. Out of scope:

- ❌ Non-Function `__len`.
- ❌ Rhs-fallback (unary op — N/A).
- ❌ TaggedValue runtime Table-tag dispatch.

## Decision

### HIR

No HIR change. `UnaryOp::Len` already accepts Table operand (lower-time check at `src/hir/mod.rs:4063`).

### Codegen

`emit_expr` UnaryOp `Len` arm: when operand kind is Table, route to a new helper `emit_len_via_metamethod` BEFORE the existing raw-length path.

`emit_len_via_metamethod(t_ptr, functions, ...)`:

1. Compile-time candidate filter: user fns with sig `(Table) → Number`.
2. If candidate set is empty OR `t.metatable_ptr == null` OR `mt["__len"]` is missing / non-Function → **fall back to raw length** (existing `emit_load(t_ptr + TABLE_OFF_LEN)` + `emit_i2f`).
3. Otherwise dispatch via `emit_dispatch_chain_from_slot_ptr` (ADR 0142 reuse) with `sig = (Table) → Number` and `args = [t_ptr]`.

Unlike `__tostring` (ADR 0142, fallback to "table" literal) and unlike arith / bitwise (ADRs 0147 / 0148, trap on missing), `__len` **falls back to raw length** — matches Lua spec.

### New module globals

- `s_metatable_len_field_name` ("__len").
- No new trap message — the fallback is non-trapping.

### Metamethod-aware kind refinement

| Key | Forced signature |
|---|---|
| `__len` | `(Table) → Number` |

## Alternatives considered

- **Trap on missing `__len`** (Lua-spec-violation choice). Rejected — `#t` without `__len` works in stock Lua.
- **Bundle with `__index = Function` / `__newindex = Function`** (the gating Tier 4 work). Rejected — `__len` is independent.
- **Static early-bind to `rawlen(t)` at HIR time**. Rejected — would require knowing the metatable at HIR time, which fails for the typical `setmetatable(t, mt)` runtime pattern.

## Consequences

**Positive**
- `#t` works the canonical Lua way — user can override length via `__len`.
- Helper reuse: `emit_dispatch_chain_from_slot_ptr` again. Net adds ~100 LOC of `emit_len_via_metamethod`.
- No new trap; missing-metamethod fall-back is silent (per spec).

**Negative**
- One more module global.
- `#t` now does (a) metatable null check, (b) `__len` probe, (c) tag check, on every Table-length call. Negligible at common call-site frequencies.

**Locked in until superseded**
- Function-form only.
- Falls back to raw length on missing metafield.

## Documentation updates

- [x] §4 LIC — new `LIC-len-metamethod-1`.
- [x] §7 — closes `__len` Table item.
- [x] §8 — adds 0149.

## Test count delta

```
Step 0:   1348 (after ADR 0148)
C2 (4 e2e Red Day 0):  1348 → 1348
C3 (impl): 1348 → 1352
```

## Critical files

- `src/codegen/emit.rs`:
  - 1 new global `s_metatable_len_field_name`.
  - `emit_expr` UnaryOp Len arm routes Table operand through new helper.
  - `emit_len_via_metamethod` (~120 LOC).
- `src/hir/mod.rs`:
  - Metamethod-aware refinement walk: `__len` arm.
- `tests/phase2_6plus_len_metamethod.rs` (NEW) — 4 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Existing `#t` raw-length path regresses | The new arm fires only when (candidate set non-empty AND mt non-null AND `__len` is Function); every other branch routes back through `emit_load(TABLE_OFF_LEN)` + `emit_i2f`. Existing length tests are the regression net. |
| `__len` returns non-Number | Compile-time candidate filter restricts to `(Table) → Number`. |
| Recursion: `__len` calls `#t` again | Same as any user-fn recursion. Stack overflow on infinite. |

## Future work

- Non-Function `__len`.
- TaggedValue runtime Table-tag dispatch.
- Possibly: `__index = Function` (a follow-up ADR that resolves call-ABI for "non-Table" `__index` results).

## References

- [ADR 0053](0053-phase2-6a-min-empty-tables.md) — original `#t` raw length.
- [ADR 0137](0137-raw-equal-len-builtins.md) — `rawlen` sibling.
- [ADR 0142](0142-tostring-metamethod.md) / [ADR 0144](0144-comparison-metamethods.md) — dispatch helper reuse.
- Lua 5.4 reference manual §3.4.7 — `#` operator + `__len` metamethod.
