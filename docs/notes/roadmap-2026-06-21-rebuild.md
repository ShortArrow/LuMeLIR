# Roadmap Rebuild — 2026-06-21 (Post-Honest-Accounting)

- **Trigger**: ADR 0253 改訂で M1-M17 closure 群が pin ADR を「shipped」扱いしていた構造を honest scorecard に置き換え。47/~87 (~54%) が実装、残 40 項目以上が pin or deferred と判明。
- **目的**: M1-M17 milestone 体系が「pin land すれば milestone close」というショートカットを許していた誤りを修正し、anti-ad-hoc な依存グラフベースで forward path を再構築する。
- **State snapshot**: commit `a67fcef` (ADR 0253 unrevised; tests 1617).

## 前ロードマップの構造的失敗

1. **「ADR 存在 ≈ 機能実装」の誤等価**
   M10 / M12 / M13 / M14 で structural blocker により実装不能と判明した item を「pin ADR + deferral note」で milestone close 扱いした。M-stretch deferral note は爽やかな contract に見えたが、現実は「ADR 0253 が 50%+ のスコアを `✅` カウント」。
2. **依存グラフ可視化の欠如**
   M-stretch 群を「並列の deferred work」として roadmap に並べた。実際は 4-5 件が **M3-extended Table DFS** という単一のボトルネックに依存している。
3. **「milestone close」と「user value 実現」の解離**
   goal hook を満たすために docs-only ADR を量産する圧力が常時かかった。session shape 制約 (1 セッション ≈ 1-2 ADR) と「multi-month structural work」の不整合。

## 新方針

### 原則 1: 依存グラフ駆動で順序付け

milestone を独立並列項目ではなく、**unblock chain** で順序付ける。1 つの structural fix が多数の下流項目を解放する箇所を優先 (CLAUDE.md 第3原則 anti-ad-hoc)。

### 原則 2: "shipped" 定義の厳格化

新ロードマップでは "✅ shipped" は以下を満たすときのみ:

- 機能が compile して run する。
- spec 通りの結果を出すか、正確に文書化された deviation を返す。
- regression test pin (e2e) が CI green。

pin ADR (surface acceptance + runtime defer) は **closed milestone とカウントしない**。closed には実装が必要。

### 原則 3: docs-only milestone を廃止

前ロードマップの M10 (`__gc`/`__mode` field pin)、M12-B (userdata opaque)、M13-A (coroutine main-thread)、M14-A (package constants) は全て docs-only で実装ボトルネックを 0 ステップも前進させなかった。新ロードマップでは「docs ADR は実装 ADR の準備工程としてのみ許可、milestone close 要件には含めない」。

### 原則 4: session-shape 不整合を認める

multi-month structural work (M3-extended, Coroutine runtime, load/require) は 1 session で close 不能。前ロードマップが「session goal で close」を強制した結果が docs-only ショートカット。新 milestone は **"5-20 session" の幅で session-goal hook と切り離して進行**。

## N1-N10 forward path

| N | Title | session 規模 | unblocks | 種別 |
|---|---|---|---|---|
| N1 | honest scorecard (ADR 0253 revision + this doc) | 1 ✅ done | — | docs |
| **N2** | **M3-extended — Table DFS (the unblocker)** | 5-10 | N3-A/B/C/D + closure-cell-as-root | implementation |
| N3-A | `__gc` finalizer runtime dispatch | 2-3 | — | implementation (precondition: N2) |
| N3-B | `__mode` weak-key/value clearing | 2-3 | — | implementation (precondition: N2) |
| N3-C | userdata `ValueKind::Userdata` 真実装 | 3-5 | bridge GC interaction | implementation (precondition: N2) |
| N3-D | `__close` runtime hook on scope exit | 1-2 | — | implementation (precondition: N2 + ADR 0237) |
| N4 | Pattern matcher 完成 (quantifiers + sets + anchors + captures + `gsub` + `gmatch`) | 4-6 | string.* full | implementation |
| N5 | Coroutine runtime (LLVM intrinsics or POSIX ucontext) | 5-8 | coroutine.* full | implementation |
| N6 | load / require runtime (embedded-pipeline self-hosting) | 6-10 | dynamic loading | implementation |
| N7 | Stdlib 残: `table.pack/sort/move`, `utf8.*`, `debug.*` subset, `io.open`/lines, `math.random/fmod`, `xpcall`, `select`, `goto`/label | 5-8 | spec gap closure | implementation |
| N8 | Performance gap closing (inlining / escape analysis / JIT 検討) | 4-8 | LuaJIT ratio | implementation |
| N9 | Multi-target CI activation (aarch64-linux, arm64-darwin) | 2-4 | shippable beyond x86_64-linux | infra |
| N10 | Real Lua 5.4 conformance declaration (supersedes 0253) | 0.5 | M17 真の close | docs |

