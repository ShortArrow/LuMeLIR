# 0074. Phase 2.6c-tag-locals-fn: Function-Return TaggedValue Widening

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

Through ADRs 0061–0073 the TaggedValue runtime model grew a
complete consumer matrix (Print / Type / ToString / Eq / Call)
on every value source **except** function returns. The HIR
layer rejected functions whose return paths produced different
kinds at the same position:

```rust
// src/hir/mod.rs (pre-ADR 0074):
for (i, (p, k)) in prev.iter().zip(kinds.iter()).enumerate() {
    if p != k {
        return Err(HirError::TypeMismatch { ... });
    }
}
```

That meant the most natural Lua idiom — **"return nil to signal
absence, otherwise return a value"** — was uncompilable:

```lua
local function maybe_square(x)
  if x > 0 then return x * x end
  return nil  -- HIR: TypeMismatch (number vs nil)
end
local v = maybe_square(-1)
print(v)
```

Codex post-ADR-0073 review made this `LIC-2.6c-tag-locals-fn-1`
the **#1 priority**: `ret_mlir_types` had `unreachable!()` on
`ValueKind::TaggedValue` as a visible loose thread, and the
function-return axis was the last big producer-side gap blocking
real Lua programs.

## Decision

### HIR: heterogeneous returns widen to `TaggedValue` (idempotent)

`lower_return_with_values` (src/hir/mod.rs) replaces the
mismatch error with monotone widening. Once a return position
sees two distinct kinds, it widens to `TaggedValue`; subsequent
return paths unconditionally route through the dispatched-store
helper:

```rust
for (i, (p, k)) in prev.iter().zip(kinds.iter()).enumerate() {
    if *p == *k || *p == ValueKind::TaggedValue {
        continue;  // already same kind, or already widened
    }
    let widened = self.in_function_ret_kinds.as_mut().unwrap();
    widened[i] = ValueKind::TaggedValue;
    let slot_id = ret_value_ids[i];
    self.locals[slot_id.0].kind = ValueKind::TaggedValue;
}
```

Once `_ret_value_i` becomes `TaggedValue`, subsequent `Assign`
into that slot dispatches by source kind via the existing
`emit_value_slot_store_dispatched` helper (ADR 0064 / 0071).

### `_ret_value_N` default initialisation

When the slot widens to TaggedValue and the function exits
without executing a `return` (Lua semantics: implicit `return
nil`), the slot must be Nil-tagged. Added to
`lower_function_body`:

```rust
ValueKind::TaggedValue => Some(HirExprKind::Nil),
```

This synthesises a `LocalInit { id: slot, value: Nil }` at the
top of the function body. Codegen lowers the Nil source through
`emit_value_slot_store_dispatched(kind: Nil)` → tag = TAG_NIL,
payload = 0.0.

### Codegen ABI: `(tag: i64, payload_raw: i64)` per TaggedValue position

`ret_mlir_types` maps each `ValueKind::TaggedValue` to **two**
MLIR results:

```rust
ValueKind::TaggedValue => vec![types.i64, types.i64],
```

Example: `local function maybe(x) ... end` widening to
`ret_kinds = [TaggedValue]` produces the MLIR signature
`func.func @user_maybe_0(%arg0: f64) -> (i64, i64)`.

`payload_raw` is **i64** — the same universal raw form used by
slot-to-slot copies (ADR 0064). The caller reassembles the
two i64 results into a 16-byte tagged slot, after which all
existing tag dispatch paths (ADR 0064 / 0067 / 0071) work
unchanged.

### Function-body return emission

`emit_function`'s return-extract loop reads a TaggedValue slot
as 2 MLIR values:

```rust
ValueKind::TaggedValue => {
    let tag = emit_load(&block, slot_ptr, types.i64, loc);
    let payload_ptr = emit_byte_offset_ptr(
        context, &block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
    let payload = emit_load(&block, payload_ptr, types.i64, loc);
    ret_values.push(tag);
    ret_values.push(payload);
}
```

### Caller-side: `emit_call_user_into_tagged_slot` + `_tmp` variants

Two new helpers wrap the call-emit + 2-result-pack pattern:

- `emit_call_user_into_tagged_slot` writes the two i64 results
  directly into a caller-provided 16-byte tagged slot. Used by
  the `LocalInit { id: TaggedValue, value: Call }` arm.
- `emit_call_user_into_tagged_tmp` allocates a tmp 16-byte slot,
  fills it via the above, returns the slot pointer. Used by
  the inline `print(f())` / `type(f())` / `tostring(f())` arms,
  mirroring the ADR 0070 pattern for `Index` source.

Three Builtin arms (`Print`, `Type`, `ToString`) gained a
`HirExprKind::Call` special case that materialises through
`_tmp` and dispatches via the existing `emit_*_tagged_local`
consumers.

### Out-of-scope (LIC entries opened by this ADR)

