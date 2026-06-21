# 0253. Lua 5.4 Conformance Scorecard — Honest Status (Revised)

- **Status:** Accepted (revised 2026-06-21 — supersedes the original "preferred-subset declaration" framing)
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Revision note

The first published version of this ADR (2026-06-21 morning) declared LuMeLIR "conforms to a Lua 5.4 preferred subset" by counting pin ADRs (0238, 0239, 0244, 0245, 0247) as ✅ in the status tables. That framing was over-optimistic: those pins document surface acceptance + storage round-trip but the **runtime semantics do not work**. A user program that depends on `__gc` firing, a coroutine actually yielding, or `require` actually loading does NOT work today.

This revision keeps the §-by-§ table structure but uses three honest status markers:

- **✅ shipped** — feature compiles AND runs with spec-correct or precisely-documented behavior.
- **⏸ pinned** — surface accepted (parser / HIR / storage), but runtime semantics are partial or deferred. User code that touches the runtime path does not get the spec answer.
- **❌ deferred** — not implemented at any layer; spec gap explicit.

The original "preferred subset met" declaration is retracted. The accurate framing: LuMeLIR is a **partial Lua 5.4 AOT compiler with a documented gap catalog**, suitable for the "dominant use case" surface (closures + tables + pcall + math + string + io.write/read + bridge), with substantial deferred work catalogued.

## Honest scorecard

### §2.1 Types

| Type | Status | Notes |
|---|---|---|
| `nil` | ✅ shipped | |
| `boolean` | ✅ shipped | |
| `number` integer subtype | ✅ shipped | Phase B silent demotion + M8 static propagation (0209-0214, 0232-0235) |
| `number` float subtype | ✅ shipped | |
| `string` (boxed, 8-bit clean) | ✅ shipped | 0112 |
| `function` (closures + upvalues) | ✅ shipped | 0083 |
| `userdata` | ❌ deferred | 0244 documents opaque-Number workaround; **no real userdata** |
| `thread` (coroutine) | ❌ deferred | `isyieldable()` / `running()` work (0245); **create/resume/yield do not exist** |
| `table` (array + hash + metatable) | ✅ shipped | 0053-0140 |

### §2.4 Metamethods

| Metamethod | Status | Notes |
|---|---|---|
| `__index` Table form | ✅ shipped | 0134 |
| `__index` Function form | ✅ shipped | 0150 / 0166 |
| `__newindex` Table form | ✅ shipped | 0135 / 0168 |
| `__newindex` Function form | ✅ shipped | 0151 / 0169 |
| `__add` ... `__idiv` | ✅ shipped | 0147 |
| `__band` ... `__shr` | ✅ shipped | 0148 |
| `__eq` / `__lt` / `__le` | ✅ shipped | 0144 |
| `__concat` / `__tostring` / `__len` | ✅ shipped | 0142 / 0143 |
| `__call` | ✅ shipped | 0146 |
| `__gc` | ⏸ pinned | 0238 — field accepted; **runtime dispatch does not fire** |
| `__close` | ⏸ pinned | 0237 — `<close>` parses + readonly; **runtime hook does not fire** |
| `__mode` | ⏸ pinned | 0239 — field accepted; **weak-key/value clearing does not fire** |
| `__name` | ❌ deferred | |
| `__metatable` (hide) | ❌ deferred | |

### §2.5 Garbage Collection

| Item | Status | Notes |
|---|---|---|
| Allocator wrapper | ✅ shipped | 0157 |
| `g_gc_head` linked list | ✅ shipped | 0158 |
| Size guard (4 GiB) | ✅ shipped | 0184 |
| Mark phase (v1 safety mode) | ✅ shipped | 0185 |
| Sweep phase (v1 safety mode) | ✅ shipped | 0185 |
| Auto-trigger (1 MiB) | ✅ shipped | 0186 |
| Chunk-safe real freeing (String / Function / TaggedValue) | ✅ shipped | 0218-0220 |
| **Table DFS through array + hash** | ❌ **deferred (M3-extended)** | Blocks `__gc` + `__mode` + userdata + GC-tracked closures |
| **Per-frame stack walk** | ❌ deferred | 0160 designed; not implemented |
| **`__gc` finalizer runtime dispatch** | ❌ deferred | Blocked on M3-extended |
| **`__mode` weak-table clearing** | ❌ deferred | Blocked on M3-extended |

### §2.3 Error Handling

