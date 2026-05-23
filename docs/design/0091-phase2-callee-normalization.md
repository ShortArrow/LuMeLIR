# 0091. Phase 2.6+-callee-norm: HIR Callee Normalization for Index-Callee Calls

- **Status:** Accepted (plan v2, post-abort)
- **Kind:** Architecture Decision
- **Date:** 2026-05-14
- **Deciders:** ShortArrow

## Replan provenance

ADR 0091 plan v1 (2026-05-10, "Method colon syntax") landed lexer +
AST + parser changes, then was aborted via `git checkout -- .` on
2026-05-11 when HIR implementation surfaced cascading prerequisites
that codex post-0090 review had not identified:

1. `lower_call` (`src/hir/mod.rs:3613-3619`) **rejects any non-Ident
   callee** with `HirError::UnsupportedCall` → `obj.method(args)`
   direct-call form does not work today, even before any method
   colon sugar is considered.
2. `Callee::IndirectDispatch` (ADR 0082) requires a **LocalId source**
   → Index results need pre-binding to a synthetic local before
   dispatch.
3. `infer_param_kinds` (`src/hir/mod.rs:639`) only refines params for
   callee-position usage → `function obj:get() return self.x end`'s
   `self` defaults to Number, breaking the Index target type check.
4. `infer_user_function_param_kinds` chunk-walker doesn't see
   Index-callee Calls → call-site arg-kind refinement missing.

Codex post-abort review (2026-05-14, 6 視点) recommended: **G
(Index-callee Call support) を単独 ADR で先行**. Methods (and the
inference fixes) are deferred to a future ADR that depends on this
one. Codex framed the issue as:

> ADR 0091 abort の真因は「syntax feature」ではなく「HIR callable
> boundary」。Methods を直すというより、Methods が依存していた
> 壊れた前提を修復するタスク。

The number `0091` is reused; v1's framing ("Methods") is superseded
by v2's framing ("Callee normalization") because v2 is the proper
foundation v1 implicitly required.

## Context

ADR 0090 (`f2db7af`, 2026-05-10) closed the `--emit` observability
investment. LIC counter remains 28/28/0 (Phase 2 tagged-semantics
consumer coverage complete since ADR 0089). With language semantics
stable, the next investment is the HIR callable boundary.

The user-visible breakage today:

```lua
local t = {}
t.m = function(x) return x + 1 end
print(t.m(2))    -- HIR error: UnsupportedCall { offset: ... }
local g = t.m    -- works (existing ADR 0082 path)
print(g(2))      -- works (IndirectDispatch via local binding)
```

`t.m(2)` is a basic Lua pattern with no method-colon sugar involved
and should compile. The fix is HIR-internal — `lower_call` must
accept Index callees by pre-binding the result to a synthetic local,
routing through the existing `Callee::IndirectDispatch` machinery.

## Codex planning guidelines (CLAUDE.md 第3原則 non-ad-hoc)

The plan v2 explicitly adopts six guidelines from codex's post-abort
review to avoid the underestimate that triggered v1's abort:

1. **"User syntax" でなく "lowering chokepoint" から書く** — the ADR
   unit is HIR callable boundary normalization, not source-level
   syntax. Methods is a future consumer.
2. **Non-goals at top of ADR** — methods / `self` inference /
   metatables / `__call` / `infer_*` extensions are explicitly out
   of scope.
3. **Red per failure surface** — parser-level vs HIR-level vs
   runtime Reds stay separate. 6 e2e split per surface (3 happy,
   1 regression-pin, 2 typed-error pins).
4. **Don't break existing safety boundaries** —
   `Callee::IndirectDispatch`'s LocalId-source invariant is preserved
   by pre-binding the Index result, not by adding a new `Callee`
   variant.
5. **Pure classifier + effectful executor split** —
   `classify_callee_form` is pure (testable in isolation),
   `materialize_callee_to_local` is the effectful executor.
6. **"Sugar only" を禁句に近く扱う** — plan v2 framing is
   "infrastructure / callable boundary normalization", explicitly
   not sugar.

## Decision

### 3-layer split (mirrors ADR 0087 / 0088 / 0089)

| Layer | Module | Role |
|---|---|---|
| **Pure classifier** | `hir/mod.rs` | `classify_callee_form(&Expr) -> Result<CalleeForm, HirError>` — DirectIdent vs IndexCallee, or `UnsupportedCall` for genuinely out-of-MVP shapes |
| **Effectful executor** | `hir/mod.rs` | `materialize_callee_to_local(target, key, span)` — lowers the Index, declares a synthetic TaggedValue local (`__callee_<seq>`), pushes a LocalInit pre-stmt, returns the LocalId |
| **Drain wrapper** | `hir/mod.rs` | `lower_stmt` snapshots / restores `pending_pre_stmts` and wraps inner stmt in a `Block` when hoists accumulated |