**Total**: 36-67 session 累計、おそらく 3-6 ヶ月 focused work。前ロードマップの「6 日で M1-M17 close」とは別次元の本格 implementation 期間。

### 推奨実装順序

1. **N2 → N3 群並列** — N2 が land すれば N3-A/B/C/D は独立に進行可。
2. **N4** (pattern matcher) は N2-N3 と並列可能。pure Rust runtime 追加のみ。
3. **N5** (Coroutine) は N4 完了後でも先でも可。 self-contained。
4. **N6** (load/require) は最後に近い位置。 self-hosting investment が大きい。
5. **N7** はその他 stdlib 拡張。1 ADR / 1 機能ペース。N1-N6 と並列着手可。
6. **N8** (performance) は機能 surface 安定後。 benchmark がガイドする。
7. **N9** infra は誰でも触れる。 N2-N7 完了を待つ必要なし。
8. **N10** 全て land してから。

## N2 = M3-extended の構造詳細

M3-extended が最 critical なので scope を pin しておく:

### N2 (5-10 session) 内訳

| sub | scope | session |
|---|---|---|
| N2-A | Table mark phase — array part walk (per-element TaggedValue 再帰 mark) | 2-3 |
| N2-B | Table mark phase — hash part walk (key + value tagged-slot 再帰 mark) | 2-3 |
| N2-C | capturing closure cell + upvalue box を g_gc_head root として登録 | 1-2 |
| N2-D | per-frame stack walk (ADR 0160 design) — user fn 内 String/Table/TV slot を root 化 | 1-3 |

N2 完了で:
- Table を含む chunk の chunk_safe_for_real_gc 制約解除可能 (ADR 0218 §M3-stretch unblock)。
- closure-captured Table が GC tracked になり、capturing closure を含む chunk も real-freeing 可。
- N3-A (`__gc`) が hook point を持てる: sweep 時に object の metatable から `__gc` Function を probe 可能。
- N3-B (`__mode`) が weak-slot 識別可能。
- N3-C (userdata) が GC tracked allocate path を共有可能。

### N2 の前提条件 (今日の状態)

すでに整っている:
- `g_gc_head` linked list + size guard + auto-trigger 全部稼働。
- `gc_type_meta` テーブルあり (ADR 0184); GC_TYPE_TABLE / GC_TYPE_HASH_BUF / GC_TYPE_ARRAY_BUF 識別済。
- Table layout 安定 (ADR 0053-0140)。
- v1 safety mode (ADR 0185) の mark/sweep スケルトンあり; mark body 差し替えるだけ。

不足:
- Table の per-element 再帰 mark walker。
- closure cell の root 登録 path。
- per-frame slot enumeration (libunwind feature flag or pure inline emit)。

技術的にはどれも実装可能; session スケールが大きいだけ。

## goal hook 運用変更

前ロードマップでは `/goal Mxx 完了` が 1 session で close することを期待した。新ロードマップでは:

- **session 内で close 可能**: N1, N10 (docs only)。
- **multi-session の structural work**: N2-N6, N8 (implementation)。これらは goal hook を「進捗単位」として運用する想定で、1 session で全 close は期待しない。

`/goal N2-A 完了` のような sub-component scope に分けて進める。

## 残存する前ロードマップ artifacts

ADR 0250 (Phase 4 close), 0252 (Phase 5 close), 0253 (Lua 5.4 declaration) は historical record として保存。0253 のみ革命的 revision を行い honest scorecard へ更新済。Phase 4/5 close declaration は「あの時点での milestone close 判定」として有効; 新ロードマップは Phase 概念から離れて N-series に移行。

## 参考

- [ADR 0253 revised](../design/0253-lua54-preferred-subset-declaration.md) — sibling honest scorecard.
- [roadmap-revision-2026-06-16.md](roadmap-revision-2026-06-16.md) — 前ロードマップ革命の経緯 (milestone 制覇).
- [roadmap-status-2026-06-20.md](roadmap-status-2026-06-20.md) — M1-M17 milestone status (前ロードマップ完了報告).
- [lua54-full-conformance-path-2026-06-20.md](lua54-full-conformance-path-2026-06-20.md) — 前 M17 path (本 doc が supersede)。
- [lua54-conformance-roadmap.md](lua54-conformance-roadmap.md) — §-by-§ gap 一次資料 (引き続き有効)。
