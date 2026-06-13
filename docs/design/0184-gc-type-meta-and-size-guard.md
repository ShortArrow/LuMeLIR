# 0184. GC Type Metadata Table + Allocation Size Guard (Tidy First Prep for Mark Phase)

- **Status:** Accepted
- **Kind:** Refactor Memo
- **Date:** 2026-06-13
- **Deciders:** ShortArrow

## Context

The pre-implementation Codex 6視点 review of Phase 3 GC ADRs 0159-0162 (memo: [`docs/notes/gc-0159-0162-preflight-review.md`](../notes/gc-0159-0162-preflight-review.md)) flagged two robustness concerns that must be settled before the mark-phase implementation ADR lands:

- **R2** — Per-type `GC_TYPE_*` dispatch lacks a chokepoint. Mark phase (ADR 0159), future sweep finalizer dispatch (ADR 0163 `__gc`), and any debug / introspection path will each open-code the same `match type_tag` if no decision table exists; silent drift between consumers is the textbook anti-ad-hoc symptom.
- **R3** — `GC_HEADER_OFF_SIZE` is u32 (offset 4 in the 16-byte header per ADR 0157). `emit_gc_alloc` truncates `payload_size` to u32 without bounds-checking. A ≥ 4 GiB payload silently wraps; `g_gc_total_bytes` accounting then desynchronises from the true heap footprint and sweep frees an incorrect size.

Both are preparatory work — neither changes observable behaviour for current 1412-test corpus. Tidy First precedent: ADR 0182 consolidated `mark_ident_as_table`/`mark_ident_as_string` with only two consumers; this ADR consolidates `GC_TYPE_*` dispatch *before* the first non-allocator consumer (mark phase) exists.

## Scope (literal)

- ✅ New `GcTypeMeta` struct + `gc_type_meta(type_tag: u8) -> &'static GcTypeMeta` decision table in `src/codegen/tagged.rs`. Fields: `name: &'static str`, `has_outgoing_refs: bool`.
- ✅ Size guard in `emit_gc_alloc` (`src/codegen/primitive.rs`): `arith.cmpi(ult, payload_size, 1<<32)` → trap on violation via new diagnostic global `s_gc_alloc_too_large`.
- ✅ Module init registers `s_gc_alloc_too_large` alongside `s_gc_oom`.
- ✅ No mark-phase code yet — that lands in ADR 0185.
- ✅ Behaviour-preserving for all real-world (< 4 GiB) allocations; 1412 tests stay green.
- ❌ Worklist capacity strategy (R1) — folded into ADR 0185 because it is intrinsic to the mark phase code, not the allocator wrapper.
- ❌ `gc_type_references` walk strategy (per-type outgoing reference layout). Deferred to ADR 0185 because the walk shape co-evolves with the mark loop; locking it here would speculate on the iteration form. The `has_outgoing_refs: bool` field is the minimum metadata mark phase needs to short-circuit STRING_OBJ / SCRATCH_BUF (which never recurse).
- ❌ Header field widening to u64. The trap chooses Option 2 from the pre-flight review (fail-fast) over Option 1 (header growth). When a real > 4 GiB string requirement emerges, a future ADR can widen the field.

## Decision

### `src/codegen/tagged.rs`

Add the metadata struct and decision table next to the existing `GC_TYPE_*` constants:

