# 0075. Phase 2.6c-tag-callee-arity: TaggedValue Indirect Call HIR-Reject

- **Status:** Accepted; supersedes ADR 0072 in part
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

ADR 0072 (Phase 2.6c-tag-fn-tbl-call) enabled
`local g = t[k]; g(...)` by reconstructing the function type
from `args.len()` at codegen time. The acknowledged hazard
(`LIC-2.6c-tag-callee-arity-1`) was that the slot's payload —
a bare `!llvm.ptr` — carries no signature descriptor, so the
reconstruction was unsound when the table held functions of
different arities or different return ABIs:

```lua
local function f1(x) return x end
local function f2(x, y) return x + y end
local t = {f1, f2}
local g = t[1]
g(1, 2)  -- compiles; passes 2 args to f1; UB
```

ADR 0074 (function-return TaggedValue widening) introduced a
second function ABI shape `(...) → (i64, i64)` for
heterogeneous-return functions. To avoid one specific UB
extension, ADR 0074 added a HIR-time backstop rejecting
"TaggedValue-returning function stored in a table" — a
producer-side fix that left the underlying consumer-side
hazard untouched.

Codex post-Tidy-First review (post-ADR-0074) made this the **#1
priority**:

> indirect call は HIR で arity が静的に証明できる場合のみ許可
> し、それ以外は reject。`args.len()` 推測は「安全な単相 world
> 内だけ」に押し戻す。

The user chose **Strict Plan C** — roll back ADR 0072's feature
entirely rather than designing a runtime arity descriptor (which
would expand the slot beyond ARRAY_ELEM_SIZE = 16 or introduce
a side table for function signatures).

## Decision

### HIR: reject every indirect call through a TaggedValue local

`lower_call` in `src/hir/mod.rs` previously had three
acceptance arms for `Callee::Indirect`:

1. `Function(arity)` local with statically-known arity
   (validated against `args.len()` upfront) — **safe, kept**.
2. `Function(arity)` parameter (arity inferred via body scan,
   ADR 0018) — same path as #1, **safe, kept**.
3. `TaggedValue` local from a table-read source (ADR 0072
   path) — **unsafe, removed**.

The arm for #3 now returns a new HIR error:

```rust
if matches!(self.locals[local_id.0].kind, ValueKind::TaggedValue) {
    return Err(HirError::IndirectCallThroughTaggedLocal {
        local_name: name.clone(),
        offset: whole.span.start,
    });
}
```

The error message points at the workaround:

> indirect call through TaggedValue local 'g' is not supported
> (LIC-2.6c-tag-callee-arity-1; ADR 0075). Use a direct call
> or static dispatch.

### Codegen: drop the TaggedValue branch in `Callee::Indirect`

With the HIR backstop, the codegen `Callee::Indirect` arm is
unreachable for `TaggedValue` locals. The branch + its
`emit_value_slot_check_function` runtime trap helper are
deleted. The remaining arm is the safe `Function(arity)` case
where arity comes from `info.kind`, never reconstructed.

```rust
let arity = match info.kind {
    ValueKind::Function(a) => a,
    _ => unreachable!(
        "Callee::Indirect on non-Function local — ADR 0075 \
         removed the TaggedValue path; HIR rejects upstream"
    ),
};
let ptr_val = emit_load(block, slots[*idx], types.ptr, loc);
let p_types: Vec<Type<'c>> = (0..arity).map(|_| types.f64).collect();
let fn_ty = FunctionType::new(context, &p_types, &[types.f64]).into();
emit_unrealized_cast(block, ptr_val, fn_ty, loc)
```

### Workaround for users

`local g = t[k]; g()` is the typical Lua dispatch-table pattern.
The supported substitutes:

```lua
-- Direct call (no intermediate local):
print(f1(1))

-- Static dispatch via a wrapper function:
local function dispatch(i, x, y)
  if i == 1 then return f1(x) end
  if i == 2 then return f2(x, y) end
end
print(dispatch(1, 5))
```

The OOP / metatable style of dispatch remains unsupported (a
separate future phase).

## Alternatives Considered

- **A. Expand `TAG_FUNCTION` payload to 16 bytes** (ptr + arity
  descriptor). Breaks `ARRAY_ELEM_SIZE = 16` invariant; the
  slot would have to grow to 24 bytes everywhere. Affects every
  array-buf / hash-buf consumer. Rejected as too invasive for
  what is fundamentally a safety-tightening phase.
