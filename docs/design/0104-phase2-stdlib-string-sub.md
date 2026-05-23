# 0104. Phase 2.7q-stdlib-string: string.sub + Bounds-Normalization Helper

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-17
- **Deciders:** ShortArrow

## Replan provenance

ADR 0103 (`70775a3`, 2026-05-17) added 3 string.* builtins
(`len`/`upper`/`lower`) and refactored the dispatch chokepoint
to namespace-generic (`Builtin::from_namespace_method`,
`extract_namespace_call`, `lower_namespace_builtin_call`). ADR
0104 extends the string.* library with **`string.sub`** — the
most-used Lua string operation after length.

Codex post-0103 6-視点 review verdict: **Refactor → Go on
A (`string.sub`)** (over B `string.rep` / C `string.reverse` /
D `string.byte` / E `table.* begin` / F malloc OOM consolidation
/ G `math.* constants`). Selection rationale:
- ADR 0103 namespace-call pattern reuses cleanly (no chokepoint
  extension).
- Pure bounds-normalization helper can be pre-extracted — highest
  TDD density of all candidates.
- `parser` / `lexer` / `cli` / `pipeline` zero-diff (CA invariant
  preserved).
- Bounded user-visible value (1 widely-used function vs `table.*`
  begin which spans semantics).

```lua
print(string.sub("hello", 2, 4))   -- → "ell"
print(string.sub("hello", -3))     -- → "llo" (suffix)
print(string.sub("hello", 1, 100)) -- → "hello" (j clamped to #s)
print(string.sub("hello", 10))     -- → ""   (i past end)
print(string.sub("hello", 3, 1))   -- → ""   (i > j after normalize)
```

## Non-goals (top-of-ADR)

- **string.rep / reverse / find / match / gmatch / byte / char /
  format** — incremental future ADRs.
- **`s:sub(i)` method syntax** — requires `__index = string`
  metatable (Phase 3).
- **`local s = string` aliasing** — ADR 0101 non-goal preserved.
- **UTF-8 / multi-byte char handling** — Lua 5.4 `string.sub` is
  byte-wise (ASCII semantics).
- **malloc OOM null-check consolidation** — carry-over from ADR
  0103 (slated for future consolidation ADR, not bundled).
- **`fptosi` on NaN / ±Inf** — UB in MLIR; Lua spec allows
  integer coercion at the call site, so `string.sub("abc",
  0/0)` is undefined behaviour. Defer to a future ADR that can
  unify with ADR 0086 hash-key NaN policy.
- **Variadic OR-arity refactor** — `Builtin::arity()` stays
  `usize`. `string.sub` takes the Assert-precedent path (special
  case in `lower_namespace_builtin_call`). A future ADR refactors
  to a min/max range once 3+ namespace builtins need it.

## Lua 5.4 §6.4 semantics

The `string.sub(s, i [, j])` algorithm in 1-based-inclusive form:

1. If `i < 0`: `i = #s + i + 1` (suffix indexing).
2. After step 1, `i = max(i, 1)` (clamp lower).
3. If `j` is absent: `j = #s` (equivalent to `j = -1` followed by
   the negative translation in step 4).
4. If `j < 0`: `j = #s + j + 1`.
5. `j = min(j, #s)` (clamp upper).
6. If `i > j` after normalization: return empty string.
7. Otherwise: return bytes `s[i-1 .. j-1]` (0-indexed,
   inclusive), length `j - i + 1`.

## New surface

- **HIR `Builtin` variant** (`src/hir/ir.rs`):
  - `StringSub` variant.
  - `Builtin::string_from_method`: `"sub"` → `StringSub`.
  - `Builtin::arity()` = 2 (the **minimum** — Assert precedent;
    the 2-or-3 check lives in `lower_namespace_builtin_call`).
  - `Builtin::name()` = `"string.sub"`.
  - `Builtin::ret_kinds()` = `&[ValueKind::String]`.
- **HIR `lower_namespace_builtin_call` arity-range special case**
  (`src/hir/mod.rs`): one new arm mirroring
  `lower_builtin_call`'s `Assert` arm — `if matches!(builtin,
  Builtin::StringSub) && (args.len() < 2 || args.len() > 3)` →
  ArityMismatch with `expected = 2` (lower bound).
- **HIR `infer_kind`**: extend the String-returning or-pattern with
  `StringSub` alongside `StringUpper | StringLower`.
- **Codegen 3 new helpers** (`src/codegen/emit.rs`):
  - `emit_empty_string` (~30 LOC): `malloc(1) + store 0` —
    per-call empty-String allocation matching the existing
    alloc-and-leak shape.
  - `emit_normalize_sub_bounds` (~120 LOC): pure SSA bounds
    normalization. Inputs: `len`, `i`, `j` (all i64). Outputs:
    `(i_norm, j_norm)` after negative translation + clamp.
    Uses `arith::cmpi(Slt/Sgt)` + `arith::select` + `arith::addi`
    only — no scf::if / control flow.
  - `emit_string_slice` (~70 LOC): `malloc(length + 1) + memcpy
    from src + (start - 1) + null-terminate`. Reusable for
    future `string.find` / `string.match` capture extraction.
- **Codegen `StringSub` emit arm** (~80 LOC):
  - Lower `args[0]` (s: String → ptr) + `args[1]` (i: Number →
    f64), `emit_f2i` → i64.
  - If `args.len() == 3`: lower `args[2]` (j: Number → f64),
    `emit_f2i` → i64. Else: `j_i64 = len_i64` (Lua spec: j
    absent ⇔ post-translate j = len).
  - Call `emit_normalize_sub_bounds`.
  - `count = j_norm - i_norm + 1`; `count_pos = (count > 0)`.
  - **`scf::r#if`** branching on `count_pos`:
    - Then: yield `emit_string_slice(src, i_norm, count)`.
    - Else: yield `emit_empty_string()`.