```rust
/// Pre-mark-phase metadata for each `GC_TYPE_*` tag (ADR 0184).
///
/// `has_outgoing_refs` is the v1 mark-phase short-circuit: types
/// flagged `false` (STRING_OBJ, SCRATCH_BUF) are immediately
/// marked BLACK without a worklist push. Per-type walk strategy
/// for `true` cases lives in mark-phase code (ADR 0185); this
/// table is the single chokepoint for any consumer that needs
/// the boolean (mark, future sweep finalizer dispatch, debug /
/// log paths).
pub(crate) struct GcTypeMeta {
    pub name: &'static str,
    pub has_outgoing_refs: bool,
}

pub(crate) fn gc_type_meta(type_tag: u8) -> &'static GcTypeMeta {
    match type_tag {
        GC_TYPE_TABLE         => &GcTypeMeta { name: "table",         has_outgoing_refs: true  },
        GC_TYPE_HASH_BUF      => &GcTypeMeta { name: "hash_buf",      has_outgoing_refs: true  },
        GC_TYPE_ARRAY_BUF     => &GcTypeMeta { name: "array_buf",     has_outgoing_refs: true  },
        GC_TYPE_STRING_OBJ    => &GcTypeMeta { name: "string_obj",    has_outgoing_refs: false },
        GC_TYPE_CLOSURE_CELL  => &GcTypeMeta { name: "closure_cell",  has_outgoing_refs: true  },
        GC_TYPE_UPVALUE_BOX   => &GcTypeMeta { name: "upvalue_box",   has_outgoing_refs: true  },
        GC_TYPE_SCRATCH_BUF   => &GcTypeMeta { name: "scratch_buf",   has_outgoing_refs: false },
        _ => unreachable!("unknown GC type tag: {type_tag}"),
    }
}
```

Rationale for `has_outgoing_refs` values:

| Type | `has_outgoing_refs` | Reason |
|---|---|---|
| TABLE | true | `array_buf` (offset 16), `hash_buf` (offset 24), `metatable_ptr` (offset 32) plus per-element refs |
| HASH_BUF | true | Each non-DELETED entry's key + value payload when tag is reference kind |
| ARRAY_BUF | true | Per-slot payload when tag is reference kind |
| STRING_OBJ | false | Immutable bytes; no outgoing references (ADR 0112) |
| CLOSURE_CELL | true | Upvalues (boxed or by-value) |
| UPVALUE_BOX | true | The boxed Lua value, which may be a reference kind |
| SCRATCH_BUF | false | `snprintf` / `tostring(Number)` raw bytes; no references |

### `src/codegen/primitive.rs`

In `emit_gc_alloc`, before the `emit_alloc_with_oom_check` call (line ~393), insert a bounds check on `payload_size`:

```rust
// ADR 0184 — guard against payload ≥ 4 GiB silently wrapping
// when truncated to u32 for GC_HEADER_OFF_SIZE.
let four_gib_const = block
    .append_operation(arith::constant(
        context,
        IntegerAttribute::new(types.i64, 1_i64 << 32).into(),
        loc,
    ))
    .result(0).unwrap().into();
let in_range: Value<'c, 'a> = block
    .append_operation(arith::cmpi(
        context,
        arith::CmpiPredicate::Ult,
        payload_size,
        four_gib_const,
        loc,
    ))
    .result(0).unwrap().into();
emit_assert_or_exit(context, block, in_range, "s_gc_alloc_too_large", types, loc);
```

Where `emit_assert_or_exit` is the existing exit-on-false helper used elsewhere (or its equivalent — implementation step verifies the helper name and inlines if needed).

### Module init

Register `s_gc_alloc_too_large` next to `s_gc_oom`:

```rust
emit_string_global(module, "s_gc_alloc_too_large",
    "out of memory: GC alloc payload >= 4 GiB\0");
```

## Alternatives considered

- **Skip the metadata table; open-code each consumer.** Rejected per Codex 第3原則 — open-coding a switch in each consumer is the textbook ad-hoc shape. The cost of a tiny table now is much less than four future `match` blocks that diverge silently.
- **Encode the walk strategy as data (offsets / lengths / iteration rules) now.** Rejected — the walk shape depends on the worklist abstraction (alloca vs heap, GREY vs BLACK ordering) which is not yet pinned. ADR 0185 picks it; the table grows from `bool` to a strategy enum at that point.
- **Widen `GC_HEADER_OFF_SIZE` to u64 (option 1 from the review).** Rejected for now — disruptive header growth (16 → 24 bytes affects every existing offset and every alloca-slot footprint) without a concrete use case. Revisit when a > 4 GiB allocation is a real user requirement.
- **Add the trap inline at each `emit_gc_alloc` call site.** Rejected — the wrapper is the chokepoint and is the correct place; per-site guards duplicate code and miss new sites.

