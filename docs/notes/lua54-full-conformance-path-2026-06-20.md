# Lua 5.4 網羅までのロードマップ — Milestone Sequence

- **Date:** 2026-06-20
- **State snapshot:** commit `4e501b1` (ADR 0214、tests 1478)
- **Companion docs:**
  - [roadmap-status-2026-06-20.md](roadmap-status-2026-06-20.md) — 直近進捗 / 次 goal
  - [roadmap-revision-2026-06-16.md](roadmap-revision-2026-06-16.md) — milestone ベース方針
  - [lua54-conformance-roadmap.md](lua54-conformance-roadmap.md) — §-by-§ gap table (本 doc の基礎データ、2026-06-15 snapshot で若干 stale)
- **Scope:** Lua 5.4 "preferred subset" 網羅 (= §2 types / §3 language / §6 standard libraries) までの milestone 列。§4 C API と §7 stand-alone interpreter は LuMeLIR の AOT model 上 out-of-scope (PRD §1)。

## 設計原則 (元 doc 継承)

1. **Milestone を session goal にする** — 各 M は 1-10 session、`/goal MX` で hook 動作。
2. **Leverage 高い順に並べる** — Structural unlock > Idiomatic surface > Spec gap > Cosmetic。
3. **Cosmetic は opportunistic** — milestone 内に挟まず、`cosmetic-backlog.md` に置く。
4. **依存関係順に直列化** — 並列 ADR 投入は subtype 中核 (M1) 既達済ゆえ可だが、session goal hook 整合のため直列。

## 全体俯瞰

| M | 名称 | session | leverage | 前提 |
|---|---|---|---|---|
| **M1** | Integer/Float subtype 中核 | ✅ 6/6-10 (minimal close) | A 中 | — |
| **M2** | pcall / error propagation | 2-3 | A 高 | — |
| **M3** | GC actual freeing | 3-5 | A 中 | — |
| **M4** | `_ENV` / true globals | 3-5 | A 中 | — |
| **M5** | Bridge marshaling | 2-3 | C 中 | M2 (error 連動) |
| **M6** | Phase 4a 完了宣言 | 0.5 | — | M2-M5 |
| **M7** | string patterns (find/match/gmatch/gsub) | ✅ 4/4-6 minimum-viable close (ADRs 0228-0231; 1519 → 1540) | B 高 | — |
| **M8** | M1-extended (runtime slot 拡張) | ✅ 4/4-6 minimum-viable close (ADRs 0232-0235; 1540 → 1561) | A 中 | M2 (error 連動でテスト安定) |
| **M9** | `<close>` / `<const>` + `__close` + goto/label | ✅ 2/2-3 minimum-viable close (ADRs 0236-0237; 1561 → 1571) | A 低 | — |
| **M10** | `__gc` + `__mode` weak tables | ✅ 2/2-3 surface-pin close (ADRs 0238-0239; 1571 → 1579) — runtime semantics defer to M10-stretch (M3-extended Table DFS gating) | A 中 | M3 (GC freeing) |
| **M11** | stdlib 拡張 (math/table/os/utf8/debug/basic select) | ✅ 3/3-5 minimum-viable close (ADRs 0240-0242; 1579 → 1599) — math.ceil/tan/asin/acos/atan, math.max/min, os.time/clock/getenv. Stretch: table.pack/unpack/sort, utf8.*, debug.*, math.random/fmod, io.open | B 中 | — |
| **M12** | userdata + Bridge 拡張 (error/GC interaction) | 4-6 | C 中 | M5, M10 |
| **M13** | Coroutines runtime + coroutine library | 5-8 | A 低 (workload 大) | — |
| **M14** | load / require / package | 6-10 | C 低 (self-hosting) | M2, M4 |
| **M15** | Phase 4 完了宣言 + production hardening | 2-3 | — | M11-M14 |
| **M16** | Phase 5 (CI matrix / release / security audit) | 4-6 | — | M15 |
| **M17** | Lua 5.4 preferred-subset 網羅宣言 | 0.5 | — | all |

**合計**: ~16 milestone、累計 **52-90 session** (M1 既達分 6 除外)、focused 3-6 ヶ月。

## Milestone 詳細

### M2 — pcall / error propagation (2-3 session) ★ 次の goal 推奨

