# 0081. Phase 2.8e-iter-next: `next(t, k)` Builtin + ForPairs HIR-desugar

- **Status:** Accepted
- **Date:** 2026-05-06
- **Deciders:** ShortArrow

## Context

ADR 0080 shipped `for k, v in pairs(t) do BODY end` via the
`HirStmtKind::ForPairs` opaque shape and a codegen-owned dual-
phase walker (`emit_for_pairs`). That ADR's §Refactor path
explicitly committed to a follow-up: «when ADR 0075's superseder
lands and unblocks `next(t, k)`, this variant becomes a
candidate for HIR-level desugaring to `for k, v in next, t, nil
do … end`».

Codex pre-ADR-0081 review pointed out that the full ADR 0075
superseder (general indirect-call re-enablement) has three
non-trivial open questions:

- **Plan B's `FuncId`-less function values**: function parameters
  (Phase 2.5b.2) carry no static `FuncId`, so a side table keyed
  by `FuncId` cannot represent them.
- **Signature encoding is weak**: arity + `ret_kind_count` cannot
  distinguish the multi-position TaggedValue ABIs (ADR 0076).
- **Tag check before bounds check**: the runtime helper must
  verify `tag == TAG_FUNCTION` before reading the payload word.

Codex therefore recommended splitting:

1. **Immediate need**: a restricted `next` builtin that lets
   ForPairs be a HIR-desugar instead of an opaque codegen shape.
2. **Long-term need**: the full ADR 0075 superseder for
   arbitrary-iter generic-for and method dispatch.

This ADR is the **Plan Alpha** half (immediate need). The
long-term superseder is parked as ADR 0082 with the three open
questions above as its design constraints.

## Decision

### Multi-return builtin infrastructure

`Builtin::ret_kinds()` returns the static return-kind vector for
each shipped builtin. Today every builtin except the new `Next`
returns at most one value, but the slot exposes the same shape
that `HirFunction::ret_kinds` uses on the user-function side, so
`MultiAssignFromCall` and `flat_result_index` (ADR 0076) flow
unchanged into builtin call sites.

`Callee::Builtin(b)` is now a permitted callee for
`MultiAssignFromCall`. The HIR `lower_local_multi` and
`lower_assign_multi` paths consult `b.ret_kinds()` to validate
the `names.len()` arity. Codegen `emit_multi_assign_from_call`
dispatches by callee kind into `emit_multi_assign_from_user_call`
(the existing logic, refactored into its own function) or
`emit_multi_assign_from_builtin` (new).

### `Builtin::Next`

A new variant `Builtin::Next` recognises the `next` keyword:

```rust
"next" => Some(Builtin::Next)
```

Arity is 2 (`table`, `prev_k`); `ret_kinds()` is `[TaggedValue,
TaggedValue]`. HIR's argument-kind gate accepts a Function value
as `prev_k` — function values are valid hash keys per Lua spec
§3.4.5 — by adding `Next` to the existing `Type | ToString`
allow-list. The arg-0 Table check is explicit.

Single-value position (`local k = next(t, c)`) is rejected at
codegen with a clear message pointing to the multi-assign form.
The vast majority of `next` use is via `for ... in pairs(t)` or
explicit multi-assign; supporting truncated single-value `next`
would require materialising a TaggedValue tmp slot at every
truncated call site and is out of scope for this ADR.

### `__lumelir_next` helper function

A module-level MLIR function `@__lumelir_next(t: ptr, prev_tag:
i64, prev_payload: i64) → (i64, i64, i64, i64)` is emitted
unconditionally per module (next to `@main`). The 4-i64 return
matches ADR 0076's flattened ABI for two consecutive TaggedValue
positions: `(next_k_tag, next_k_payload, next_v_tag,
next_v_payload)`. When `next` is unused in a translation unit
the LLVM backend's dead-code pass removes the function.

**Resume strategy**: a naive linear scan with a `found` flag.

