# 0114. Phase 2.7w-emit-f2i-gate-sweep: NaN/Inf/integer gate sweep + emit_trap_if consolidation

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-22
- **Deciders:** ShortArrow

## Replan provenance

ADR 0113 (`6de3812`, 2026-05-22) で `emit_check_byte_arg`
chokepoint と `emit_trap_if(cond, msg_global)` generic helper
の precedent が確立、`string.char` 1 site で NaN/Inf/integer
gate を実装。本 ADR は同 pattern を **未保護 7 + 3 site** の
raw `emit_f2i` 呼び出しに sweep + ADR 0086 hardcoded helper の
generic 化 migration を bundle。

Codex post-0113 6-視点 review verdict: **M (A + B bundle) =
Strong Go** (over single-purpose A 単独 / multi-result builtin
C / printf-like D / patterns F / pcall K)。
- A = `emit_f2i` NaN/Inf/integer gate sweep (10 sites)
- B = `emit_table_index_nan_trap_if (hardcoded "s_table_index_nan")`
  を `emit_trap_if(cond, "s_table_index_nan")` 上に畳む migration
  (3 callers + 1 helper deletion)

なぜ M bundle が strong:
1. **0113 precedent 確立済** — `emit_check_byte_arg`
   (`src/codegen/emit.rs:9519-9622`) で range + integer gate
   pattern が動作、`emit_trap_if`
   (`src/codegen/emit.rs:9486-9491`) が generic helper として
   導入済。
2. **B 単独は trigger 不足** — ADR 0086 helper 1 個の migration
   は cleanup level、stand-alone ADR には小さすぎる (codex
   critical)。A の途中 tidy として吸収する non-ad-hoc。
3. **Security debt 返済** — `emit_f2i` (`src/codegen/emit.rs:8861-8876`)
   は raw `arith.fptosi` のまま、NaN/Inf UB を抱えた caller が
   7 + 3 site 残存。ADR 0113 doc が "range gate 後にのみ
   `emit_f2i` を呼ぶことで NaN/Inf UB を解消した" と precedent
   declared 済。
4. **Lua 5.4 §3.4.2 compliance** — bitwise operators: "no integer
   representation → error" は spec mandate (silent fptosi UB は
   spec violation)。

## Non-goals (top-of-ADR — codex critical)

- **`string.byte(s, i, j)` multi-byte form** — multi-result
  builtin policy が必要、`Builtin::ret_kinds` static slice は
  arity-dependent return count に不適合。独立 framework ADR。
- **`string.format`** — printf-like scope explosion。
- **`string.find/match/gmatch`** — Lua patterns runtime。
- **`table.remove`/`unpack`/`pack`/`sort`/`move`** — feature
  lane、本 ADR は cross-cutting Tidy First。
- **`pcall`/`error` 値伝播** — cross-cutting state machine、
  時期尚早。
- **OOM consolidation 全方位** — ADR 0112 で意図的 scope 外。
- **Per-caller policy enum (`integer-required` vs `truncate-ok`)**
  — 全 caller が `integer-required` で揃うので enum 不要 (Lua
  spec uniform)。将来 truncate-ok caller が増えた時に再考。
- **`emit_check_byte_arg` 統合** — 0113 helper は range check
  も含む string.char 専用、本 ADR の `emit_check_integer_arg`
  と sibling として併存。

## Lua 5.4 spec compliance

- **§3.4.2 bitwise**: "All bitwise operations convert their
  operands to integers... If a number value does not have a
  proper integer representation, a runtime error is raised."
- **§6.4 string**: index/count args は integer per
  `lua_Integer` (`luaL_checkinteger` ref impl)。
- **§6.8 table**: bounds / pos args は integer per
  `lua_Integer`。
- **Deviation**: spec で "runtime error"、本実装は
  `emit_exit_with_message` (exit(1) + printf) で trap (既存
  precedent + `pcall` 対応 ADR で再考可)。

## 設計

### 1. New chokepoint `emit_check_integer_arg`

