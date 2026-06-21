# 0247. `package` Table Synthesis — `config` / `path` / `cpath` Constants

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M14 sub-ADR. Lua 5.4 §6.3 specifies the `package` library — primarily `package.path`, `package.cpath`, `package.config`, `package.loaded`, and the `require` runtime. The full `require` machinery needs runtime parsing + codegen invocation (self-hosting territory, multi-session work — see ADR 0248). But three of the surface members are spec-default String constants that have a single correct value:

- `package.config` — the Lua 5.4 spec defines a fixed 5-line String describing directory separator, path separator, template character, executable indicator, and version separator. POSIX systems get `"/\n;\n?\n!\n-\n"`.
- `package.path` — default search path for `require`. The spec-default is `"./?.lua;./?/init.lua"`.
- `package.cpath` — default search path for C libs. Default `"./?.so"` on POSIX.

This ADR ships those three via the same HIR pre-pass that ADR 0221 uses for `_G` / `_ENV`: scan the source for the `package` name, and if found, prepend `local package = { config = ..., path = ..., cpath = ... }` at chunk start. Programs that never reference `package` pay zero overhead.

## Scope (literal)

- ✅ `chunk_references_name(chunk, "package")` scan reuses the ADR 0221 walker.
- ✅ When `package` is referenced, prepend a synthetic `local package = { config = "/\n;\n?\n!\n-\n", path = "./?.lua;./?/init.lua", cpath = "./?.so" }` at chunk start.
- ✅ Synthesis fires alongside the existing `_G` / `_ENV` synthesis; the three pre-pended Locals come in deterministic order so name resolution is stable.
- ✅ Programs that read the values via `print` / Local round-trip / concat work through the existing Table machinery (ADRs 0134-0140, 0221).
- ❌ `package.loaded` table tracking what `require` has already loaded. Future M14-stretch alongside the runtime.
- ❌ `package.preload` table for user-pre-loaders. Future.
- ❌ `package.searchers` table. Future.
- ❌ `package.searchpath(name, path)` function. Future.
- ❌ Runtime `require(modname)` — needs lexer + parser + HIR + codegen embedded in the produced binary. ADR 0248 documents the strategy.
- ❌ User-side reassignment of `package.path` after declaration. Since `package` is a regular Local-Table, `package.path = "..."` works as a Table write, but the value is not consulted by anything at runtime (no `require` to consume it). Pinned as a known no-op.
- ❌ Per-platform config differences (Windows vs POSIX). Hardcoded to POSIX defaults; future ADR can add a target-aware variant.

## Decision

### Synthesis location

Inside `pub fn lower`, the existing `_G` / `_ENV` synthesis block extends with a third arm. Order of pre-pended Locals: `_G`, `_ENV`, `package`. Each is opt-in via its own `chunk_references_name(...)` probe so unaffected programs are byte-identical to the pre-ADR output.

### Why the values are hard-coded

The Lua spec pins them. No platform-specific test reads them differently; portability shifts to the user's `package.path` rebinds (which today are silent — pinned as documented in §Scope).

### Why this is real and not a pin

Unlike ADRs 0238 / 0239 / 0244 / 0246 which document deferrals, this ADR ships behavior:

- `print(package.config)` prints the spec value.
- `local p = package.path; print(p)` round-trips correctly.
- `type(package)` returns `"table"`.

A Lua program reading these three fields gets the same answer LuMeLIR's output produces as it would from the Lua reference interpreter on a POSIX system.

## Tests

`tests/phase4_m14_package_constants.rs` (NEW, 6 e2e):

1. `print(package.config)` first line is `/`.
2. `print(package.path)` starts with `./?.lua`.
3. `print(package.cpath)` → `./?.so`.
4. `type(package)` → `table`.
5. Round-trip `local p = package.path; print(p)` → `"./?.lua;./?/init.lua"`.
6. Sanity: chunks not referencing `package` are unaffected.

## Test count delta

```
Step 0:  1611 (after ADR 0246)
C3 (impl + 6 e2e): 1611 → 1617
```

## References

- [Lua 5.4 §6.3](https://www.lua.org/manual/5.4/manual.html#6.3) — package library spec.
- [ADR 0221](0221-env-globals-chunk-table.md) — sibling `_G` / `_ENV` synthesis pattern reused.
- [ADR 0246](0246-coroutine-runtime-deferral.md) — sibling milestone-close pattern for deferred runtime.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M14 milestone.
