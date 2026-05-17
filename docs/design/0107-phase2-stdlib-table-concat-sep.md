# 0107. Phase 2.7r-stdlib-table: table.concat(t, sep) + Builtin::arity() Range Refactor

- **Status:** Accepted
- **Date:** 2026-05-17
- **Deciders:** ShortArrow

## Replan provenance

ADR 0106 (`d3a7ec9`, 2026-05-17) validated
`Builtin::from_namespace_method` for a third namespace (`table`)
and intentionally held `table.concat` at fixed arity 1, marking
the `Builtin::arity()` range refactor "deferred until 3+ range
builtins exist". ADR 0107 is that trigger:

- Adding `table.concat(t, sep)` makes TableConcat the **third
  range-arity builtin** (after Assert 1-2 and StringSub 2-3).
- The refactor is co-delivered with the feature that creates
  the trigger — Codex-critical non-ad-hoc pattern: "feature
  surfaces the abstraction debt; ADR closes both in one move".

Codex post-0106 6-視点 verdict (over 8 candidates including
arity refactor standalone / table.insert / string.reverse / io.*
begin / malloc OOM consolidation / etc.): **Refactor → Go on
bundle A**. Critical fixes:

- `Builtin::arity() -> usize` → `(usize, usize)` (min, max).
- Eliminate **3 existing special-case branches** at HIR call
  sites:
  - `lower_builtin_call` Assert (1-2 hardcoded)
  - `lower_builtin_call` Print (skip arity check)
  - `lower_namespace_builtin_call` StringSub (2-3 hardcoded)
- Both call sites become uniform:
  `let (min, max) = builtin.arity(); if args.len() < min ||
  args.len() > max { ArityMismatch }`.
- 2-pass `emit_table_concat_runtime` extended with
  `(sep_ptr, sep_len)` parameters; arity-1 path materialises
  empty sep so the helper has one uniform shape.

```lua
print(table.concat({"a", "b", "c"}, ", "))   -- → "a, b, c"
print(table.concat({"a", "b", "c"}, ""))     -- → "abc"
print(table.concat({}, ", "))                 -- → ""
print(table.concat({"only"}, ", "))           -- → "only"
print(table.concat({1, 2, 3}, "-"))           -- → "1-2-3"
local s = ", "; print(table.concat({"a", "b"}, s))  -- → "a, b"
```

## Non-goals (top-of-ADR)

- **`table.concat(t, sep, i, j)` arity 3-4** — bounds
  normalisation; future ADR (string.sub bounds pattern reusable).
- **Other table.* fns** (`insert` / `remove` / `unpack` / `sort`
  / `pack` / `move`) — incremental.
- **sep runtime type trap** — non-String sep silently passes ptr
  to strlen (UB if not null-terminated). Carry-over: same risk
  pattern as `string.len(non_string)`. Future arg-kind
  validation policy ADR can address all builtins uniformly.
- **`ArityMismatch` richer error format** (e.g. `"expects 1-2,
  got 3"`) — keeps `expected: usize` reporting `min` for
  backward-compat with existing test assertions that only check
  the `"ArityMismatch"` substring. Future diagnostic-improvement
  ADR.
- **malloc OOM / size-overflow consolidation** — Codex critical:
  pairs better with a future alloc-heavy ADR.
- **Number-stringify pass-2 cache** — MVP simplicity carry-over.

## Lua 5.4 §6.8 semantics (arity 2 form)

```
table.concat(list, sep)
  -- list[1] .. sep .. list[2] .. sep .. ... .. sep .. list[#list]
  -- No leading/trailing sep; (#list - 1) total sep insertions.
  -- Empty list → ""; single-element list → element only.
```

## New surface

- **HIR `Builtin::arity()` signature refactor**
  (`src/hir/ir.rs`, ~25 LOC):
  - `fn arity(self) -> usize` → `fn arity(self) -> (usize, usize)`.
  - 22 variant arms updated. Print: `(0, usize::MAX)`.
    Assert: `(1, 2)`. StringSub: `(2, 3)`. **TableConcat:
    `(1, 2)`**. Fixed-arity: `(N, N)`.

- **HIR uniform range check** (`src/hir/mod.rs`):
  - `lower_builtin_call`: 3 special-case branches removed
    (Assert / Print / else-fixed). Single uniform check.
    Net delta: -15 LOC.
  - `lower_namespace_builtin_call`: 1 special-case branch
    removed (StringSub). Single uniform check. Net delta:
    -10 LOC.

- **Codegen `emit_table_concat_runtime` extension**
  (`src/codegen/emit.rs`):
  - Signature gains `sep_ptr: Value<ptr>`, `sep_len: Value<i64>`
    parameters; `#[allow(clippy::too_many_arguments)]` (now 7
    args).
  - Pass 1 total adjustment: after element-only accumulation,
    add `sep_len * (length - 1)` (always non-negative since
    the outer scf::if guarantees `length > 0`).
  - Pass 2 inner loop: before each element, `scf::r#if(i > 0)`
    branches between `{memcpy(buf + off, sep_ptr, sep_len);
    off + sep_len}` and `{off}` (no-op), yielding the adjusted
    offset for the element memcpy. The i=0 path is a no-op so
    sep precedes only the 2nd, 3rd, ... elements.