`Callee::IndirectDispatch` is unchanged; `emit.rs` is unchanged
(`src/codegen/` zero-diff, CA invariant verified via
`git diff --stat src/codegen/`).

### `LowerCtx` field additions

```rust
struct LowerCtx {
    // ... existing fields ...
    /// HIR pre-stmt hoisting buffer; drained at every lower_stmt boundary.
    pending_pre_stmts: Vec<HirStmt>,
    /// Monotonic counter for synthetic `__callee_<N>` local names.
    callee_seq: usize,
}
```

### `lower_stmt` drain wrapper

```rust
fn lower_stmt(&mut self, stmt: &Stmt) -> Result<HirStmt, HirError> {
    let outer_pre = std::mem::take(&mut self.pending_pre_stmts);
    let inner_result = self.lower_stmt_match_arms(stmt);
    let mut my_pre = std::mem::replace(&mut self.pending_pre_stmts, outer_pre);
    let inner = inner_result?;
    if my_pre.is_empty() {
        return Ok(inner);
    }
    my_pre.push(inner);
    Ok(HirStmt {
        kind: HirStmtKind::Block { stmts: my_pre },
        span: stmt.span,
    })
}

fn lower_stmt_match_arms(&mut self, stmt: &Stmt) -> Result<HirStmt, HirError> {
    // existing match-arms body, unchanged
}
```

The snapshot/restore keeps each stmt's hoists local to its own
boundary — recursive `lower_stmt` calls (e.g. If-body) drain at
their own boundaries, not at the outer caller's. Empty-hoists path
returns inner stmt unchanged, preserving existing behavior 100%.

### Pure classifier

```rust
enum CalleeForm<'a> {
    DirectIdent,
    IndexCallee { target: &'a Expr, key: &'a Expr },
}

fn classify_callee_form(callee: &Expr) -> Result<CalleeForm<'_>, HirError> {
    match &callee.kind {
        ExprKind::Ident(_) => Ok(CalleeForm::DirectIdent),
        ExprKind::Index { target, key } => Ok(CalleeForm::IndexCallee {
            target: target.as_ref(),
            key: key.as_ref(),
        }),
        _ => Err(HirError::UnsupportedCall { offset: callee.span.start }),
    }
}
```

### Effectful executor

`materialize_callee_to_local` reuses `widen_index_for_local_init`
(ADR 0063) so the synthetic local widens to `TaggedValue` — the
same storage rule that `local g = t.m` already uses. This means the
synth-local routes through ADR 0082's TaggedValue dispatch path
(line 3753+) — no new code path on the dispatcher side.

### `lower_call` entry dispatch

```rust
fn lower_call(&mut self, callee, args, whole) -> Result<HirExprKind, HirError> {
    match classify_callee_form(callee) {
        Ok(CalleeForm::DirectIdent) => { /* existing flow */ }
        Ok(CalleeForm::IndexCallee { target, key }) => {
            let synth_id = self.materialize_callee_to_local(target, key, whole.span)?;
            let synth_name = self.locals[synth_id.0].name.clone();
            let synth_callee = Expr::new(ExprKind::Ident(synth_name), whole.span);
            return self.lower_call(&synth_callee, args, whole);
        }
        Err(e) => return Err(e),
    }
    // ... existing flow continues for DirectIdent
}
```

The recursion terminates after one step — the synthetic Ident
classifies as DirectIdent and flows through the existing path.

## Alternatives Considered

- **Add a `Callee::IndexedDispatch { target, key, ... }` variant.**
  Rejected by codex critical #3: preserve `IndirectDispatch`'s
  LocalId-source invariant; adding a new variant duplicates dispatch
  logic and risks soundness drift.
- **Bundle methods (`obj:method()`) with this ADR.** Rejected by
  codex critical #1 (failure originates in HIR callable boundary,
  not syntax). Bundling reintroduces the v1 abort risk: failure
  surfaces span multiple layers, Red gets murky, SoT becomes
  ambiguous. Methods returns as a future ADR depending on this one.
- **Extend `lower_call` inline without classifier/executor split.**
  Rejected by codex critical #5 — keeping the classifier pure
  enables future ADRs (Methods, `__call`) to reuse the same
  classification without re-inferring shape from raw AST.
- **Synthesize ASTs at parser time** (parse `t.m(args)` directly
  as a sugar that emits a Block AST). Rejected — would require AST
  changes and parser sugar logic; instead, the AST stays unchanged
  and HIR alone handles the normalization.
