# 0210. `math.type(x)` — Static-Shape Subtype Distinction

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-17
- **Deciders:** ShortArrow

## Context

Second sub-ADR of milestone **M1** (Integer/Float subtype core). ADR 0209 preserved the integer literal through AST + HIR (`HirExprKind::Integer(i64)`). This ADR makes that distinction observable at the user-Lua surface via `math.type(x)`.

Lua 5.4 §6.7 `math.type(x)`:
- Returns `"integer"` if `x` is an integer.
- Returns `"float"` if `x` is a float.
- Returns `nil` if `x` is not a number.

LuMeLIR's Phase B silent demotion (ADR 0209) means subtype is lost once an Integer value flows into a Local slot, a Call return, or a BinOp result. So this ADR delivers `math.type` for the **statically-derivable** cases only: literal arguments. Other shapes return `nil` — documented as a Phase B limitation, lifted in ADR 0211+.

## Scope (literal)

- ✅ New `Builtin::MathType` variant.
- ✅ `math_from_method("type")` returns it.
- ✅ Arity `(1, 1)`, accepts Number-kind arg, returns TaggedValue (String-or-nil).
- ✅ Codegen pattern-matches `args[0].kind`:
  - `HirExprKind::Integer(_)` → `TAG_STRING + ptr("integer")`
  - `HirExprKind::Number(_)` → `TAG_STRING + ptr("float")`
  - Else → `TAG_NIL`
- ✅ Two new global strings (`s_subtypename_integer`, `s_subtypename_float`) registered alongside `s_typename_*` constants.
- ❌ Runtime subtype tracking through Locals, Calls, BinOps. Phase C / future ADR (0211+). Today these all return `nil` from `math.type`.
- ❌ `math.tointeger(x)` — paired Phase B function. Separate ADR.

## Decision

### `src/hir/ir.rs`

```rust
pub enum Builtin {
    // ...
    MathType,
}

pub fn math_from_method(method: &str) -> Option<Self> {
    match method {
        // ...
        "type" => Some(Builtin::MathType),
        _ => None,
    }
}
```

Metadata table rows: arity `(1, 1)`, name `"math.type"`, `ret_kinds = &[ValueKind::TaggedValue]`, `param_kinds_for_arity = &[ValueKind::Number]`.

### `src/hir/mod.rs::infer_kind`

```rust
Callee::Builtin(Builtin::MathType) => ValueKind::TaggedValue,
```

### `src/codegen/emit.rs`

- Module init registers `s_subtypename_integer` / `s_subtypename_float` global strings.
- `Callee::Builtin(Builtin::MathType)` emit arm allocates a TaggedValue tmp slot, pattern-matches the arg's `HirExprKind`, writes the appropriate tagged value, and returns the slot ptr.

### Tests

`tests/phase4_math_type.rs` (NEW, 3 e2e):

1. `print(math.type(42))` → `"integer"`.
2. `print(math.type(42.5))` → `"float"`.
3. `local x = 42; print(math.type(x))` → `"nil"` (Phase B limitation pin — documents the boundary).

## Alternatives considered

- **Use the existing `s_typename_number` global for both subtypes.** Rejected — would conflate Lua's `type()` (returns `"number"`) with `math.type()` (returns `"integer"` or `"float"`). Spec violation.
- **Track subtype at runtime via tagged slots.** Rejected for this ADR — requires touching every Local slot codegen path. Phase C scope.
- **Pattern-match more shapes (BinOp where both operands are Integer literals).** Rejected for MVP — adds constant-folding logic that belongs in a separate optimisation ADR.

## Consequences

**Positive**
- First user-visible difference between Integer and Float subtypes.
- `math.type(<literal>)` returns spec-conformant string for the most common code-time pattern.
- Subtype distinction visible in tests + program output, enabling future ADRs to verify subtype-aware behavior.

**Negative**
- `math.type` returns `nil` for Locals / Calls / BinOps under Phase B. Documented and pinned in test 3; lifted in ADR 0211+.

**Locked in until superseded**
- `s_subtypename_integer` / `s_subtypename_float` are the canonical subtype strings; future ADRs reading subtype tags reuse these globals.

## Documentation updates

- [x] §8 — adds 0210.
- [x] M1 milestone progress: 2 of estimated 6-10 sub-ADRs landed.

## Test count delta

```
Step 0: 1457 (after e12f9ed)
C3 (impl + 3 e2e): 1457 → 1460
```

## Critical files

- `docs/design/0210-math-type-static-shape.md` (this doc).
- `docs/design/README.md` index entry.
- `src/hir/ir.rs` — Builtin::MathType + table rows.
- `src/hir/mod.rs` — infer_kind arm.
- `src/codegen/emit.rs` — global string registrations + MathType emit arm.
- `tests/phase4_math_type.rs` (NEW) — 3 e2e.

## Future work

- ADR 0211+ — runtime subtype tracking through Locals and arithmetic results, so `local x = 42; math.type(x)` returns `"integer"`. Requires `ValueKind::Integer` introduction and Local slot subtype field.
- `math.tointeger(x)` — paired conversion function. Separate ADR.

## References

- [Lua 5.4 §6.7 math.type](https://www.lua.org/manual/5.4/manual.html#pdf-math.type)
- [ADR 0196](0196-integer-float-subtype-design.md) — Integer/Float subtype design entry.
- [ADR 0209](0209-integer-ast-hir-variant.md) — Phase B AST + HIR variant; this ADR's prerequisite.
- [`docs/notes/roadmap-revision-2026-06-16.md`](../notes/roadmap-revision-2026-06-16.md) — M1 milestone definition.
