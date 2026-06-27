# Roadmap Rebuild — 2026-06-27 (Sub-task decomposition mandate)

- **Trigger**: 2026-06-21 rebuild acknowledged session-shape mismatch (原則 4) but left N4/N5/N6 as bare milestones. Over the subsequent arc, `/goal N4-N9 done` fired repeatedly while every actually-tractable session-sized task fell to N7 (stdlib breadth). Net: 16 N7 sub-ADRs (0262-0277) landed; N4/N5/N6 sat at 0% structurally because no session-sized entry point existed.
- **Purpose**: Force the same N2-style sub-task decomposition onto N4/N5/N6 so `/goal` can hook a 1-3 session unit. Demote N7 from "spec gap closure" to "fill-while-blocked." Publish a `/goal` ↔ session-shape correspondence table so hook conditions stop being structurally unsatisfiable.
- **State snapshot**: commit `5a59528` (ADR 0277, tests 1697).

## 前ロードマップ (2026-06-21) の残存欠陥

1. **N4/N5/N6 が「milestone」のまま** — N2 だけが N2-A/B/C/D に分解されていた。N4 を「Pattern matcher 完成」と書いたままだと `/goal N4 done` を session-goal にできず、構造的に空転する。
2. **N7 = stdlib remainder の sink 化** — N4/N5/N6 が遠すぎる結果、毎 session が N7 に流れた。N7 増分は spec gap を埋めるが、user value 観点では「実 Lua プログラムが動く」距離を縮めない (1-arg builtin が増えるだけ)。
3. **`/goal` 与え方の guidance 不在** — 06-21 doc は「sub-component scope」と一文書いたのみ。具体的にどの phrase が 1-session 安全かが不明だった。

## 新原則 (06-21 の 4 原則を継承し追加)

### 原則 5: sub-task 分解は milestone 設定の前提
全ての N-item は session 1-3 単位の sub-task に分解した上で roadmap に載せる。分解されていない milestone は roadmap が "open question" として扱い、`/goal` hook 対象にしない。

### 原則 6: N7 は filler、progress 指標ではない
N7 ペース (週 N サブ ADR) は「実 Lua プログラムが動く」距離を縮めない。月次レビューでは N4/N5/N6 sub-task land 数を主指標とする。N7 land 数は補助。

### 原則 7: `/goal` hook condition は session-shape table に従う
ユーザは N1-N10 milestone 全体を `/goal` 文字列にしない。下記対応表のみ。

## `/goal` ↔ session-shape 対応表

| `/goal` 文字列 | 想定 session | 安全度 |
|---|---|---|
| `N7-NN done` (sub-ADR 単位) | 1 | ✅ 安全 |
| `N2-A done` / `N2-B done` 等 | 2-3 | ✅ 安全 |
| `N4-A done` / `N4-B done` (下記分解後) | 1-3 | ✅ 安全 |
| `N5-A done` / `N5-B done` 等 | 2-3 | ✅ 安全 |
| `N6-A done` 等 | 3-5 | ⚠️ 上限気味 |
| `N4 done` / `N5 done` / `N6 done` | 4-10 | ❌ multi-session、hook 条件にしない |
| `N4-N9 done` のような複合 | 25-40+ | ❌ 構造的不能、絶対に hook 条件にしない |
| `N10 done` | 0.5 | ✅ docs only |
| `N9 done` | 2-4 | ⚠️ infra、sub-task 分解 (下記) すれば 1-2 session ずつ可能 |

## N4 — Pattern matcher 完成 (sub-task 分解)

Lua 5.4 §6.4.1 patterns。pure Rust runtime 追加で完結 (codegen 影響少)。

| sub | scope | session |
|---|---|---|
| N4-A | Pattern class基本: `%a %d %s %w %p` + literal char + `.` | 1-2 |
| N4-B | Quantifier: `*` (greedy 0+), `+` (greedy 1+), `?` (0/1) | 1-2 |
| N4-C | Anchor: `^` 行頭、`$` 行末 | 1 |
| N4-D | Capture: `(...)` + capture indexing in `string.match` return | 2 |
| N4-E | Set: `[abc]` `[^abc]` `[a-z]` | 1-2 |
| N4-F | `string.gmatch(s, pat)` iterator (Lua 5.4 §6.4.1) | 2 |
| N4-G | `string.gsub(s, pat, repl)` — repl=string only (Function repl 別 ADR) | 2-3 |
| N4-H | Pattern `%b` balanced match + `%f` frontier (optional, 残仕様) | 1-2 |

**Total**: 11-17 session。各 sub は 1-3 session で session goal にできる。

## N5 — Coroutine runtime (sub-task 分解)

LLVM intrinsics より POSIX `ucontext_t` (`makecontext` / `swapcontext`) が x86_64-linux で最小実装。

| sub | scope | session |
|---|---|---|
| N5-A | `ucontext_t` stack alloc + `makecontext` 配線; main thread 専用 noop coroutine の `coroutine.create` 値生成 | 2-3 |
| N5-B | `coroutine.resume(co)` — swapcontext to coroutine entry; trivial yield-less body 完走 | 1-2 |
| N5-C | `coroutine.yield(...)` — swapcontext back; resume が yield 値を受け取る | 2-3 |
| N5-D | `coroutine.status` / `wrap` / `running` 整備 | 1 |
| N5-E | error in coroutine — uncaught error propagation to caller | 1-2 |

