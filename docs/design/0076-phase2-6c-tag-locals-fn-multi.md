# 0076. Phase 2.6c-tag-locals-fn-multi: Multi-Position TaggedValue Caller-Side Walker

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

ADR 0074 (function-return TaggedValue widening) established the
`(...) → (i64 tag, i64 payload_raw)` ABI for a function whose
return position widens to TaggedValue. The HIR widening logic in
`lower_return_with_values` was already per-position-independent
— each return position's kind is upgraded in isolation when a
later return path produces a different kind at that position.
The codegen `ret_mlir_types` was also already correct for
multi-position TaggedValue returns: its `flat_map` expansion
maps `[Number, TaggedValue, Bool]` to `[f64, i64, i64, i1]`.

What ADR 0074 did **not** ship was the caller-side
**result-index walker**. The two affected code sites both
assumed "1 MLIR result per logical return position":

```rust
// emit_call_user_into_tagged_slot (used by LocalInit /
// Builtin Print/Type/ToString inline arms):
let tag = op_ref.result(0).unwrap().into();
let payload = op_ref.result(1).unwrap().into();

// emit_multi_assign_from_call (used by `local a, b = f()`):
for (i, dst) in dst_ids.iter().enumerate() {
    let v = op_ref.result(i).unwrap().into();  // ← position 1:1 with MLIR result
    ...
}
```

Both broke when a position other than 0 widened to TaggedValue
(or when more than one position widened). Tracked as
`LIC-2.6c-tag-locals-fn-multi-1`. Codex post-ADR-0075 review
made this the **#1 priority** with the scope hint:

> direct user call path only — `args.len()` から関数型を再構築
> する危険を完全に閉じた ADR 0075 の安全境界はそのまま維持し、
> caller-side result walker の一般化として multi-return ×
> TaggedValue を片付けるのが最も筋が良い。

## Decision

### Two pure helpers

```rust
fn ret_kind_result_width(kind: ValueKind) -> usize {
    match kind {
        ValueKind::TaggedValue => 2,
        _ => 1,
    }
}

fn flat_result_index(ret_kinds: &[ValueKind], pos: usize) -> usize {
    ret_kinds[..pos]
        .iter()
        .map(|k| ret_kind_result_width(*k))
        .sum()
}
```

`ret_kind_result_width` mirrors the per-kind `flat_map`
expansion in `ret_mlir_types`. `flat_result_index` is the
prefix sum that maps a logical position to its starting MLIR
result index.

### One pack helper

```rust
fn emit_pack_tagged_result_at_pos<'a, 'c, 'op>(
    context: &'c Context,
    block: &'a Block<'c>,
    op_ref: &OperationRef<'c, 'op>,
    dst_slot: Value<'c, 'a>,
    ret_kinds: &[ValueKind],
    pos: usize,
    types: &Types<'c>,
    loc: Location<'c>,
) where 'op: 'a
{
    debug_assert!(matches!(ret_kinds[pos], ValueKind::TaggedValue));
    let start = flat_result_index(ret_kinds, pos);
    let tag: Value<'c, 'a> = op_ref.result(start).unwrap().into();
    let payload: Value<'c, 'a> = op_ref.result(start + 1).unwrap().into();
    emit_store(block, tag, dst_slot, loc);
    let payload_ptr =
        emit_byte_offset_ptr(context, block, dst_slot, ARRAY_ELEM_OFF_VALUE, types, loc);
    emit_store(block, payload, payload_ptr, loc);
}
```

Read 2 results starting at `flat_result_index(ret_kinds, pos)`,
store them as `(tag, payload_raw)` into the 16-byte tagged
destination slot. The `'op: 'a` bound captures that the
operation reference outlives the block reference (the block
owns the operation).

### `emit_multi_assign_from_call` walker

Replace the loop body with per-position dispatch:

```rust
for (i, dst) in dst_ids.iter().enumerate() {
    let info = &locals[dst.0];
    let dst_slot = slots[dst.0];
    match info.kind {
        ValueKind::TaggedValue => {
            emit_pack_tagged_result_at_pos(
                context, block, &op_ref, dst_slot,
                &target.ret_kinds, i, types, loc,
            );
        }
        _ => {
            let result_idx = flat_result_index(&target.ret_kinds, i);
            let v = op_ref.result(result_idx).unwrap().into();
            // existing per-kind store path (Function ucast etc.)
            ...
        }
    }
}
```

