# 0240. Math Unary Expansion — `ceil`, `tan`, `asin`, `acos`, `atan`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M11 sub-ADR. ADRs 0101 / 0102 landed the math library MVP (`sqrt`, `floor`, `abs`, `pow`, `sin`, `cos`, `log`, `exp`). Five more Lua 5.4 §6.7 unary functions share the same `f64 → f64` shape: `ceil`, `tan`, `asin`, `acos`, `atan`. All map directly to libm symbols. Bundle them in one sub-ADR — the per-function overhead is one HIR variant + one libm extern decl + one match arm row.

## Scope (literal)

- ✅ 5 new HIR `Builtin` variants: `MathCeil`, `MathTan`, `MathAsin`, `MathAcos`, `MathAtan`.
- ✅ `math_from_method` recognises `"ceil"`, `"tan"`, `"asin"`, `"acos"`, `"atan"`.
- ✅ `arity` `(1, 1)`, `ret_kinds` `[Number]`, `param_kinds` `[Number]`, `infer_kind` → `Number`.
- ✅ `emit_libm_decls` registers `ceil`, `tan`, `asin`, `acos`, `atan` externs (shared loop with existing sin/cos/log/exp).
- ✅ Unary math emit arm extends the existing dispatch with 5 new libm names.
- ❌ `math.atan2` (two-arg). Lua 5.4 `math.atan(y, x)` accepts a second arg; deferred until first user demand.
- ❌ `math.fmod` / `math.modf`. Both have f64-f64-or-multi-return shapes; future M11 sub-ADR.
- ❌ `math.max` / `math.min`. Variadic; require their own dispatch shape; future M11 sub-ADR.
- ❌ `math.random` / `math.randomseed`. Stateful (PRNG seed global); needs runtime state; future M11 sub-ADR.

## Decision

```rust
// hir/ir.rs:
MathCeil, MathTan, MathAsin, MathAcos, MathAtan,

// math_from_method:
"ceil" => Some(MathCeil),
"tan"  => Some(MathTan),
"asin" => Some(MathAsin),
"acos" => Some(MathAcos),
"atan" => Some(MathAtan),
```

```rust
// emit.rs::emit_libm_decls:
for libm_name in ["sin", "cos", "log", "exp", "ceil", "tan", "asin", "acos", "atan"] { ... }

// emit_expr unary math arm:
Builtin::MathCeil => "ceil",
Builtin::MathTan  => "tan",
...
```

## Tests

`tests/phase4_m11a_math_expansion.rs` (NEW, 8 e2e):

1. `math.ceil(3.2)` → `4`.
2. `math.ceil(-3.2)` → `-3`.
3. `math.ceil(7)` → `7` (Integer identity).
4. `math.tan(0)` → `0`.
5. `math.asin(0)` → `0`.
6. `math.acos(1)` → `0`.
7. `math.atan(0)` → `0`.
8. `math.atan(1)` ≈ π/4 (starts with `0.7853`).

## Test count delta

```
Step 0:  1579 (after ADR 0239)
C3 (impl + 8 e2e): 1579 → 1587
```

## References

- [ADR 0101](0101-phase2-7q-stdlib-math.md) — math library MVP.
- [ADR 0102](0102-phase2-7q-stdlib-math-continuation.md) — sin/cos/log/exp/pow expansion.
- [Lua 5.4 §6.7](https://www.lua.org/manual/5.4/manual.html#6.7) — math library spec.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M11 milestone.
