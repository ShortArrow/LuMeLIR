# やり残しロードマップ — 既存 ADR 残務整理

- **Date:** 2026-06-15
- **Scope:** 過去 ADR が残した未片付け項目を一覧化。Lua 5.4 conformance とは別系統で、「過去にした決定が未消化のまま残ってる」項目だけを対象。
- **State snapshot:** commit `854df26` (ADR 0196, tests 1431)
- **Methodology:** 各 ADR §Future work / §Non-goals / 部分実装の状態を grep + manual audit して分類。

## 整理視点

| Bucket | 内容 |
|---|---|
| A | **設計済 ADR + 実装未着手** (decision-only で止まってる) |
| B | **部分実装 ADR** (v1 safety mode 等で機械は動くが本来の効果が出てない) |
| C | **ADR §Future work bullets** (各 ADR の末尾で「次にやる」と書かれた未消化項目) |
| D | **ADR §Non-goals で deferred** (scope literal で ❌ にしたが実装が必要な項目) |
| E | **probe / TODO 未確認** (Lua 5.4 ロードマップで「probe 必要」と flag した項目) |
| F | **ADR 番号 misallocation** (元 plan で予約されてた番号が別件に使われ、本来の予定 ADR が未着工) |

## A. 設計済 ADR + 実装未着手

各々 decision-only で止まってる。実装に着手すれば multi-session。

| ADR | テーマ | Phase | impl 規模 |
|---|---|---|---|
| **0153** | `pcall` / `error` value propagation | 4a | 2-3 ADR |
| **0154** | `_ENV` / true globals | 4a | 2-3 ADR |
| **0155** | string patterns (find/match/gmatch/gsub) | 4a/4b | 3-4 ADR (pattern engine 自体が大型) |
| **0156** | GC architecture v1 (parent meta) | 4a | — (子 ADR で消化) |
| **0159** | GC mark phase | 4a | partial via 0185 v1 safety mode |
| **0160** | GC stack walk for alloca roots | 4a | 2-3 ADR (per-frame slot table = big) |
| **0161** | GC sweep phase | 4a | partial via 0185 v1 safety mode |
| **0162** | GC auto-trigger threshold | 4a | ✅ DONE in ADR 0186 |
| **0163** | `__gc` finalizer dispatch | 4a | 1-2 ADR (GC freeing 後) |
| **0164** | `__mode` weak tables | 4a | 2-3 ADR (GC 結合) |
| **0190** | Phase 2 epilogue defer (この ADR 自体は decision、defer 先 = Phase 4) | 4 | 全項目 deferred 状態 |
| **0192** | Embedded register-ops entry | 5+ | implementation 未着手 (user-triggered) |
| **0193** | Phase 4 entry criteria | 4 | benchmark harness のみ (ADR 0194) |
| **0195** | Phase 5 entry criteria | 5 | 全項目 implementation 未着手 |
| **0196** | Integer/Float subtype design entry | 4a | 10 sub-ADR (0197-0206) 未着工 |

**A 累計**: 13 ADR、合計 25-40 sub-ADR、25-40 session 規模。

## B. 部分実装 ADR

機械は動くが Lua spec / PRD 仕様には未到達。残作業 = 「safety mode 解除」「partial 拡張」。

| ADR | 状態 | 残作業 |
|---|---|---|
| **0185** GC mark + sweep v1 safety mode | 全 BLACK pre-mark で freeing 発火せず | DFS lift (ADR 0160 stack walk + 0159 worklist) を ADR 0190+ で実装 → 実 freeing 起動 |
| **0186** GC auto-trigger | 1 MiB trigger + doubling 動作 | 実 freeing 起動後の挙動検証 (現状は無意味) |
| **0152** string.format | `%d` / `%f` / `%s` / `%%` のみ | `%e` / `%g` / `%x` / `%o` / `%q` / width / precision 等の suffix 未対応 → 1-2 ADR |
| **0048** auto-declared globals | `_ENV` 機構の前段スタブ | ADR 0154 実装で正式置換 |
| **0157** GC allocator wrapper | string-obj alloc は GC-tracked | 全 alloc サイト migrate 済 (ADR 0158) — 完了扱い |
| **0184** gc_type_meta | `has_outgoing_refs: bool` のみ | per-type walk strategy enum 拡張 = ADR 0160 stack walk 実装時 |
| **0109** string.byte | single-position form `(s, i)` | multi-byte form `(s, i, j)` 未対応 |
| **0119** io.read | `*l` / `l` line input のみ | `*a` / `*n` / `*L` / number 等の format 未対応 |
| **0116** io.write | basic | file handle / `io.output` / variadic non-Number-String reject |
| **0104** string.sub | full | malloc OOM check (ADR 0112 で部分対応) |

