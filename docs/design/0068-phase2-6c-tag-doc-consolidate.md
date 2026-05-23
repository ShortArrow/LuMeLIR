# 0068. Phase 2.6c-tag-doc-consolidate: tagged-semantics SoT Doc

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-05
- **Deciders:** ShortArrow

## Context

ADRs 0061–0067 added seven sub-phases of TaggedValue semantics
(slot layout, producer shapes, consumer dispatch, LIC tracking).
Each ADR carried its own LIC table, and each new sub-phase
modified the same conceptual model — the result is real but
spread across seven documents.

Codex review (post-ADR-0067) flagged the consequences:

- ADR 群へ知識が分散しすぎ、TaggedValue の **SoT が不在**。
- LIC tracker 12 行は **ADR 内重複**で limit に近い。
- `type` / `tostring` 等 consumer-side special-case が
  **暗黙知** に偏り、CA 観点の負債。
- TaggedValue の **producer/source taxonomy が未整理**。

優先順位は明確に「**実装より先に意味論を固定する**」。コード
変更は次フェーズに譲り、本フェーズは doc 1 本 + 軽量な ADR で
SoT を立ち上げる。

## Decision

### `docs/design/tagged-semantics.md` を SoT として導入

新規 doc に集約する内容:

1. **Slot layout** — `{i64 tag, 8-byte payload}` の正規化、tag
   value 一覧 (TAG_NIL=0..TAG_STRING=3, reserved 4-5)、payload
   type 表
2. **Producer / source taxonomy** — どの HIR shape が tagged
   slot を生む / 持つかの一覧 (Table, IndexAssign, IndexTagged,
   Local(TaggedValue), 将来: function-return / iterator)
3. **Consumer coverage matrix** — consumer × source-shape ×
   runtime-tag = 動作 の 3 軸表 (`print` / `type` / `tostring`
   / `==` / arith / cmp)
4. **LIC consolidation** — 12 entry を Resolved / Partial /
   Pending の 3 グループに集約
5. **Runtime tag invariants** — 6 つの不変条件 (tag at +0、
   payload type は tag identity に依存、defensive fallback の
   意味、etc.)
6. **Producer-Consumer cross-reference** — 「新 consumer / 新
   producer を加える時の手順」を明文化
7. **Open questions / known gaps** — Codex 推奨ロードマップ
8. **ADR index** — chronological ADR 一覧 + contribution の
   一文

### 運用ルール

- ADR は引き続き **decision を記録**。
- `tagged-semantics.md` は **現行状態の SoT**。
- 新 sub-phase (ADR 0069+) を起こす際の責務:
  1. ADR で decision を記録 (今まで通り)
  2. tagged-semantics.md の該当 section を更新 (producer / consumer
     / LIC / ADR index)
  3. ADR の Consequences で「tagged-semantics.md updated」と
     明示 (drift 防止)
- LIC tracker は **doc 側に集約**、ADR からは「tagged-semantics.md
  §4 を参照」で済ます。ADR 0068 自身は最後の "duplicated LIC table"
  になる (歴史的 trace のため)。

### AGENTS.md 更新

- 2.6c-tag-doc-consolidate フェーズ row 追加
- "Required Reading" に tagged-semantics.md を Phase 2.6c 前提
  として追加

### CA invariants preserved

| Layer    | Change                          |
|----------|---------------------------------|
| Lexer    | None                            |
| Parser   | None                            |
| AST      | None                            |
| HIR      | None                            |
| Codegen  | None                            |
| Tests    | None (819 green を維持)         |
| Docs     | +tagged-semantics.md, AGENTS    |

## Alternatives Considered

- **LIC tracker を全 ADR で重複保持し続ける.** 各 ADR が自己
  完結するメリットはあるが、ADR 0067 時点で既に同じ表を 7 回
  書き写しており、drift リスクが顕在化。Reject。
- **ADR 0068 を hub にして他 ADR をリンクで束ねる.** ADR は
  historical な decision、SoT は current state。役割を分離する
  方が clean。Reject。
- **rustdoc / module-level docstring に集約.** `src/codegen/`
  か `src/hir/` に `//!` doc を置く案。markdown より遠く、ADR
  workflow との親和性が低い。一方 SoT として日本語 / 表組みが
  自由。Reject。
- **Phase 2.6c 完了まで集約を待つ.** 現状でも 7 ADR — 追加 sub-
  phase を待つほど統合コストは増える。Codex 指摘どおり今着手。

## Consequences

- **819 tests green を維持** (コード変更なし)。
- 新 ADR (0069+) は LIC tracker 重複を廃止、SoT doc の更新
  責任を負う。
- AGENTS.md "Required Reading" に tagged-semantics.md が
  Phase 2.6c 前提として追加 (新参加者の onboarding コスト削減)。
- doc drift リスクは ADR template への運用ルール組み込みで
  mitigate (本 ADR で運用を明示)。
- Codex 指摘の "consumer special-case が暗黙知" を表化することで
  顕在化、次の sub-phase (function-return widening 等) が doc
  の section を更新する形で進む。

## Out of Scope

- **`tagged.rs` module split** — Codex roadmap #3 (本 doc の §7
  に open question として記録)。SoT が固まった後の方が clean。
- **コード変更 / helper 抽出** — 本フェーズは doc only。
- **新 LIC エントリ追加** — 12 行の整理のみ。新 LIC は次の
  sub-phase で。
