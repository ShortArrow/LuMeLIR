# 0110. Phase 2.7t: Namespace Stdlib Arg-Kind Validation Policy

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-17
- **Deciders:** ShortArrow

## Replan provenance

ADR 0103-0109 で 7 ADR 連続して stdlib surface を拡張した結果、
**namespace builtin (math.* / string.* / table.*) の引数 kind
が HIR 段階でまったく検証されていない**歪みが明確に見える状態
になった:

- `lower_builtin_call` (global builtins): Assert / Error /
  ToNumber / Next などは per-builtin の arg-kind check 持ち
  (`src/hir/mod.rs:4484-4607`).
- `lower_namespace_builtin_call` (namespace builtins): ADR 0107
  以降は arity 範囲 check のみ。`string.len(42)` /
  `math.sqrt("x")` / `table.concat(123)` は HIR を素通りし
  codegen の `strlen(8)` / `fptosi(ptr)` 等で silent UB。

Codex post-0109 6-視点 verdict (10 候補): **A
(`namespace stdlib arg-kind validation policy`) を 0110 として
切る**を推奨。横断 safety baseline ADR。

```lua
string.len(42)              -- BuiltinArgKindMismatch (was: silent UB)
math.sqrt("hello")          -- BuiltinArgKindMismatch
table.concat(123)           -- BuiltinArgKindMismatch
table.concat({1,2}, 42)     -- BuiltinArgKindMismatch (sep)
string.sub("a", "b")        -- BuiltinArgKindMismatch (i)
```

## Non-goals (top-of-ADR)

- **Global builtin** (Print / Assert / Error / ToString /
  ToNumber / Type / Next) の検証強化 — 既に個別 check あり。
  本 ADR scope 外。将来 unification ADR で `param_kinds()`
  機構に統合可能 (各 global builtin は `param_kinds()` が
  `&[]` を返す: 既存個別 check 維持)。
- **TaggedValue arg の runtime trap** — `infer_kind` が
  `TaggedValue` を返すケース (table lookup / function param
  origin) は静的判定不可。MVP では skip (silent pass-through);
  後続 ADR で runtime tag-check chokepoint を追加
  (ADR 0089 `policy_for_tagged_arith_operand` precedent 踏襲).
- **String → Number coercion** — namespace builtin args は
  strict kind 要求 (Lua spec 準拠).
- **`ArityMismatch` 詳細化** — 別 ADR.

## 設計

### `Builtin::param_kinds() -> &'static [ValueKind]`

namespace builtin の per-position 期待 kind を pure static
data として宣言。Slice length = max arity (range-arity は
`.get(i)` で optional position が naturally skip)。

| Builtin | param_kinds slice |
|---|---|
| math.sqrt/floor/abs/sin/cos/log/exp | `[Number]` |
| math.pow | `[Number, Number]` |
| string.len/upper/lower | `[String]` |
| string.sub | `[String, Number, Number]` |
| string.rep | `[String, Number]` |
| string.byte | `[String, Number]` |
| table.concat | `[Table, String, Number, Number]` |
| Global (Print/Assert/Error/ToString/ToNumber/Type/Next) | `&[]` (個別 check 維持) |

### `HirError::BuiltinArgKindMismatch`

```rust
#[error("builtin '{builtin}' arg #{arg_index} expects {expected}, got {actual} (ADR 0110)")]
BuiltinArgKindMismatch {
    builtin: String,
    arg_index: usize,      // 1-based
    expected: String,      // ValueKind::name()
    actual: String,
    offset: usize,
}
```

`offset()` arm 拡張。

### `lower_namespace_builtin_call` 拡張

arity check 後に per-arg kind check:

```rust
let param_kinds = builtin.param_kinds();
for (i, lowered) in lowered_args.iter().enumerate() {
    let Some(&expected) = param_kinds.get(i) else { continue; };
    let actual = infer_kind(lowered, &self.locals, &self.functions);
    if matches!(actual, ValueKind::TaggedValue) { continue; }  // future ADR
    if actual != expected {
        return Err(HirError::BuiltinArgKindMismatch { ... });
    }
}
```

`ValueKind::name()` を `pub(crate)` に格上げ (HirError builder
からのアクセス用).

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**: scope を
  namespace stdlib に限定。Global builtin の既存個別検証は touch
  せず、将来統合の余地を残す。
- [x] **#2 TDD (Codex critical)**: 12 negative pins (math 3 +
  string 5 + table 4)。既存 34 positive tests stay green (探索
  で全 test が正しい kind を使うことを確認済み)。全 additive。
- [x] **#3 FP**: `param_kinds()` は pure static data; check
  logic は pure read + comparison + TaggedValue skip。