- ADR 0153 設計済、impl 未着工。
- Lua §2.3 + §6.1。`pcall(f, ...)` / `xpcall(f, msgh, ...)` / `error(v, lvl)` value 伝播。
- 実装方針: MLIR LLVM dialect の `llvm.invoke` + landing pad、or setjmp/longjmp emit。
- Unlock: stdlib の error-returning 関数 (`tonumber` failure / `io.open` failure / `assert` chain)、Bridge error propagation (M5)、test harness の `assert_error_msg` shape。

### M3 — GC actual freeing (3-5 session)

- ADRs 0160 (stack walk) + 未番号 DFS lift。
- 現状: ADR 0185 v1 safety mode で全 BLACK pre-mark、freeing 発火せず。
- 実装: per-frame alloca-slot table を codegen で埋め、root scan → worklist DFS → sweep で WHITE 解放。
- Unlock: long-running script の memory pressure 解放、`__gc` (M10)、`__mode` (M10)、Bridge GC interaction (M12)。

### M4 — `_ENV` / true globals (3-5 session)

- ADR 0154 設計済。現 ADR 0048 auto-declared globals は接続子。
- 実装: top-level chunk の `_ENV` upvalue 化、free-name lookup を `_ENV[name]` に書き換え、`_G = _ENV` で expose。
- Unlock: idiomatic Lua の global rewrite (`_G[k] = v` / `setfenv`-shaped patterns)。stdlib impl の glue。

### M5 — Bridge marshaling (2-3 session)

- ADR 0191 §Future。現 Bridge MVP は Number 専用。
- 拡張: String / Bool / TaggedValue / Table (shallow) の Rust ↔ Lua 変換。
- 前提: M2 (error 伝播) — Rust panic → Lua error 経路。
- Unlock: 外部 Rust crate 取り込みの実用化。

### M6 — Phase 4a 完了宣言 (0.5 session)

- meta-ADR doc 1 本。Phase 4a の close criteria (M2-M5 着地 + ADR 0193 success metrics 通過) を満たした宣言。

### M7 — string patterns (4-6 session)

- ADR 0155 設計済 (pattern engine 自体が大型)。
- `string.find` / `match` / `gmatch` / `gsub` の 5.4 pattern 文法 (`%a` `%d` `%w` `%s` `%p` `[]` `*` `+` `-` `?` `^$` `()` captures)。
- 単独 self-contained workstream、M2-M6 と並列着手可だが session goal hook 整合のため直列。
- Unlock: 実用 Lua program の最大 stdlib gap 解消。

### M8 — M1-extended runtime slot 拡張 (7-12.5 session)

- M1 (≥6 sub-ADR) では HIR static-shape 限定。runtime Integer-tagged slot の拡張で `local i = 1; print(math.type(i))` → `"integer"` を解放。
- 内訳: A) `LocalInfo.subtype` field + ValueKind 経由 propagation (2-4)、B) `tostring(integer)` precision (0.5-1)、C) `io.write(integer)` `%lld` arm (0.5)、D) Phase C i64 arithmetic (3-5)、E) Integer 比較 (1-2)。
- ADR 0214 §Out of scope 全項目を回収。

### M9 — `<close>` / `<const>` + `__close` + goto/label (2-3 session)

- Lua 5.4 new 構文 3 件をまとめて。`<const>` は parser-only に近い、`<close>` は M2 error propagation + `__close` metamethod (本 M 内 1 sub-ADR) を要する。
- goto/label (Lua 5.2+) も同 milestone で吸収 — 単独 0.5 session で session-goal shape に乗らないため bundling。

### M10 — `__gc` finalizer + `__mode` weak tables (3-4 session)

- ADRs 0163 (`__gc`) + 0164 (`__mode`) 設計済、impl 全 deferred。
- 前提: M3 (GC freeing 起動)。
- Lua §2.5。リソース管理 + cache 解放の idiomatic pattern を解放。

### M11 — stdlib 拡張 (8-12 session)

| sub | 内容 | session |
|---|---|---|
| 11a | math (ceil/fmod/max/min/三角/random + randomseed/log2/log10) | 3-5 |
| 11b | table.pack/unpack/sort/move | 2-3 |
| 11c | os (time/date/clock/difftime/getenv/exit/remove/rename/tmpname) | 2-3 |
| 11d | utf8 (char/codepoint/len/offset/codes) | 2-3 |
| 11e | debug subset (traceback/getinfo/getlocal/setlocal/getupvalue/setupvalue) | 2-3 |
| 11f | basic 残 (select、partial load/loadfile/dofile はスタブ) | 1-2 |
| 11g | string.format suffix 拡張 (width/precision/`%q`/flags) | 1-2 |

