# 0109. Phase 2.7q-stdlib-string: string.byte(s, i?) + 3rd Consumer of emit_normalize_sub_bounds

- **Status:** Accepted
- **Date:** 2026-05-17
- **Deciders:** ShortArrow

## Replan provenance

ADR 0108 (`257c972`, 2026-05-17) proved cross-namespace reuse of
ADR 0104's `emit_normalize_sub_bounds` helper (string.sub →
table.concat). ADR 0109 adds **`string.byte(s, i?)`** as the
**3rd consumer** of the same helper, further hardening the
substring/bounds-normalization contract before any more variants
land. After this ADR the helper has been validated across three
distinct call shapes:

1. **Range slice** — `string.sub(s, i, j)` (ADR 0104)
2. **Join walk** — `table.concat(t, sep, i, j)` (ADR 0108)
3. **Single-position read** — `string.byte(s, i)` (ADR 0109)

Codex post-0108 6-視点 verdict (over 9 candidates): **Go on A —
`string.byte`**. Critical rationale:
- 3rd consumer settles the "is this abstraction general?"
  question without modifying the helper.
- Smallest-useful-cut: HIR builtin + 1 codegen arm + 9 e2e.
- Number-returning (sole bounds-normalize-family consumer with
  Number return; others return String).
- string lane natural continuation (`2.7q-stdlib-string`); no
  new lane, no `tagged.rs` touch.

```lua
print(string.byte("ABC"))        -- → 65   (default i=1)
print(string.byte("ABC", 2))     -- → 66
print(string.byte("ABC", -1))    -- → 67   (negative → from-end)
print(string.byte("ABC", -3))    -- → 65
print(string.byte("a"))          -- → 97
print(string.byte("ABC", 10))    -- → runtime trap (out-of-range)
```

## Non-goals (top-of-ADR)

- **`string.byte(s, i, j)` multi-byte form** — Lua spec returns
  multiple values when j is provided. Defer to a future
  multi-result-builtin ADR (likely jointly with `string.find` /
  `string.match`).
- **`string.char(...)`** — variadic. Separate ADR.
- **string.reverse / find / match / gmatch / format** —
  incremental.
- **Out-of-range → nil return** — Lua spec returns nil; we trap
  because the Number-return contract has no nil representation.
  Future multi-result/TaggedValue-return ADR may restore nil
  semantics uniformly across stdlib.
- **Strict i-kind validation** — non-Number i passes through
  `fptosi` → UB. Carry-over from ADR 0104/0108.
- **`s:byte(i)` method syntax** — Phase 3 metatables.

## Lua 5.4 §6.4 single-position semantics

```
string.byte(s [, i])
  default i = 1
  if i < 0: i = #s + i + 1
  if i < 1 OR i > #s: out-of-range
    Lua: return nil (no value)
    ADR 0109 MVP: runtime trap
  else: return byte code of s[i] (0-255 unsigned)
```

The single-position trick:
`emit_normalize_sub_bounds(len, i_raw, i_raw) → (i_norm, j_norm)`:
- In-range: `i_norm == j_norm == clamped i_raw` ∈ [1, #s]
- Out-of-range: helper's asymmetric clamp (i clamped UP to 1, j
  clamped DOWN to #s) splits them — `i_norm > j_norm` exactly
  when i_raw is past either boundary.

## New surface

- **HIR `Builtin::StringByte`** (`src/hir/ir.rs`):
  - `string_from_method("byte") → StringByte`.
  - `arity()` = `(1, 2)` — 5th range-arity builtin after Print,
    Assert, StringSub, TableConcat.
  - `name()` = `"string.byte"`.
  - `ret_kinds()` = `&[ValueKind::Number]` (extends or-pattern
    with `StringLen`).
- **HIR `infer_kind`**: Number-returning arm extended with
  `StringByte` alongside `MathSqrt | … | StringLen`.
- **Codegen diagnostic global** `s_string_byte_out_of_range`
  registered at module init: `"bad argument #2 to 'byte' (out of
  range)\0"` (matches `s_table_concat_bad_element` pattern from
  ADR 0106).
- **Codegen `StringByte` emit arm** (`src/codegen/emit.rs`,
  ~120 LOC):
  - Lower `args[0]` (s: String → ptr).
  - Materialise `i_raw`: `args.len() == 2 ?
    emit_f2i(args[1]) : const_i64(1)`.
  - `len = strlen(s)`.
  - `(i_norm, j_norm) = emit_normalize_sub_bounds(len, i_raw,
    i_raw)` — **single-position trick**, ADR 0104 helper reuse.
  - `in_range = (j_norm >= i_norm)`.
  - `scf::r#if (in_range)`:
    - Then: `byte_ptr = src + (i_norm - 1)`; `byte_i8 =
      load i8 at byte_ptr`; `byte_i64 = extui i8 → i64`;
      `byte_f64 = sitofp i64 → f64`. Yield.
    - Else: `emit_addressof(s_string_byte_out_of_range)` +
      `emit_exit_with_message` (diverges). Placeholder yield
      of `0.0_f64` for scf::r#if type-check.

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `emit_normalize_sub_bounds` (ADR 0104) | `src/codegen/emit.rs` | **3rd consumer reuse** — bounds clamp + out-of-range detect via single-position trick |
| `emit_libc_call_i64` (strlen) | `src/codegen/primitive.rs` | string length |
| `emit_byte_offset_ptr_dynamic` | `src/codegen/primitive.rs` | `src + (i - 1)` |
| `emit_load` (i8) | `src/codegen/primitive.rs` | byte read |
| `arith::extui` i8 → i64 | melior 0.27 | unsigned widen (0-255) |
| `emit_i2f` (ADR 0022) | `src/codegen/emit.rs` | i64 → f64 (Number return) |
| `emit_exit_with_message` | `src/codegen/primitive.rs` | trap on out-of-range |
| `emit_addressof` | `src/codegen/primitive.rs` | global ptr load |
| `emit_string_global` (init time) | `src/codegen/emit.rs` | new diagnostic global |
| `emit_f2i` (ADR 0022) | `src/codegen/emit.rs` | i arg f64 → i64 |
| `scf::r#if` | melior 0.27 | in-range / trap branch |
| `Builtin::arity()` range form (ADR 0107) | `src/hir/ir.rs` | (1, 2) tuple support |

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**: 3rd
  consumer of `emit_normalize_sub_bounds`. No helper modification;
  single-position trick reuses the helper verbatim. Validates
  the helper's generality across three distinct call shapes.
- [x] **#2 TDD (Codex critical)**: 9 e2e — 5 happy (default,
  explicit, neg-last, neg-first, single-char) + 1 boundary trap
  + 2 arity pins (0-arg, 3-arg) + 1 shadowing positive pin.
  Existing string + table + math regression coverage stays green.
