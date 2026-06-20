# 0229. `string.find` Multi-Return `(start, end)`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second M7 sub-ADR. [ADR 0228](0228-string-find-plain.md) shipped single-return `string.find(s, sub)` in plain (literal-byte) mode — returns the 1-indexed start position as `TaggedValue` Number-or-Nil. Lua 5.4 §6.4 specifies the full signature returns `(start, end)` plus any pattern captures; this ADR adds the `end` position via the existing ADR 0021 / 0081 multi-assign builtin dispatch.

The `end` derivation in plain mode is trivial: `end = start + sub_len - 1`. The Rust runtime helper is unchanged; the addition happens in MLIR.

## Scope (literal)

- ✅ Widen `Builtin::StringFind::ret_kinds` from `[TaggedValue]` to `[TaggedValue, TaggedValue]`.
- ✅ Add `Builtin::StringFind` arm in `emit_multi_assign_from_builtin` dispatching to a new `emit_call_string_find_into_locals` helper (sibling of `emit_call_pcall_into_locals` from ADR 0217).
- ✅ Helper invokes `lumelir_string_find_plain` to get the start position, then computes `end = start + sub_len - 1` where `sub_len` reads the i64 length header at offset 0 of the `sub` argument (ADR 0112 layout).
- ✅ On the match path, both slots receive `TAG_NUMBER + f64(position)`; on no-match both receive `TAG_NIL`.
- ✅ Single-assign `local s = string.find(...)` continues to return start-only via the existing `Callee::Builtin(Builtin::StringFind)` arm — `infer_kind` of the call site stays `ValueKind::TaggedValue` (position 0 truncation, per ADR 0081 Next precedent).
- ❌ Captures `(...)` returning positions 2..N. Pattern matcher port owns those.
- ❌ Magic-char patterns. Plain mode only; ADR 0228 §Scope literal still applies.
- ❌ `init` / `plain` spec args. Future sub-ADRs.

## Decision

### `end` position computation

In plain mode `sub` matches its own bytes literally, so on a match starting at 1-indexed `start`, the matched range is `[start, start + sub_len - 1]`. The `sub_len` value is the i64 stored at offset 0 of the `sub` boxed-string-object (ADR 0112). The codegen:

```mlir
%pos     = llvm.call @lumelir_string_find_plain(%s, %sub) : i64
%is_hit  = arith.cmpi ne, %pos, 0
%sub_len = llvm.load %sub : i64
%end_pos = arith.addi %pos, (arith.subi %sub_len, 1)
scf.if %is_hit {
  emit_value_slot_store_number(%start_slot, sitofp %pos)
  emit_value_slot_store_number(%end_slot,   sitofp %end_pos)
} else {
  emit_value_slot_store_nil(%start_slot)
  emit_value_slot_store_nil(%end_slot)
}
```

No new runtime helper, no Rust-side change.

### Why split off from ADR 0228

ADR 0228 deliberately deferred multi-return so the plain-substring + non-trapping TaggedValue-Local-cond machinery could land in one focused commit. Splitting:

1. Keeps each commit's diff scannable.
2. Lets the M7 milestone count progress with low-risk increments (per the roadmap minimum-viable framing).
3. Gives the eventual pattern matcher port a stable `Builtin::StringFind` shape (multi-return + TaggedValue captures) to extend rather than reshape.

## Tests

`tests/phase4_string_find_multireturn.rs` (NEW, 4 e2e):

1. Multi-word substring → `(start=7, end=11)` for `"hello world" / "world"`.
2. Pattern at start → `(1, 3)` for `"abcdef" / "abc"`.
3. No match → `(nil, nil)`.
4. Single-char pattern → `(start, end)` where start == end.

The 6 e2e from ADR 0228 stay green via the single-return truncation path.

## Test count delta

```
Step 0:  1525 (after ADR 0228)
C3 (impl + 4 e2e): 1525 → 1529
```

## References

- [ADR 0228](0228-string-find-plain.md) — single-return foundation.
- [ADR 0021](0021-phase2-5d-multi-return.md) — multi-return ABI.
- [ADR 0081](0081-phase2-8e-iter-next.md) — Next multi-return precedent + single-assign truncation rule.
- [ADR 0217](0217-pcall-multireturn-abi.md) — sibling `emit_call_*_into_locals` precedent (Pcall).
- [ADR 0112](0112-string-abi-refactor.md) — boxed-string `i64 len` header read.
- [Lua 5.4 §6.4.1](https://www.lua.org/manual/5.4/manual.html#6.4.1) — `string.find` return spec.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M7 milestone.
