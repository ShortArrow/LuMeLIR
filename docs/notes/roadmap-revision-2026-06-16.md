# Roadmap Revision — 2026-06-16

- **Trigger:** Goal "A-F完了" set 2026-06-15、本 stretch で 12 ADR (0197-0208) 着地後、引き続き 70-100 session 残予測。Stop hook が継続要求する構造になっていたが、その大半は **trivial completeness ADR** (string.format spec letters / math constants 等) で、実際の **leverage 改善** (Integer/Float arc 中核 / GC stack walk / pcall / `_ENV`) は進捗 0 のまま。
- **問題認識:** 「A-F全完了」を session-scoped goal に置くと、最小コストで完了カウントを進められる項目に逃げる incentive が出る。実 value 改善は session-shape に乗らない (Integer/Float HIR = multi-session) ため、後回しになる構造的 bias。
- **結論:** 「A-F完了」を session goal として保持しない方が project value 最大化に資する。bucket counts を KPI とせず、**leverage 指標 × 完了 milestone** で再評価。

## 本 stretch (ADR 0197-0208) のレビュー

### 着地した 12 ADR の value 分類

| ADR | テーマ | 分類 |
|---|---|---|
| 0197 | Integer/Float Phase A (lexer) | **Leverage (small)** — Integer/Float arc の唯一の前進 |
| 0198 | next(t) arity 1 | Spec conformance |
| 0199 | Table constructor keyed fields | **Spec conformance (high)** — idiomatic Lua 即解放 |
| 0200 | collectgarbage("setpause", n) | Spec conformance |
| 0201 | string.reverse | Cosmetic stdlib |
| 0202 | string.format %x/%X | Cosmetic stdlib |
| 0203 | string.format %o/%g | Cosmetic stdlib |
| 0204 | string.format %e/%c | Cosmetic stdlib |
| 0205 | string.format %i alias | Cosmetic stdlib |
| 0206 | Benchmark string-concat | Measurement infra |
| 0207 | Benchmark table-ops | Measurement infra |
| 0208 | math.pi / math.huge | Spec conformance |

**実態:**
- **Leverage** (project の structural lock を解く): 1 件 (0197)
- **Spec conformance — idiomatic** (一般的 Lua code が動く): 3 件 (0198 / 0199 / 0208)
- **Spec conformance — niche** (めったに使わない spec carve-out): 2 件 (0200 / 0201)
- **Cosmetic stdlib** (format spec letters 等、深さなし): 4 件 (0202-0205)
- **Measurement infra** (Phase 4a の prerequisite): 2 件 (0206 / 0207)

### 観察

- **probe 駆動の bucket 圧縮** (bucket E / D で 5-6 件 "already done" 発見) は実 value — roadmap memo 更新の根拠を与えた。
- **format spec letter 連発** (0202-0205) は **scope-shrinking** であって **scope-deepening** ではない。ADR 1 本 ≈ 0.5 セッションで count を進められるが、project value は薄い。
- **table constructor keyed fields** (0199) は idiomatic Lua の relative 解放幅が大きい。`{name=v, [k]=v}` が動かないと真の Lua program は 1 つも書けない。high leverage。
- **Benchmark 拡張** (0206 / 0207) は Phase 4a § "no measurement floor → speculative" の前提条件をさらに固めた。今後の perf 最適化 ADR で baseline 比較可能。

## 新方針 (調整後)

### 原則 1: 「leverage 指標」を最上位に

各 ADR を着手前に以下で評価:

1. **Structural unlock?** — 他 ADR や future work の前提を解除するか
2. **Idiomatic Lua surface?** — 一般 Lua program で実際に使われる形を解放するか
3. **Spec gap closure?** — Lua 5.4 reference manual の明示的 gap を埋めるか
4. **Cosmetic completeness?** — alias / 細目で count しか進まない場合は backlog 行き

優先度: 1 > 2 > 3 > 4。 4 は「demand-driven backlog」に分類、user 要求が来るまで着手しない。

### 原則 2: Session-scoped goal を milestone ベースに

「A-F完了」(80-115 session) は session-scoped goal として不適。代わりに以下の **milestone** を順次 goal にする:

| Milestone | 推定 session | 内訳 |
|---|---|---|
| **M1** Integer/Float subtype 中核 | 6-10 | ADR 0196 sub-ADR 0209-0215 相当 (Phase B opt-in 完成、subtype-aware arithmetic + tostring) |
| **M2** pcall / error propagation | 2-3 | ADR 0153 実装 |
| **M3** GC actual freeing | 3-5 | ADR 0160 (stack walk) + 0190 (DFS lift) |
| **M4** `_ENV` / true globals | 3-5 | ADR 0154 実装 |
| **M5** Bridge sub-pieces (marshaling) | 2-3 | ADR 0191 §Future マーシャリング |
| **M6** Phase 4a 完了宣言 | 0.5 | meta-ADR doc |

M1 → M6 で **17-26 session, 2-3 ヶ月**。これは session-scoped 目標として shape OK。

### 原則 3: Cosmetic ADR は「opportunistic」扱い

format spec letter 系、math 個別関数追加、io.write file handle 拡張等は:
- それ自体を goal にしない
- 大きな ADR の合間に挟む息抜き or 周辺整理として 1-2 本/月
- backlog list で可視化、demand あれば即取りに行く

## 即時アクション

1. **Goal "A-F完了" の取り下げ推奨** — `/goal clear` で hook を停止し、`/goal M1` (= Integer/Float subtype 中核) 等の小スコープに置き換える。session-scoped に shape し直す。
2. **Cosmetic backlog 化** — 残 format spec (`%q`, width/precision) や `math.<fn>` 追加群は `docs/notes/cosmetic-backlog.md` に list 化、leverage 作業の合間 only に取りに行く。
3. **次の leverage ADR は ADR 0209 = HIR `ValueKind::Integer` 導入** (Phase B opt-in)。estimated 1.5-2 session。
4. **Bucket A 進捗測定方法を変更** — "ADR数/13" ではなく Integer/Float arc, pcall, `_ENV`, GC freeing 各 milestone の進捗を個別に追跡。

## 反省

- 「next」連発に対し 1 turn = 1 ADR ペースで応答した結果、本来 leverage がない work を多量生産した。これは ADR 0182 §3 (anti-ad-hoc) と矛盾する shape — "ADR を生むことが ADR になる" 状態。
- Goal hook 設計の意図 (= session 単位で進捗保証) と、目標 scope (= multi-month) の mismatch を 11 turn 続けて積み残した。再現性のある meta-pattern として認識すべき。
- 次回以降の Goal 設定は「1 session で物理的に達成可能」を必須条件にする。multi-month milestone は手前の m1-m2 (1-2 session に shape できる粒度) に分解してから goal hook に投げる。

## References

- [leftover-roadmap.md](leftover-roadmap.md) — bucket A-F 分類 (本 doc で session-scoped 評価軸を変更)
- [lua54-conformance-roadmap.md](lua54-conformance-roadmap.md) — spec gap 視点
- [sweep-0166-0177-retrospective.md](sweep-0166-0177-retrospective.md) — Codex 第3原則 anti-ad-hoc 系の precedent
- [sweep-0182-0188-retrospective.md](sweep-0182-0188-retrospective.md) — 10 lessons / extract-before-third-consumer
