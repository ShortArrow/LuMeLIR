# 0108. Phase 2.7r-stdlib-table: table.concat(t, sep, i, j) Full Spec + Cross-Namespace Bounds Helper Reuse

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-17
- **Deciders:** ShortArrow

## Replan provenance

ADR 0107 (`17bed9f`, 2026-05-17) extended `table.concat` to arity
`(1, 2)` and co-delivered the `Builtin::arity()` range refactor.
The full Lua 5.4 §6.8 signature is `table.concat(list, sep, i, j)`
— arity 3-4 with optional integer bounds remained explicit
future work. ADR 0108 closes that gap.

The architectural payoff is **cross-namespace reuse of
`emit_normalize_sub_bounds`** (the pure SSA bounds-normalize
helper extracted in ADR 0104 for `string.sub`). Lua §6.4 and
§6.8 specify identical negative-translate + clamp semantics for
i/j bounds. ADR 0104's helper was written without
string-specific assumptions; ADR 0108 reuses it verbatim in
`table.concat`, proving the abstraction was correctly factored.

Codex post-0107 6-視点 verdict (over 9 candidates — including
table.insert/remove, io.write, string.reverse/byte/find, malloc
OOM consolidation, broader arg-kind validation): **Refactor →
Go on A — `table.concat(t, sep, i, j)`**. Critical rationale:
- ADR 0104 bounds helper cross-namespace reuse → strongest Tidy
  First payoff among candidates.
- TableConcat arity in range-form `(1, 2)` post-0107; widening
  to `(1, 4)` is one tuple change.
- CA invariant preserved (`parser/lexer/cli/pipeline/tagged.rs`
  zero-diff).

```lua
print(table.concat({"a","b","c","d"}, "-", 2, 3))   -- → "b-c"
print(table.concat({"a","b","c"}, "-", -2))         -- → "b-c"
print(table.concat({"a","b","c","d"}, "-", 1, -2))  -- → "a-b-c"
print(table.concat({"a","b","c"}, "-", 2))          -- → "b-c"   (default j=#t)
print(table.concat({"a","b","c"}, "-", 2, 2))       -- → "b"     (i==j)
print(table.concat({"a","b","c"}, "-", 3, 1))       -- → ""      (i > j)
print(table.concat({"a","b"}, "-", 1, 100))         -- → "a-b"   (clamp j)
```

## Non-goals (top-of-ADR)

- **Other table.* fns** (`insert` / `remove` / `unpack` / `sort`
  / `pack` / `move`) — incremental.
- **Strict out-of-bounds error** — Lua 5.4 spec mandates a
  runtime error for out-of-range i/j. ADR 0108 follows ADR
  0104's `string.sub` precedent and **clamps to [1, #t]**
  instead. Documented as a deliberate string.sub-consistent
  deviation. A future arg-validation policy ADR may restore
  strict trapping uniformly across stdlib.
- **sep / i / j runtime type-trap** — non-String sep / non-Number
  i / non-Number j is carry-over from ADR 0104 / 0107 (silent
  UB at strlen / fptosi). Same future ADR.
- **malloc OOM / size overflow consolidation** — carry-over.
- **Number-stringify pass-2 cache** — MVP simplicity carry-over.
- **`ArityMismatch` richer error format** — keep substring-only
  assertion-compatible reporting.
- **io.* library** — separate ADR.

## Lua 5.4 §6.8 full semantics

```
table.concat(list, sep, i, j)
  defaults: sep = "", i = 1, j = #list
  if i < 0: i = #list + i + 1
  clamp i to >= 1
  if j < 0: j = #list + j + 1
  clamp j to <= #list
  if i > j after normalize: return ""
  else: list[i] .. sep .. list[i+1] .. ... .. sep .. list[j]
```

Bounds normalization formula is **identical** to `string.sub`
(Lua §6.4), so `emit_normalize_sub_bounds` (ADR 0104) reuses
verbatim.

## New surface

- **HIR `Builtin::arity(TableConcat)`** (`src/hir/ir.rs`):
  - `(1, 2)` → `(1, 4)`. One tuple change.
- **Codegen `TableConcat` emit arm extension**
  (`src/codegen/emit.rs`):
  - sep materialisation unchanged (ADR 0107 path).
  - Load `length` from t_ptr (used for default j and bounds
    normalize).
  - `i_raw`: `args.len() >= 3 ? emit_f2i(args[2]) : 1_i64`.
  - `j_raw`: `args.len() == 4 ? emit_f2i(args[3]) : length`.
  - `(i_norm, j_norm) = emit_normalize_sub_bounds(length,
    i_raw, j_raw)` — ADR 0104 helper, **cross-namespace
    reuse**.
  - `emit_table_concat_runtime(t_ptr, sep_ptr, sep_len,
    i_norm, j_norm)`.
