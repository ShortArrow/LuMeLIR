# 0083. Phase 2.5c-full: Closures (Plan B) — cell-ptr-first ABI for all user fns

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-10 (Commits 1–3c landed across `e6b256f` → `ed34703`)
- **Deciders:** ShortArrow
- **Supersedes:** ADR 0044 (closure escape reject) in full

## Replan provenance

ADR 0044 reject-on-escape was the standing closure-with-upvalues
backstop. Plan B (this ADR) lifts that backstop by giving **every
user function** a `!llvm.ptr` cell argument as the first parameter
and threading captured-upvalue heap boxes through a per-call cell.
Mutual-capturing-recursion still rejects at HIR-time
(`MutualCapturingRecursion`) and a tagged-slot escape path still
goes through the dispatch chain — closures that satisfy the
non-cyclic shape now compile and run.

ADR 0083 was delivered as a multi-commit landing rather than a
single atomic commit because the cell-ptr-first ABI must flip
producer and consumer sites simultaneously to avoid an
intermediate broken state; the prep commits introduce dormant
shapes and the body commit flips them all at once.

## Non-goals (top-of-ADR)

- **Mutual-capturing recursion** — still HIR-rejected; future ADR
  may relax with explicit chain analysis.
- **`Callee::Indirect` non-Number return** — ADR 0075's reject
  stays (lifts in a future ADR that widens indirect dispatch to
  Function-kind upvalue support).
- **Upvalue mutation across closures** — boxes are heap-allocated
  per-instance; sharing a box requires capturing the same parent
  binding (no `setupvalue` API).
- **Lua spec `<close>` / TBC** — Phase 3 (GC-adjacent).
- **TaggedValue-key closures inside dispatch chains beyond what
  Commit 3c covers** — only the loaded-cell-ptr threading path is
  added; full TaggedValue closure landing matrix is still
  incremental.

## Goals

1. **Cell-ptr-first ABI**: every user `llvm.func` accepts a
   `!llvm.ptr` cell ptr as the first arg (even non-capturing
   functions; uniform shape avoids per-call signature branching).
2. **Heap upvalue boxes** for `is_captured` outer-scope locals,
   allocated at parent-function entry and threaded into closures
   via the cell.
3. **`@user_fn_NN_closure` singletons** for non-capturing
   functions so `FunctionRef` produces a stable ptr without
   per-call allocation.
4. **`Callee::User { fid, holding_local }` struct variant** so
   the dispatch chain can carry the holding local through the
   call without re-resolving it.
5. **5 `ClosureEscapes` reject sites + `closure_with_upvalues`
   filter removed** — the natural `make_adder` / `closure_return`
   / `table_capture` idioms compile.
6. Test corpus: 980 → 990 green (+10) across Commit 3b body / 3c.

## 設計

### ABI

```
user_fn_NN(cell_ptr: !llvm.ptr, arg0, arg1, ...) -> ret
                ^^^^^^^^^^^^^^
                cell {fn_ptr: ptr, upvalue_box[N]: ptr, ...}
```

The first parameter is the cell ptr. The function prologue
unpacks `cell.upvalue_box[i]` for each upvalue declared by
`f.upvalues`. Non-capturing functions accept the cell ptr but
ignore it (uniform call shape).

### Per-fn closure singleton

```mlir
llvm.mlir.global internal constant @user_fn_42_closure {
  // {fn_ptr: ptr, /* no upvalue boxes for non-capturing */}
}
```

`FunctionRef` for a non-capturing `f` emits an addressof on the
singleton; capturing functions allocate a fresh cell at the
`FunctionRef` site with malloc'd upvalue boxes.

### Capturing-cell allocation

For each `is_captured` outer local, a heap box is allocated at
parent-function entry (or at the closure-creation site for non-
local captures). The box ptr is stored in the closure cell's
`upvalue_box[i]` slot.

### `Callee::User` struct variant

```rust
enum Callee {
    User { fid: FuncId, holding_local: Option<LocalId> },
    // ...
}
```

`holding_local = Some(LocalId)` when the call site loaded the
fn ptr from a local (e.g. `local g = f; g()`); `None` for direct
calls. The struct disambiguates "FunctionRef → call" vs
"Local-known-FuncId → call" without inspecting the call site
twice (Codex P1 ambiguity resolved by `f2ffcb9` prep fix).

### `emit_call_user_with_cell` chokepoint

All 4 direct-call sites route through this helper:

1. `Call` expression
2. Multi-assign caller side
3. Dispatch chain then-branch (post-tag-check direct call)
4. TaggedValue pack helper (Index-source FunctionRef materialise)

The helper picks the cell ptr based on `holding_local`:
- `Some(idx)` → load cell ptr from `slots[idx]`
- `None` + non-capturing target → addressof the singleton
- `None` + capturing target → ERROR (only reachable via
  `FunctionRef`, which allocates the cell explicitly)

### Mutual-capturing-recursion reject

Two functions that mutually capture each other (cycle in the
capture graph) cannot both have valid heap boxes at the same
call site. The HIR post-pass `MutualCapturingRecursion` reject
fires on the SCC.

### Generic-for closure-as-iter filter relaxation

ADR 0085's `f.upvalues.is_empty()` filter is dropped — capturing
closures can now serve as the iterator function in
`for k, v in iter_factory(t)` because the loaded cell ptr is
threaded through.

## Commit breakdown

