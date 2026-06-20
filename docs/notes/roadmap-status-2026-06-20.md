# Roadmap Status — 2026-06-20

差分整理。元 doc: [roadmap-revision-2026-06-16.md](roadmap-revision-2026-06-16.md)。M1 minimal close + M2-M6 残量再確認。

## Roadmap doc 階層 (canonical 順)

| Doc | 役割 | 鮮度 |
|---|---|---|
| [**roadmap-status-2026-06-20.md**](roadmap-status-2026-06-20.md) (本 doc) | 最新進捗 + 次 goal 推奨。`/goal` 設定時に最初に読む | ✅ 鮮 |
| [roadmap-revision-2026-06-16.md](roadmap-revision-2026-06-16.md) | 方針 — milestone ベース goal + leverage 原則 | ✅ 鮮 |
| [lua54-conformance-roadmap.md](lua54-conformance-roadmap.md) | Lua 5.4 spec gap 視点 | ✅ 鮮 |
| [cosmetic-backlog.md](cosmetic-backlog.md) | opportunistic 行先 (format spec, math fn 等) | ✅ 鮮 |
| [leftover-roadmap.md](leftover-roadmap.md) | bucket A-F comprehensive snapshot | ⚠ stale (2026-06-15 時点) — 俯瞰用のみ |
| [bucket-d-probe-results.md](bucket-d-probe-results.md) / [bucket-e-probe-results.md](bucket-e-probe-results.md) | probe driven 圧縮の根拠 | 履歴 |
| [sweep-0166-0177-retrospective.md](sweep-0166-0177-retrospective.md) / [sweep-0182-0188-retrospective.md](sweep-0182-0188-retrospective.md) | meta-pattern 学び | 履歴 |

## M1 (Integer/Float subtype 中核) — **6/6-10 minimal close**

`/goal M1` で 6 sub-ADR が連続着地。range 下限到達で goal hook 仕様上は完了扱い可。

| ADR | テーマ | 値 |
|---|---|---|
| 0209 | Integer AST + HIR variant + sitofp 葉 demotion | Phase B opt-in entry |
| 0210 | `math.type(x)` static-shape (integer / float / nil) | First user-visible subtype distinction |
| 0211 | `math.tointeger(x)` static-shape | Sibling of 0210 |
| 0212 | `math.maxinteger` / `math.mininteger` 定数 | Integer-kind constants |
| 0213 | Integer+Integer BinOp constant fold (Add/Sub/Mul/FloorDiv/Mod/Bitwise/Shifts) | 静的算術 subtype 保存 |
| 0214 | `print(Integer)` で `%lld` printf | precision preservation + `.0` artifact 除去 |

**Test 数:** 1464 → 1478 (+14)。zero regression。

### M1 残オプション (deferred、Phase C 入口)

leverage は依然高いが session shape 不安定 (125-site ValueKind::Number 経由の slot/Local 拡張が要)。次 goal に閉じ込めるか、より小さい sub-milestone に再分解する。

- **M1-extended A** Runtime Integer-tagged TaggedValue slot (ValueKind::Integer or LocalInfo.subtype field) — 2-4 session
- **M1-extended B** `tostring(integer)` precision preservation — 0.5-1 session (M1-A 完了が前提)
- **M1-extended C** `io.write(integer)` `%lld` arm (Print arm の mirror) — 0.5 session、leverage 低、cosmetic backlog 移動候補
- **M1-extended D** Mixed Integer/Float arithmetic Phase C codegen (i64 ops、`+ - * // %` 整数同士、`/` `^` は float) — 3-5 session、Phase C entry
- **M1-extended E** Integer 比較 (`==`, `<`, `<=`) subtype-aware path — 1-2 session

Total M1-extended: 7-12.5 session — M1 を range 上限まで完走するなら 13-22.5 session 累計。session goal としては大きすぎるので **M2 を先に取りに行く** ことを推奨。

## 残 milestone (M2-M6) 再評価

| Milestone | 推定 session | 前提 / 観察 |
|---|---|---|
| **M2** pcall / error propagation | 2-3 | ADR 0153 実装。前提 0 (M1 不依存)。最も leverage 高 — error throw が落ちる stdlib が一気に増える |
| **M3** GC actual freeing | 3-5 | ADR 0160 (stack walk) + 0190 (DFS lift)。Phase 2 GC trigger 無発火 invariant を保ったまま runtime collector 起動 |
| **M4** `_ENV` / true globals | 3-5 | ADR 0154。global hash-slot path の重ね書き |
| **M5** Bridge marshaling | 2-3 | ADR 0191 §Future。Rust-Lua arg/ret 型自動変換 |
| **M6** Phase 4a 完了宣言 | 0.5 | meta-ADR doc 1 本 |

### 推奨順序

`M2 → M3 → M4 → M5 → (M1-extended A-B) → M6`

理由:
1. **M2 (pcall)** が最も idiom 解放幅広い。Lua program で `pcall(f, ...)` を期待しない場合は稀。
2. **M3 (GC)** は long-running script の memory pressure を解放。perf benchmark の baseline 整備にも効く。
3. **M4 (_ENV)** は idiomatic Lua の global rewrite parity (`_G[k] = v` / `setfenv` 系) を解放。
4. **M5** は外部ユースケース解放 — ただし Bridge MVP (ADR 0191) で hello-world は通っているので緊急度は中。
5. **M1-extended A-B** は M2-M4 完了で得られる error/global infra が integer slot tracking の test harness を支える。順序入れ替えメリット。
6. **M6** は他 5 完了後の宣言。

## Cosmetic backlog 動向

[cosmetic-backlog.md](cosmetic-backlog.md) は 2026-06-16 land から touch なし。M1 中 6 ADR は full leverage で cosmetic 0 件挟まず — 原則 3 (opportunistic) が遵守された。

## 反省 / 学び

- M1 を range 下限で `/goal` 達成判定する shape は機能した — 1 session ≈ 1 ADR ペースを 6 turn 維持できた。これは元 doc §"1 session で物理的に達成可能" 原則の実証。
- 一方、M1-extended (runtime slot 拡張) は session-shape に乗らないため明示的に分離。同 milestone 内部でも sub-decomposition が要、という meta-learning。
- 次 goal は `/goal M2` (= pcall) を推奨。前提依存 0、session shape OK (2-3 session)、idiomatic leverage 最大。

## References

- [roadmap-revision-2026-06-16.md](roadmap-revision-2026-06-16.md) — 元方針 / milestone 定義
- [cosmetic-backlog.md](cosmetic-backlog.md) — opportunistic 行先
- [lua54-conformance-roadmap.md](lua54-conformance-roadmap.md) — spec gap 視点