## Reuse

- ADR 0103 infrastructure: `lower_namespace_builtin_call`,
  `Builtin::from_namespace_method`, `extract_namespace_call`,
  `emit_string_case_map` (sibling pattern reference for malloc +
  memcpy + null-term).
- `emit_libc_call_i64` (strlen), `emit_libc_call_ptr` (malloc /
  memcpy) from `primitive.rs`.
- `emit_byte_offset_ptr_dynamic` (`primitive.rs:98`) for
  `src + offset` gep computation.
- `emit_f2i` (`emit.rs:8287`) for Number f64 → i64.
- `arith::cmpi` / `arith::select` / `arith::addi` / `arith::subi`
  / `arith::constant` (melior 0.27).
- `scf::r#if` with result-yielding regions (existing pattern at
  `emit.rs:8902` etc.).

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**: 3 helpers
  pre-extracted (empty/normalize/slice). String-slice helper is
  future-reusable for `string.find` etc.
- [x] **#2 TDD (Codex critical)**: 5 happy + 3 boundary + 2
  arity pin + 1 shadowing positive pin = 11 e2e in
  `tests/phase2_stdlib_string.rs`.
- [x] **#3 FP**: bounds normalization is pure SSA value-in/value-out
  (no control flow); effectful side isolated to strlen + malloc +
  memcpy + scf::if.
- [x] **#4 CA (Codex critical)**: `src/cli/`, `src/pipeline.rs`,
  `src/parser/`, `src/lexer/` zero-diff.
- [x] **#5 Security**: fptosi NaN UB documented as non-goal;
  malloc OOM carry-over documented; out-of-range / negative-i
  arithmetic safe (i64 two's-complement is wide enough for any
  reasonable string).
- [x] **#6 Documentation**: ADR 0104 doc + tagged-semantics §8
  row + AGENTS.md `‣ 2.7q-stdlib-string` row extended (same lane
  per ADR 0101→0102 math precedent).

## Test count delta

```
Step 0:  1078 → 1078 (11 Red Day 0 — 10 UndefinedName, 1
                      shadowing test happens to fall through
                      to user-table path naturally)
Step 1:  1078 → 1078 (Builtin variant; tests still Red)
Step 2:  1078 → 1078 (HIR dispatch + infer_kind; tests Red)
Step 3:  1078 → 1078 (3 helpers added; tests Red)
Step 4:  1078 → 1089 (emit arm + scf::if; 11 tests Red → Green)
Step 5:  1078 → 1089 (clippy + fmt; doc_lazy_continuation fix)
Step 6:  1078 → 1089 (docs only)

Final: 1078 → 1089 green, single atomic commit
  feat(hir,codegen,docs): string.sub + bounds-normalize helper (ADR 0104)
```

## Verification

- `cargo test --no-fail-fast` → **1078 → 1089**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/cli/ src/pipeline.rs src/parser/ src/lexer/` → **0**
- Manual smoke:
  ```bash
  echo 'print(string.sub("hello", 2, 4))
  print(string.sub("hello", -3))
  print(string.sub("hello", 1, 100))
  print(string.sub("hello", 10))
  print(string.sub("hello", 3, 1))' > /tmp/sub.lua
  cargo run --quiet -- compile /tmp/sub.lua && /tmp/sub
  # Expected: ell / llo / hello / (empty) / (empty)
  ```

## Future work

- **string.rep / reverse / find / match / gmatch / byte / char**
  — incremental.
- **`s:sub(i)` method syntax** — Phase 3 metatables.
- **UTF-8 / multi-byte handling** — Lua 5.4 `utf8.*` library.
- **malloc OOM null-check consolidation**.
- **NaN/Inf guards for f64 → i64 coercion** in string/math args
  (unify with ADR 0086 hash-key NaN policy).
- **`Builtin::arity()` range refactor** — when 3+ range builtins
  exist (today: Assert + StringSub = 2, on the boundary).
- **`emit_string_slice` reuse** by `string.find` / `string.match`
  capture extraction.
- **table.* / io.* libraries** — separate ADRs exercising the
  namespace generic dispatcher further.

## ADR number / phase tag

ADR 0104 = `string.sub` + bounds-normalize helper.
Phase tag: `2.7q-stdlib-string` (continues ADR 0103 sub-lane;
AGENTS.md row extended, not new row, per ADR 0101→0102 math
precedent).