**B 累計**: 10 ADR、合計 5-10 sub-ADR、5-10 session 規模。

## C. ADR §Future work bullets (未消化)

各 ADR 末尾の「次にやる」項目から、まだ消化されてないもの抜粋。完全リストは grep 必要だが代表例:

### Bridge sub-pieces (ADR 0191 §Future work)

| 項目 | 元 plan 番号 | 現状 | 推定 session |
|---|---|---|---|
| Type marshaling (String/Table/Bool/TaggedValue across boundary) | 0192 → renumber 必要 | 未着工 | 2-3 |
| Error propagation (Rust panic → Lua error、pcall 連動) | 0193 → renumber 必要 | 未着工 (pcall 0153 待ち) | 1-2 |
| GC interaction (Rust-held Lua refs、stack walk 連動) | 0194 → renumber 必要 | 未着工 (GC 実 freeing 待ち) | 2-3 |
| User-defined bridge crates (drop-in extensibility) | (TBD) | 未着工 | 2-3 |

注: ADR 0191 が予約してた番号 0192/0193/0194 は別件に使われた (Embedded entry / Phase 4 entry / benchmark harness)。Bridge sub-pieces 用に **0207+ 番号確保が必要**。

### Benchmark suite expansion (ADR 0194 §Future work)

| 項目 | 推定 |
|---|---|
| 追加 micro-benchmark (prime sieve / string concat / table ops) | 各 1 ADR |
| Regression-detection gate (baseline file + CI-failing assert) | 1 ADR |
| 統計的検定 (signif testing) | 1 ADR |
| Compile-time accounting (codegen 時間自体の計測) | 1 ADR |

### Phase 5 各 workstream (ADR 0195 §Workstreams)

| 項目 | 推定 ADR | session |
|---|---|---|
| Coroutines runtime | 3-5 | 4-6 |
| coroutine library | 1-2 | 1-2 |
| load / loadfile / dofile | 2-3 | 4-6 |
| require / package.* | 3-5 | 3-5 |
| CI matrix | 1 | 1-2 |
| Release artifacts | 1 | 1 |
| Security review | 1 | 2-3 |
| unsafe-block audit | 1 | 0.5-1 |

### Lua 5.4 conformance roadmap §Conformance gap (重複なので注釈のみ)

`docs/notes/lua54-conformance-roadmap.md` 参照。Phase 4a/4b/4c/4d/5 に分解済。

## D. ADR §Non-goals で deferred な実装必要項目

各 ADR が ❌ にした項目のうち、Lua spec 上は必要なもの:

| 元 ADR | deferred 項目 | 推定 |
|---|---|---|
| 0135 | `setmetatable(t, nil)` clear | 0.5-1 session |
| 0135 | `__metatable` field hiding | 0.5-1 session |
| 0103/0104/0105/0113/0152 | malloc OOM null-check 集約 (ADR 0157/0184 で string-obj 完了、table grow / hash grow / closure cell は範囲外) | 1-2 session |
| 0024 | malloc OOM 全サイト統一 | 0157 で部分対応 |
| 0111 | `table.insert(list, pos, value)` arity-3 (現状 arity-2 のみ) | 0.5-1 session |
| 0118 | table.remove arity-2 (explicit pos) — 確認要 | probe 後決定 |
| 0142 | `__tostring` chain (metatable.__tostring が自分の __tostring を持つ場合) | 1 session |
| 0143 | `__concat` 高次連鎖 | probe 後決定 |
| 0146 | `__call` chain (metatable の metatable...) | 1 session |
| 0147 | 算術 metamethod の chain (今 1-hop のみ?) | probe |
| 0148 | bitwise metamethod — Integer subtype 実装に lift される (ADR 0196) | A の 0196 で吸収 |
| 0167 | multi-hop chain の最大深度 (現 8 hop) — 動的 grow 不要か再評価 | 0.5 session |
| 0181 | Bool inference (Lua truthiness で false positive 回避困難) | 1 session (narrow scope) or 永久 defer |
| 0185 | v1 safety mode lift = ADR 0190+ 実装で吸収 | B 0185 と重複 |
| 0186 | `collectgarbage("setpause", n)` 設定可能化 | 1 session |
| 0186 | `collectgarbage("stop"/"restart"/"step"/"setstepmul")` | 1 session |
| 0157 | `__gc` / weak tables hook (ADR 0163/0164 内包) | A と重複 |
| 0191 | 非-Number Bridge signature | C の Bridge sub-pieces と重複 |

**D 累計**: 約 12 unique deferred items, 8-12 session 規模 (重複除外後)。

## E. probe / TODO 未確認 (Lua 5.4 ロードマップから)

`docs/notes/lua54-conformance-roadmap.md` §Open probes:

| probe | action | 推定 |
|---|---|---|
| Hex リテラル `0xFF` | lexer 読 + e2e probe | 30 min |
| 科学記法 `1e5` | 同上 | 30 min |
| Long strings `[[...]]` | 同上 | 30 min |
| Long comments `--[[...]]` | 同上 | 30 min |
| Bracket-key table `{[1]=v}` | parser probe | 30 min |
| Vararg `...` 関数本体 | parser/HIR probe | 30 min |
| Mid-block return | parser probe | 30 min |
| `next(t)` arity 1 (Lua spec は arity 1 OK、本実装は 2 強制) | ADR 0207 候補 | 1 ADR |

これらは 0.5-2 session で probe + 1-3 ADR にまとめられる。

## F. ADR 番号 misallocation — RESOLVED (2026-06-16)

ADR 0191 §Scope (literal) と §Future work の Bridge sub-piece 予約番号を current 状態に更新済 (commit TBD):

| 元予約 | 実際の使用 | 現在の状態 |
|---|---|---|
| 0192 Bridge marshaling | Embedded register-ops entry | ADR 0191 doc が `ADR (TBD, post-Phase-4a)` に書き換え済 |
| 0193 Bridge error propagation | Phase 4 entry criteria | 同上 (paired with pcall when triggered) |
| 0194 Bridge GC interaction | Benchmark harness MVP | 同上 (paired with GC stack walk) |

Bridge sub-piece ADR を着手する時に、その時点で next-available な番号を振る。番号予約方式を撤回し、相対参照 (`(TBD, post-Phase-4a)` 等) に統一。

## 統合ロードマップ (やり残し全体)

### 優先順位 1 — Phase 4a closeout (現在進行中)

```
ADR 0196 (DONE) Integer/Float design entry
  ↓
ADR 0197-0206 (10 ADR, 10-15 session) Integer/Float impl
ADR (TBD)      (2-3 ADR, 2-3 session) pcall/error impl per ADR 0153
ADR (TBD)      (2-3 ADR, 3-5 session) _ENV impl per ADR 0154
ADR (TBD)      (2-3 ADR, 3-5 session) GC stack walk + DFS lift per ADRs 0159/0160/0161
ADR (TBD)      (1-2 ADR, 1-2 session) __gc finalizer per ADR 0163
ADR (TBD)      (1 ADR, 1 session)     __close
ADR (TBD)      (1 ADR, 0.5 session)   goto/labels
ADR (TBD)      (2 ADR, 1-2 session)   <close>/<const> attributes
ADR (TBD)      (1 ADR, 0.5 session)   table named-key constructor {k=v}
ADR (TBD)      (1 ADR, 0.5 session)   next() arity 1
ADR (TBD)      (1 ADR, 0.5 session)   probe items closeout
```

