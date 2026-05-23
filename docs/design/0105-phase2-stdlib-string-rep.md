# 0105. Phase 2.7q-stdlib-string: string.rep (Fixed-Arity 2 Form)

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-17
- **Deciders:** ShortArrow

## Replan provenance

ADR 0104 (`d4e13a7`, 2026-05-17) added `string.sub` + the
bounds-normalization pure helper, completing the first
non-trivial string.* builtin. ADR 0105 extends the string.* lane
with **`string.rep(s, n)`** (Lua 5.4 §6.4) — the simplest of the
remaining string.* allocation builtins (`rep` / `reverse` /
`byte` / `char`).

Codex post-0104 6-視点 review verdict: **Refactor → Go**. Critical:
- `emit_string_rep_runtime` 1 effectful helper, inner copy-loop
  NOT extracted (no other caller for the "same-src N copies"
  shape today; `table.concat`'s "multiple distinct sources"
  shape is different).
- Fixed arity 2 only (variadic `sep` form deferred).
- `n * #s` overflow and malloc OOM documented as carry-over
  from existing string alloc sites (do not partial-harden).
- `n <= 0 → ""` via runtime branch (Lua spec compliance, no trap).
- `Builtin::arity()` range refactor NOT bundled (string.rep is
  fixed-arity 2; refactor stays deferred until 3+ range builtins).

```lua
print(string.rep("ab", 3))   -- → "ababab"
print(string.rep("ab", 0))   -- → ""
print(string.rep("ab", 1))   -- → "ab"
print(string.rep("ab", -1))  -- → ""    (Lua spec: n <= 0 → "")
print(string.rep("", 5))     -- → ""    (empty src × any n)
```

## Non-goals (top-of-ADR)

- **`string.rep(s, n, sep)` variadic form** — the 3-arg
  separator-joining form requires a join-loop different from the
  N-copy loop. Future ADR.
- **string.reverse / find / match / gmatch / byte / char / format**
  — incremental.
- **`s:rep(n)` method syntax** — Phase 3 metatables.
- **UTF-8 / multi-byte handling** — `string.rep` is byte-wise.
- **malloc OOM null-check consolidation** — carry-over from ADR
  0103/0104 alloc sites. Future ADR consolidates.
- **`n * #s` overflow check** — carry-over from existing string
  alloc sites; same alloc-and-leak policy.
- **`fptosi` on NaN / ±Inf for `n`** — UB in MLIR, documented as
  non-goal (matches ADR 0104 string.sub stance; could be unified
  with ADR 0086 NaN policy in a future ADR).
- **`Builtin::arity()` range refactor** — string.rep is fixed
  arity 2; doesn't trigger the refactor. Deferred until 3+ range
  builtins exist.

## Lua 5.4 §6.4 semantics

```
string.rep(s, n)
  if n <= 0: return ""
  else: return concat(s, s, ..., s)  -- n copies
```

Byte-wise; `#result = n * #s`. `s == ""` produces `""` regardless
of `n` (the loop runs but each `memcpy(_, src, 0)` is a no-op,
and `n * 0 = 0` → 1-byte buffer with just null term).

## New surface

- **HIR `Builtin::StringRep`** (`src/hir/ir.rs`):
  - `string_from_method`: `"rep"` → variant.
  - `arity()` = 2 (fixed, no range special case).
  - `name()` = `"string.rep"`.
  - `ret_kinds()` = `&[ValueKind::String]`.
- **HIR `infer_kind`**: extend String-returning or-pattern with
  `StringRep` alongside `StringUpper | StringLower | StringSub`.
- **Codegen `emit_string_rep_runtime` helper**
  (`src/codegen/emit.rs`, ~150 LOC):
  - `len = strlen(src)`, `count = fptosi(n_f64)`.
  - `count_pos = (count > 0)`.
  - **`scf::r#if`** branching:
    - Then: `total = count * len`; `buf = malloc(total + 1)`;
      **`scf::r#while`** carrier `i` over `0..count`:
      `dst = buf + i*len`; `memcpy(dst, src, len)`; `i += 1`.
      Null-terminate at `buf[total]`. Yield `buf`.
    - Else: yield `emit_empty_string()` (ADR 0104 helper reuse).
