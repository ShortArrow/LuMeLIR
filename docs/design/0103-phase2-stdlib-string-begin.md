# 0103. Phase 2.7q-stdlib-string: string.* Library Begin + Namespace Dispatch Generic

- **Status:** Accepted
- **Date:** 2026-05-17
- **Deciders:** ShortArrow

## Replan provenance

ADR 0101 (`77512f8`, 2026-05-16) and ADR 0102 (`e380bb8`,
2026-05-16) established the stdlib axis with 8 math.* builtins,
but the `lower_call` entry hard-coded `target_name == "math"`:

```rust
if let ExprKind::Index { target, key } = &callee.kind
    && let ExprKind::Ident(target_name) = &target.kind
    && let ExprKind::Str(key_str) = &key.kind
    && target_name == "math"
    && self.resolve("math").is_none()
    && !self.function_names.contains_key("math")
    && let Some(builtin) = Builtin::math_from_method(key_str)
{ ... }
```

Adding `string.*` with the same hardcoded pattern would create
ad-hoc accumulation. ADR 0103 introduces 3 string.* builtins
AND refactors the dispatch to namespace-generic.

```lua
print(string.len("hello"))   -- → 5
print(string.upper("abc"))   -- → ABC
print(string.lower("XYZ"))   -- → xyz
```

Codex post-0102 review (6 視点) verdict: **Refactor → Go**.
Critical:
- **Generic namespace dispatch NOW** — extract `(ns, method)`
  shape walker; dispatch via `Builtin::from_namespace_method`.
- **`emit_string_case_map` helper extract** — upper/lower share
  malloc + memcpy + scf::while case-map loop; don't inline both
  arms.
- **Separate AGENTS.md row** `‣ 2.7q-stdlib-string` (not
  extending the math row).
- **malloc OOM unchecked** documented as carry-over.

## Non-goals (top-of-ADR)

- **Other string.* functions** — `string.sub` (bounds +
  slicing), `string.format` (variadic printf shape), `string.rep`
  (allocation × length), `string.find/match/gmatch` (pattern
  matching), `string.byte` / `string.char` (codes),
  `string.reverse`. Future incremental ADRs.
- **`s:len()` method syntax** — requires `__index = string`
  metatable (Phase 3 territory).
- **`local s = string` aliasing** — ADR 0101 non-goal preserved.
- **UTF-8 / multi-byte char handling** — Lua 5.4 `string.upper`
  / `lower` is byte-wise (ASCII semantics); spec match, no
  multi-byte processing.
- **malloc OOM null-check** — carry-over from `emit_concat` /
  closure / table alloc sites; future ADR consolidates.

## New surface

- **HIR `Builtin` variants** (`src/hir/ir.rs`):
  - `StringLen` (Number-returning), `StringUpper` /
    `StringLower` (String-returning).
  - `Builtin::string_from_method(method)` constructor
    (`"len"` / `"upper"` / `"lower"` → variant).
  - `Builtin::from_namespace_method(ns, method)` generic
    dispatcher: `match ns { "math" => math_from_method,
    "string" => string_from_method, _ => None }`.
  - `arity()` = 1, `name()` = `"string.len"` etc., `ret_kinds()`
    = `[Number]` for StringLen / `[String]` for Upper/Lower.
- **HIR `lower_call` refactor** (`src/hir/mod.rs`):
  - New pure helper `extract_namespace_call(callee) ->
    Option<(String, String)>` walks `Index{Ident(ns),
    Str(method)}`.
  - Replaces inline `target_name == "math"` check with generic
    `extract_namespace_call` + `from_namespace_method` lookup.
  - `lower_math_builtin_call` renamed →
    `lower_namespace_builtin_call` (semantics unchanged).
  - `infer_kind` extended: StringLen → Number;
    StringUpper/Lower → String.
- **Codegen libc extern decls** (`src/codegen/emit.rs`):
  - `toupper(i32) -> i32` and `tolower(i32) -> i32` declared in
    `emit_string_runtime_decls` (`strlen` / `malloc` / `memcpy`
    already declared).
- **Codegen `emit_string_case_map` helper** (~130 LOC, Codex
  critical):
  - `strlen(src) -> i64`, `malloc(length+1)`, `memcpy(buf, src,
    length+1)` (full copy including null terminator).
  - `scf::r#while`-driven for-i-in-0..length loop body:
    `gep buf[i]` (i8 elem) → `load i8` → `extsi i8→i32` →
    `mapper(i32)` libc call → `trunci i32→i8` → `store i8`.
  - Returns the new String ptr.
