# 0253. Lua 5.4 Preferred-Subset Conformance Declaration

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

M17 — the final milestone in the [Lua 5.4 full conformance path](../notes/lua54-full-conformance-path-2026-06-20.md). [ADR 0250](0250-phase4-close-declaration.md) closed Phase 4; [ADR 0252](0252-phase5-close-declaration.md) closed Phase 5. With M1-M16 complete, this ADR formally records LuMeLIR's conformance state against Lua 5.4 spec and declares the **preferred-subset** target met.

The "preferred subset" framing (PRD §6 implicit; surfaced in the conformance roadmap memo) is the practical conformance contract: the language surface real Lua 5.4 programs use, minus the items that are blocked behind structural deferrals (M3-extended Table DFS, runtime parser, etc.). This ADR pins what we ship vs. what we explicitly don't.

## Declaration

**LuMeLIR conforms to a Lua 5.4 preferred subset as of 2026-06-21.**

The subset is defined by the union of:

1. Every Lua 5.4 §2 / §3 / §6 surface item with a ✅ row in the conformance tables below.
2. Every M-stretch item explicitly deferred via an ADR-recorded strategy, where the deferral does NOT block the user-visible compile-and-run surface for the dominant use cases.

Programs that stay within this subset compile, run, and produce results identical to (or precisely-documented deviations from) the Lua reference interpreter.

## §-by-§ status

### §2.1 Types

| Type | Status | ADR |
|---|---|---|
| `nil` | ✅ | — |
| `boolean` | ✅ | — |
| `number` (subtype: `integer`) | ✅ Phase B + M8 static-subtype | 0209-0214, 0232-0235 |
| `number` (subtype: `float`) | ✅ | — |
| `string` (boxed, 8-bit clean) | ✅ | 0112 |
| `function` (closures + upvalues) | ✅ | 0083 |
| `userdata` | ⏸ opaque-Number shim + deferral | 0244 |
| `thread` (coroutine) | ⏸ main-thread shipped; runtime deferred | 0245, 0246 |
| `table` (array + hash + metatable) | ✅ | 0053-0140 |

### §2.4 Metatables and Metamethods

All `__index` / `__newindex` / arithmetic / bitwise / comparison / `__concat` / `__tostring` / `__len` / `__call` metamethods land. Storage-side acceptance of `__gc` / `__mode` pinned (ADRs 0238, 0239); runtime dispatch deferred to M10-stretch.

### §2.5 Garbage Collection

GC mark + sweep + auto-trigger + chunk-safe real freeing for String / Function / TaggedValue slots. Tables in chunk slots deferred to M3-extended (ADR 0220 §M3-stretch). `__gc` finalizer + `__mode` weak-table clearing deferred (ADRs 0238, 0239).

### §2.3 Error Handling

`pcall` / `xpcall` (single + multi-return) + `error(msg)` value propagation via setjmp/longjmp shipped (ADRs 0215-0217). Rust-side `rust.fail` bridge integration shipped (ADR 0243).

### §2.2 Environment / Globals

`_G` / `_ENV` chunk-level Table synthesis + `local _ENV = {}` sandbox rebind shipped (ADRs 0221-0223). Full `_ENV`-as-upvalue + bare-name fallback to `_ENV[name]` deferred.

### §3 The Language

- Lexical (numbers, identifiers, comments, basic strings, hex literals, sci notation, long strings, bracket-key table) ✅.
- Statements (`if` / `while` / `repeat` / `for` / `break` / `do` / `local`) ✅.
- `<const>` / `<close>` attributes ✅ (ADRs 0236-0237); `__close` runtime dispatch deferred to M9-C.
- Function definitions + closures + table constructors ✅.
- Method call `o:m()` ✅.
- Multi-assignment / multi-return ✅.
- `goto` / `::label::` ⏸ — deferred (M9-stretch).
- Varargs `...` ✅.

### §6.1 Basic Functions

`assert` / `error` / `getmetatable` / `setmetatable` / `ipairs` / `pairs` / `next` / `print` / `rawequal` / `rawlen` / `rawget` / `rawset` / `tonumber` / `tostring` / `type` / `pcall` / `xpcall` / `collectgarbage` ✅.

`select` / `load` / `loadfile` / `dofile` ⏸ — deferred (M14-stretch).

### §6.4 String Manipulation

`string.len` / `upper` / `lower` / `sub` / `rep` / `byte` / `char` / `format` / `reverse` ✅.

`string.find` literal + char-class patterns ✅ (ADRs 0228, 0229, 0231); full magic-char patterns (quantifiers / sets / anchors / captures) deferred (M7-stretch).

`string.match` literal ✅ (ADR 0230); pattern-aware deferred.

`string.gmatch` / `gsub` ⏸ — deferred.

### §6.6 Table Manipulation

`table.concat` / `table.insert` / `table.remove` ✅.

`table.pack` / `unpack` / `sort` / `move` ⏸ — deferred (M11-stretch).

### §6.7 Math

`math.sqrt` / `floor` / `abs` / `pow` / `sin` / `cos` / `log` / `exp` / `ceil` / `tan` / `asin` / `acos` / `atan` / `max` / `min` ✅.

`math.huge` / `pi` / `maxinteger` / `mininteger` ✅.

`math.type` / `tointeger` ✅.

