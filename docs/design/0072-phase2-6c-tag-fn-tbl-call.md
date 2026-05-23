# 0072. Phase 2.6c-tag-fn-tbl-call: Calling a Function Value Through a Tagged Slot

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

ADR 0071 opened TAG_FUNCTION (= 4) so a closure-less function
value can live in an `array_buf` / `hash_buf` slot, but stopped
short of letting users *call* it once retrieved:

```lua
local function f() return 42 end
local t = {f}
local g = t[1]   -- g : TaggedValue{tag=TAG_FUNCTION}
print(g())       -- HIR: UnknownFunction { name: "g" } -- before
                 --      g is TaggedValue, not Function-kind
```

That gap was tracked as `LIC-2.6c-tag-hetero-fn-tbl-call-1`.
The Codex post-ADR-0071 review put it at the top of the queue:
the producer side ("Function を格納できる") shipped, but the
consumer side ("呼び出せない") still trapped, leaving an
asymmetric semantic that surprised users — readability passes
(`type(g)`, `print(g)`, `tostring(g)` from ADR 0071) all worked
but the most natural follow-up call did not.

The fail-fast trap from ADR 0069 means any tag the call site
doesn't yet handle (e.g. TAG_TABLE) exits cleanly, so the
relaxation is safe to land incrementally.

## Decision

### HIR: extend `lower_call` to accept TaggedValue locals

`lower_call` already special-cased `ValueKind::Function(arity)`
locals before falling through to the function-name table /
builtin lookup. The new arm sits immediately after it:

```rust
if matches!(self.locals[local_id.0].kind, ValueKind::TaggedValue) {
    let lowered_args = args.iter()
        .map(|a| self.lower_expr(a))
        .collect::<Result<Vec<_>, _>>()?;
    for arg in &lowered_args {
        let k = infer_kind(arg, &self.locals, &self.functions);
        if !matches!(k, ValueKind::Number) {
            return Err(HirError::TypeMismatch {
                op: format!("call-{name}"),
                lhs_kind: "number".to_owned(),
                rhs_kind: k.name().to_owned(),
                offset: arg.span.start,
            });
        }
    }
    return Ok(HirExprKind::Call {
        callee: Callee::Indirect(local_id),
        args: lowered_args,
    });
}
```

Three deliberate omissions vs. the Function-kind arm:

1. **No static arity check.** A tagged slot's payload is a bare
   `!llvm.ptr` (ADR 0071) with no arity descriptor. The codegen
   side reconstructs the function type from `args.len()`.
2. **Args are Number-only** (no `Function(_)` / `Number-or-
   function` slack). Function-typed args through tagged-call
   would need cascading widenings; defer until needed.
3. **No closure-with-upvalues escape check.** ADR 0071 already
   blocks closures with upvalues from entering the slot at
   storage time, so by the time we reach the call site the
   payload is provably closure-less.

### Codegen: tag-aware dispatch in `Callee::Indirect`

The existing `Callee::Indirect` arm extracts the function
pointer from a Function-kind slot. Extend it with a TaggedValue
arm:

```rust
let (ptr_val, arity) = match info.kind {
    ValueKind::Function(a) => {
        let ptr_val = emit_load(block, slots[*idx], types.ptr, loc);
        (ptr_val, a)
    }
    ValueKind::TaggedValue => {
        emit_value_slot_check_function(context, block, slots[*idx], types, loc);
        let payload_ptr = emit_byte_offset_ptr(
            context, block, slots[*idx], ARRAY_ELEM_OFF_VALUE, types, loc,
        );
        let ptr_val = emit_load(block, payload_ptr, types.ptr, loc);
        (ptr_val, args.len())
    }
    _ => unreachable!("Callee::Indirect on non-Function/TaggedValue local"),
};
let p_types: Vec<Type<'c>> = (0..arity).map(|_| types.f64).collect();
let fn_ty: Type<'c> = FunctionType::new(context, &p_types, &[types.f64]).into();
emit_unrealized_cast(block, ptr_val, fn_ty, loc)
```

`emit_value_slot_check_function` is a new helper modelled on
`emit_value_slot_check_number` (ADR 0063): load the i64 tag,
compare against `TAG_FUNCTION`, exit with
`s_table_type_mismatch` on mismatch. This makes
`local g = t[1]; g()` where `t[1]` is a Number / String / Table
fail fast at the call site rather than blindly bit-casting the
payload.