- **Reject Index-callee Calls until methods/closures fully land.**
  Rejected — basic Lua patterns (`t.m(args)`) should compile
  independently of sugar features. The HIR breakage is a Phase 2
  semantic gap, not a deferred feature.

## Consequences

- **Pre-stmt hoisting infrastructure is general-purpose.** Future
  ADRs (Methods, `__call` metamethod, let-binding rewrites,
  expression-position complex callees) reuse `pending_pre_stmts` +
  the drain wrapper.
- **Test totals: 1018 → 1024 green** (6 new e2e: 3 happy + 1
  regression-pin + 2 typed-error pin per failure surface).
- **LIC counter unchanged** (no new spec gap).
- **Source LOC delta**:
  - `src/hir/mod.rs`: +2 LowerCtx fields (~6 LOC), drain wrapper
    (~25 LOC), `CalleeForm` + `classify_callee_form` (~30 LOC),
    `materialize_callee_to_local` (~40 LOC), `lower_call` entry
    dispatch (~15 LOC). **~+115 LOC total.**
  - `tests/phase2_index_callee.rs` (new, ~170 LOC, 6 e2e).
  - **`src/codegen/`, `src/cli/`, `src/pipeline.rs`, `src/parser/`,
    `src/lexer/`: zero diff** (CA invariant).
- **User-visible behavior**: `t.m(args)` / `t[i](args)` /
  `obj.field(args)` now compile and run correctly. Existing
  `local g = t.m; g(args)` flows unchanged.

### Carry-overs

- **Methods (`obj:method()`)** — future ADR depends on this one.
  Adds `ExprKind::MethodCall` + parser sugar arm + HIR desugar to
  Index-callee Call (this ADR's machinery), plus method-def
  desugar to `IndexAssign` + `self` param-kind handling. The
  AST/parser changes already designed in plan v1 can be reused.
- **`infer_user_function_param_kinds` chunk-walker extension** —
  currently doesn't see Index-callee Calls, so user functions
  called only via `t.f(arg)` get default Number param kinds. Workaround: use the existing literal-arg call-site inference
  (`t.f({})` refines first param to Table), or declare via direct
  call once. Future ADR extends the chunk-walker.
- **Complex non-Ident non-Index callees** — `(fn or alt)()`,
  `(expr_returning_fn)()`, `function_literal()(...)`.
  `classify_callee_form` returns `UnsupportedCall` for these.
  Same `pending_pre_stmts` mechanism can extend coverage.
- **`__call` metamethod** (Lua metatables). Same dispatch chain,
  with metamethod lookup added.

## TDD Process

1. **Red.** 6 e2e in new `tests/phase2_index_callee.rs`:
   - 3 happy-path Red (UnsupportedCall fires today):
     `index_field_callee_dispatches`, `index_numeric_callee_dispatches`,
     `index_callee_body_arith_works`.
   - 1 regression-pin always-Green: `existing_local_binding_unchanged`
     pins the `local g = t.m; g(x)` flow (ADR 0082 IndirectDispatch
     via local binding).
   - 2 typed-error pins:
     - `index_callee_no_candidates_reports_typed_error`: today
       `UnsupportedCall`; after Step 4, `IndirectCallNoCandidates`
       (the correct compile-time error per ADR 0082).
     - `index_callee_on_non_function_traps_at_runtime`: today
       `UnsupportedCall` (HIR rejects); after Step 4, HIR accepts
       and runtime traps via `s_call_non_function` (ADR 0082).
2. **Green.**
   - Step 1: LowerCtx fields → build green.
   - Step 2: drain wrapper → build green; 1018 → 1019 (regression
     pin Green; 5 still Red).
   - Step 3: `CalleeForm` + classifier → build green.
   - Step 4: `lower_call` dispatch + `materialize_callee_to_local`
     → 5 Red flip Green; 1019 → 1024.
3. **Refactor.** None needed — the classifier/executor split arrived
   in the initial implementation (per codex guideline #5).

## Documentation updates

- [x] **ADR 0091** (this file) authored with replan provenance,
      codex guideline adoption, non-goals.
- [x] **`docs/design/tagged-semantics.md`** §8 ADR index — 1 row.
- [x] **`AGENTS.md`** — `‣ 2.6+-callee-norm` row added. Notes the
      plan v1 abort and the v2 reframing.
- [ ] **`docs/PRD.jp.md`** — no change (this is HIR-internal).

## Lua-Incompatibility Tracker

No new LIC entries. The fix closes a HIR call-form gap that was
not LIC-tracked (it was an `UnsupportedCall` reject, not a Lua
spec deviation). See `docs/design/tagged-semantics.md` §4 for the
authoritative LIC list (28 / 28 / 0 unchanged).
