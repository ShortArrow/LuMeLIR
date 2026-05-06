# 0082. Phase 2.5x-callee-dispatch: General Indirect-Call Re-Enablement (Plan B3)

- **Status:** Accepted; supersedes ADR 0075 in part
- **Date:** 2026-05-06
- **Deciders:** ShortArrow

## Context

ADR 0072 originally enabled `local g = t[k]; g(...)` by reconstructing
the function type from `args.len()` at codegen time. That was UB-prone
on heterogeneous tables (LIC-2.6c-tag-callee-arity-1). ADR 0075 hardened
by rejecting the path entirely — every indirect call through a
TaggedValue local was HIR-rejected. The cost was a real feature
regression: dispatch-table patterns and the `for ... in iter, state, ctl
do … end` generic-for protocol both depend on calling a function value
loaded from a tagged slot.

ADR 0080 (`pairs(t)`) and ADR 0081 (`next` builtin + ForPairs HIR-
desugar) restored *one* iterator path via a builtin shortcut. The
arbitrary-callable generic-for and OOP-style method dispatch still
need general indirect-call support.

Codex pre-ADR-0082 review surveyed three families of approaches:

- **Plan A** (expand TAG_FUNCTION payload to 16 bytes): rejected as too
  invasive — `ARRAY_ELEM_SIZE = 16` is foundational.
- **Plan B** (signature side table): three sub-variants; **B3** =
  per-call-site dispatch with static ptr-match + direct call branches
  (no `func.call_indirect` cast). Codex picked B3 as "the right shape
  for the current phase" and added three constraints:
  1. **FuncId-less function values**: function parameters carry no
     static FuncId; payload-as-FuncId encoding (B2) breaks them.
  2. **Signature encoding** must be full `(param_kinds, ret_kinds)`
     vectors, not just arity — ADR 0076's multi-position TaggedValue
     ABI cannot be distinguished by counts alone.
  3. **Tag-check-first** is mandatory — calling a non-function tagged
     slot must trap before the payload is interpreted as a fn ptr.
- **Plan E** (uniform-ABI trampoline): premature for this phase.

## Decision

### `Callee::IndirectDispatch` enum extension

`Callee::Indirect(LocalId)` (the safe parameter-call path from ADR
0075) is preserved. `Callee::IndirectDispatch` is a new variant:

```rust
pub enum Callee {
    Builtin(Builtin),
    User(FuncId),
    Indirect(LocalId),                            // unchanged
    IndirectDispatch {
        local_id: LocalId,
        sig: IndirectSig,                         // (param_kinds, ret_kinds)
        candidates: Vec<FuncId>,                  // compile-time compatible-set
    },
}

pub struct IndirectSig {
    pub param_kinds: Vec<ValueKind>,
    pub ret_kinds: Vec<ValueKind>,
}
```

Reusing `Callee` (rather than introducing a new `HirExprKind`)
means `HirExprKind::Call` and `HirStmtKind::MultiAssignFromCall`
both flow through the same descriptor — direct user calls,
builtin multi-return (ADR 0081), and the new dispatch all share
one code path. (Codex pre-ADR-0082 review §3 — "extend `Callee`
rather than add a new expr shape".)

### `compatible_user_functions` pure helper

Implemented as a local fn inside `lower_call`. Pure: filters the
module's user functions to those whose `(params, ret_kinds)`
exactly matches the call site's signature. No table provenance
analysis (Codex review §2 — `IndexAssign` breaks any provenance
immediately).

### HIR `lower_call` for TaggedValue local

```text
1. Lower args to HirExpr; infer their kinds = `param_kinds`.
2. Filter all user fns by `param_kinds` only → `param_only_matches`.
3. If empty: HirError::IndirectCallNoCandidates (compile error).
4. Take the first match's `ret_kinds` as the canonical
   `sig.ret_kinds`. The contract is "all candidates share full
   signature"; non-matching `ret_kinds` candidates are dropped.
5. Re-filter by full sig → `candidates`.
6. Return `HirExprKind::Call { callee: Callee::IndirectDispatch
   { local_id, sig, candidates }, args }`.
```

### Multi-value position re-search

`lower_local_multi` and `lower_assign_multi` see the lowered
`Call(IndirectDispatch)` and discover its `sig.ret_kinds` is the
single-value default. They re-run candidate filtering with
`names.len()`-aware `ret_kinds`:

1. Find user fns whose `params == sig.param_kinds` AND
   `ret_kinds.len() == names.len()`.
2. All matches must share the same `ret_kinds` vector (otherwise
   the dispatch chain branches would yield incompatible MLIR
   types). If not, raise `IndirectCallNoCandidates` with the
   attempted multi-position vector.
3. Replace the `Callee::IndirectDispatch` with a fresh one whose
   `sig.ret_kinds` and `candidates` reflect the multi-value
   contract.

### Codegen `emit_indirect_dispatch_call`

Three steps, in order:

1. **Tag check**: load tag at slot+0; if `≠ TAG_FUNCTION`, exit
   with `s_call_non_function`. (Codex review §4 — MUST run
   before payload interpretation.)
2. **Payload load**: load `!llvm.ptr` at slot+8.
3. **Dispatch chain** (`emit_dispatch_chain_recursive`): nested
   `scf.if` over candidates. Each level:
   - `func.constant @user_fn_X` → `unrealized_cast` to ptr →
     `llvm.ptrtoint` → `arith.cmpi Eq` against the loaded ptr.
   - **Then** branch: direct `func.call @user_fn_X(args)` with
     the candidate's actual MLIR signature (NOT `call_indirect`),
     yield results.
   - **Else** branch: recurse with the remaining candidates.
   - Base case (no more candidates): unconditional trap with
     `s_call_unknown_fn_ptr` and dummy result-typed yields to
     satisfy MLIR's region-yield type check.

Result types are uniform across branches (`ret_mlir_types(
sig.ret_kinds)`) — the compatible-set contract guarantees every
candidate has the same signature. Single-value caller takes
`result(0)`; multi-value caller (`emit_multi_assign_from_indirect_
dispatch`) walks all results via the existing `flat_result_index`
+ direct-store packing path (ADR 0076).

### `callabi.rs` Tidy First (Commit 1)

Three pure helpers extracted from `emit.rs` into a new
`src/codegen/callabi.rs`:

- `ret_mlir_types(ctx, ret_kinds, types) → Vec<Type>`
- `ret_kind_result_width(kind) → usize`
- `flat_result_index(ret_kinds, pos) → usize`

Responsibility split: `tagged.rs` owns `{tag, payload}` slot
layout; `callabi.rs` owns the kind-vector ↔ MLIR-result-flatten
mapping. `emit_pack_tagged_result_at_pos` stays in `emit.rs` due
to melior lifetime complexity; the move list could grow in a
future Tidy First.

### Unchanged producer surface

`emit_value_slot_store_function` continues to write the raw
`!llvm.ptr` payload — the slot encoding doesn't change. (Codex
review §5 — Plan B3 stays clear of payload rewrites.)