| Commit | Hash | Scope |
|---|---|---|
| 1 | `e6b256f` | `src/codegen/closure.rs` skeleton |
| 2a | `551d51c` | `emit_function` / `emit_main` / `emit_lumelir_next_function` → `LLVMFuncOperationBuilder`; multi-return `!llvm.struct<(...)>` |
| 2a-fix | `c81f16b` | HIR reject of non-Number ret_kinds on `Callee::Indirect` (ADR 0075 amend) |
| 2b | `a5e8a3e` | per-fn `@user_fn_NN_closure` singletons; producer + consumer flip; `emit_load_closure_fn_ptr` consumer normalisation |
| 3a | `20e563e` | `closure.rs` 6 capturing helpers + `LocalInfo::is_captured` + `HirFunction::parent_scope` + post-pass |
| 3b prep | `e8db350` | `Callee::User` struct variant + `emit_call_user_with_cell` helper (`#[allow(dead_code)]`) |
| 3b prep fix | `f2ffcb9` | synthetic local for FunctionDef + post-pass `MutualCapturingRecursion` reject; local-resolve per-arg Function(arity) compat check |
| 3b body | `18bee17` | every user `llvm.func` accepts cell ptr as 1st arg; entry-block upvalue unpack; 4 direct-call sites route through `emit_call_user_with_cell`; `FunctionRef` allocates fresh capturing cells; `Local`-known-FuncId branches on `target.upvalues.is_empty()`; LocalInit storage rule stores cell ptr unconditionally for capturing targets; outer-scope `is_captured` locals get heap upvalue boxes at function entry |
| 3c | `ed34703` | removed all 5 `HirError::ClosureEscapes` reject sites + `closure_with_upvalues` helper + `f.upvalues.is_empty()` generic-for filter; `Callee::Indirect` and dispatch chain then-branch thread loaded cell ptr (not `cell.fn_ptr`) as `in_function_cell_ptr` so capturing closures reach their boxes through tagged-slot escape paths |

## Tests

7 new e2e in `tests/phase2_5c3_capturing_e2e.rs`:

- `box_sharing` — multiple closures capturing the same outer local share the box
- `make_adder` — classic closure factory
- `closure_return` — closure returned through `return` statement
- `table_capture` — closure stored in a table slot
- `closure_identity` — `f == f` after passing through tagged-slot
- `generic_for_capturing` — capturing closure as `for k, v in iter() do ...` iter
- IR-shape hardening pins (entry-block cell unpack / self-recursion / nested forward / alias)

Plus 7 previously-negative escape tests across 6 files inverted
to positive lowering pins.

980 → 990 green (+10).

## Reuse

| Helper / Pattern | Path | Purpose |
|---|---|---|
| `src/codegen/closure.rs` | Commit 1 skeleton + Commit 3a expansion | Closure metadata + cell layout |
| `LLVMFuncOperationBuilder` | melior 0.27 | LLVM dialect function decl |
| `LocalInfo::is_captured` | `src/hir/` | Capture analysis result |
| `HirFunction::parent_scope` | `src/hir/` | Scope-walk for outer-local resolution |
| `emit_call_user_with_cell` | `src/codegen/emit.rs` (Commit 3b prep) | Single chokepoint for direct user calls |
| `Callee::User { fid, holding_local }` | `src/hir/` (Commit 3b prep) | Struct variant for cell-ptr resolution |
| ADR 0082 `Callee::IndirectDispatch` | preceding ADR | Dispatch chain consumer |

## Codex 6-視点 checklist

- [x] **#1 non-ad-hoc / Tidy First**: uniform cell-ptr-first ABI is
  the non-ad-hoc choice over per-call signature branching;
  ADR 0044 supersede + ADR 0075 amend are the natural follow-ons.
- [x] **#2 TDD**: 7 new positive e2e + 7 inverted negative pins
  (rejected `ClosureEscapes` → positive lowering).
- [x] **#3 FP**: pure scope analysis (`is_captured` / SCC walk)
  separated from effectful codegen (cell allocation / call
  routing).
- [x] **#4 CA**: `src/cli/`, `src/pipeline.rs`, `src/parser/`,
  `src/lexer/` zero-diff. HIR + codegen + new `closure.rs`
  module.
- [x] **#5 Security**: heap upvalue boxes are malloc'd; no GC
  yet (Phase 3 territory). No new alloc-site OOM consolidation
  in scope.
- [x] **#6 Documentation**: this doc (backfilled by ADR docs
  cleanup); ADR 0044 supersede note; tagged-semantics.md §8 row
  added when 0083 first landed; AGENTS.md row tracks commit
  breakdown.

## Risks

| Risk | Mitigation |
|---|---|
| Intermediate broken state during cell-ptr-first ABI flip | Multi-commit landing with prep (`#[allow(dead_code)]` shapes) + atomic body commit (`18bee17`) |
| Capturing closure escape through tagged slot reads `cell.fn_ptr` instead of cell ptr | Commit 3c routes loaded cell ptr (not the fn ptr inside it) as `in_function_cell_ptr` |
| Mutual-capturing recursion produces a cell cycle | Post-pass HIR reject (`MutualCapturingRecursion`) |
| `holding_local` 3-way ambiguity (FunctionRef vs Local vs Indirect) | `Callee::User { holding_local: Option<LocalId> }` struct variant disambiguates at HIR time |
| Cell-ptr first-arg breaks `__lumelir_next` ABI | `__lumelir_next` migrated in Commit 2a same as user fns |

## Future work

- **Mutual-capturing recursion support** — explicit cycle
  detection + late binding.
- **`Callee::Indirect` Function-kind upvalue support** — lifts
  the ADR 0075 amend's non-Number-return reject.
- **Upvalue mutation across closures (`<close>`/TBC)** — Phase 3.
- **Closure GC integration** — Phase 3 (heap upvalue boxes
  currently leak).
- **TaggedValue closure landing matrix completeness** —
  incremental as feature ADRs need it.

## Phase tag

`2.5c-full` (Plan B full closures, supersedes ADR 0044's reject).