- **Codegen `StringRep` emit arm** (~30 LOC): pure plumbing —
  lower s + n, call `emit_string_rep_runtime`.

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `lower_namespace_builtin_call` | `src/hir/mod.rs:4633` | dispatch entry |
| `Builtin::string_from_method` | `src/hir/ir.rs:457` | namespace mapping |
| `extract_namespace_call` | `src/hir/mod.rs:509` | shape walker |
| `emit_libc_call_i64` (strlen) | `src/codegen/primitive.rs:227` | length |
| `emit_libc_call_ptr` (malloc, memcpy) | `src/codegen/primitive.rs:265` | alloc + copy |
| `emit_byte_offset_ptr_dynamic` | `src/codegen/primitive.rs:98` | `buf + offset` |
| `emit_f2i` | `src/codegen/emit.rs:8287` | f64 → i64 |
| `emit_empty_string` | `src/codegen/emit.rs:8463` (ADR 0104) | `n <= 0` branch |
| `arith::muli / addi / cmpi(Sgt/Slt) / constant` | melior 0.27 | int arith |
| `scf::r#if` | melior 0.27 | n<=0 vs positive branch |
| `scf::r#while` | melior 0.27 | copy-loop carrier |

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**: 1
  effectful helper `emit_string_rep_runtime`; copy-loop NOT
  extracted (no current second consumer).
- [x] **#2 TDD (Codex critical)**: 8 e2e — 1 happy + 4 boundary
  (n=0/1/2/empty-src/negative) + 2 arity pin (0-arg, 3-arg —
  the variadic-`sep` rejection) + 1 shadowing positive pin.
- [x] **#3 FP**: pure HIR dispatch; effectful logic isolated in
  the one helper.
- [x] **#4 CA (Codex critical)**: `src/cli/`, `src/pipeline.rs`,
  `src/parser/`, `src/lexer/` zero-diff.
- [x] **#5 Security**: `n * #s` overflow + malloc OOM + fptosi
  NaN UB all documented as carry-over (no partial-harden).
- [x] **#6 Documentation**: ADR 0105 doc + tagged-semantics §8
  row + AGENTS.md `‣ 2.7q-stdlib-string` row extended (same
  lane per ADR 0101→0102 / 0103→0104 precedent).

## Test count delta

```
Step 0:  1089 → 1098 (8 Red Day 0 — 1 shadow passes via
                       index-callee fall-through)
Step 1:  1089 → 1098 (Builtin variant; rep tests Red)
Step 2:  1089 → 1098 (infer_kind; tests Red at codegen)
Step 3:  1089 → 1098 (helper added; tests Red — non-exhaustive)
Step 4:  1089 → 1098 (emit arm; 8 Red → Green, +1 already-Green)
Step 5:  1089 → 1098 (clippy + fmt)
Step 6:  1089 → 1098 (docs only)

Final: 1089 → 1098 green, single atomic commit
  feat(hir,codegen,docs): string.rep + runtime copy-loop helper (ADR 0105)
```

## Verification

- `cargo test --no-fail-fast` → **1089 → 1098**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/cli/ src/pipeline.rs src/parser/ src/lexer/` → **0**
- Manual smoke:
  ```bash
  echo 'print(string.rep("ab", 3))
  print(string.rep("ab", 0))
  print(string.rep("ab", -1))
  print(string.rep("", 5))' > /tmp/rep.lua
  cargo run --quiet -- compile /tmp/rep.lua && /tmp/rep
  # Expected: ababab / (empty) / (empty) / (empty)
  ```

## Future work

- **`string.rep(s, n, sep)` variadic** — separator-joining form.
- **string.reverse / find / match / gmatch / byte / char** —
  incremental.
- **`s:rep(n)` method syntax** — Phase 3 metatables.
- **UTF-8** — Lua 5.4 `utf8.*` library.
- **malloc OOM + alloc-size overflow consolidation** —
  cross-cutting policy ADR.
- **NaN/Inf guards for fptosi** — unify with ADR 0086 hash-key
  NaN policy.
- **`Builtin::arity()` range refactor** — when 3+ range
  builtins exist.
- **table.* / io.* libraries** — separate ADRs.

## ADR number / phase tag

ADR 0105 = `string.rep` (fixed-arity 2 form).
Phase tag: `2.7q-stdlib-string` (continues ADR 0103/0104
sub-lane; AGENTS.md row extended).