- **B. Function signature side table** (per-function descriptor
  with arity + ret ABI, indexed by a stable function id stored
  in the slot). Cleanest long-term path; requires designing the
  id-allocation scheme, the runtime lookup, and a discipline
  for who owns the table. Out of scope; will revisit when
  full closures / metatables land.
- **D. Drop `Callee::Indirect` entirely** (function values can
  no longer be called through any local — only direct
  named-function calls). Breaks ADR 0018's function-parameter
  feature, which is heavily used in tests. Rejected as
  destructive.
- **E. Trampoline / adapter functions** (auto-generate a thunk
  matching the call-site signature). Beautiful but requires a
  significant codegen pass and runtime-allocated code regions.
  Defer.
- **Limited tracking** (HIR analyses each table to determine if
  all its function elements share an arity, lifting the local's
  kind back to `Function(arity)` in the homogeneous case).
  Doable but the analysis grows complex once tables flow
  through assignments. Implementation surface ≫ Strict's, ROI
  unclear. Rejected for this phase; could revisit later.

The chosen Strict Plan C is the smallest implementation
that **closes the UB root cause** at the HIR layer. It accepts
a real feature regression (ADR 0072's pattern is gone) in
exchange for soundness consistency with ADR 0074's rejection
philosophy.

## Consequences

- **`LIC-2.6c-tag-callee-arity-1` → resolved.** The
  `args.len()` reconstruction is gone.
- **`LIC-2.6c-tag-locals-fn-indirect-1` → resolved.** ADR 0074
  opened this when it rejected TaggedValue-returning functions
  in tables; the broader rejection in ADR 0075 subsumes it.
- **`LIC-2.6c-tag-hetero-fn-tbl-call-1` status revisited.**
  ADR 0072 marked it Resolved; ADR 0075 reverts the feature.
  Re-classified as **"Resolved by removal"** — the LIC entry
  remains in the Resolved column but the resolution is "not
  supported", with a forward pointer to a future phase that
  may re-enable via path B or E.
- **Test totals: 872 → 876 green.** 5 new (`tests/phase2_6c_
  tag_callee_arity.rs`), 6 reframed (`phase2_6c_tag_fn_tbl_
  call.rs` — positive → negative), 1 deleted
  (`call_non_function_tagged_local_traps`, runtime trap
  unreachable post-ADR-0075).
- **HIR**: +50 LOC (new `HirError::IndirectCallThroughTagged
  Local` variant, `lower_call` arm change, offset() entry).
  Net diff after the simplification: +20 LOC.
- **Codegen**: -70 LOC (TaggedValue branch + `emit_value_slot_
  check_function` helper deleted).
- **Tests**: +50 LOC net (5 new tests dominate).
- **`emit.rs::tests` ABI shape suite (ADR 0074 prep)** stays
  intact — function-return widening is unaffected.
- **No SoT tag table changes** — the TAG_FUNCTION constant
  itself stays (still used by `print` / `type` / `tostring` /
  eq dispatchers when consuming a TaggedValue local that was
  only assigned from a table read, never *called* through).

## Documentation updates

- [x] §1 slot layout — n/a (TAG_FUNCTION still defined; only
      its call-site consumer is removed).
- [x] §2 producer / source taxonomy — n/a (Function values are
      still storable in tables via ADR 0071; only the
      `Callee::Indirect` consumer is rejected).
- [x] §3 consumer coverage matrix — new "calling a tagged-slot
      Function" row marking it Rejected at HIR (with ADR 0075
      pointer).
- [x] §4 LIC consolidation — `callee-arity-1` and
      `locals-fn-indirect-1` move to Resolved;
      `hetero-fn-tbl-call-1` annotated "resolved by removal".
      Totals: **16 resolved / 1 partial / 2 pending**.
- [x] §5 runtime tag invariants — n/a (invariants unchanged;
      the trap helper deletion does not affect them).
- [x] §6 cross-reference — new "Indirect call safety boundary"
      subsection enumerating the acceptance criteria
      (`Function(arity)` direct / parameter — safe;
      `TaggedValue` from any source — rejected).
- [x] §7 open questions — re-prioritised: arity hardening
      removed (resolved); `multi-return × TaggedValue
      interleaving` promoted to #1; future indirect-call
      re-enablement path (B/E) added.
- [x] §8 ADR index — ADR 0075 row added; "Last updated" stamp
      bumped.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
