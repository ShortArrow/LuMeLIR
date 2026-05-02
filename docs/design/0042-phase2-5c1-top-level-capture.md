# 0042. Phase 2.5c.1: Top-Level `local function` Captures Chunk Locals

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

ADR 0037 (Phase 2.5c-min) lit up capture-by-value closures for
nested function bodies and anonymous `function() ... end`
expressions, but called out one limitation:

> Top-level `local function` capturing chunk-level locals:
> blocked because top-level function bodies are lowered in pass
> 2, before main chunk locals exist. Use the anonymous form
> (`local f = function() ... end`) until the chunk lowering is
> reordered to interleave local-decl and function-body lowering.

That limitation surfaced as soon as anyone wrote idiomatic Lua
at the top level:

```lua
local m = 10
local function calc(x) return x * m + 1 end  -- wants `m`
```

The workaround (rewrite as `local calc = function(x) ... end`)
diverges from how Lua programs are actually written. This phase
removes the limitation.

## Decision

### Interleave Pass 2 with the main-chunk walk

Previously `lower()` ran in two passes after registration:

```rust
// Old pass 2a — every function body up front.
for stmt in chunk {
    if let StmtKind::FunctionDef { params, body, .. } = ... {
        lower_into_function(fid, params, body, ..., HashMap::new())?;
        // ↑↑↑ empty outer_visible — captures impossible.
    }
}

// Old pass 2b — main chunk, FunctionDefs already lowered.
let mut ctx = LowerCtx::new(...);
for s in chunk { stmts.push(ctx.lower_stmt(s)?); }
```

The new shape walks once, lowering each function body at its
declared position with the *current* outer visibility:

```rust
let mut ctx = LowerCtx::new(...);
let mut funcdef_seq = 0;
for s in chunk {
    if let StmtKind::FunctionDef { params, body, .. } = ... {
        let fid = FuncId(funcdef_seq);
        funcdef_seq += 1;
        let outer_visible = ctx.outer_visible_snapshot();
        lower_into_function(
            fid, params, body,
            &ctx.function_names,
            &mut ctx.functions,
            outer_visible,
        )?;
        continue;
    }
    stmts.push(ctx.lower_stmt(s)?);
}
```

Two consequences fall out:

1. **Captures see only locals declared above the function**.
   That matches Lua semantics — the local doesn't exist yet at
   the FunctionDef statement. A forward-reference to a
   later-declared local now surfaces as `UndefinedName`,
   exactly as for any other identifier outside its
   declaration's scope.
2. **Sibling forward-reference still works**. Pass 1 (signature
   registration) runs unchanged, so `local function a()
   return b() end; local function b() ... end` still resolves
   `b` via `function_names`. Only *upvalue* capture is bound
   by source order — function calls are not.

### `idx_of_funcdef` is gone

The previous pass-2a indexed FunctionDefs by their absolute
chunk position via `idx_of_funcdef`. The interleaved walk uses
a running counter instead, so the helper is dead and was
deleted with the refactor.

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | None                                                |
| Parser   | None                                                |
| AST      | None                                                |
| HIR      | `lower()`'s pass 2 collapses two loops into one; `idx_of_funcdef` deleted; `outer_visible_snapshot` now invoked at top level too |
| Codegen  | None                                                |

The shared `lower_into_function` helper is unchanged — it
already accepted an `outer_visible: HashMap<...>` and was
ready for a non-empty top-level snapshot.

## TDD Process

1. **Tidy First (review only).** No prior cleanup warranted.
   The two-loop shape was the cleanup target itself, and the
   shared `lower_into_function` from Phase 2.5f had already
   removed duplication that would otherwise have made the
   merge messy.
2. **Red.** 8 e2e tests added (`tests/phase2_5c1_top_level_capture.rs`)
   covering basic capture, capture+param, multi-capture, the
   "arithmetic chain" form that ADR 0037's workaround test
   used, no-capture regression, sibling forward-ref
   regression, live-binding, and forward-ref-to-later-local
   error. Five behaviour tests failed; three boundary cases
   already passed.
3. **Green.** Pass 2 collapsed into a single chunk walk;
   `idx_of_funcdef` removed. All tests passed at 587 (579 + 8).
4. **Refactor.** None — the merged loop is structurally
   simpler than what it replaced.

## Alternatives Considered

- **Keep the two-pass split, but pre-lower all function bodies
  with a "pessimistic" outer_visible** built from chunk-level
  locals. Adds a separate pre-scan; complicates the order
  semantics (a function defined at line 1 would see locals at
  line 100). Rejected — Lua's source-order rule is the right
  one and falls out for free with the interleave.
- **Lift function bodies to a deferred queue and lower them at
  end of chunk processing**, recording the snapshot per body
  at registration time. Adds an extra data structure to
  preserve information that the natural walk already has.
  Rejected.
- **Reuse pass 1's enumeration counter** rather than
  introducing `funcdef_seq`. Pass 1 doesn't expose its
  counter; threading it through is ceremony. Rejected — a
  fresh counter that mirrors pass-1's iteration order is
  trivial to follow.

## Consequences

- HIR `lower()` shrinks by ~10 lines net (one loop deleted,
  one helper function deleted).
- 8 new e2e tests; total green at 587.
- ADR 0037's "Out of Scope: Top-level `local function`
  capturing chunk-level locals" item retires.
- The "Use anonymous form until chunk lowering is reordered"
  workaround is no longer needed; existing Phase 2.5c-min
  tests that demonstrate the workaround still pass (anonymous
  form continues to work).

## Out of Scope

The remaining ADR-0037 limitations stand:

- **Closure escape** (return / pass as arg / outlive creation
  scope). Phase 2.5c-full or a separate first-class-closure
  phase.
- **Mutually-capturing nested functions** (`a` captures `b`
  while `b` captures `a`). Hard with the pass-1/pass-2 split
  unless upvalue analysis runs as a third pass.
- **Non-Number captures** (Bool / Nil / String / Function).
- **Static `ClosureEscapes` rejection** for closures with
  upvalues passed as Indirect arguments.