- **Codegen emit arms** (3 new):
  - `StringLen` — `strlen(src) -> i64` then `sitofp` to f64 via
    `emit_i2f`.
  - `StringUpper` / `StringLower` — call
    `emit_string_case_map(src, "toupper" | "tolower")`.

## Reuse

- ADR 0101 infrastructure: `lower_namespace_builtin_call` helper
  (arity check + arg lower + Call{Builtin} emit), the shape-
  predicate dispatch at `lower_call` entry, `emit_libc_call_f64`
  helper.
- `strlen` / `malloc` / `memcpy` extern decls (ADR 0024+).
- `emit_libc_call_i64` / `i32` / `ptr` helpers (`primitive.rs`).
- `emit_concat` alloc + memcpy pattern (ADR 0025) — helper
  factors out shared shape.
- `emit_i2f` (ADR 0022) for i64 → f64 cast on strlen result.
- `scf::r#while` pattern (used throughout codebase since melior
  0.27 has no `scf::for`).

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**: namespace
  dispatch generic refactored NOW; future `table.*` / `io.*`
  ADRs add one arm in `from_namespace_method` instead of
  another hardcoded check.
- [x] **#2 TDD (Codex critical)**: 6 e2e tests — 3 happy (len /
  upper / lower) + 1 shadowing positive pin + 1 unknown-method
  negative pin + 1 arity pin.
- [x] **#3 FP**: pure shape extraction (`extract_namespace_call`);
  codegen side-effects isolated to `emit_string_case_map`.
- [x] **#4 CA (Codex critical)**: `emit_string_case_map` helper
  extracted — both upper/lower emit arms become ~3 LOC each.
  `src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/`
  zero-diff.
- [x] **#5 Security**: malloc OOM unchecked carry-over
  documented; same boundary as existing alloc sites.
- [x] **#6 Documentation (Codex critical)**: new AGENTS.md row
  `‣ 2.7q-stdlib-string` independent from math row.

## Test count delta

```
Step 0:  1072 → 1072 (6 Red Day 0 — UndefinedName / similar)
Step 1:  1072 → 1072 (Builtin variants; tests still Red)
Step 2:  1072 → 1072 (HIR dispatch refactor; ADR 0101+0102 math
                       tests stay Green via the generic helper)
Step 3:  1072 → 1074 (string.len emit arm + arity pin Green;
                       upper/lower still Red)
Step 4:  1072 → 1078 (case_map helper + upper/lower emit arms;
                       3 happy + shadowing + unknown-method
                       pins Green)
Step 5:  1072 → 1078 (clippy + fmt)
Step 6:  1072 → 1078 (docs only)

Final: 1072 → 1078 green, single atomic commit
  feat(hir,codegen,docs): string.* begin + namespace dispatch generic (ADR 0103)
```

## Verification

- `cargo test --no-fail-fast` → **1072 → 1078**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/cli/ src/pipeline.rs src/parser/ src/lexer/` → **0**
- Manual smoke:
  ```bash
  echo 'print(string.len("hello"))
  print(string.upper("abc"))
  print(string.lower("XYZ"))' > /tmp/s.lua
  cargo run --quiet -- compile /tmp/s.lua && /tmp/s
  # Expected: 5 / ABC / xyz
  ```

## Future work

- **string.sub / format / rep / find / match / gmatch / byte /
  char / reverse** — incremental.
- **`s:len()` method syntax** — requires `__index = string`
  metatable (Phase 3).
- **UTF-8 / multi-byte char handling** — Lua 5.4 `utf8.*`
  library; future ADR.
- **malloc OOM null-check consolidation** — across concat /
  closure / table / string alloc sites.
- **table.* / io.*** — separate ADRs that exercise the
  `from_namespace_method` generic dispatcher.
- **Variadic `string.format`** — different shape (varargs);
  separate ADR.

## ADR number / phase tag

ADR 0103 = string.* library begin + namespace dispatch generic.
Phase tag: `2.7q-stdlib-string` (new sub-lane, independent from
math per Codex preference). Continues the stdlib axis pivot
started in ADR 0101.