**4a 残**: 約 26 ADR、25-37 session。

### 優先順位 2 — Phase 4b stdlib 拡張

```
ADR (TBD) string patterns per ADR 0155        (3-4 ADR, 4-6 session)
ADR (TBD) math 完成 (ceil/fmod/max/min/三角/random/定数) (3-5 ADR, 3-5 session)
ADR (TBD) table.pack/unpack/sort/move          (3-4 ADR, 3-4 session)
ADR (TBD) os library                            (2-3 ADR, 2-3 session)
ADR (TBD) utf8 library                          (2-3 ADR, 2-3 session)
ADR (TBD) string.format 拡張                    (1-2 ADR, 1-2 session)
ADR (TBD) io.open/lines (userdata 後)           (2-3 ADR, 2-3 session)
ADR (TBD) debug library subset                  (2-3 ADR, 2-3 session)
ADR (TBD) basic 残 (select/load/loadfile/dofile partial) (2-3 ADR, 4-6 session)
ADR (TBD) __mode weak tables impl per ADR 0164  (2-3 ADR, 2-3 session)
ADR (TBD) per-metamethod chain 改修             (3-5 ADR, 2-3 session)
ADR (TBD) malloc OOM 全サイト集約               (1-2 ADR, 1-2 session)
```

**4b 残**: 約 28 ADR、29-42 session。

### 優先順位 3 — Phase 4c extensibility + 4d runtime eval

```
ADR (TBD) userdata (Bridge との結合)              (2-3 ADR, 2-3 session)
ADR (TBD) Bridge type marshaling (0191 §Future)   (2-3 ADR, 2-3 session)
ADR (TBD) Bridge error propagation (0191 §Future) (1-2 ADR, 1-2 session)
ADR (TBD) Bridge GC interaction (0191 §Future)    (2-3 ADR, 2-3 session)
ADR (TBD) Bridge user-defined crates              (2-3 ADR, 2-3 session)
ADR (TBD) load family (4d)                        (2-3 ADR, 4-6 session)
ADR (TBD) require / package.* (4d)                (3-5 ADR, 3-5 session)
```

**4c/4d 残**: 約 16 ADR、16-24 session。

### 優先順位 4 — Phase 4 closing + Phase 5

```
Phase 4 close declaration (ADR 0193 update)        (0 ADR, 0.5 session)
ADR (TBD) Coroutines runtime (Phase 5)             (3-5 ADR, 4-6 session)
ADR (TBD) coroutine library (Phase 5)              (1-2 ADR, 1-2 session)
ADR (TBD) CI matrix (Phase 5)                      (1 ADR, 1-2 session)
ADR (TBD) Release artifacts (Phase 5)              (1 ADR, 1 session)
ADR (TBD) Security review (Phase 5)                (1 ADR, 2-3 session)
ADR (TBD) unsafe audit (Phase 5)                   (1 ADR, 0.5-1 session)
Phase 5 close declaration                          (0 ADR, 0.5 session)
```

**4 close + Phase 5 残**: 約 8 ADR、10-15 session。

### 優先順位 5 — benchmark 拡張 / production hardening 細目

```
ADR (TBD) prime sieve benchmark                    (1 ADR, 1 session)
ADR (TBD) string concat heavy benchmark            (1 ADR, 1 session)
ADR (TBD) table ops benchmark                      (1 ADR, 1 session)
ADR (TBD) Regression-detection gate                (1 ADR, 1 session)
ADR (TBD) Compile-time accounting harness          (1 ADR, 1 session)
ADR (TBD) Embedded impl (user-triggered)           (3-5 ADR, 3-5 session)
ADR (TBD) ADR 番号 misallocation 整理               (0 ADR, 0.5 session)
```

**bench/prod 残**: 約 8 ADR、8-12 session。

## 全体総計

### 元見積 (probe 前、roadmap 初稿時点)