```rust
/// Validate one f64 arg as integer-valued Number, return f2i'd i64.
/// (1) Finite check via `(x - x) == 0.0` (NaN/±Inf → NaN ≠ 0)
/// (2) Integer check via `x == libm floor(x)` (safe on finite x)
/// (3) f2i: only on validated finite-integer x
/// Single trap branch with caller-supplied diagnostic global.
fn emit_check_integer_arg(context, block, arg_f64, msg_global, types, loc) -> Value
```

設計判断:
- **Single trap** (vs 0113 2-trap split) — Lua ref impl は
  "number has no integer representation" 1 message で NaN/Inf/
  non-integer 全 case をカバー。0113 split は range vs integer
  の別 message のため本 ADR とは別 policy。
- **`x - x == 0` finite idiom** — NaN-NaN = NaN; Inf-Inf = NaN;
  finite-finite = 0。`cmpf Oeq` Ord で 1 比較で NaN/±Inf 自然
  reject。
- **`libm floor` reuse** — ADR 0101 declared、FloorDiv /
  lua_mod / `emit_check_byte_arg` 既存 caller。
- **`emit_trap_if` reuse** — ADR 0113 generic helper。

### 2. 6 new diagnostic globals

`emit_string_global` (boxed object form per ADR 0112):

| Global | Message | Used at (post-0114) |
|---|---|---|
| `s_string_byte_non_integer` | "bad argument #2 to 'byte' (number has no integer representation)" | string.byte i |
| `s_string_sub_non_integer` | "bad argument to 'sub' (number has no integer representation)" | string.sub i, j |
| `s_string_rep_non_integer` | "bad argument #2 to 'rep' (number has no integer representation)" | string.rep n |
| `s_table_concat_non_integer` | "bad argument to 'concat' (number has no integer representation)" | table.concat i, j |
| `s_table_insert_non_integer` | "bad argument #2 to 'insert' (number has no integer representation)" | table.insert pos |
| `s_bitwise_non_integer` | "number has no integer representation" | bitwise lhs/rhs/bnot |

Per-builtin global で error message を user-meaningful に保つ
(ADR 0109 `s_string_byte_out_of_range` precedent と同 style)。

### 3. 10 sites swap

| # | Site | Builtin | Global |
|---|---|---|---|
| 1 | string.byte `i` | StringByte | `s_string_byte_non_integer` |
| 2 | string.sub `i` | StringSub | `s_string_sub_non_integer` |
| 3 | string.sub `j` | StringSub | `s_string_sub_non_integer` |
| 4 | table.concat `i` | TableConcat | `s_table_concat_non_integer` |
| 5 | table.concat `j` | TableConcat | `s_table_concat_non_integer` |
| 6 | table.insert `pos` | TableInsert | `s_table_insert_non_integer` |
| 7 | string.rep `n` | StringRep | `s_string_rep_non_integer` |
| 8 | BinOp::Bit* lhs | bitwise | `s_bitwise_non_integer` |
| 9 | BinOp::Bit* rhs | bitwise | `s_bitwise_non_integer` |
| 10 | UnaryOp::BitNot | bitwise | `s_bitwise_non_integer` |

`emit_f2i(block, x_f64, types, loc)` →
`emit_check_integer_arg(context, block, x_f64, "s_<family>_non_integer", types, loc)`.

### 4. `emit_table_index_nan_trap_if` → `emit_trap_if` migration

3 + 1 callers (table-index NaN guards via `cmpf Une self self`):
- IndexAssign NaN preflight (emit.rs)
- IndexRead NaN preflight (emit.rs)
- IndexTagged NaN preflight (emit.rs)
- `emit_hash_key_runtime_validity_gate` 内の 1 callsite

`emit_table_index_nan_trap_if(context, block, is_nan, types, loc)`
→ `emit_trap_if(context, block, is_nan, "s_table_index_nan", types, loc)`.

`emit_table_index_nan_trap_if` 関数自体は削除 (legacy helper)。
docstring references も update。

## Reuse (file:line citations)