- 並列着手可だが、各 sub-M は cosmetic backlog からの promotion 候補 — 必要に応じて bundle/split。

### M12 — userdata + Bridge 拡張 (4-6 session)

- userdata 型を Lua 値層に追加 (TaggedValue tag 拡張 or 独立 representation)。
- ADR 0191 §Future 残: error propagation (Rust panic → Lua error、M2 連動)、GC interaction (Rust-held Lua refs を root 化、M3 連動)、user-defined bridge crates。
- 前提: M5、M10。

### M13 — Coroutines runtime + library (5-8 session)

- 単独 self-contained mountain。runtime stack 切替 (ucontext / makecontext or 手書き setjmp + alloca 退避 or LLVM coroutine intrinsic)。
- coroutine.create/resume/yield/wrap/status/running/isyieldable。
- Lua §6.2 / §2.6。
- 大半の production Lua program は使わない workload — 後送り可。

### M14 — load / require / package (6-10 session)

- 「runtime parser + codegen 起動」= self-hosting territory。
- 実装方針: LuMeLIR の lexer/parser/hir/codegen を runtime crate 化し、`load(src)` で in-process 呼出。
- require + package.path / package.cpath / package.loaders。
- 前提: M2 (load 失敗の error 伝播)、M4 (`_ENV` injection)。
- 多くの Lua program は静的 only で運用可、defer 候補。

### M15 — Phase 4 完了宣言 + production hardening (2-3 session)

- ADR 0193 success metrics 全通過確認 + close meta-ADR。
- malloc OOM 全サイト集約 (ADR 0157 で部分対応、残: table grow / hash grow / closure cell)。
- unsafe-block audit (ADR 0195 §Workstreams)。
- benchmark regression-detection gate (CI failing assert)。

### M16 — Phase 5 (CI matrix / release / security audit) (4-6 session)

- ADR 0195 §Workstreams: CI matrix (Linux x64 / Linux aarch64 / macOS arm64 / WSL)、release artifact、security review、unsafe audit。
- 一部 M15 と overlap。

### M17 — Lua 5.4 preferred-subset 網羅宣言 (0.5 session)

- meta-ADR 1 本。M1-M16 完了 + `lua54-conformance-roadmap.md` §-by-§ 全 ✅。

## 推奨着手順序 (再掲)

```
M1 ✅  →  M2 → M3 → M4 → M5 → M6
          ↓
          M7 → M8 → M9 → M10 → M11
          ↓
          M12 → M13 → M14
          ↓
          M15 → M16 → M17  (網羅宣言)
```

**Minimum viable Lua 5.4** (= M1-M6 + M7 + M11 抜粋 = 0.x release 候補): 約 22-30 session、3-4 ヶ月。
**Full preferred-subset 網羅**: 約 52-90 session、3-6 ヶ月 focused work。

## 別視点 — "全部はやらない" 選択肢

Phase 4a (M1-M6) 完走時点で 0.x release 可能。M7 + M11 部分着地で実用 Lua program の 8-9 割を回せる想定。M13 (coroutine) と M14 (load) は demand-driven defer も合理的選択 — PRD §6 の "preferred subset" の解釈次第。

## References

- [Lua 5.4 Reference Manual](https://www.lua.org/manual/5.4/) — §2 / §3 / §6 primary source
- [lua54-conformance-roadmap.md](lua54-conformance-roadmap.md) — §-by-§ gap table (2026-06-15 snapshot)
- [roadmap-revision-2026-06-16.md](roadmap-revision-2026-06-16.md) — milestone 原則
- [roadmap-status-2026-06-20.md](roadmap-status-2026-06-20.md) — 直近進捗
- [leftover-roadmap.md](leftover-roadmap.md) — bucket A-F snapshot (俯瞰用)
- ADR 0193 — Phase 4 entry criteria
- ADR 0195 — Phase 5 entry criteria
- PRD `docs/PRD.jp.md` §6 / §7 — phase 定義