- **Codegen `emit_table_concat_runtime` extension**
  (`src/codegen/emit.rs`):
  - Signature gains `i_norm, j_norm` (now 8 args; existing
    `#[allow(too_many_arguments)]`).
  - Internal `length` load dropped (bounds carry the info).
  - **Outer guard**: `len_pos = (length > 0)` →
    `range_nonempty = (j_norm >= i_norm)` (`arith::cmpi(Sge)`).
    Catches both empty-table and `i > j` after normalize.
  - **Carrier 0-based start**: `i_zero_start = i_norm - 1`
    (1-based-inclusive → 0-based array index).
  - **Pass 1 + Pass 2 carrier init**: `(0_i64, 0_i64)` →
    `(i_zero_start, 0_i64)`.
  - **Pass 1 + Pass 2 loop cond**: `i < length` → `i < j_norm`.
  - **Sep accounting**: `length - 1` → `j_norm - i_norm`
    (= range_count - 1, safe since `j_norm >= i_norm` ⇒
    `j_norm - i_norm >= 0`).
  - **Pass 2 sep check**: `i > 0` → `i > i_zero_start` —
    inserts sep before every non-first element of the bounded
    range (the carrier-init iteration `i == i_zero_start` skips
    sep prefix correctly).

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `emit_normalize_sub_bounds` (ADR 0104) | `src/codegen/emit.rs` | **cross-namespace reuse** — bounds clamp |
| `emit_table_concat_runtime` (ADR 0106/0107) | `src/codegen/emit.rs` | 2-pass concat (extended sig) |
| `emit_table_concat_dispatch_len` / `_dispatch_str` (ADR 0106) | `src/codegen/emit.rs` | tag dispatch (unchanged) |
| `emit_empty_string` (ADR 0104) | `src/codegen/emit.rs` | arity-1 empty sep + range-empty branch |
| `emit_f2i` (ADR 0022) | `src/codegen/emit.rs` | i/j Number f64 → i64 |
| `emit_byte_offset_ptr` / `_ptr_dynamic` | `src/codegen/primitive.rs` | gep helpers |
| `emit_libc_call_i64` (strlen) / `_ptr` (malloc/memcpy) | `src/codegen/primitive.rs` | string runtime |
| `Builtin::arity()` range form (ADR 0107) | `src/hir/ir.rs` | (1, 2) → (1, 4) |
| `lower_namespace_builtin_call` uniform check (ADR 0107) | `src/hir/mod.rs` | unchanged |

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**:
  cross-namespace reuse of ADR 0104 helper is the gate. No new
  helpers; existing helper picks up its 2nd consumer.
- [x] **#2 TDD (Codex critical)**: 8 new e2e + 1 test removal
  (arity_three_fails inverts). Existing 7 ADR 0107 + 8 ADR
  0106 + 11 ADR 0104 StringSub + Assert + StringRep + all other
  arity tests stay green (helper reuse must not regress
  string.sub).
- [x] **#3 FP**: emit arm purely orchestrates existing helpers.
  Bounds normalization stays pure (no new effects).
- [x] **#4 CA (Codex critical)**: `src/cli/`, `src/pipeline.rs`,
  `src/parser/`, `src/lexer/`, `src/codegen/tagged.rs`
  **zero-diff**.
- [x] **#5 Security**: bounds-clamp matches `string.sub`
  discipline (no new attack surface). NaN/Inf fptosi on i/j is
  same carry-over. Out-of-bounds clamp matches `string.sub`
  (deliberate deviation from strict Lua spec, documented).
- [x] **#6 Documentation**: ADR 0108 doc + tagged-semantics §8
  row + AGENTS.md `‣ 2.7r-stdlib-table` row extended (same
  lane per ADR 0106→0107 precedent).

## Test count delta

```
Step 0:  1113 → 1113 passing, corpus +7 net (8 new, 1 removed)
         - Day 0: 7 Red (happy/boundary that need bounds codegen)
         - 1 already-Green (arity-5-fails, rejected by current (1,2)
           upper bound)
         - 15 unchanged ADR 0106/0107 tests stay Green
Step 1:  no Δ (HIR arity widened; tests Red at codegen)
Step 4:  1113 → 1120 passing (codegen bounds extension; 7 Red →
         Green; 22 total table.concat tests + all other corpus)
Step 5:  1113 → 1120 (clippy + fmt)
Step 6:  1113 → 1120 (docs only)

Final: 1113 → 1120 green, single atomic commit
  feat(hir,codegen,docs): table.concat(t, sep, i, j) full spec + bounds helper reuse (ADR 0108)
```

## Verification

- `cargo test --no-fail-fast` → **1113 → 1120**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/cli/ src/pipeline.rs src/parser/ src/lexer/ src/codegen/tagged.rs` → **0**
- Manual smoke:
  ```bash
  echo 'print(table.concat({"a","b","c","d"}, "-", 2, 3))
  print(table.concat({"a","b","c"}, "-", -2))
  print(table.concat({"a","b","c","d"}, "-", 1, -2))
  print(table.concat({"a","b","c"}, "-", 3, 1))
  print(table.concat({"a","b"}, "-", 1, 100))' > /tmp/c.lua
  cargo run --quiet -- compile /tmp/c.lua && /tmp/c
  # Expected: b-c / b-c / a-b-c / (empty) / a-b
  ```

## Future work

- **table.insert / remove / unpack / pack / sort / move** —
  incremental.
- **Strict out-of-bounds trap** — coordinated cross-cutting
  arg-validation policy ADR.
- **sep / i / j runtime type-trap** — same policy ADR.
- **`ArityMismatch` richer error format** — diagnostic ADR.
- **Number-stringify ptr cache** — pass-1 stash to skip pass-2
  re-snprintf.
- **malloc OOM + alloc-size overflow consolidation** ADR.
- **NaN/Inf fptosi guards** unification with ADR 0086.
- **io.* library** — 4th generic-dispatcher consumer.
- **`emit_normalize_sub_bounds` 3rd consumer** — future
  `string.find` / `string.byte` / similar bounded slice
  builtins.

## ADR number / phase tag

ADR 0108 = `table.concat(t, sep, i, j)` full Lua 5.4 §6.8
spec + cross-namespace `emit_normalize_sub_bounds` reuse.
Phase tag: `2.7r-stdlib-table` (continues ADR 0106/0107
sub-lane; AGENTS.md row extended per same-lane precedent ADR
0103→0104→0105 string and ADR 0106→0107 table).