- **Codegen `TableConcat` emit arm extension**
  (`src/codegen/emit.rs`):
  - Branch on `args.len()`:
    - 1 arg: `sep_ptr = emit_empty_string()`, `sep_len = 0`.
    - 2 args: `sep_ptr = emit_expr(args[1])`,
      `sep_len = strlen(sep_ptr)`.
  - Single call to `emit_table_concat_runtime` with the
    materialised sep — arity 1 reuses the arity-2 dispatch via
    empty-sep synthesis.

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `lower_builtin_call` | `src/hir/mod.rs` | uniform range check |
| `lower_namespace_builtin_call` | `src/hir/mod.rs` | uniform range check |
| `Builtin::arity()` | `src/hir/ir.rs` | sole source of truth for arity bounds |
| `emit_table_concat_runtime` | `src/codegen/emit.rs` | 2-pass concat (extended sig) |
| `emit_table_concat_dispatch_len` / `_dispatch_str` | `src/codegen/emit.rs` | tag dispatch (unchanged) |
| `emit_empty_string` (ADR 0104) | `src/codegen/emit.rs` | arity-1 empty sep |
| `emit_libc_call_i64` (strlen) | `src/codegen/primitive.rs` | sep length |
| `emit_libc_call_ptr` (memcpy) | `src/codegen/primitive.rs` | sep insertion |
| `emit_byte_offset_ptr_dynamic` | `src/codegen/primitive.rs` | offset advance |
| `arith::cmpi(Sgt) / muli / subi / addi` + `scf::r#if` | melior 0.27 | sep math + i>0 branch |

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**: refactor
  trigger-driven, not future-use cleanup. 3 special cases →
  1 uniform check (net negative LOC at call sites).
- [x] **#2 TDD (Codex critical)**: 7 new e2e + regression
  coverage. Existing arity tests (StringSub 0-arg, 4-arg,
  StringRep 0-arg + 3-arg, TableConcat 0-arg, Assert, etc.)
  all stay green — they directly exercise the refactored
  dispatch. ArityMismatch tests assert only substring, so
  message format unchanged.
- [x] **#3 FP**: `arity()` signature change is pure refactor.
  Call-site mutation localised to 2 helpers.
- [x] **#4 CA (Codex critical)**: `src/cli/`, `src/pipeline.rs`,
  `src/parser/`, `src/lexer/`, `src/codegen/tagged.rs`
  **zero-diff**.
- [x] **#5 Security**: sep arg kind unvalidated (carry-over
  pattern matching `string.len(non_string)`). Other Lua spec
  traps unchanged. No new attack surface.
- [x] **#6 Documentation**: ADR 0107 doc + tagged-semantics §8
  row + AGENTS.md `‣ 2.7r-stdlib-table` row extended (same
  lane per ADR 0103→0104→0105 precedent).

## Test count delta

```
Step 0:  1106 → 1106 (7 Red Day 0: 5 happy/dynamic-sep tests
                       Red, arity-3-pin Green via current
                       fixed-arity=1 rejection)
Step 1:  cargo build fails — refactor mid-flight (mismatched
        types at 4 sites in mod.rs)
Step 2:  1106 → 1106 (3 of 6 happy tests Red, 3 Green because
                       sep-ignored bytes happen to match expected
                       — e.g., empty/single/empty-sep cases)
Step 3:  no-op (Step 1 already widened TableConcat to (1, 2))
Step 4:  1106 → 1113 (codegen sep insertion; 7 Red → Green)
Step 5:  1106 → 1113 (clippy + fmt; #[allow(too_many_arguments)]
                       added to emit_table_concat_runtime)
Step 6:  1106 → 1113 (docs only)

Final: 1106 → 1113 green, single atomic commit
  feat(hir,codegen,docs): table.concat sep + arity range refactor (ADR 0107)
```

## Verification

- `cargo test --no-fail-fast` → **1106 → 1113**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/cli/ src/pipeline.rs src/parser/ src/lexer/ src/codegen/tagged.rs` → **0**
- `git diff src/hir/mod.rs` — net delta around -25 LOC at the
  two refactored call sites (3 special-case branches removed).
- Manual smoke:
  ```bash
  echo 'print(table.concat({"a", "b", "c"}, ", "))
  print(table.concat({1, 2, 3}, "-"))
  print(table.concat({}, ", "))
  print(table.concat({"only"}, ", "))' > /tmp/c.lua
  cargo run --quiet -- compile /tmp/c.lua && /tmp/c
  # Expected: a, b, c / 1-2-3 / (empty) / only
  ```

## Future work

- **`table.concat(t, sep, i, j)`** — full Lua 5.4 spec; bounds
  reusable from string.sub.
- **table.insert / remove / unpack / pack / sort / move** —
  incremental.
- **sep arg runtime type-trap** — broader builtin arg-kind
  validation policy ADR.
- **`ArityMismatch` richer error** (include max bound) —
  diagnostic improvement ADR.
- **Number-stringify ptr cache** — pass-1 stash to skip pass-2
  re-snprintf.
- **malloc OOM + alloc-size overflow consolidation** ADR.
- **NaN/Inf fptosi guards** unification with ADR 0086.
- **io.* library** — 4th generic-dispatcher consumer.

## ADR number / phase tag

ADR 0107 = `table.concat(t, sep)` + `Builtin::arity()` range
refactor (bundle).
Phase tag: `2.7r-stdlib-table` (continues ADR 0106 sub-lane;
AGENTS.md row extended per ADR 0101→0102 / 0103→0104→0105
same-lane precedent).
