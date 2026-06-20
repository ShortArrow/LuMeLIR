# 0221. `_G` and `_ENV` as Chunk-Level Tables

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M4 sub-ADR. [ADR 0154](0154-env-true-globals-strategy.md) decided the long-term strategy: a chunk-level implicit `function(_ENV)` wrapping the chunk body, with every bare Ident lowering to `_ENV[name]`. That migration is large — every existing `Builtin::*` early-bind path needs a dual `_ENV.print`-style fallback, plus the closure-cell ABI grows by one upvalue per fn.

This ADR lands the narrow user-facing surface: `_G[k] = v` and `_G[k]` work as expected, and `_ENV` is a synonym at chunk scope. Existing ADR 0048 auto-declared-locals stays in place for builtin name resolution. Programs that never reference `_G` or `_ENV` are unaffected (including the ADR 0218 chunk-safe GC predicate).

## Scope (literal)

- ✅ Pre-pass at the top of `lower(chunk)` scans the AST for bare `Ident("_G")` or `Ident("_ENV")`. The walker covers every `StmtKind` / `ExprKind` recursively (Local / LocalMulti / Assign / IndexAssign / AssignMulti / Block / If / While / Repeat / ForNumeric / ForIpairs / ForPairs / ForGeneric / Return / ReturnMulti / FunctionDef / MethodDef / Break / ExprStmt; Call / BinOp / UnaryOp / FunctionExpr / Table / Index / MethodCall / scalar literals).
- ✅ When found, prepend `local _G = {}` to the chunk. If `_ENV` is also referenced, prepend `local _ENV = _G` after it (aliased binding).
- ✅ Existing Table machinery handles `_G.x = v` / `_G[k] = v` / `_G.x` / `_G[k]` reads and writes — no codegen change.
- ❌ Builtin migration to `_ENV` entries. `print` / `tostring` / etc. still resolve via the HIR early-bind path. Programs that try `_G.print = my_print; my_print()` won't shadow the builtin.
- ❌ Bare-name fallback to `_ENV[name]`. Free-name resolution still goes through ADR 0048 auto-declared locals.
- ❌ Nested function `_ENV` upvalue. The synthesized `_G` is a chunk-level Local; user functions don't capture it. `local function f() _G.x = 1 end` would HIR-reject because `_G` is not visible inside f (matches ADR 0083 closure rules) — by this scope literal, users put `_G[]` access at chunk level.
- ❌ `local _ENV = something` rebind. The synthetic `_ENV` is a fixed alias of `_G`; user-level rebind would require detection + suppression of the synthesizer, deferred to a future M4 sub-ADR.
- ❌ `setfenv` / `getfenv`. Lua 5.4 dropped these; out of scope by language version target.

## Decision

### AST pre-pass (HIR `lower`)

```rust
let needs_g = chunk_references_name(chunk, "_G");
let needs_env = chunk_references_name(chunk, "_ENV");
if needs_g || needs_env {
    let mut new_chunk = Vec::new();
    new_chunk.push(stmt!("local _G = {}"));
    if needs_env { new_chunk.push(stmt!("local _ENV = _G")); }
    new_chunk.extend(chunk.iter().cloned());
    lower(&new_chunk)
} else {
    lower(chunk)   // unchanged
}
```

The scanner is a pure recursive walk over `Stmt` / `Expr`; cost is O(N) in the AST size. Programs without `_G` / `_ENV` references pay zero overhead and stay byte-for-byte identical to pre-ADR-0221 output.

### Table choice

A plain Lua table literal `{}`. Tables in LuMeLIR are heap-allocated via `emit_gc_alloc`, so the `_G` Local has `ValueKind::Table`. This disqualifies the chunk from the ADR 0218 chunk-safe predicate: programs using `_G` revert to v1 GC safety mode (mark-all-BLACK, sweep frees nothing). M3-extended (DFS through Tables) will lift this when it lands.

### Why prepend, not synthesize at use sites

A pre-pass at `lower()` entry is the smallest possible HIR-layer change: the rest of the HIR + codegen sees the chunk as if the user wrote `local _G = {}` themselves. No new HirExprKind, no new Builtin variant, no codegen arm. Future expansion (builtin migration, `_ENV` upvalue threading) builds on this baseline.

## Tests

`tests/phase4_env_globals.rs` (NEW, 4 e2e):

1. `_G.foo = 42; print(_G.foo)` → `42` (round-trip).
2. `_G["answer"] = 42; print(_G.answer)` → `42` (string-key form).
3. `_G.a = 1; _ENV.b = 2; print(_ENV.a); print(_G.b)` → `1\n2` (alias works in both directions).
4. Multiple distinct keys round-trip.

## Test count delta

```
Step 0:  1494 (after ADR 0220)
C3 (impl + 4 e2e): 1494 → 1498
```

## References

- [ADR 0048](0048-phase2-0a-auto-declare-globals.md) — auto-declared locals; still active for builtin names.
- [ADR 0154](0154-env-true-globals-strategy.md) — long-term `_ENV` strategy; M4 first concrete step.
- [ADR 0199](0199-table-keyed-fields.md) — keyed Table constructor (used by `_G[k] = v`).
- [Lua 5.4 §3.5](https://www.lua.org/manual/5.4/manual.html#3.5) — visibility / `_ENV` semantics.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M4 milestone.