| Helper | Path | Purpose |
|---|---|---|
| `emit_trap_if` (ADR 0113) | `src/codegen/emit.rs:9486-9491` | generic msg_global trap |
| `emit_f2i` | `src/codegen/emit.rs:8861-8876` | f64 → i64 (post-gate) |
| `emit_libm_call` ("floor") | `src/codegen/emit.rs:1067` (declared) | integer check |
| `emit_string_global` (ADR 0112) | `src/codegen/emit.rs:532-538` | diagnostic global emit |
| `emit_addressof` | `src/codegen/primitive.rs` | global ptr |
| `arith::subf` / `cmpf` / `andi` / `xori` | melior 0.27 | finite + integer SSA |

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First**: cross-cutting Tidy First
  lane。0113 precedent 直接活用、`emit_table_index_nan_trap_if`
  を generic helper 上に畳む B migration を A の途中 tidy として
  bundle。
- [x] **#2 TDD**: 17 e2e — 10 sites × NaN + Inf + 1.5 trap pin。
- [x] **#3 FP**: pure policy 単一 (integer-required, no truncate
  branch)。effectful は emit_check_integer_arg 1 chokepoint。
- [x] **#4 CA**: `src/cli/`, `src/pipeline.rs`, `src/parser/`,
  `src/lexer/`, `src/codegen/primitive.rs`, `src/codegen/tagged.rs`,
  `src/hir/` **zero-diff**。codegen 内 hardening のみ。
- [x] **#5 Security**: 10 NaN/Inf UB site を一括解消。Lua §3.4.2
  bitwise compliance。
- [x] **#6 Documentation**: NEW lane `‣ 2.7w-emit-f2i-gate-sweep`
  (cross-cutting Tidy First lane、2.7v string.char feature lane
  と別カテゴリ)。Lua §3.4.2 spec compliance 明示。
  tagged-semantics.md は §8 row のみ。

## Test corpus delta

- `tests/phase2_stdlib_string.rs`: 5 new e2e (~50 LOC)
- `tests/phase2_stdlib_table.rs`: 4 new e2e (~50 LOC)
- `tests/phase2_2c_floor_and_bitwise.rs`: 8 new e2e (~120 LOC)

**Final: 1181 → 1198 green (+17)** (16 Red Day 0 + 1 Day-0 Green
via fptosi(+Inf) UB coincidence — 全 post-fix で proper trap
経由 Green)。

## Risks

| Risk | Mitigation |
|---|---|
| NaN/Inf UB 依存テストが存在 | full corpus run + 1181 stay green 確認 → 該当なし。 |
| `cmpf Oeq(x-x, 0.0)` NaN 動作 | NaN-NaN = NaN; Oeq(NaN, 0.0) = false (Ord) → trap ✓ |
| `cmpf Oeq(±Inf, libm_floor(±Inf))` | floor(Inf) = Inf, Oeq(Inf, Inf) = true。但し finite check で先 reject (Inf-Inf = NaN ≠ 0)。 |
| `string.rep(s, 0)` / `(s, -1)` 動作 | integer pass → 既存 count_pos branch で空文字列、動作不変。 |
| `emit_table_index_nan_trap_if` 削除で漏れ callers | grep 確認後 0 callers、helper 削除。 |

## Future work (carry-over)

- **ADR 0115 候補**: `string.byte(s, i, j)` multi-byte form
  (`Builtin::ret_kinds` arity-dependent への framework 拡張、
  multi-result builtin policy)。
- **ADR 0116 候補**: `table.remove(t [, pos])` (table.insert
  mirror、ADR 0114 で integer gate 既設済)。
- **ADR 候補**: `string.format` / `reverse` / `find` / `match` /
  `gmatch`。
- **ADR 候補**: `math.pi` / `huge` / `maxinteger` / `mininteger`
  constants。
- **ADR 候補**: io.* library (4th generic-dispatcher consumer)。
- **`pcall` / `error` 値伝播** — cross-cutting ADR。
- **OOM consolidation 全方位** — table grow / hash grow /
  closure cell。

## Phase tag

`2.7w-emit-f2i-gate-sweep` (cross-cutting Tidy First lane、
2.7v stdlib-string-char feature lane とは別カテゴリ; codex
critical "feature lane に寄せず独立 tidy-first lane")。