- [x] **#4 CA (Codex critical)**: `src/cli/`, `src/pipeline.rs`,
  `src/parser/`, `src/lexer/`, `src/codegen/` (emit + tagged +
  primitive) **zero-diff**。HIR-only ADR。
- [x] **#5 Security**: silent UB を 14 namespace builtins 全位置
  で解消 (TaggedValue arg を除く)。最大の safety baseline shift
  in stdlib lane。
- [x] **#6 Documentation**: ADR 0110 doc + tagged-semantics §8
  row + AGENTS.md NEW lane `‣ 2.7t-stdlib-arg-kind-validation`
  (Codex critical: 横断 ADR は lane ベースより policy ベース
  文書化が必要)。

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `infer_kind` | `src/hir/mod.rs` | per-arg static kind |
| `ValueKind::name()` (now pub(crate)) | `src/hir/mod.rs` | diagnostic strings |
| `HirError::ArityMismatch` pattern | `src/hir/error.rs` | new variant model |
| `lower_namespace_builtin_call` chokepoint (ADR 0107) | `src/hir/mod.rs` | extension site |
| `Builtin::arity()` (ADR 0107 range form) | `src/hir/ir.rs` | sibling data method |
| `Builtin::name()` / `ret_kinds()` precedent | `src/hir/ir.rs` | static slice return model |

## Test count delta

```
Step 0:  1129 → 1129 passing, +12 new negative pins Red Day 0
Step 1:  1129 → 1129 (HIR: param_kinds + HirError variant;
                       pins still Red — check not wired)
Step 2:  1129 → 1141 (lower_namespace_builtin_call extension;
                       12 Red → Green; existing 1129 stay green)
Step 3:  1129 → 1141 (clippy + fmt)
Step 4:  1129 → 1141 (docs only)

Final: 1129 → 1141 green, single atomic commit
  feat(hir,docs): namespace stdlib arg-kind validation policy (ADR 0110)
```

## Verification

- `cargo test --no-fail-fast` → **1129 → 1141**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/cli/ src/pipeline.rs src/parser/ src/lexer/ src/codegen/` → **0** (HIR-only ADR)
- Manual smoke:
  ```bash
  echo 'print(string.len(42))' > /tmp/bad.lua
  cargo run --quiet -- compile /tmp/bad.lua 2>&1 | head -5
  # Expected: BuiltinArgKindMismatch error
  ```

## Critical files

- `src/hir/ir.rs`:
  - `Builtin::param_kinds()` method (~60 LOC including doc)
- `src/hir/error.rs`:
  - `BuiltinArgKindMismatch` variant (~20 LOC including doc)
  - `offset()` arm extension (~1 LOC)
- `src/hir/mod.rs`:
  - `ValueKind::name()` visibility upgrade (`fn` → `pub(crate) fn`)
  - `lower_namespace_builtin_call` per-arg kind check (~20 LOC)
- `tests/phase2_stdlib_math.rs` — +3 negative pins
- `tests/phase2_stdlib_string.rs` — +5 negative pins
- `tests/phase2_stdlib_table.rs` — +4 negative pins
- `docs/design/0110-phase2-stdlib-arg-kind-validation.md` (NEW)
- `docs/design/tagged-semantics.md` §8 — 1 row
- `AGENTS.md` — NEW `‣ 2.7t-stdlib-arg-kind-validation` row

`src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/`,
`src/codegen/` (emit + tagged + primitive) **unchanged**.

## Future work

- **TaggedValue arg runtime tag-check chokepoint** — emit-side
  policy enum + scf::r#if trap (ADR 0089 precedent).
- **Global builtin unification** — Assert/Print/ToNumber/Next
  個別 check → `param_kinds()` 機構統合.
- **`ArityMismatch` 詳細化** — max bound 含む.
- **string.char / find / match / gmatch / reverse / format** —
  feature ADRs.
- **table.insert / remove / unpack / pack / sort / move** —
  mutation primitive ADRs.
- **math.pi / huge / maxinteger / mininteger** — Index-read
  chokepoint ADR.
- **malloc OOM consolidation** — pairs with alloc-heavy ADR.
- **NaN/Inf fptosi guards** — ADR 0086 unify.
- **io.* library** — 4th namespace consumer.

## ADR number / phase tag

ADR 0110 = namespace stdlib arg-kind validation policy
(cross-cutting safety baseline).
Phase tag: `2.7t-stdlib-arg-kind-validation` (NEW cross-cutting
lane independent from feature lanes `2.7q-stdlib-string` /
`2.7r-stdlib-table`).