| Item | Status | Notes |
|---|---|---|
| `error(msg)` | ✅ shipped | 0033 + 0215 |
| `pcall` (single + multi-return) | ✅ shipped | 0216 / 0217 |
| `xpcall` | ❌ deferred | |
| Bridge error propagation | ✅ shipped | 0243 — `rust.fail` |

### §2.2 Environment

| Item | Status | Notes |
|---|---|---|
| Auto-declared globals (ADR 0048) | ✅ shipped | |
| `_G` / `_ENV` chunk-level Table | ✅ shipped | 0221-0223 |
| `local _ENV = ...` sandbox rebind | ✅ shipped | 0223 |
| Free-name fallback → `_ENV[name]` | ❌ deferred | |
| Builtin migration to `_ENV` entries | ❌ deferred | |

### §3 Language Surface

| Item | Status | Notes |
|---|---|---|
| Lexical (numbers + identifiers + comments + strings) | ✅ shipped | |
| Hex / sci notation / long strings | ✅ shipped | |
| `if` / `while` / `repeat` / `for` / `break` / `do` / `local` | ✅ shipped | |
| `<const>` | ✅ shipped | 0236 |
| `<close>` (readonly only; no `__close` dispatch) | ⏸ pinned | 0237 |
| Method call `o:m()` | ✅ shipped | 0092 / 0183 |
| Multi-assign / multi-return | ✅ shipped | |
| Varargs `...` | ✅ shipped | |
| Function definitions + closures | ✅ shipped | |
| Table constructor (array + named + bracket key) | ✅ shipped | 0199 |
| **`goto` / `::label::`** | ❌ deferred | |

### §6.1 Basic Functions

| Function | Status |
|---|---|
| `assert` / `error` / `getmetatable` / `setmetatable` / `ipairs` / `pairs` / `next` / `print` / `rawequal` / `rawlen` / `rawget` / `rawset` / `tonumber` / `tostring` / `type` / `pcall` / `collectgarbage` | ✅ shipped |
| `xpcall` | ❌ deferred |
| `select` | ❌ deferred |
| `load` / `loadfile` / `dofile` | ❌ deferred |

### §6.2 Coroutines

| Function | Status | Notes |
|---|---|---|
| `coroutine.isyieldable` | ✅ shipped | Always `false` from main thread (spec-correct) |
| `coroutine.running` | ✅ shipped | Always `(nil, true)` from main thread (spec-correct) |
| `coroutine.create` / `resume` / `yield` / `wrap` / `status` / `close` | ❌ deferred | 0246 strategy |

### §6.3 Modules

| Item | Status | Notes |
|---|---|---|
| `package.config` | ⏸ pinned | 0247 — synthesised constant; no runtime consumer |
| `package.path` | ⏸ pinned | 0247 — synthesised constant; no runtime consumer |
| `package.cpath` | ⏸ pinned | 0247 — synthesised constant; no runtime consumer |
| `package.loaded` / `package.searchers` / `package.searchpath` | ❌ deferred | |
| `require` | ❌ deferred | 0248 strategy |

### §6.4 String Manipulation

| Function | Status | Notes |
|---|---|---|
| `string.len` / `upper` / `lower` / `sub` / `rep` / `byte` / `char` / `reverse` | ✅ shipped | |
| `string.format` (`%d`/`%f`/`%s`/`%x`/`%X`/`%o`/`%g`/`%e`/`%c`/`%i`/`%%`) | ✅ shipped | 0152, 0202-0205 |
| `string.format` width / precision / `%q` | ❌ deferred | |
| `string.find` (literal + char classes + `.`) | ⏸ pinned | 0228 / 0229 / 0231 — magic-char patterns (quantifiers, sets, anchors, captures) **deferred** |
| `string.match` (literal only) | ⏸ pinned | 0230 — magic-char patterns deferred |
| `string.gmatch` / `gsub` | ❌ deferred | |

### §6.6 Table Manipulation

| Function | Status |
|---|---|
| `table.concat` / `table.insert` / `table.remove` | ✅ shipped |
| `table.pack` / `unpack` / `sort` / `move` | ❌ deferred |

### §6.7 Math

| Function | Status |
|---|---|
| `math.sqrt` / `floor` / `abs` / `pow` / `sin` / `cos` / `log` / `exp` / `ceil` / `tan` / `asin` / `acos` / `atan` / `max` / `min` | ✅ shipped |
| `math.huge` / `pi` / `maxinteger` / `mininteger` | ✅ shipped |
| `math.type` / `tointeger` | ✅ shipped (Phase B static-shape + M8 Local subtype) |
| `math.random` / `randomseed` / `fmod` / `modf` | ❌ deferred |