## Alternatives Considered

- **Plan B1 (payload = ptr, runtime side-table lookup by ptr)**:
  works but adds runtime overhead per call for what compile-time
  enumeration already pins down. Rejected as not earning its
  weight at this phase.
- **Plan B2 (payload = FuncId)**: breaks function parameters
  (no static FuncId), pessimises future closure identity, and
  forces a producer-path rewrite. Rejected (Codex review §5).
- **Plan E (uniform-ABI trampoline)**: long-term cleanest but
  requires designing thunks per (callee, expected-shape) pair.
  Premature for ADR 0082's scope; revisit when full closures
  and external/native callable land.

## Consequences

- **`LIC-2.6c-tag-hetero-fn-tbl-call-1`**: ADR 0075 marked it
  "resolved by removal"; ADR 0082 reframes as "resolved by safe
  static dispatch". The LIC entry stays in the resolved column.
- **`LIC-2.6c-tag-locals-fn-indirect-1`** stays resolved (ADR
  0074/0075). ADR 0082 demonstrates multi-position TaggedValue
  return through the dispatch chain (`tests/phase2_5x_indirect_
  dispatch::multi_return_indirect_call_via_tagged_local`).
- **`LIC-2.6c-tag-callee-arity-1`** stays resolved — `args.len()`
  reconstruction is not reintroduced. The new dispatch uses the
  candidate's actual signature for the direct `func.call`.
- **New `LIC-2.5x-callee-dispatch-1` (resolved)**: TaggedValue
  local indirect call via per-call-site static dispatch chain
  with full `(param_kinds, ret_kinds)` signature matching.
- **Test totals: 940 → 944 green**. 11 reframed (ADR 0072/0075
  reject tests now positive); 4 new (multi-return indirect,
  closure-escape regression, no-candidates compile error,
  same-signature dispatch).
- **LIC totals: 22 / 0 / 4 → 23 / 0 / 4** (resolved / partial /
  pending). One new resolved.
- **Source LOC**: HIR `+150`, codegen `+340` (`emit_indirect_
  dispatch_call` + chain recursion + multi-assign variant +
  `emit_dummy_value` helper), `callabi.rs` `+88` (extract).
  Tests `+90` net (4 new e2e + 11 reframed in-place).

## Refactor path

When **full closures** ship (heap environment, future ADR 0083
candidate), the dispatch chain admits new producers:

- Closure-with-upvalues stored in tables (LIC-2.6c-tag-hetero-
  closure-escape-1) — needs a heap-backed environment binding.
- External / FFI callables — need a `Callee::ExternalDispatch`
  variant or descriptor side table that `IndirectCallNoCandidates`
  defers to.

The candidate-set computation becomes a stable extension point:
new producers register their FuncId equivalents and the same
`compatible_user_functions` (or its successor) walks them.

The runtime trap for "unknown fn ptr" — currently a defensive
backstop unreachable from Lua source — stays useful when the
above producers land, since dynamic loaders can put unknown
addresses in tagged slots that the compile-time dispatch chain
cannot anticipate.

## Documentation updates

- [x] §1 slot layout — n/a.
- [x] §2 producer / source taxonomy — n/a.
- [x] §3 consumer matrix — n/a (consumer matrix indexes by
      *value* source; the new dispatch is a *call-site* construct).
- [x] §4 LIC consolidation — `iter-pairs-1` resolution mechanism
      already updated by ADR 0081; this ADR adds new resolved
      `callee-dispatch-1` and updates `hetero-fn-tbl-call-1`'s
      annotation. Totals: **25 entries — 23 resolved / 0 partial
      / 4 pending**.
- [x] §5 runtime tag invariants — n/a.
- [x] §6 cross-reference — new "Indirect dispatch via static
      candidate chain" subsection describing tag-check + ptr-match
      + direct call.
- [x] §7 open questions — `general indirect-call re-enablement`
      removed; `full closures` promoted to #1.
- [x] §8 ADR index — ADR 0082 row added; "Last updated" stamp
      bumped.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
