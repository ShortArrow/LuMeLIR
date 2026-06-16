# Cosmetic Backlog

- **Created:** 2026-06-16 (concurrent with `roadmap-revision-2026-06-16.md`)
- **Purpose:** ADR が 1 セッションで着地できるが、leverage 観点では薄い項目を集約。Leverage 作業の合間に「機会主義的」に取りに行く対象。
- **Policy:** ここの項目を session goal にしない。月 1-2 本程度の opportunistic landing が上限。新規 user demand があれば優先順位を引き上げて leverage 扱いに昇格。

## 残 `string.format` format specs

| Spec | Lua 仕様 | 推定 | 備考 |
|---|---|---|---|
| `%q` | Lua-readable quoted string | 1 session (中) | escape logic 必要。`"\n"` → `"\\n"` 等 |
| `%u` | unsigned decimal | 0.3 session | `%llu` 派生、small |
| 幅指定 `%5d` / `%-5s` | 数値リテラル前置 | 1-2 session | parser でリテラル数字を spec の一部として読む |
| 精度 `.N` `%5.2f` | `.` + 数字 | 1 session | width と同じ枠 |
| flag `+` `-` `0` `#` ` ` | flag chars | 0.5 session each | pattern letter 並列追加 |

## 残 `math` namespace 関数

| Function | Lua 仕様 | 推定 | 備考 |
|---|---|---|---|
| `math.ceil(x)` | ⌈x⌉ | 0.3 session | libm `ceil`、math.floor precedent |
| `math.fmod(x, y)` | 浮動余剰 | 0.3 session | libm `fmod` |
| `math.max(...)` | 変項可変、最大 | 0.5 session | variadic、Print precedent |
| `math.min(...)` | 同 | 0.5 session | |
| `math.tan(x)` | tan | 0.3 session | libm `tan` |
| `math.asin(x)` / `acos(x)` / `atan(x[, y])` | inverse trig | 0.5 session 全体 | atan は arity 1/2 |
| `math.atan2(y, x)` | 2-arg arctan | 0.3 session | atan の Lua 5.4 既廃 alias、defer |
| `math.random()` / `random(m)` / `random(m, n)` | PRNG | 1 session | `g_random_state` global + libc `rand()`/`srand()` |
| `math.randomseed(n)` | seed 設定 | 0.3 session | `srand(n)` |
| `math.maxinteger` / `math.mininteger` | i64 limits | bucket A 経由 | Integer subtype (ADR 0196 arc) 完了後 |
| `math.tointeger(x)` / `math.type(x)` | subtype判定 | bucket A 経由 | 同上 |

## 残 `string` namespace 関数

| Function | Lua 仕様 | 推定 | 備考 |
|---|---|---|---|
| `string.find` / `match` / `gmatch` / `gsub` | パターンマッチ | **bucket A レベル** (ADR 0155) | パターンエンジン自体が 3-4 ADR 大物 |
| `string.byte(s, i, j)` 範囲形式 | i から j までのバイト列を multi-return | 1 session | multi-return ABI 連動、若干複雑 |
| `string.dump(fn)` | 関数 bytecode | defer | Lua 5.4 runtime 統合まで |

## 残 `table` namespace 関数

| Function | Lua 仕様 | 推定 |
|---|---|---|
| `table.pack(...)` | varargs → table | varargs 解放後 |
| `table.unpack(t)` | table → multi-return | multi-return ABI 連動 |
| `table.sort(t [, fn])` | in-place sort | 1 session、comparator function 引数 |
| `table.move(a, f, e, t [, b])` | range copy | 0.5 session |

## 残 `io` namespace 関数

| Function | Lua 仕様 | 推定 |
|---|---|---|
| `io.read` 残 format (`"a"` / `"n"` / `"L"`) | 多様な読込 | 1 session each |
| `io.write` file handle 形式 | userdata 統合 | bucket A 経由 (userdata) |
| `io.open` / `io.close` / `io.lines` / `io.popen` / `io.tmpfile` | file handles | userdata 前提、defer |
| `io.input` / `io.output` | default stream 切替 | userdata 前提 |

## 残 `os` namespace 関数

| Function | Lua 仕様 | 推定 |
|---|---|---|
| `os.time()` | UNIX time | 0.3 session、libc `time(NULL)` |
| `os.date([fmt[, t]])` | strftime ラッパ | 0.5 session、libc `strftime` |
| `os.clock()` | プロセス CPU 時間 | 0.3 session、libc `clock()` |
| `os.difftime(a, b)` | 単純減算 | 0.1 session |
| `os.getenv(name)` | 環境変数 | 0.3 session、libc `getenv` |
| `os.exit([code])` | プロセス終了 | 0.3 session、libc `exit` |
| `os.remove(file)` / `os.rename(from, to)` / `os.tmpname()` | filesystem | 0.3 session each |

## 残 metamethod chain / arith

| Item | 推定 |
|---|---|
| `__add` chain (current は 1 hop) | bucket D, 1-2 session (runtime probe path) |
| `__mul` / `__sub` / `__div` 等のチェーン | 0.5 session each (テンプレ展開) |
| `__call` chain | 1 session (recursive probe) |
| `__tostring` を `tostring()` builtin から消費 | 1 session (codegen ToString arm 拡張) |
| `__concat` metamethod fallback | 1-2 session (HIR kind-check を runtime probe に降ろす) |
| `__metatable` field hiding | 既存、規約再確認 0.1 session |

## opportunistic 取り方

1. **大物 leverage 作業の合間に挟む**: M1 Integer/Float 中核を 6-10 session 進める間に、1-2 本 cosmetic を sandwich。
2. **probe 駆動で「実は DONE」発見** を優先: 本 backlog から取りに行く前に `cargo run -- run '...'` で probe してから ADR 化判定。Bucket D / E で見たように既存実装でカバーされている可能性あり。
3. **user demand があれば即昇格**: 「`os.time()` が欲しい」等の具体要望があれば leverage 扱いに格上げ、その session の優先タスクにする。

## opportunistic 取らない条件

- 本 backlog 項目を session goal にしない
- 「ADR 数を増やす」が動機の場合は取らない
- 1 session で 3 個以上の backlog を burn しようとしない (ADR の連続生産が新たな pattern 化するため)

## References

- [roadmap-revision-2026-06-16.md](roadmap-revision-2026-06-16.md) — 本 backlog の親 doc
- [leftover-roadmap.md](leftover-roadmap.md) — bucket A-F 分類
- [lua54-conformance-roadmap.md](lua54-conformance-roadmap.md) — spec gap 視点
