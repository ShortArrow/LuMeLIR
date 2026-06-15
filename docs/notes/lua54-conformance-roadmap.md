# Lua 5.4 Conformance Roadmap

- **Date:** 2026-06-15
- **Reference:** [Lua 5.4 Reference Manual](https://www.lua.org/manual/5.4/) — §2 (Basic Concepts), §3 (The Language), §6 (Standard Libraries)
- **State snapshot:** commit `fb01441` (ADR 0194, tests 1431)
- **Scope:** language semantics + runtime + standard library coverage required for "Lua 5.4 preferred subset" conformance. C API (§4) and stand-alone interpreter (§7) are out of scope (LuMeLIR is an AOT compiler, not an embeddable VM).

## Methodology

A coarse gap analysis against the spec, categorised by:

- **A — Language / runtime spec obligations** (a 5.4-compliant impl must have these or it isn't 5.4)
- **B — Standard library coverage** (idiomatic Lua programs depend on these)
- **C — Extensibility / module system** (load, package, userdata)

For each gap, the existing decision-only ADR (if any) and a session-count estimate are noted.

## Current state by spec section

### Lua 5.4 §2.1 Types

| Type | Status | Gap |
|---|---|---|
| `nil` | ✅ | — |
| `boolean` | ✅ | — |
| `number` (subtype: `integer`) | ❌ | LuMeLIR has only f64. `math.type` distinguishing `"integer"` vs `"float"`, `//` floor-div integer result, bitwise `~` / `&` / `|` / `<<` / `>>` on integers, `1 == 1.0` value equivalence all absent. |
| `number` (subtype: `float`) | ✅ | — |
| `string` | ✅ | Boxed object, embedded NUL OK per ADR 0112. |
| `function` | ✅ | Closures via heap-cell + upvalue boxes per ADR 0083. |
| `userdata` | ❌ | Bridge MVP (ADR 0191) does not require it for primitive marshaling. Needed when first non-trivial Rust struct is exposed. |
| `thread` (coroutine) | ❌ | No coroutine support at all. |
| `table` | ✅ | Array + hash hybrid with metatables. |

### §2.4 Metatables and Metamethods

| Metamethod | Status | ADR | Notes |
|---|---|---|---|
| `__index` Table form | ✅ | 0134 | Hash + array key paths |
| `__index` Function form | ✅ | 0150 / 0166 | Single + multi-hop |
| `__newindex` Table form | ✅ | 0135 / 0168 | |
| `__newindex` Function form | ✅ | 0151 / 0169 | |
| `__add` / `__sub` / `__mul` / `__div` / `__mod` / `__pow` / `__unm` / `__idiv` | ✅ | 0147 | |
| `__band` / `__bor` / `__bxor` / `__bnot` / `__shl` / `__shr` | ✅ | 0148 | Bitwise relies on integer subtype semantically; today routes through f64 path with caveats. |
| `__eq` / `__lt` / `__le` | ✅ | 0144 | |
| `__concat` / `__tostring` / `__len` | ✅ | 0142 / 0143 | |
| `__call` | ✅ | 0146 | |
| `__gc` | ⚠️ designed | 0163 | Needs GC actual freeing first. |
| `__close` (Lua 5.4 new) | ❌ | — | Paired with `<close>` local attribute. |
| `__mode` (weak tables) | ⚠️ designed | 0164 | Needs GC freeing. |
| `__name` | ❌ | — | Display name in error messages; cosmetic. |
| `__metatable` (hide) | ❌ | — | Documented Non-goal in ADR 0135. |

### §2.5 Garbage Collection

| Item | Status | ADR | Gap |
|---|---|---|---|
| Allocator wrapper | ✅ | 0157 | |
| `g_gc_head` linked list | ✅ | 0158 | All 8 allocator sites migrated |
| Size guard (4 GiB) | ✅ | 0184 | |
| Mark phase | ⚠️ v1 safety mode | 0185 | Walks `g_gc_head` and marks every node BLACK; no DFS, no real roots |
| Sweep phase | ⚠️ v1 safety mode | 0185 | Resets BLACK → WHITE; frees nothing (because nothing is WHITE) |
| Auto-trigger (1 MiB) | ✅ | 0186 | Threshold + doubling logic |
| **Stack walk (real roots)** | ❌ designed | 0160 | Per-frame alloca-slot table |
| **DFS lift (real freeing)** | ❌ | — | Worklist + per-type walks; consumes `gc_type_meta` |
| **`__gc` finalizer dispatch** | ❌ designed | 0163 | |
| **`__mode` weak tables** | ❌ designed | 0164 | |

### §2.3 Error Handling

| Item | Status | ADR | Gap |
|---|---|---|---|
| `error(msg)` parse | ✅ | — | Builtin |
| `pcall(f, ...)` | ❌ designed | 0153 | Phase 4 |
| `xpcall(f, msgh, ...)` | ❌ | — | Phase 4 |
| Error value propagation through frames | ❌ | — | Requires unwinding infrastructure |
| `assert(v, msg)` | ✅ | — | |

### §2.2 Environment / Globals

| Item | Status | ADR | Gap |
|---|---|---|---|
| Auto-declared globals (current) | ✅ | 0048 | Functional substitute |
| **True `_ENV`** | ❌ designed | 0154 | Phase 4 |
| **`_G` global table** | ❌ | — | Depends on `_ENV` |

### §3 The Language — syntax + statements

| Item | Status | Notes |
|---|---|---|
| Lexical (numbers, identifiers, comments, basic strings) | ✅ | |
| Hex literals `0xFF`, sci notation `1e5` | ⚠️ partial — probe required | |
| Long strings `[[...]]`, long comments `--[[...]]` | ⚠️ probe required | |
| `if` / `elseif` / `else` | ✅ | |
| `while` / `repeat...until` / `for` (numeric + generic) | ✅ | |
| `break` | ✅ | |
| `do...end` block | ✅ | |
| `local` | ✅ | |
| **`goto` / `::label::`** | ❌ | Lua 5.2+ feature |
| **`return` from middle of block** | ⚠️ probe required | |
| **`<close>` / `<const>` local attributes** | ❌ | Lua 5.4 new |
| Function definitions + closures | ✅ | |
| Table constructor `{e1, e2}` (list form) | ✅ | |
| **Table constructor `{k=v}` (named key)** | ❌ | Parser confirmed gap (probe 2026-06-14) |
| **Table constructor `{[k]=v}` (bracket key)** | ⚠️ probe required | |
| Method call `o:m()` | ✅ | ADR 0092 + 0183 |
| Multi-assignment / multi-return | ✅ | |
| Varargs `...` | ⚠️ probe required | |

### §6.1 Basic Functions

| Function | Status |
|---|---|
| `assert` | ✅ |
| `collectgarbage` | ⚠️ partial (no-op stub for freeing in v1) |
| `error` | ✅ (parse only; no `pcall`) |
| `getmetatable` / `setmetatable` | ✅ |
| `ipairs` / `pairs` | ✅ |
| `next` | ✅ (arity 2 — Lua 5.4 allows arity 1) |
| `print` | ✅ |
| `rawequal` / `rawlen` / `rawget` / `rawset` | ✅ |
| `select` | ❌ |
| `tonumber` / `tostring` / `type` | ✅ |
| `xpcall` / `pcall` | ❌ |
| `load` / `loadfile` / `dofile` | ❌ |

### §6.4 String Manipulation

| Function | Status | ADR |
|---|---|---|
| `string.len` / `upper` / `lower` | ✅ | 0103 |
| `string.sub` | ✅ | 0104 |
| `string.rep` | ✅ | 0105 |
| `string.byte` | ✅ | 0109 |
| `string.char` | ✅ | 0113 |
| `string.format` | ✅ partial | 0152 — supports `%d/%f/%s/%%` only |
| `string.find` / `match` / `gmatch` / `gsub` | ❌ designed | 0155 |
| `string.reverse` | ❌ | — |

### §6.6 Table Manipulation

| Function | Status | ADR |
|---|---|---|
| `table.concat` | ✅ | 0106 |
| `table.insert` | ✅ | 0111 |
| `table.remove` | ✅ | 0118 |
| `table.pack` / `unpack` / `sort` / `move` | ❌ | — |

### §6.7 Math

| Function | Status |
|---|---|
| `math.sqrt` / `floor` / `abs` / `pow` / `sin` / `cos` / `log` / `exp` | ✅ |
| `math.ceil` / `fmod` / `max` / `min` / `atan` / `tan` / `asin` / `acos` | ❌ |
| `math.random` / `randomseed` | ❌ (stateful — needs PRNG state) |
| `math.huge` / `pi` / `maxinteger` / `mininteger` (constants) | ❌ |
| `math.type` / `tointeger` | ❌ (requires integer subtype) |

### §6.8 Input/Output

| Function | Status | ADR |
|---|---|---|
| `io.write` | ✅ | 0116 |
| `io.read` | ✅ | 0119 |
| `io.open` / `close` / `lines` / `popen` / `tmpfile` | ❌ | — |
| `io.input` / `output` / `stderr` / `stdout` | ❌ | — |
| File handles (userdata) | ❌ | Depends on userdata |

### §6.9 Operating System

| Function | Status |
|---|---|
| `os.time` / `date` / `clock` / `difftime` | ❌ |
| `os.getenv` / `setenv` | ❌ |
| `os.exit` | ❌ |
| `os.remove` / `rename` / `tmpname` | ❌ |

### §6.2 Coroutines

| Function | Status |
|---|---|
| `coroutine.create` / `resume` / `yield` / `wrap` / `status` / `running` / `isyieldable` | ❌ — entire library blocked on coroutine runtime |

### §6.3 Modules

| Function | Status |
|---|---|
| `require` / `package.*` | ❌ — needs module system + likely `load` |

### §6.5 utf8

| Function | Status |
|---|---|
| `utf8.char` / `codepoint` / `len` / `offset` / `codes` | ❌ |

### §6.10 debug

| Function | Status |
|---|---|
| `debug.traceback` / `getinfo` / `getlocal` / `getupvalue` / `sethook` etc. | ❌ |

## Conformance gap summary

### Priority A — Language / runtime spec obligations

| Workstream | Estimated ADR count | Session estimate | Blocker for |
|---|---|---|---|
| Integer/Float subtype (number split) | 5-8 | 4-8 | bitwise correctness, `//`, `math.type`, `tointeger`, equality semantics |
| Coroutines | 3-5 | 4-6 | coroutine lib, async patterns |
| `goto` / labels | 1 | 0.5-1 | nothing else |
| `<close>` / `<const>` attributes | 2 | 1-2 | `__close` metamethod |
| Table named-key constructor `{k=v}` | 1 | 0.5 | idiomatic table literals |
| `pcall` / `xpcall` / `error` propagation | 2-3 | 2-3 | Bridge error propagation |
| `_ENV` / `_G` | 2-3 | 3-5 | dynamic globals, library impls |
| GC real freeing (stack walk + DFS lift) | 2-3 | 3-5 | `__gc`, `__mode`, Bridge GC |
| `__gc` finalizer | 1-2 | 1-2 | resource management |
| `__close` | 1 | 1 | RAII-like cleanup |
| `__mode` weak tables | 2-3 | 2-3 | caches |

**A total**: 22-34 ADRs, 22-39 sessions.

### Priority B — Standard library coverage

| Workstream | Estimated ADR count | Session estimate |
|---|---|---|
| String patterns (find/match/gmatch/gsub) | 3-4 | 4-6 — pattern engine itself is large |
| math library completion (ceil/fmod/max/min/trig/random/constants) | 3-5 | 3-5 |
| table.pack/unpack/sort/move | 3-4 | 3-4 |
| io.open/close/lines (depends on userdata) | 2-3 | 2-3 |
| os library | 2-3 | 2-3 |
| utf8 library | 2-3 | 2-3 |
| coroutine library (after A coroutine runtime) | 1-2 | 1-2 |
| debug library subset | 2-3 | 2-3 |
| Basic function completions (select, load, loadfile, dofile) | 2-3 | 4-6 (load is huge) |

**B total**: 20-30 ADRs, 23-34 sessions.

### Priority C — Extensibility / module system

| Workstream | Estimated ADR count | Session estimate |
|---|---|---|
| userdata (for Bridge non-primitive marshaling) | 2-3 | 2-3 |
| package / require | 3-5 | 3-5 |
| load / loadfile / dofile (runtime parser+codegen invocation) | 2-3 | 4-6 — self-hosting territory |

**C total**: 7-11 ADRs, 9-14 sessions.

## Aggregate

- **ADRs**: 49-75 new ADRs to reach Lua 5.4 preferred-subset conformance
- **Sessions**: 54-87 single-feature sessions (variance reflects unknown unknowns)
- **Calendar**: at one substantial ADR per ~1-hour session, full conformance is **~3-6 months** of focused work

## Highest-leverage workstreams (where to spend first)

Ranked by "conformance unlock per session":

1. **Integer/Float subtype** — biggest single language-level gap. Every later workstream that touches arithmetic / bitwise / comparison would carry temporary hacks if this lands late. **Do early.**
2. **`pcall` / `error` propagation** — unblocks Bridge error propagation (ADR 0191 §Future), enables stdlib error-returning functions to behave correctly.
3. **GC real freeing** — removes the v1 leak, enables `__gc`, makes Bridge GC interaction concrete. ADR 0160 design exists; mostly implementation.
4. **String patterns** — single largest stdlib gap. Pattern engine is self-contained.
5. **Coroutines** — only unblocks itself + coroutine lib, but conceptually a runtime mountain. Defer until last A item.
6. **`load`** — also a mountain (runtime parser/codegen), and most production-Lua programs don't `load`. Defer.

Combined recommendation: **start with Integer/Float subtype**. The longer it waits, the more retrofit pressure on bitwise, math, comparison, and metamethod dispatchers.

## Recommended phase decomposition

Aligns with ADR 0193 §Workstreams + this analysis:

- **Phase 4a — language-level conformance** (Integer/Float, `pcall`, `_ENV`, GC freeing, `goto`, `<close>`/`<const>`, table named-key, finalizer)
- **Phase 4b — standard library expansion** (string patterns, math/table/utf8/os/io expansion, debug subset)
- **Phase 4c — extensibility** (userdata + Bridge sub-pieces, package/require)
- **Phase 4d — runtime evaluation** (load / loadfile / dofile — defer-able if static-only Lua is acceptable)
- **Phase 5 (optional) — Coroutines** — large enough to warrant its own phase if the project goes that far

Each "Phase 4x" is a sub-phase, all conceptually inside ADR 0193's Phase 4. The decomposition is for sequencing only; no new meta-ADR is required unless the project decides to split formally.

## Open probes (unknowns to resolve before scoping)

| Item | Action |
|---|---|
| Hex / sci-notation literal support | Read lexer; probe `0xFF` and `1e5` end-to-end |
| Long strings `[[...]]` | Same as above |
| Bracket-key table constructor `{[1]=v}` | Probe parser |
| Vararg `...` in function bodies | Probe — existing tests likely cover, confirm scope |
| `return` from middle of block (non-tail return) | Probe |
| `next(t)` arity 1 (Lua 5.4 allows nil default key) | Confirmed gap from earlier MethodCall audit; current arity is (2,2) |

Resolving these tightens estimates ~10-20%.

## References

- [Lua 5.4 Reference Manual](https://www.lua.org/manual/5.4/) — primary source
- ADRs 0048, 0083, 0103, 0106, 0109, 0111, 0113, 0116, 0118, 0119, 0134, 0135, 0142, 0143, 0144, 0146, 0147, 0148, 0150, 0151, 0152, 0153, 0154, 0155, 0157, 0158, 0163, 0164, 0166, 0168, 0169, 0184, 0185, 0186 — feature implementations and decision records cited in the §Current state tables
- ADR 0193 — Phase 4 entry criteria
- PRD `docs/PRD.jp.md` §6 / §7 — phase definition and success metrics