```text
function body:
  result_slots = (NIL, 0, NIL, 0)   on stack
  done         = false              on stack
  found        = (prev_tag == NIL)  on stack — start "past" if no
                                              prev_k was given
  // Phase 1: array part (i = 1..=len, gated on !done)
  scf.while %i = 1 {
    after:
      slot = arr_buf + (i-1)*16
      tag  = load slot
      live = (tag != TAG_NIL)
      // Mark-found: was this slot the resume point?
      if !found && (prev_tag == NUMBER) && (sitofp(i) == prev_value):
        found = true
      // Record: live && found && this slot isn't the resume point.
      if live && found && !is_resume_slot:
        store result_slots = (NUMBER, i-as-i64, slot_tag, slot_payload)
        done = true
      yield i + 1
  }
  // Phase 2: hash part (bi = 0..cap, gated on !done)
  if hash_buf != null && !done {
    scf.while %bi = 0 {
      after:
        entry  = hash_buf + 16 + bi*32
        ktag   = load entry
        live   = (ktag != NIL && ktag != DELETED)
        keq    = (ktag == prev_tag && load(entry+8) == prev_payload)
        if live && !found && keq:
          found = true
        if live && found && !keq:
          store result_slots = (ktag, kpay, vtag, vpay)
          done = true
        yield bi + 1
    }
  }
  return result_slots
```

The walk is O(N) per call (vs the earlier `emit_for_pairs`'s
O(1) per step inside a single while). Across a full `for k, v in
pairs(t) do … end` loop the total work is O(N²). For typical
small Lua tables this is acceptable; future optimisation
(bucket-resume via the existing dispatched probe, ADR 0079)
becomes easy if measurements demand it.

The hash-key equality check uses raw i64 payload comparison,
matching ADR 0079's `emit_hash_key_eq_dispatched` for tag-equal
non-Number kinds. Number keys round-trip correctly because the
i64 payload is the bitcast of the f64 value, and we never
synthesize NaN here (NaN keys are HIR-rejected upstream).

### `ForPairs` HIR-desugar

`StmtKind::ForPairs` (parser AST) now lowers in HIR to:

```text
do
  local __t  = TABLE
  local __ctl = nil    -- TaggedValue, control variable
  local _broken_N = false
  while true do
    local k, v = next(__t, __ctl)   -- MultiAssignFromCall(Next)
    if IsNil(k) then _broken_N = true
    else BODY ; __ctl = k end
  end
end
```

User statements inside `BODY` are wrapped with `if not
_broken_N then STMT end` by `lower_scoped_body_no_push` (ADR
0015 break pattern), so a mid-body `break` skips the rest of
the iteration; the next `while` re-check terminates the loop
because the `break` flag is AND-extended into `cond` by the
While codegen.

### Removals

`HirStmtKind::ForPairs` (the opaque shape) is deleted. With it
go:
- `emit_for_pairs` (~140 LOC)
- `emit_for_pairs_array_phase` (~240 LOC)
- `emit_for_pairs_hash_phase` (~260 LOC)
- `emit_set_broken_if` (~30 LOC)
- `emit_ptrtoint_i64` (~22 LOC) — used only by the above; the
  `__lumelir_next` body uses an inline `OperationBuilder("llvm.
  ptrtoint", …)` instead
- `emit_copy_value_slot_16b` from `tagged.rs` — added in ADR
  0080 commit 1 expecting `__lumelir_next` to use it; the actual
  `__lumelir_next` body uses inline 2× i64 load/store, so this
  helper is dead. ~28 LOC removed
- `HirStmtKind::ForPairs` arm in `emit_stmt`, `collect_string_
  pool::visit_stmt`

The parser AST `StmtKind::ForPairs` and its visitor companions
remain — the source-level construct still exists; only the HIR
shape is replaced.

## Alternatives Considered