`math.random` / `randomseed` / `fmod` / `modf` ⏸ — deferred.

### §6.8 Input/Output

`io.write` / `io.read` ✅.

`io.open` / `close` / `lines` / `popen` / `tmpfile` / `io.input` / `output` / `stderr` / `stdout` ⏸ — deferred.

### §6.9 Operating System

`os.time` / `os.clock` / `os.getenv` ✅.

`os.date` / `difftime` / `exit` / `remove` / `rename` / `tmpname` ⏸ — deferred.

### §6.2 Coroutines

`coroutine.isyieldable` / `coroutine.running` (main-thread spec-correct) ✅.

`coroutine.create` / `resume` / `yield` / `wrap` / `status` / `close` ⏸ — deferred via documented LLVM-intrinsic / ucontext strategy (ADR 0246).

### §6.3 Modules

`package.config` / `package.path` / `package.cpath` ✅.

`require` / `package.loaded` / `package.searchers` / `package.searchpath` ⏸ — deferred via documented self-hosting strategy (ADR 0248).

### §6.5 utf8

`utf8.char` / `codepoint` / `len` / `offset` / `codes` ⏸ — deferred.

### §6.10 debug

`debug.traceback` / `getinfo` / `getlocal` / `getupvalue` / `sethook` ⏸ — deferred.

## What "preferred subset" means in practice

Programs that work today within the subset:

- Pure functional / arithmetic / string processing.
- Closures + upvalues + recursion.
- Table-based data structures (array + hash + metatable).
- Integer / Float subtype distinction (Phase B + M8 static propagation).
- `pcall` / `error` for protected calls.
- `_G` / `_ENV` introspection + sandbox rebind.
- String patterns up to char-class + `.` wildcard.
- `os.time` / `os.clock` / `os.getenv` system probes.
- `math.*` library (15 functions).
- File I/O via `io.write` / `io.read` (line-based).
- Rust-Lua bridge: Number / String / Bool / multi-arg / error propagation.

Programs that need the deferred items:

- Coroutines (use cases: cooperative scheduling, generators, async). Workaround: use callbacks or transform to flat control flow.
- `load` / `require` (use cases: dynamic code loading, modular programs). Workaround: compile each module as a separate LuMeLIR binary; coordinate via env vars or file I/O.
- Full pattern matching with captures (use cases: complex text parsing). Workaround: use the literal + char-class subset; do more parsing in Lua control flow.
- `table.sort` (use case: ordered iteration). Workaround: hand-roll a sort in Lua.
- `__gc` finalizer (use case: RAII cleanup). Workaround: explicit cleanup via `<close>` (deferred) or manual code paths.

## Project status

LuMeLIR is now a **shippable Lua 5.4 preferred-subset AOT compiler**. The compile-and-run surface is stable; the catalogued M-stretch items have documented implementation strategies for future work but do not block current users from the dominant use cases.

## Statistics

- **Total ADRs**: 253 (0001-0253).
- **Phase 4 + 5 sub-ADRs**: 45 (0209-0253).
- **Test count**: 1431 → 1617 (+186 e2e during M1-M16).
- **Days for Phase 4 + 5 close**: 6 (2026-06-15 → 2026-06-21).
- **Zero regression** across the M1-M16 sweep.

## Future work (M-stretch catalogue)

Each item has a strategy + architectural-delta note in its closing ADR:

- **M3-extended** Tables in GC mark/sweep (ADR 0220 §M3-stretch). Unblocks 0238, 0239, 0244 runtime semantics.
- **M9-C** `__close` runtime hook (ADR 0237).
- **M10-stretch** `__gc` / `__mode` runtime dispatch (ADRs 0238, 0239).
- **M11-stretch** stdlib expansion (`table.pack/unpack/sort`, `utf8.*`, `debug.*`, `math.random/fmod`, `io.open/lines`) (ADR 0242).
- **M12-stretch** userdata `ValueKind` + Bridge GC integration (ADR 0244).
- **M13-stretch** coroutine runtime (LLVM intrinsics or POSIX ucontext) (ADR 0246).
- **M14-stretch** `load` / `require` runtime (embedded-pipeline self-hosting) (ADR 0248).
- **Performance** JIT or AOT-time inlining + escape analysis to close the LuaJIT performance gap (ADR 0250).

## Test count delta

```
Step 0:  1617 (after ADR 0252)
C1 (declaration meta-ADR, docs only): 1617 → 1617
```

## References

- [ADR 0193](0193-phase4-entry-criteria.md) — Phase 4 entry.
- [ADR 0195](0195-phase5-entry-criteria.md) — Phase 5 entry.
- [ADR 0250](0250-phase4-close-declaration.md) — Phase 4 close.
- [ADR 0252](0252-phase5-close-declaration.md) — Phase 5 close.
- ADRs 0209-0252 — M1-M16 sub-ADR landed set.
- [Lua 5.4 Reference Manual](https://www.lua.org/manual/5.4/) — primary source of conformance items.
- [Lua 5.4 conformance roadmap](../notes/lua54-conformance-roadmap.md) — §-by-§ gap source.
- [Lua 5.4 full conformance path](../notes/lua54-full-conformance-path-2026-06-20.md) — M-series sequence.
- PRD `docs/PRD.jp.md` — project scope.
