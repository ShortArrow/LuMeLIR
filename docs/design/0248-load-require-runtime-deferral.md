# 0248. `load` / `require` Runtime — Strategy Decision + M14-Stretch Deferral

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second (closing) M14 sub-ADR. [ADR 0247](0247-package-table-synthesis.md) shipped the `package.config` / `path` / `cpath` constants. The remaining `load` / `loadfile` / `dofile` / `require` / `package.loaded` / `package.searchers` machinery all need a runtime that can parse + compile + execute Lua source. For an AOT compiler that's "self-hosting territory" — embedding the lexer + parser + HIR + codegen + linker chain into every produced binary.

This ADR records the strategy decision and the explicit deferral. The implementation is multi-session work; future ADRs will land each piece.

## Scope (literal)

- ✅ Document the candidate strategies for runtime parsing + codegen invocation.
- ✅ Enumerate architectural deltas needed regardless of strategy choice.
- ✅ Pin the M14-stretch test plan as a contract for the future implementation ADR.
- ❌ Implement any of the runtime. M14-stretch — separate multi-session ADR.
- ❌ Commit to a single strategy. Multiple paths remain viable.

## Decision

### Strategy ranking

1. **Embedded LuMeLIR pipeline** (self-hosting). Compile the lexer + parser + HIR + codegen into the runtime, exposed via `lumelir_load_string(src) -> closure_ptr`. Pure Lua semantics; benefits from incremental improvements to the compiler. **Cost:** binary size jumps significantly (the compiler is large). **Long-term answer for most use cases.**

2. **External tinted Lua bytecode loader**. Pre-compile imports to a serialised IR, ship a thin loader that links the IR into the running module's call dispatch. **Trade-off:** avoids embedding the parser at runtime but requires a tighter IR + serialisation contract.

3. **Pre-baked AOT-compiled imports**. At build time, statically resolve `require` calls and bake the imported modules into the binary. **Trade-off:** trivial runtime cost; but breaks dynamic `require(arg[1])`-style usage.

4. **Bridge-style userdata-typed module handle**. `require` returns a userdata that carries the foreign-module's namespace. **Limit:** requires the M3-extended userdata implementation; doesn't actually load Lua code, just exposes pre-baked modules.

### Architectural deltas needed (strategy-independent)

- **Runtime symbol table** for `package.loaded`. Map String → Closure-cell ptr.
- **Module-cache invariant**: `require("x")` returns the same table on repeated calls.
- **Search-path resolution**: walk `package.path` / `package.searchers` to locate a `.lua` source for a name.
- **Sandbox semantics**: a loaded module sees its own `_ENV` per Lua 5.4 §6.3.
- **Error propagation**: a `require` failure should raise `error("module 'x' not found")` via the ADR 0215 path.
- **GC interaction**: cached modules root through `package.loaded`, must survive `collectgarbage()` across the M3-extended Table DFS gate.

### Pre-conditions

- **Self-hosting strategy** needs the compiler to compile itself. Today the compiler is written in Rust and depends on melior; embedding it in the produced binary requires a significant build-system pivot.
- **External-bytecode strategy** needs a stable serialisation format that survives compiler revisions.
- **Pre-baked AOT strategy** needs a build-time mechanism to detect `require` calls and trigger imports.
- **All strategies** benefit from M3-extended Table DFS (the `package.loaded` cache needs to be GC-tracked).

### M14-stretch test plan (deferred)

When the runtime implementation lands, the test suite should pin:

- `load("return 1+1")()` returns `2`.
- `loadstring` is the Lua 5.1-compatibility alias; should match `load`.
- `loadfile(path)` reads + compiles a file.
- `dofile(path)` reads + compiles + executes in one step.
- `require("mod")` loads `mod.lua` from `package.path`, returns the module's value.
- Module cache: `require("mod")` twice returns the same Table.
- Failed `require` raises a clear error: `"module 'mod' not found"`.
- `package.searchpath("foo", "./?.lua")` returns the resolved path or nil.
- Cyclic `require` is detected and errors gracefully.

## M14 milestone close

ADRs 0247 + 0248 close M14 at 2/2-3 minimum-viable.

M14-stretch (deferred):
- `load` / `loadstring` / `loadfile` / `dofile`.
- `require` + `package.loaded` cache.
- `package.searchers` + `package.searchpath`.
- Sandboxed `_ENV` per-loaded-module.
- Cyclic-require detection.
- Self-hosting OR pre-baked OR external-bytecode strategy implementation.

## Test count delta

```
Step 0:  1617 (after ADR 0247)
C1 (docs only): 1617 → 1617
```

## References

- [Lua 5.4 §6.3](https://www.lua.org/manual/5.4/manual.html#6.3) — package library spec.
- [Lua 5.4 §6.1 `load`](https://www.lua.org/manual/5.4/manual.html#pdf-load) — primary loader spec.
- [ADR 0247](0247-package-table-synthesis.md) — sibling M14 sub-ADR.
- [ADR 0246](0246-coroutine-runtime-deferral.md) — sibling milestone-close strategy/deferral pattern.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M14 milestone close.
