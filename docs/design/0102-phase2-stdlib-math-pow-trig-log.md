# 0102. Phase 2.7q-stdlib-math: math.* Continuation (pow / sin / cos / log / exp)

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-16
- **Deciders:** ShortArrow

## Replan provenance

ADR 0101 (`77512f8`, 2026-05-16) established the stdlib axis with
3 math.* builtins (sqrt / floor / abs) and the infrastructure for
incremental extension. ADR 0102 adds 5 more functions:

```lua
print(math.pow(2, 10))   -- → 1024  (binary; the only binary today)
print(math.sin(0))       -- → 0
print(math.cos(0))       -- → 1
print(math.log(1))       -- → 0     (natural log)
print(math.exp(0))       -- → 1
```

Codex post-0101 review (6 視点) verdict: **Go** (no Refactor). Critical:
- pow is binary; tests must pin it separately from the unary group.
- 6-point checklist per new Builtin variant (don't miss any touch
  point).

## Non-goals (top-of-ADR)

- **math.pi / math.huge / math.maxinteger / math.mininteger** —
  constants (Index without Call). Future ADR.
- **math.random / randomseed** — needs RNG state.
- **math.tan / asin / acos / atan / atan2** — incremental.
- **math.modf / fmod / deg / rad / max / min** — incremental.
- **string.* / table.* / io.*** — separate ADRs.
- **`local m = math` aliasing** — ADR 0101 non-goal preserved.
- **math.log binary form** — `math.log(x, base)` Lua 5.4
  signature. We implement 1-arg natural log only for MVP.

## Codex 6-point checklist

Each new Builtin variant (`MathPow`, `MathSin`, `MathCos`,
`MathLog`, `MathExp`) touches:

1. **`Builtin::math_from_method`** — method-string mapping.
2. **`Builtin::arity()`** — MathPow=2, others=1.
3. **`Builtin::name()`** — stringified `"math.pow"` etc.
4. **`Builtin::ret_kinds()`** — `&[ValueKind::Number]` (all 5).
5. **`infer_kind(Callee::Builtin(...))`** in `src/hir/mod.rs:221+`.
6. **emit arm** in `src/codegen/emit.rs` (unary group + separate
   binary MathPow arm).

All 5 variants × 6 touch points = 30 changes; the implementation
walks the checklist systematically.

## New surface

- **HIR `Builtin` variants** (`src/hir/ir.rs`):
  - `MathPow` (binary), `MathSin`, `MathCos`, `MathLog`, `MathExp`
    (unary).
- **HIR `infer_kind`** (`src/hir/mod.rs`): math-builtin Number-
  returning arm extended with 5 new variants via or-pattern.
- **Codegen libm extern decls** (`src/codegen/emit.rs`):
  `sin`, `cos`, `log`, `exp` (`pow` already declared for Lua `^`
  operator). All `f64 -> f64`.
- **Codegen emit arms**:
  - Unary group (sqrt/floor/abs/sin/cos/log/exp) — extended
    or-pattern with `match b { ... }` mapping to libm name.
  - Binary group (pow only) — separate arm with explicit
    `&[base, exponent]` slice construction.

## Reuse

- ADR 0101 infrastructure: `lower_math_builtin_call` helper
  (arity check + arg lower + Call{Builtin} emit), the shape-
  predicate dispatch at `lower_call` entry, `emit_libc_call_f64`
  helper.
- libm `pow` extern decl (existing, for Lua `^` operator).
- `LLVMFuncOperationBuilder` pattern.

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First**: simple enumeration; no table
  driven (Codex's preference at 8 functions).
- [x] **#2 TDD (Codex critical)**: 5 happy + 1 binary-arity pin
  for pow + existing ADR 0101 6 tests retained as regression.
- [x] **#3 FP**: pure pattern match dispatch.
- [x] **#4 CA**: HIR + codegen, no parser/lexer/cli change.
- [x] **#5 Security**: unchanged from ADR 0101 — same
  `resolve("math").is_none()` guard applies.
- [x] **#6 Documentation**: ADR 0102 + tagged-semantics §8 +
  AGENTS.md `‣ 2.7q-stdlib-math` row updated (no new row per
  Codex preference).

## Test count delta

```
Step 0:  1066 → 1066 (6 Red Day 0 — all UndefinedName)
Step 1:  1066 → 1066 (Builtin variants; tests still Red)
Step 2:  1066 → 1066 (infer_kind; tests still Red)
Step 3:  1066 → 1072 (codegen arms; 5 happy + 1 arity-pin
                      Red → Green)
Step 4:  1066 → 1072 (clippy + fmt)
Step 5:  1066 → 1072 (docs only)

Final: 1066 → 1072 green, single atomic commit
  feat(hir,codegen,docs): math.* continuation (pow/sin/cos/log/exp) (ADR 0102)
```

## Verification

- `cargo test --no-fail-fast` → **1066 → 1072**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/cli/ src/pipeline.rs src/parser/ src/lexer/` → **0**
- Manual smoke:
  ```bash
  echo 'print(math.pow(2, 10))
  print(math.sin(0))
  print(math.cos(0))
  print(math.log(1))
  print(math.exp(0))' > /tmp/m.lua
  cargo run --quiet -- compile /tmp/m.lua && /tmp/m
  # Expected: 1024 / 0 / 1 / 0 / 1
  ```

## Future work

- **math.pi / huge / maxinteger / mininteger** — constants
  (different shape: Index without Call).
- **math.random / randomseed** — RNG state.
- **math.tan / asin / acos / atan / atan2** — incremental.
- **math.log binary form** — `math.log(x, base)`.
- **string.* / table.* / io.*** — separate ADRs.

## ADR number / phase tag

ADR 0102 = math.* continuation. Phase tag: `2.7q-stdlib-math`
(same sub-lane as ADR 0101 per Codex preference).