- **Alpha-2 (keep ForPairs HIR shape, replace codegen body
  with a `__lumelir_next` call loop)**: smaller delete-and-add
  diff, but leaves the HIR shape opaque and forfeits the
  natural HIR-desugar that ADR 0080 §Refactor path promised.
- **Alpha-3 (introduce `HirStmtKind::IteratorStep` opaque
  shape)**: replaces one opaque shape with another. Codegen
  surface ends up the same; HIR doesn't gain reusable
  primitives.
- **Beta (full ADR 0075 superseder, signature side table)**:
  Codex review flagged 3 unresolved design issues (FuncId-less
  functions, signature encoding, tag check). Tackling those is
  ADR 0082; doing them under the same ADR would have triple
  the LOC and three unbounded design questions in one commit.
- **Bucket-resume implementation of `next`**: faster (O(1) per
  step) but requires the hash-probe-with-bucket-index machinery
  to be exposed as a helper. Defer until a benchmark
  demonstrates the linear-scan cost matters.

## Consequences

- **`LIC-2.8e-iter-pairs-1` resolution mechanism** changes from
  ADR 0080 (codegen walker) to ADR 0081 (HIR-desugar via
  `Builtin::Next` + `__lumelir_next`). The LIC entry stays in
  the resolved column; the SoT cross-reference is rewritten.
- **New `LIC-2.8e-builtin-multi-return-1` (resolved):** Builtin
  callees can now declare multi-position return signatures and
  flow through `MultiAssignFromCall`. Future builtins (`unpack`,
  `string.match`, etc.) inherit this infrastructure for free.
- **Test totals: 935 → 940 green.** 5 new e2e in
  `tests/phase2_8e_next.rs` (empty / array-only / hash-only-
  sorted / array-then-hash transition / tombstone-skip). The
  16 e2e in `tests/phase2_8e_pairs.rs` are unchanged and
  continue to pass — the regression suite for the desugar.
- **LIC totals: 21 / 0 / 4 → 22 / 0 / 4** (resolved / partial /
  pending). One new resolved (`builtin-multi-return-1`); the
  ADR 0080-resolved `iter-pairs-1` keeps its column slot.
- **Source LOC**: HIR `+105` (Builtin variant + ret_kinds +
  arity/name + multi-assign callee path + ForPairs desugar +
  Next arg-kind check), codegen `+750` (`__lumelir_next` body
  + multi-assign-from-builtin + extract-prev-k + module init
  wiring), `−707` (`emit_for_pairs` + 4 helpers deleted) =
  **+148 net code**. Tests `+135`. ADR 0081 + SoT updates ~360
  doc lines.
- **Refactor path closed**: ADR 0080 §Refactor path is now
  realised. ADR 0081 §Refactor path forwards: when ADR 0082
  ships generic-for, `Builtin::Next` is the natural starting
  point — the same `MultiAssignFromCall(Builtin::Next)` shape
  that ForPairs uses today is what `for k, v in next, t, nil
  do … end` would parse to.

## Documentation updates

- [x] §1 slot layout — n/a.
- [x] §2 producer / source taxonomy — n/a (no new TaggedValue
      producer; `Builtin::Next` reuses `MultiAssignFromCall`).
- [x] §3 consumer matrix — n/a.
- [x] §4 LIC consolidation — `iter-pairs-1` resolution
      mechanism updated (ADR 0080 → ADR 0081); new
      `builtin-multi-return-1` added resolved. Totals: **24
      entries — 22 resolved / 0 partial / 4 pending**.
- [x] §5 runtime tag invariants — n/a.
- [x] §6 cross-reference — "Iteration: pairs codegen"
      subsection rewritten as "Iteration: pairs via next
      builtin".
- [x] §7 open questions — `pairs` codegen walker delete noted;
      ADR 0082 (Beta superseder) replaces ADR 0075 superseder
      placeholder at #1.
- [x] §8 ADR index — ADR 0081 row added; "Last updated" stamp
      bumped.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