| Bucket | ADR | session |
|---|---|---|
| A 設計済未実装 | 13 (decision-only meta) | 0 (実装は B/C 経由) |
| B 部分実装 | 10 | 5-10 (拡張のみ) |
| C §Future work bullets | 20 unique | 12-20 |
| D §Non-goals deferred | 12 | 8-12 |
| E probes | 7 | 4-7 |
| F 番号 fix | — | 0.5 |
| 統合 (重複除外) | **約 75-85 unique ADR** | **約 80-115 session** |

### 実測 (probe + 進捗反映後、2026-06-16)

| Bucket | DONE | 真の gap | session 残 |
|---|---|---|---|
| **A** 設計済未実装 | 1/13 (ADR 0197) | 12 (Integer/Float 残 9 sub-ADR + pcall/_ENV/GC freeing/__gc/weak tables) | 50-70 |
| **B** 部分実装 | 3/10 (0186/0157/0184 実は DONE) + 2 redirect (0185→A、0048→A) | 5 (0152/0109/0119/0116/0104 spec 拡張) | 5-10 |
| **C** §Future work bullets | 0/20 | 20 (Bridge sub-pieces + benchmark expansion + Phase 5 workstreams) | 12-20 |
| **D** §Non-goals deferred | 5/12 ([probe results](bucket-d-probe-results.md): setmetatable nil, table.insert arity-3, table.remove arity-2, setpause, __metatable hide DONE) | 4 confirmed + ~3 deep (__tostring/__concat/__call/__add partial, OOM aggregation, hop-depth review) | 4-8 |
| **E** probes | 7/8 ([probe results](bucket-e-probe-results.md): hex/sci/long-strings/long-comments/mid-block-return/next/bracket-key DONE) | 1 (E7 vararg) | 1-2 |
| **F** 番号 fix | DONE | 0 | 0 |
| **統合** (重複除外) | **約 18 DONE** | **約 50-60 真の gap** | **約 70-110 session** |

probe 駆動で **約 15-25 session 分** が "既に DONE" と判明 (元 roadmap が overcounted)。残作業見積もほぼ同じ規模で **3-6 ヶ月 focused work**。

probe しない B (部分実装は intent-by-design) や C (planned future work) は probe 駆動圧縮の余地が薄い。残作業の主体はそれら + bucket A (multi-session 大物群) + bucket D 残 + E7。

## 推奨着手順序

1. **Phase 4a 6-9 セッション**: ADRs 0197-0202 (Integer/Float 主要 6 ADR) で言語コア最大 gap を埋める
2. **Phase 4a 4-6 セッション**: pcall/error + GC stack walk + DFS lift で Phase 4a 主要 unblock
3. **Phase 4a 残 + 4b 並列**: small ADR (goto/`<close>`/table named key/next 等) を間に挟みつつ stdlib 拡張
4. **Phase 4c**: Bridge sub-pieces (userdata → marshaling → error → GC) ※ Phase 4a 完了後
5. **Phase 4d**: load + require (大型、後送り)
6. **Phase 4 close 宣言**
7. **Phase 5 突入**: Coroutines が最大、production hardening 並列
8. **Phase 5 close 宣言**

## 別視点 — やり残し最小化なら

**「全部はやらない」ロードマップ**もあり得る:

- Phase 4a まで完走して **「Lua 5.4 minimum 準拠 + AOT Bridge demo」** として 0.x release
- Phase 4b/4c/4d/5 は demand-driven (user/use-case 出現で着手)
- Embedded register-ops は **永久 user-triggered** (ADR 0192 で既にこの方針)

この場合 **30-40 session で release 1.0 候補** に到達可能。完璧主義より「minimum viable conformance」狙い。

## References

- [Lua 5.4 conformance roadmap](lua54-conformance-roadmap.md) — spec-gap 視点 (この doc と相補)
- [ADR 0133](../design/0133-phase2-completion-criteria.md) — Phase 2 close
- [ADR 0189](../design/0189-phase3-entry-criteria.md) — Phase 3 close
- [ADR 0193](../design/0193-phase4-entry-criteria.md) — Phase 4 entry
- [ADR 0195](../design/0195-phase5-entry-criteria.md) — Phase 5 entry
- [ADR 0196](../design/0196-integer-float-subtype-design.md) — Integer/Float subtype design entry
