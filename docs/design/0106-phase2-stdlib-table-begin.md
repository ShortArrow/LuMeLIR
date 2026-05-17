# 0106. Phase 2.7r-stdlib-table: table.* Library Begin (table.concat arity 1)

- **Status:** Accepted
- **Date:** 2026-05-17
- **Deciders:** ShortArrow

## Replan provenance

ADR 0103 (`70775a3`, 2026-05-17) abstracted the namespace builtin
dispatch into `Builtin::from_namespace_method(ns, method)`, but
the generic dispatcher has only ever been exercised with two
namespaces — `math` and `string` (ADR 0103/0104/0105). ADR 0106
introduces the **`table.*` library lane** with the minimum
viable `table.concat(t)` (Lua 5.4 §6.8, arity 1 only — implicit
`sep=""`, no `i`/`j` bounds). The architectural payoff:

1. **Validates `from_namespace_method` for a third namespace**.
2. **Establishes the new `2.7r-stdlib-table` phase tag and
   AGENTS.md row** (Codex critical: independent lane, NOT
   extending `2.7q-stdlib-string`).
3. **Introduces a dedicated 2-pass multi-source string-assembly
   runtime helper** distinct from `emit_concat` (binary) and
   `emit_string_rep_runtime` (same-source × N).
4. **Strict Number-or-String element trap** — Lua 5.4 spec
   mandates runtime error on Bool/Nil/Table/Function elements.

Codex post-0105 6-視点 verdict: **Refactor → Go on Option A**.
Critical fixes baked in:
- Option A (arity 1) NOT B (arity 1+2 with sep) — adding sep
  would push us to 3 range-arity builtins (Assert + StringSub +
  TableConcat) and Codex would then demand the
  `Builtin::arity()` range refactor as a prerequisite.
- 2-pass dedicated helper (not iterated `emit_concat`, which
  would be O(N²)).
- Strict trap on non-Number/non-String (do NOT reuse
  `emit_tostring_tagged_local` — accepts Bool/Nil, spec
  violation).
- Number → string via `emit_tostring` Number arm (snprintf).
- New `s_table_concat_bad_element` diagnostic global.
- Empty-table → `emit_empty_string()` (ADR 0104 reuse).

```lua
print(table.concat({"a", "b", "c"}))   -- → "abc"
print(table.concat({1, 2, 3}))          -- → "123"
print(table.concat({1, "x", 2}))        -- → "1x2"
print(table.concat({}))                 -- → ""
print(table.concat({"only"}))           -- → "only"
print(table.concat({true}))             -- → runtime trap
```

## Non-goals (top-of-ADR)

- **`table.concat(t, sep)` arity 2** — separator support; future
  ADR (likely triggers `Builtin::arity()` range refactor).
- **`table.concat(t, sep, i, j)` arity 3-4** — bounds; future
  ADR (string.sub bounds pattern reusable).
- **Other table.* fns** (`insert` / `remove` / `unpack` / `sort`
  / `pack` / `move`) — incremental.
- **`Builtin::arity()` range refactor** — still deferred (Option
  A is fixed-arity 1).
- **Hash part elements** — Lua spec: `table.concat` walks the
  **array part only**. Hash part entries ignored.
- **non-Number/String → tostring coercion** — Lua spec mandates
  trap, not coercion.
- **malloc OOM / size overflow** — carry-over from ADR
  0103/0104/0105 alloc sites.
- **Re-stringify Numbers in pass 2 optimization** — accepted
  redundant snprintf; per-call temp intentionally leaked
  (no GC).

## Lua 5.4 §6.8 semantics (arity 1 form)

```
table.concat(list)
  -- equivalent to:
  --   list[1] .. list[2] .. ... .. list[#list]
  -- with implicit sep="" between elements.
  -- Each element must be Number or String; otherwise raise
  -- "invalid value in table for 'concat'" runtime error.
```

Byte-wise concatenation of the array part of `list`, indices
`1..#list`. Hash part entries are ignored. Empty array (`#list
== 0`) returns `""`.

## New surface

- **HIR `Builtin::TableConcat`** (`src/hir/ir.rs`):
  - Variant added.
  - `Builtin::table_from_method(method)` NEW constructor:
    `"concat"` → `TableConcat`.
  - `Builtin::from_namespace_method` extended: `"table"` arm.
  - `arity()` = 1 (fixed).
  - `name()` = `"table.concat"`.
  - `ret_kinds()` = `&[ValueKind::String]`.
- **HIR `infer_kind`**: String-returning or-pattern extended.
- **Codegen diagnostic global** `s_table_concat_bad_element`
  registered at module init (`emit_string_global`).
- **Codegen `emit_table_concat_runtime` helper** (~280 LOC):
  - Loads `length`, `array_buf` from table header.
  - `scf::r#if (length > 0)`:
    - Then:
      - Pass 1: `scf::r#while` carrier `(i, total)` over
        `0..length` accumulating `total_len`.
      - `buf = malloc(total + 1)`.
      - Pass 2: `scf::r#while` carrier `(i, offset)` over
        `0..length` copying bytes.
      - Null-terminate at `buf[total]`. Yield `buf`.
    - Else: yield `emit_empty_string()`.
- **Codegen `emit_table_concat_dispatch_len` / `_dispatch_str`
  private helpers**: tag-dispatch shape extracted to two
  private (file-scope) helpers shared by pass 1 (length only)
  and pass 2 (str_ptr + length). Codex critical: "private な分岐"
  is honored by file-scope private fns, not by top-level
  abstraction with multiple callers (Codex anti-pattern is
  cross-consumer extract; single-fn-internal extract is fine).