**Total**: 7-11 session。

## N6 — load/require runtime (sub-task 分解)

最も重い。AOT compiler は run-time に parser+codegen を抱える必要があり、self-hosting 領域。妥協案として N6 全体を deferred milestone とする選択肢も別途検討:

| sub | scope | session |
|---|---|---|
| N6-A | embedded parser invocation — Rust runtime 内に `lumelir::parser::parse` を bridge call として再露出 | 2-3 |
| N6-B | embedded HIR + codegen — runtime 内で MLIR module build + JIT 実行 (LLVM ExecutionEngine) | 3-5 |
| N6-C | `load(chunk_string)` Lua-callable wrapper、返値は Function | 1-2 |
| N6-D | `package.path` resolver + filesystem read | 1 |
| N6-E | `require(name)` — `package.path` resolve + `load` + cache (`package.loaded`) | 1-2 |
| N6-F | `package.loaded` / `package.preload` / `dofile` / `loadfile` 整備 | 1 |

**Total**: 9-14 session。**注**: N6-B が技術的に最も重い (JIT linkage)。先行 spike 推奨。

**代替案 A (defer N6 全体)**: AOT compiler thesis を守るなら `load`/`require` を out-of-scope 宣言し、`docs/notes/lua54-conformance-roadmap.md` §6.3 を deviation document 化。N10 (conformance declaration) で「§6.3 not implemented (AOT thesis)」と書く。実用 Lua プログラムの 80% は `require` を使うので thesis 維持コストは大きい。最終判断保留。

## N7 — Stdlib remainder (filler classification)

進捗:
- ✅ landed (ADRs 0262-0277, this session arc): math.fmod / random / deg / rad / atan-2arg / log-2arg / ult / modf, os.exit / remove / rename / tmpname / difftime / date / execute / setlocale, utf8.len
- ❌ remaining: table.pack / sort / move、utf8.\* 残 (char / codepoint / codes / offset)、debug.\* subset、io.open / lines、xpcall、select、goto / label、string.gsub / gmatch (← N4-F/G 配下に再配置)

N7 sub-ADR 1 個 = 1 session の filler。N4-A 等 multi-session sub-task に着手して blocker に遭ったとき、間に N7 を入れて context refresh。

## N8 — Performance gap closing (sub-task 分解)

| sub | scope | session |
|---|---|---|
| N8-A | bench harness — LuaJIT / lua5.4 vs LuMeLIR 比較スクリプト | 1-2 |
| N8-B | inlining heuristic for hot small functions | 2-3 |
| N8-C | escape analysis for closure cells (stack-promote when not captured) | 2-3 |
| N8-D | JIT 検討 (decision-only ADR or spike) | 1 |

**Total**: 6-9 session。N8-A が他 sub の前提。

## N9 — Multi-target CI (sub-task 分解)

| sub | scope | session |
|---|---|---|
| N9-A | aarch64-linux ローカルビルド再現 (cross-compile from x86_64) | 1 |
| N9-B | GitHub Actions matrix activation — aarch64 runner / qemu fallback | 1-2 |
| N9-C | arm64-darwin (Apple Silicon) build & test | 1-2 |
| N9-D | release artifact 生成 (tag → multi-target binary) | 1 |

**Total**: 4-6 session。**N2-N7 完了を待たない**。

## 推奨実装順序 (改訂)

| 優先 | 項目 | 理由 |
|---|---|---|
| P0 | N4-A〜N4-G (pattern matcher) | 実 Lua の string-heavy code が大量に必要; 各 sub 1-3 session |
| P0 | N9-A〜N9-D 並走 | infra; 既存 binary を multi-target にすぐ拡げる |
| P1 | N5-A〜N5-E (coroutine) | iterator/generator 重視のコードを動かすため |
| P1 | N7 filler 継続 | session 余白活用 |
| P2 | N6 (要 thesis 判断) | `load`/`require` deferred 宣言の可否を決めてから |
| P2 | N8-A bench harness | 機能 surface が落ち着く前に着手しても得る情報少 |
| P3 | N10 (conformance 宣言) | N4/N5 完了後 |

## 月次レビュー指標 (改訂)

主指標:
- N4 sub-task landed / 月 (目標: 3-4)
- N5 sub-task landed / 月 (目標: 1-2、N4 後)
- N9 sub-task landed / 月 (目標: 1-2、並走)

補助指標:
- N7 sub-ADR landed / 月 (filler measure)
- tests delta / 月

進捗が N7 だけで前進している月は **構造前進ゼロ** と判定する。

## 残存する 06-21 rebuild artifacts

[`roadmap-2026-06-21-rebuild.md`](roadmap-2026-06-21-rebuild.md) は historical record。今回の rebuild がその直接 supersede。原則 1-4 は継承、5-7 を追加。

## 参考

- [roadmap-2026-06-21-rebuild.md](roadmap-2026-06-21-rebuild.md) — 前 rebuild (本 doc が supersede)。
- [lua54-conformance-roadmap.md](lua54-conformance-roadmap.md) — §-by-§ gap 一次資料 (引き続き有効)。
- [ADR 0253 revised](../design/0253-lua54-preferred-subset-declaration.md) — honest scorecard。