### §6.8 I/O

| Function | Status |
|---|---|
| `io.write` / `io.read` (line-based) | ✅ shipped |
| `io.open` / `close` / `lines` / `popen` / `tmpfile` | ❌ deferred |
| `io.input` / `output` / `stderr` / `stdout` | ❌ deferred |

### §6.9 OS

| Function | Status |
|---|---|
| `os.time` / `os.clock` / `os.getenv` | ✅ shipped |
| `os.date` / `difftime` / `exit` / `remove` / `rename` / `tmpname` | ❌ deferred |

### §6.5 utf8 / §6.10 debug

| Library | Status |
|---|---|
| `utf8.*` | ❌ deferred |
| `debug.*` | ❌ deferred |

## Quantitative honesty

Counting strictly:

- **✅ shipped**: 47 items work as spec-described.
- **⏸ pinned**: 9 items have surface acceptance but no runtime semantics.
- **❌ deferred**: 31+ items have no implementation at any layer.

The original 0253 publication conflated ⏸ and ✅, declaring 56 items shipped. The honest count is **47 / ~87** (~54%).

## What this means for users

**Programs that actually work today:**

- Closures, recursion, tables, metatables (excluding `__gc` / `__close` / `__mode` runtime), arithmetic, comparison, concat, length, bitwise.
- Integer / Float subtype distinction via M8 static propagation.
- `pcall` + `error` value propagation.
- `_G` / `_ENV` introspection + `local _ENV = {}` sandbox.
- `string.find` / `match` with literal + char-class patterns only (no quantifiers, sets, anchors, captures).
- `string.format` with the basic conversion letters.
- 15 `math.*` functions.
- `io.write` + line-based `io.read`.
- `os.time` / `os.clock` / `os.getenv`.
- Rust-Lua bridge with error propagation.

**Programs that do NOT work today (need ⏸ → ✅ or ❌ → ✅ work):**

- Anything using coroutines (no `create` / `resume` / `yield`).
- Anything using `require` or `load` (no runtime parser).
- Anything depending on `__gc` finalizer firing (storage works; dispatch does not).
- Anything depending on `__close` scope-exit hook (parser works; runtime does not).
- Anything depending on weak tables clearing (`__mode` stored but not honored).
- Anything using `string.find/match` with quantifiers / sets / anchors / captures.
- Anything using `string.gmatch` / `gsub`.
- Anything using `table.pack` / `unpack` / `sort` / `move`.
- Anything using `utf8.*` / `debug.*`.
- Anything using `xpcall` / `select` / `goto`.
- Anything using file I/O beyond `io.write` + line-based `io.read`.
- Anything using `os.date` / `os.exit` / `os.remove` / etc.

## Project status after honest accounting

LuMeLIR is a **partial Lua 5.4 AOT compiler** with the dominant computational + stringy I/O surface working, plus a **documented gap catalog** of substantial language features (coroutines, modules, full patterns, full GC) that need real implementation work. The original "preferred subset met" framing is retracted; the new framing is "47 of ~87 spec items shipped, with strategy + architectural-delta notes for each deferred item."

The forward path lives in [`docs/notes/roadmap-2026-06-21-rebuild.md`](../notes/roadmap-2026-06-21-rebuild.md) (sibling commit). M3-extended Table DFS is the structural unblocker for 4+ deferred items and is prioritised first.

## Test count delta

```
Step 0:  1617 (after ADR 0252)
C1 (declaration meta-ADR — revised, docs only): 1617 → 1617
```

## References

- [ADR 0193](0193-phase4-entry-criteria.md) — Phase 4 entry.
- [ADR 0195](0195-phase5-entry-criteria.md) — Phase 5 entry.
- [ADR 0250](0250-phase4-close-declaration.md) — Phase 4 close.
- [ADR 0252](0252-phase5-close-declaration.md) — Phase 5 close.
- ADRs 0209-0252 — M1-M16 sub-ADR landed set.
- [Lua 5.4 Reference Manual](https://www.lua.org/manual/5.4/) — primary source of conformance items.
- [Lua 5.4 conformance roadmap](../notes/lua54-conformance-roadmap.md) — §-by-§ gap source.
- [Roadmap 2026-06-21 rebuild](../notes/roadmap-2026-06-21-rebuild.md) — forward path N1-N10.
- PRD `docs/PRD.jp.md` — project scope.