- **Codegen `TableConcat` emit arm**: 20-LOC plumbing — lower
  args[0], call `emit_table_concat_runtime`.

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `lower_namespace_builtin_call` | `src/hir/mod.rs:4633` | dispatch entry |
| `Builtin::from_namespace_method` | `src/hir/ir.rs` | generic dispatcher |
| `extract_namespace_call` | `src/hir/mod.rs:509` | shape walker |
| `TABLE_OFF_LEN` (=0), `TABLE_OFF_ARRAY_BUF` (=16) | `src/codegen/emit.rs` | header fields |
| `ARRAY_ELEM_SIZE` (=16), `ARRAY_ELEM_OFF_VALUE` (=8) | `src/codegen/tagged.rs` | slot layout |
| `TAG_NUMBER` (=1), `TAG_STRING` (=3) | `src/codegen/tagged.rs` | tag constants |
| `emit_byte_offset_ptr` / `_ptr_dynamic` | `src/codegen/primitive.rs` | gep |
| `emit_load` | `src/codegen/primitive.rs` | typed load |
| `emit_tostring` (Number arm) | `src/codegen/emit.rs` | Number → str via snprintf |
| `emit_libc_call_i64` (strlen) | `src/codegen/primitive.rs` | length |
| `emit_libc_call_ptr` (malloc, memcpy) | `src/codegen/primitive.rs` | alloc + copy |
| `emit_empty_string` (ADR 0104) | `src/codegen/emit.rs` | length==0 branch |
| `emit_exit_with_message` | `src/codegen/primitive.rs` | trap |
| `emit_addressof` | `src/codegen/primitive.rs` | global ptr load |
| `arith::cmpi / addi / muli / constant` + `scf::r#if / r#while` | melior 0.27 | int arith + control |

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First**: Option A arity-1 fixed
  avoids triggering arity refactor. 2-pass dedicated helper
  avoids `emit_concat` O(N²) anti-pattern (Codex critical:
  `emit_string_rep_runtime` precedent comments already noted
  `table.concat` is different shape).
- [x] **#2 TDD**: 8 e2e — 4 happy + 1 boundary + 1 trap + 1
  shadow + 1 arity. All Codex-critical edge pins present.
- [x] **#3 FP**: 2-pass effectful inside one helper boundary;
  tag-dispatch shape in two file-scope private fns
  (`_dispatch_len` / `_dispatch_str`) — not pure but scoped.
- [x] **#4 CA**: `src/cli/`, `src/pipeline.rs`, `src/parser/`,
  `src/lexer/`, `src/codegen/tagged.rs` **zero-diff**.
- [x] **#5 Security**: strict element type trap (Lua spec
  compliance, NOT optional). Size overflow / malloc OOM
  carry-over documented.
- [x] **#6 Documentation**: NEW lane `2.7r-stdlib-table` — new
  AGENTS.md row, new tagged-semantics §8 row, NEW test file.

## Test count delta

```
Step 0:  1098 → 1098 (8 Red Day 0 — 1 shadow passes via
                       index-callee fall-through)
Step 1:  1098 → 1098 (Builtin variant; Red)
Step 2:  1098 → 1098 (infer_kind; Red)
Step 3:  1098 → 1098 (diag global + 3 helpers; Red — non-exhaustive)
Step 4:  1098 → 1106 (emit arm; 7 Red → Green, 1 already-Green)
Step 5:  1098 → 1106 (clippy + fmt; #[allow(too_many_arguments)]
                       on the 2 private dispatch helpers)
Step 6:  1098 → 1106 (docs only)

Final: 1098 → 1106 green, single atomic commit
  feat(hir,codegen,docs): table.concat (arity 1) + table stdlib begin (ADR 0106)
```

## Verification

- `cargo test --no-fail-fast` → **1098 → 1106**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/cli/ src/pipeline.rs src/parser/ src/lexer/ src/codegen/tagged.rs` → **0**
- Manual smoke:
  ```bash
  echo 'print(table.concat({"a", "b", "c"}))
  print(table.concat({1, 2, 3}))
  print(table.concat({1, "x", 2}))
  print(table.concat({}))' > /tmp/concat.lua
  cargo run --quiet -- compile /tmp/concat.lua && /tmp/concat
  # Expected: abc / 123 / 1x2 / (empty)
  ```

## Future work

- **`table.concat(t, sep)`** — likely triggers `Builtin::arity()`
  range refactor (3rd range builtin).
- **`table.concat(t, sep, i, j)`** — full Lua spec (string.sub
  bounds pattern reusable).
- **table.insert / remove / unpack / pack / sort / move** —
  incremental.
- **Number-stringify ptr cache** — pass-1 stash to skip pass-2
  re-snprintf.
- **Generic `emit_concat_element_to_string_or_trap` helper** —
  extract if a second consumer (e.g., `table.unpack` over
  TaggedValue results) emerges.
- **`emit_string_slice` reuse** by `string.find` / `match` (carry).
- **malloc OOM + alloc-size overflow consolidation ADR**.
- **NaN/Inf fptosi guards** unification with ADR 0086.
- **io.* library** — 4th generic-dispatcher consumer.

## ADR number / phase tag

ADR 0106 = `table.concat` (arity 1) + table stdlib begin.
Phase tag: `2.7r-stdlib-table` (NEW sub-lane independent from
`2.7q-stdlib-string`, per Codex critical — same precedent as
`2.7q-stdlib-math` vs `2.7q-stdlib-string` split in ADR 0103).
