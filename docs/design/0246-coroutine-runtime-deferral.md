# 0246. Coroutine Runtime — Strategy Decision + M13-Stretch Deferral

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second (closing) M13 sub-ADR. [ADR 0245](0245-coroutine-main-thread-introspection.md) shipped the two `coroutine.*` functions that work without a runtime. The five remaining (`create` / `resume` / `yield` / `wrap` / `status`) all need stack switching. This ADR pins the runtime strategy decision so a future implementation ADR has a clear starting point, and documents what blocks M13-stretch.

## Scope (literal)

- ✅ Strategy decision: **LLVM coroutine intrinsics** are the long-term path; **POSIX `ucontext`** is the pragmatic short-term path if the project ships before the intrinsics integration lands.
- ✅ Enumerate the architectural deltas needed regardless of strategy choice.
- ✅ Document the M13-stretch test plan.
- ❌ Implement any of the runtime. M13-stretch — separate multi-session ADR.
- ❌ Pick a single strategy without escape hatches. Both strategies stay viable until the implementation ADR.

## Decision

### Strategy ranking

1. **LLVM coroutine intrinsics** (`llvm.coro.id`, `llvm.coro.begin`, `llvm.coro.suspend`, `llvm.coro.resume`, `llvm.coro.end`). LLVM-native; works for the AOT target without runtime dependencies. Requires emitting the coroutine intrinsics in MLIR LLVM dialect and integrating with the upstream LLVM coroutine passes. The cleanest long-term answer.

2. **POSIX `ucontext` (`makecontext` / `swapcontext` / `getcontext`)**. Mature, portable to most Unix platforms. Officially deprecated by POSIX-2008 but still ubiquitous and stable. Faster path to first working coroutine. Wraps cleanly via libc externs.

3. **Hand-rolled `setjmp` + stack swap**. Reuses the existing `_setjmp` infrastructure (ADR 0215) but requires per-target inline-assembly for stack switching. Skipped — not portable enough to justify over the other two.

4. **Stackful via threads**. Each coroutine runs on a real OS thread, synchronised via condvars. Rejected — way too heavy for the per-coroutine overhead, against Lua spec ("coroutines are cheap").

### Architectural deltas needed (strategy-independent)

- **New `ValueKind::Thread`** (or TaggedValue tag bit). Coroutines are first-class Lua values per §2.1.
- **Per-coroutine state struct**: stack pointer, suspended PC, value-passing slots, parent-coroutine ptr.
- **`coroutine_table` global**: maps Thread handles → state struct ptrs for runtime introspection (`coroutine.status` reads from it).
- **GC integration**: live coroutines participate in mark + sweep. Blocked on M3-extended.
- **`yield` longjmp-equivalent**: needs to bridge from the coroutine's stack back to the resume site without unwinding host code. setjmp/longjmp won't compose cleanly with `pcall`'s setjmp — need a parallel jmpbuf chain or LLVM's coroutine.suspend.
- **Main-thread isyieldable / running updates**: when inside a coroutine, `isyieldable` reads the active state, `running` returns the current thread.

### Pre-conditions

- M3-extended (Table DFS) for GC interaction. Coroutine objects need to be GC-tracked so leaked coroutines free.
- M2-extended (error-propagation interaction). A `yield` inside a `pcall` body has spec-defined behavior.

### M13-stretch test plan (deferred)

When the runtime implementation lands, the test suite should pin:

- `coroutine.create + resume + yield` round-trip with value passing both ways.
- `coroutine.wrap` simpler-callable form.
- `coroutine.status` returns the four states: `suspended`, `running`, `normal`, `dead`.
- `coroutine.isyieldable` returns `true` inside an active coroutine.
- `coroutine.running` returns `(thread, false)` inside an active coroutine.
- yield + resume + value passing across many round trips (1000+) without leaking memory.
- Nested coroutines (coroutine inside coroutine).
- `pcall(coroutine.resume(co))` catches errors raised inside the coroutine.

## M13 milestone close

ADRs 0245 + 0246 close M13 at 2/2-3 minimum-viable.

M13-stretch (deferred):
- Coroutine runtime (LLVM coroutine intrinsics preferred, `ucontext` fallback).
- `coroutine.create / resume / yield / wrap / status / close`.
- `ValueKind::Thread`.
- GC interaction (needs M3-extended).
- `isyieldable` / `running` updates to read the active coroutine state.

## Test count delta

```
Step 0:  1611 (after ADR 0245)
C1 (docs only): 1611 → 1611
```

## References

- [Lua 5.4 §6.2](https://www.lua.org/manual/5.4/manual.html#6.2) — coroutine library spec.
- [LLVM Coroutine Intrinsics](https://llvm.org/docs/Coroutines.html) — long-term strategy candidate.
- [POSIX `ucontext`](https://pubs.opengroup.org/onlinepubs/9699919799/functions/makecontext.html) — pragmatic short-term candidate.
- [ADR 0215](0215-pcall-setjmp-infrastructure.md) — setjmp infra that the runtime composes against.
- [ADR 0220](0220-gc-tagged-value-roots.md) — GC scope that gates the runtime.
- [ADR 0245](0245-coroutine-main-thread-introspection.md) — main-thread intro that this ADR closes M13 around.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M13 milestone close.