### Why arity inference from `args.len()` is safe today

LuMeLIR's current function ABI is fixed at "all params f64,
single f64 return" for user-defined functions reachable via
`Callee::Indirect` (ADR 0018). A TaggedValue-bound function
that crossed through a table necessarily satisfied ADR 0071's
`closure_with_upvalues = empty` precondition, which means it
was either a `local function` literal or a top-level
`function f(...)`. Both compile to `func.func @user_f_NN`
whose MLIR signature is `(f64,...,f64) -> f64`. The reconstructed
`fn_ty` therefore matches the callee's actual MLIR type — no
verifier failure.

If the user passes the wrong arity (e.g. `g()` when the
underlying function takes one arg), the LLVM-level
`func.call_indirect` lowers to a plain function pointer call;
the resulting binary either reads a garbage register or
segfaults. This matches Lua's "missing args become nil" only
in the trivial zero-arg case; we accept the gap as a
follow-up. Returns are similarly assumed to be a single f64.
Multi-return through a tagged callee is out of scope.

## Alternatives Considered

- **Embed arity in TAG_FUNCTION payload.** Use the high bits of
  the i64 tag word (or a 16-byte payload) to carry arity.
  Requires re-engineering the universal 16-byte slot layout
  (ADR 0064) just for one tag — too costly for a feature that
  the static arity inference covers in practice.
- **Reject the call at HIR time and emit a typed error.** Would
  surface the arity-unknown problem early but blocks the
  natural `local g = t[1]; g()` shape that the user
  demonstrably wants. The runtime trap on tag mismatch is the
  honest middle ground.
- **Make the consumer extract path tag-aware in `emit_expr` for
  `Local(TaggedValue)` instead.** Generalises beyond the call
  site (would also enable e.g. `local g = t[1]; t[2] = g`),
  but balloons the diff and runs into the multi-tag dispatch
  problem each non-call extract site already solved separately.
  Keep `Local(TaggedValue)` extract Number-only as ADR 0063
  decided; do tag dispatch at the consumer (ADR 0064 + 0071
  pattern). The call site is just one more consumer.

## Consequences

- ~30 LOC in `src/hir/mod.rs` (new TaggedValue arm in
  `lower_call`).
- ~70 LOC in `src/codegen/emit.rs` (new
  `emit_value_slot_check_function` helper + TaggedValue arm in
  `Callee::Indirect`).
- 7 new e2e tests in `tests/phase2_6c_tag_fn_tbl_call.rs`.
  Total green at **858** (= 851 + 7).
- **`LIC-2.6c-tag-hetero-fn-tbl-call-1` → resolved.** Pending
  count drops from 2 → 1.
- The `emit_tagged_unknown_tag_trap` site stays as the
  truly-unknown guard rail for tags ≥ 6; the new
  `emit_value_slot_check_function` is the targeted "wrong
  tag at a Function-only consumer" trap (analogous to ADR 0063's
  `emit_value_slot_check_number`).
- Calling a tagged Function value with the *wrong* arity falls
  through to a raw function-pointer call — the binary will
  miscompute or crash. Acceptable today; tracked as the
  "arity assumption" sentence in the Lua-Incompatibility
  follow-ups (no separate LIC because the contract is honest:
  args.len() must match the underlying function).

## Documentation updates

- [x] §1 slot layout — n/a (tag space and payload types unchanged).
- [x] §2 producer / source taxonomy — n/a (no new producer).
- [x] §3 consumer coverage matrix — new "Calling a TaggedValue
      callee" sub-section added under §3 covering TAG_FUNCTION
      (call) + truly-unknown tag (trap).
- [x] §4 LIC consolidation — `fn-tbl-call-1` promoted to
      Resolved. Totals: **13 resolved / 1 partial / 1 pending**.
- [x] §5 runtime tag invariants — n/a (invariants unchanged).
- [x] §7 open questions — re-prioritised: function-return
      widening moves to #1; tagged.rs split #2;
      closure-escape #3; `fn-tbl-call-1` removed (resolved).
- [x] §8 ADR index — ADR 0072 row added; "Last updated" bumped
      to "after ADR 0072".

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