The non-TaggedValue branch still uses `flat_result_index` so
positions after a TaggedValue position correctly skip past its
2 MLIR results.

### `emit_call_user_into_tagged_slot` simplification

The single-position helper now delegates to the pack helper at
position 0:

```rust
let op_ref = block.append_operation(call_op);
emit_pack_tagged_result_at_pos(
    context, block, &op_ref, dst_slot, &target.ret_kinds, 0,
    types, loc,
);
```

Same observable behaviour, single source of truth for the pack
sequence.

### HIR / `ret_mlir_types` — no change

Both were already multi-position-correct. Verified by ADR 0076's
test suite: every cross-position interleave case (Number/Nil,
Bool/Nil, String/Nil, Number/String) lowers, compiles, and runs
without HIR changes.

## Alternatives Considered

- **Defer**: leave LIC-2.6c-tag-locals-fn-multi-1 pending and
  prioritise other features (string→number coerce, iteration).
  Rejected — the gap is the natural completion of ADR 0074 and
  blocks the most common Lua "return value or nil per position"
  pattern in multi-return contexts (e.g. `pcall`-style success/
  error pairs, even though pcall itself is a separate phase).
- **Trampoline / adapter functions**: synthesise per-call wrapper
  functions matching the call-site shape. Heavy implementation
  for what is fundamentally an indexing problem at the call
  site. Rejected.
- **Out-param ABI**: caller allocates the destination slot and
  passes it as an extra parameter; callee writes through it.
  More uniform but breaks every existing call site. Rejected
  for the same reason ADR 0074 chose the multi-result approach.

## Consequences

- **`LIC-2.6c-tag-locals-fn-multi-1` → resolved.** Pending
  count drops 2 → 1.
- **Test totals: 876 → 888 green.** 11 new e2e tests
  (`tests/phase2_6c_tag_locals_fn_multi.rs`) plus 1 MLIR-shape
  test (`emit_function_with_multi_position_tagged_return_uses_
  4_i64_results`).
- **`src/codegen/emit.rs`**: +50 LOC (3 helpers + walker
  generalisation). No removed code — the original
  `emit_call_user_into_tagged_slot` body simplifies but its
  signature is unchanged.
- **Function-return ABI surface gains a richer shape**:
  `(...) → (i64, i64, i64, i64)` for two TaggedValue
  positions; `(f64, i64, i64, i1)` for `[Number, TaggedValue,
  Bool]`; etc. The `flat_map`-based `ret_mlir_types` continues
  to drive the callee side.
- **`Callee::Indirect` remains TaggedValue-rejected** (ADR
  0075). Multi-return × TaggedValue only flows through
  `Callee::User` direct calls.
- The MLIR-shape test pinning `(i64, i64, i64, i64)` provides a
  refactor safety net for any future helper consolidation
  (e.g. tag-dispatch skeleton extraction reconsidered).

## Documentation updates

- [x] §1 slot layout — n/a (slot layout unchanged).
- [x] §2 producer / source taxonomy — n/a (function-return
      widening producer row already covers multi-position; the
      ABI extension is a codegen concern).
- [x] §3 consumer coverage matrix — n/a (consumer matrix is
      per-source × per-tag; multi-position changes neither
      axis).
- [x] §4 LIC consolidation — `locals-fn-multi-1` promoted to
      Resolved. Totals: **17 resolved / 1 partial / 1 pending**.
- [x] §5 runtime tag invariants — n/a (invariants unchanged).
- [x] §6 cross-reference — "Function-return ABI" subsection
      gains a `ret_kinds → flat MLIR result indices` table
      example showing `[Number, TaggedValue, Bool]` →
      `[f64, i64, i64, i1]` mapping.
- [x] §7 open questions — `multi-return × TaggedValue` removed
      (resolved); `string → number arith coerce` promoted to #1.
- [x] §8 ADR index — ADR 0076 row added; "Last updated" stamp
      bumped.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
