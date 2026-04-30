# 0015. Phase 2.4: `break` via HIR-time Desugar to a Hidden Flag

- **Status:** Accepted
- **Date:** 2026-04-30
- **Deciders:** ShortArrow

## Context

Phase 2.3d completed numeric `for`. The remaining hole in Lua's
control-flow set (before functions arrive in Phase 2.5) is `break`.
Without it, conditional early-exit from a loop has to be expressed
via flag variables manually — exactly the desugar we automate here.

The strategy is to keep the `scf.while` lowering established in
Phase 2.3b/d unchanged for codegen, and express `break` as an
HIR-level transformation:

1. Each loop body gets a hidden `_broken_<n>: bool` local.
2. A `break` statement lowers to `Assign { _broken_<n>, true }`.
3. The loop's condition is augmented to `original_cond and not load(_broken_<n>)`.
4. Each statement inside the loop body (recursively, including inside
   `if`/`do` blocks) is wrapped in `if not load(_broken_<n>) then ... end`
   so the body short-circuits as soon as a break runs.

This lets `scf.while` exit naturally on the next condition check.

## Decision

### 1. AST and Lexer

`Keyword::Break`, `StmtKind::Break` — no fields. The parser maps
the keyword to the variant; everything else lives in HIR.

### 2. HIR

- New `HirError::BreakOutsideLoop { offset }`.
- `LowerCtx::loop_break_targets: Vec<LocalId>` — innermost loop's
  hidden flag is on top.
- `lower_stmt(Break)`:
  - Empty stack → `BreakOutsideLoop`.
  - Non-empty → emit `HirStmtKind::Assign { id: top, value: Bool(true) }`.
- `lower_stmt(While { cond, body })`:
  1. Declare `_broken_<n>: Bool = false` (LocalInit at the outer scope).
  2. Push the new `LocalId` to `loop_break_targets`.
  3. Lower the body via the loop-aware helper (each statement wrapped
     in an `if not load(_broken_<n>)`).
  4. Pop.
  5. Synthesise `cond_aug = original_cond and not load(_broken_<n>)`.
  6. Wrap the result as a `Block { stmts: [LocalInit, While { cond_aug, body_guarded }] }`
     so the hidden flag does not leak to the surrounding scope.
- `lower_stmt(ForNumeric)`: same pattern, but the cond augmentation
  happens at codegen time — the hidden flag's `LocalId` is passed as
  `HirStmtKind::ForNumeric { ..., break_id: Option<LocalId> }`.

### 3. Loop-aware body lowering (the guard wrap)

`lower_scoped_body` becomes loop-aware: when `loop_break_targets` is
non-empty, every statement in the lowered body is wrapped in an
`If { cond: not load(top), then_body: [stmt], elifs: [], else_body: None }`.
The wrap is uniform (every statement, not just those statically
known to follow a break) — LLVM's optimiser folds the trivially-true
guards in the common no-break case.

Nested `if`/`do`/`while`/`for` bodies use the same `lower_scoped_body`,
so the guard naturally recurses.

### 4. Codegen

- `while`: no codegen change. The augmented cond is already part of
  the lowered HIR.
- `for`: `emit_for_numeric` gains a `break_id: Option<LocalId>`
  parameter. When `Some`, the before region's natural cond
  (`step > 0 ? i ≤ stop : i ≥ stop`) is AND-combined with
  `not load(slots[break_id])` before reaching `scf.condition`.

### 5. Read-only flag handling

The `_broken_<n>` local is **not** registered in
`LowerCtx::readonly_locals`, so the synthetic `Assign` from the
break path passes the readonly check. The loop variable in `for`
remains read-only as established in Phase 2.3d.

## Alternatives Considered

- **`cf.cond_br` based loops.** Native fit for `break`'s "exit
  current loop" semantics, but requires rewriting `emit_while` and
  `emit_for_numeric` away from `scf.while`. Far larger blast radius.
  Rejected.
- **`scf.while` loop-carried `i1 broken`.** Cleaner signalling
  but the after region cannot terminate early in `scf` — the rest
  of body still runs after the assignment. The same body-guard
  problem returns. Rejected.
- **Skip the body-guard step (run post-`break` statements).** Lua
  spec says `break` exits *immediately*; running side-effecting
  code after `break` would be a real divergence. Rejected.
- **HIR `Break` variant carried into codegen.** Adds a new arm in
  `emit_stmt` for no benefit — the HIR transform fully expresses
  the semantics in terms already supported by codegen. Rejected.
- **Static break-presence analysis to skip wrapping in loops with
  no `break`.** Optimisation, not correctness. Defer; LLVM folds
  the guards anyway.

## Consequences

- `Keyword` +1 (`Break`); `StmtKind` +1; `HirError` +1.
- `HirStmtKind::ForNumeric` gains `break_id: Option<LocalId>`.
- `LowerCtx` gains `loop_break_targets: Vec<LocalId>`.
- `lower_scoped_body` becomes loop-aware (guard wraps when inside a
  loop). Existing tests using bodies that don't contain `break`
  still pass — the guard always sees `_broken == false`.
- `emit_for_numeric` gains the optional break-flag AND step.

## Out of Scope (still deferred)

- `continue` (not in Lua 5.4) — never planned.
- `goto` / labels → Phase 2.5+ or later.
- `repeat ... until` — defer; demand unclear.
- Function definitions / `return` → Phase 2.5.
- `break` inside a nested function (delegating to the outer loop)
  — re-evaluate when functions land.