- **`LIC-2.6c-tag-locals-fn-indirect-1`** — calling a
  TaggedValue-returning function through `Callee::Indirect`
  (e.g. `local g = t[1]; g()`). The call-site arity
  reconstruction (ADR 0072) has no static signature to build
  the `(...) → (i64, i64)` return type. Mitigation: HIR
  rejects storing a TaggedValue-returning function in a table,
  alongside the existing closure-with-upvalues check. Backed
  by `tagged_return_function_called_through_tagged_local_
  rejected_at_hir` regression test.
- **`LIC-2.6c-tag-locals-fn-multi-1`** — `return 1, nil` vs
  `return nil, 1` cross-position interleaving. The codegen
  ABI handles single-position TaggedValue returns; multi-
  position TaggedValue interleaving needs the caller-side
  result-index walker to know per-position result widths.
- **`LIC-2.6c-tag-callee-arity-1`** — Codex called this out
  as the safety rider for ADR 0072's `args.len()` arity
  reconstruction. Same-phase implementation would need to
  expand the TAG_FUNCTION payload (currently 8 bytes for
  the bare ptr) to also carry signature info, which expands
  the slot beyond ARRAY_ELEM_SIZE = 16. Tracked separately.

## Alternatives Considered

- **Out-param ABI**: caller allocates a 16-byte slot and
  passes it as an extra `out_slot: ptr` parameter; callee
  writes through the pointer and returns void. More
  uniform but requires every existing call site to alloca
  even when the callee returns Number — significant code
  churn for what is currently a single-position widening.
  Rejected; multi-result return is MLIR-native and reuses
  the existing multi-return infrastructure (ADR 0021).
- **i128 (struct) return**: pack `(tag, payload)` into a single
  16-byte struct return. MLIR's `func` dialect doesn't natively
  support aggregate returns — would need lowering through
  `llvm.struct`, breaking the abstraction layer. Rejected.
- **Heap-box return**: return a `ptr` to a heap-allocated
  TaggedValue. Requires a GC strategy or a small allocator
  pool. Rejected as overkill for a value-type return.
- **Lazy widening at call site only**: keep `ret_kinds[i]` as
  the union of observed kinds, widen only when the caller
  context demands TaggedValue. Conflicts with multi-return's
  pre-commit ABI (the function signature must be fixed before
  any caller binds). Rejected.

## Consequences

- ~50 LOC in `src/hir/mod.rs` (widening logic, Nil-init for
  TaggedValue slot, indirect-call rejection in `Table` and
  `IndexAssign`).
- ~150 LOC in `src/codegen/emit.rs` (`ret_mlir_types`
  TaggedValue arm, return-emit TaggedValue path, two new
  call-pack helpers, three new builtin Call-arms).
- ~190 LOC in `tests/phase2_6c_tag_locals_fn.rs` (11 new
  e2e tests covering Number/Nil, Bool/Nil, String/Nil,
  Number/Bool widening; type/tostring/eq consumers; indirect
  rejection; pure-Number / pure-Nil regressions).
- 1 reframed unit test (`hir::tests::lower_inconsistent_bool_
  number_returns_widens_to_tagged`) — the old
  `lower_inconsistent_bool_number_returns_error` rejection test
  becomes a positive widening assertion.
- Total **858 → 869 tests** green.
- **`LIC-2.6c-tag-locals-fn-1` → resolved.** Three new
  pending LIC entries opened. Totals: **14 resolved /
  1 partial / 4 pending**.
- The MLIR signature surface gains a new shape: `(...) → (i64,
  i64)` for TaggedValue-returning functions. Existing
  `(f64,...) → f64` callers are unchanged — the widening only
  applies to functions that actually have heterogeneous return
  paths.

## Documentation updates

- [x] §1 slot layout — n/a (slot layout unchanged; the new ABI
      reuses the same `{tag, payload}` interpretation, just
      transmitted across the function boundary as 2 MLIR values).
- [x] §2 producer / source taxonomy — new row "function-return
      widening" added, citing ADR 0074.
- [x] §3 consumer coverage matrix — `Local(TaggedValue) from
      function return` shares all dispatchers with the existing
      `Local(TaggedValue)` row; cross-referenced. Also added
      the "inline `Call(User)` returning TaggedValue" entries
      for Print / Type / ToString.
- [x] §4 LIC consolidation — `fn-1` promoted to Resolved;
      `indirect-1`, `multi-1`, `callee-arity-1` added as
      Pending. Totals: 14 / 1 / 4.
- [x] §5 runtime tag invariants — n/a (invariants unchanged).
- [x] §6 cross-reference — new "Function-return ABI"
      subsection after Module Layout describing the
      `(tag, payload_raw)` MLIR result shape.
- [x] §7 open questions — re-prioritised: function-return
      widening removed (resolved); tag-dispatch skeleton
      extraction now #1; tagged-callee arity hardening #2;
      multi-return × TaggedValue interleaving and indirect
      tagged-return added.
- [x] §8 ADR index — ADR 0074 row; "Last updated" stamp
      bumped to "after ADR 0074".

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