## Consequences

**Positive**
- Mark phase implementation (ADR 0185) gets a clean lookup: `if !gc_type_meta(tag).has_outgoing_refs { mark_black; continue; }`.
- Future debug logging / introspection / sweep finalizer dispatch share the same `name` field.
- 4 GiB silent truncation becomes a fail-fast trap with a clear diagnostic. Codex 第3原則 "Robust" axis satisfied.
- No observable change for any real-world program.

**Negative**
- Every allocation pays one `arith.cmpi` + `scf.if` (or its assert lowering). Negligible — branch is statically biased toward in-range; LLVM's branch-weight inference handles it.
- Adds one new diagnostic global to module init. Cosmetic.

**Locked in until superseded**
- `gc_type_meta` is the SoT for per-type GC metadata. New `GC_TYPE_*` variants must add a row; the `unreachable!` arm fail-fasts on omission.

## Documentation updates

- [x] §8 — adds 0184.
- [x] ADR 0157 pre-impl note — R3 (size guard) marked DONE here.
- [x] ADR 0159 pre-impl note — R2 (per-type chokepoint) partially DONE here; remainder (`gc_type_references` walk strategy) tracked for ADR 0185.

## Test count delta

```
Step 0: 1412 (after 60932bf)
C1 (doc): 1412 → 1412
C2 (impl): 1412 → 1412 (behaviour-preserving)
```

No new tests — the metadata table is consumed only by future ADR 0185, and the size guard fires only at ≥ 4 GiB which is impractical to exercise in CI. Manual policy verification: code review of the cmpi + trap branch.

## Critical files

- `src/codegen/tagged.rs`:
  - Add `GcTypeMeta` struct.
  - Add `gc_type_meta(type_tag: u8) -> &'static GcTypeMeta` function.
- `src/codegen/primitive.rs`:
  - Insert size guard at top of `emit_gc_alloc`.
- `src/codegen/emit.rs` (or wherever module init registers GC globals):
  - Register `s_gc_alloc_too_large` diagnostic global.

## Risks

| Risk | Mitigation |
|---|---|
| `gc_type_meta` row drift when a new `GC_TYPE_*` is added | `unreachable!()` arm forces compilation error if the new tag is omitted from the table. |
| Size-guard cmpi blocks SSA optimisation of small-constant allocations | LLVM constant-folds: when `payload_size` is a compile-time constant < 4 GiB, the cmpi reduces to `true` and the trap branch is dead-code-eliminated. |
| `emit_assert_or_exit` helper name not present in current codebase | Implementation step verifies and inlines the equivalent (cmpi + scf.if + emit_exit_with_message) if no helper exists. |
| ADR 0185 needs richer metadata than `has_outgoing_refs: bool` | Expected — ADR 0185 extends the struct (likely adds an enum variant for per-type walk strategy) in the same chokepoint. |

## Future work

- ADR 0185 — Mark phase implementation. Consumes `gc_type_meta`; extends metadata with per-type walk strategy. Resolves R1 (worklist capacity).
- ADR 0186+ — Sweep phase + auto-trigger per existing 0161 / 0162 designs.
- ADR 0163 — `__gc` finalizers; consumes `name == "table"` from this metadata.

## References

- [ADR 0156](0156-gc-architecture-v1.md) — parent architecture.
- [ADR 0157](0157-gc-allocator-wrapper.md) — `emit_gc_alloc` chokepoint; receives the size guard here.
- [ADR 0158](0158-gc-migrate-remaining-types.md) — all 8 allocator sites already routed through the chokepoint.
- [ADR 0159](0159-gc-mark-phase.md) — first consumer of the metadata.
- [ADR 0182](0182-param-inference-kind-parameterised-helpers.md) — Tidy First-before-third-consumer precedent.
- [`docs/notes/gc-0159-0162-preflight-review.md`](../notes/gc-0159-0162-preflight-review.md) — review memo introducing R1 / R2 / R3.