- [x] **#3 FP**: pure helper reused; effectful side limited to
  strlen load + 1-byte load + optional trap. No new pure
  helpers (correctly avoided premature extraction).
- [x] **#4 CA (Codex critical)**: `src/cli/`, `src/pipeline.rs`,
  `src/parser/`, `src/lexer/`, `src/codegen/tagged.rs`
  **zero-diff**.
- [x] **#5 Security**: out-of-range trap is explicit (no silent
  UB). Lua spec deviation (nil → trap) documented as MVP
  consequence of Number-return contract.
- [x] **#6 Documentation**: ADR 0109 doc + tagged-semantics §8
  row + AGENTS.md `‣ 2.7q-stdlib-string` row extended (same
  lane per ADR 0103→0104→0105 precedent).

## Test count delta

```
Step 0:  1120 → 1121 passing, +9 corpus (1 shadowing Day-0 Green
                                          via index-callee fall-through)
Step 1:  1121 (HIR Builtin; tests Red at codegen)
Step 2:  1121 (infer_kind; tests Red)
Step 3:  1121 (diagnostic global; tests still Red — no emit arm)
Step 4:  1129 passing (emit arm; 8 Red → Green; corpus 1129)
Step 5:  1129 (clippy + fmt)
Step 6:  1129 (docs only)

Final: 1120 → 1129 green (+9 new tests, all passing).
Single atomic commit:
  feat(hir,codegen,docs): string.byte + 3rd consumer of bounds helper (ADR 0109)
```

## Verification

- `cargo test --no-fail-fast` → **1120 → 1129**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/cli/ src/pipeline.rs src/parser/ src/lexer/ src/codegen/tagged.rs` → **0**
- Manual smoke:
  ```bash
  echo 'print(string.byte("ABC"))
  print(string.byte("ABC", 2))
  print(string.byte("ABC", -1))
  print(string.byte("a"))' > /tmp/b.lua
  cargo run --quiet -- compile /tmp/b.lua && /tmp/b
  # Expected: 65 / 66 / 67 / 97
  ```

## Future work

- **`string.byte(s, i, j)` multi-byte form** — multi-result
  builtin; joint ADR with `string.find` / `string.match`.
- **`string.char(...)`** — variadic builtin.
- **string.reverse / find / match / gmatch / format** —
  incremental.
- **Out-of-range → nil return** — multi-result/TaggedValue
  return integration.
- **table.insert / remove / unpack / pack / sort / move** —
  separate mutation-primitive ADRs.
- **arg-kind validation policy ADR** — cross-cutting safety
  baseline (Codex's next priority after string.byte).
- **malloc OOM consolidation** — pairs with alloc-heavy feature.
- **NaN/Inf fptosi guards** unification with ADR 0086.
- **io.* library** — 4th generic-dispatcher consumer.

## ADR number / phase tag

ADR 0109 = `string.byte(s, i?)` single-position form + 3rd
consumer of `emit_normalize_sub_bounds`.
Phase tag: `2.7q-stdlib-string` (continues ADR 0103/0104/0105
sub-lane; AGENTS.md row extended per same-lane precedent).
