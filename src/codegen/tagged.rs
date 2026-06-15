//! TaggedValue runtime model — slot layout, tag constants, and the
//! per-tag store / check helpers that read or write a 16-byte
//! tagged value slot.
//!
//! Phase 2.6c-tag-rs-split (ADR 0073) extracted this from
//! `emit.rs`. The dependency direction is one-way: `tagged.rs` →
//! `primitive.rs` → `melior`. `emit.rs` then uses both. See
//! `docs/design/tagged-semantics.md` for the SoT — slot layout in
//! §1, tag values in §1, store / check helper inventory in §3,
//! invariants in §5.

use melior::{
    Context,
    dialect::{arith, scf},
    ir::{
        Block, BlockLike, Location, Region, RegionLike, Value,
        attribute::{FloatAttribute, IntegerAttribute},
        operation::OperationBuilder,
    },
};

use crate::hir::ValueKind;

use super::primitive::{
    Types, emit_addressof, emit_byte_offset_ptr, emit_exit_with_message, emit_load,
    emit_print_string_obj, emit_printf, emit_store, emit_string_obj_eq,
};

// =============================================================
// Phase 2.6c-tag-arr (ADR 0059): tagged array_buf slots.
//
// Each array element is a 16-byte slot:
//   offset 0  : i64 tag    (TAG_NIL=0, TAG_NUMBER=1; 2..=5 reserved)
//   offset 8  : f64 value  (zero when tag is Nil)
//
// Holes (`t[i] = v` for `i > length+1`) fill intermediate slots
// with TAG_NIL; reads check the tag and trap on mismatch.
// =============================================================
pub(crate) const ARRAY_ELEM_SIZE: i64 = 16;
// `ARRAY_ELEM_OFF_TAG = 0`: tag at slot start. Implicit in the
// `emit_load(elem_ptr, types.i64, …)` calls; not referenced by
// name to avoid an unused-const warning.
pub(crate) const ARRAY_ELEM_OFF_VALUE: i64 = 8;
pub(crate) const TAG_NIL: i64 = 0;
pub(crate) const TAG_NUMBER: i64 = 1;
// Phase 2.6c-tag-hetero (ADR 0064): widen the tagged slot to
// carry Bool and String values.
pub(crate) const TAG_BOOL: i64 = 2;
pub(crate) const TAG_STRING: i64 = 3;
// Phase 2.6c-tag-fn-tbl (ADR 0071): Function and Table values
// occupy tags 4 and 5. Phase 2.5c-full Commit 2b (ADR 0083):
// the TAG_FUNCTION payload is now a closure cell ptr (`!llvm.ptr`
// to `!llvm.struct<(ptr, i64)>` — see `closure.rs`), not a raw
// fn ptr. Producers store cell ptr; consumers `cell.fn_ptr`-load
// (offset 0) before issuing the actual call. Capturing closures
// remain HIR-rejected until Commit 3
// (LIC-2.6c-tag-hetero-closure-escape-1).
pub(crate) const TAG_FUNCTION: i64 = 4;
pub(crate) const TAG_TABLE: i64 = 5;
// Phase 2.6b-hash-keys (ADR 0079): hash-bucket tombstone tag.
// Marks a hash entry whose key was deleted by `t.k = nil`
// (ADR 0062) but whose bucket position must remain reserved
// to keep linear-probing chains intact. `TAG_DELETED` is only
// ever observed in a hash entry's *key* slot — the value
// dispatchers (print/type/tostring/eq) never see it because
// the probe loop skips deleted buckets before any lookup
// resolves. Replaces the previous ptr-sentinel
// `HASH_DELETED_KEY=1` (retired by ADR 0079).
pub(crate) const TAG_DELETED: i64 = 6;

// ADR 0156 / 0157 — Phase 3 GC header layout. Every GC-managed
// allocation is prefixed with a 16-byte header:
//   offset 0: u8  mark        (0=WHITE / 1=GREY / 2=BLACK)
//   offset 1: u8  type_tag    (one of GC_TYPE_*)
//   offset 2: u16 padding     (reserved for weak-flag bits etc.)
//   offset 4: u32 payload_size
//   offset 8: ptr next        (singly-linked global list `g_gc_head`)
pub(crate) const GC_HEADER_SIZE: i64 = 16;
pub(crate) const GC_HEADER_OFF_MARK: i64 = 0;
pub(crate) const GC_HEADER_OFF_TYPE_TAG: i64 = 1;
pub(crate) const GC_HEADER_OFF_SIZE: i64 = 4;
pub(crate) const GC_HEADER_OFF_NEXT: i64 = 8;

/// ADR 0186 — auto-trigger threshold constants for
/// `emit_gc_alloc`. When `g_gc_total_bytes + payload_size` reaches
/// `g_gc_threshold`, the allocator calls `@gc_mark` + `@gc_sweep`
/// before allocating. Post-sweep the threshold doubles to
/// `min(post_sweep_total * 2, GC_THRESHOLD_CAP)` to avoid
/// thrashing on large live working sets.
pub(crate) const GC_THRESHOLD_INIT: i64 = 1_048_576; // 1 MiB
pub(crate) const GC_THRESHOLD_CAP: i64 = 1_073_741_824; // 1 GiB

/// ADR 0200 — `g_gc_pause` initial value (Lua 5.4 default: 200).
/// Acts as a percentage; doubling logic uses `g_gc_pause / 100`.
pub(crate) const GC_PAUSE_INIT: i64 = 200;

/// ADR 0185 — v1 mark colour values stored in
/// `*(obj + GC_HEADER_OFF_MARK)`. Mark phase sets BLACK; sweep
/// resets BLACK → WHITE for the next cycle. GREY is reserved
/// for ADR 0187 worklist DFS.
pub(crate) const GC_MARK_WHITE: i64 = 0;
#[allow(dead_code)] // first consumer: ADR 0187 (worklist DFS)
pub(crate) const GC_MARK_GREY: i64 = 1;
pub(crate) const GC_MARK_BLACK: i64 = 2;

pub(crate) const GC_TYPE_TABLE: u8 = 1;
pub(crate) const GC_TYPE_HASH_BUF: u8 = 2;
pub(crate) const GC_TYPE_ARRAY_BUF: u8 = 3;
pub(crate) const GC_TYPE_STRING_OBJ: u8 = 4;
pub(crate) const GC_TYPE_CLOSURE_CELL: u8 = 5;
pub(crate) const GC_TYPE_UPVALUE_BOX: u8 = 6;
pub(crate) const GC_TYPE_SCRATCH_BUF: u8 = 7;

/// ADR 0184 — pre-mark-phase metadata per `GC_TYPE_*` tag.
///
/// `has_outgoing_refs` is the v1 mark-phase short-circuit: types
/// flagged `false` (STRING_OBJ, SCRATCH_BUF) are immediately
/// marked BLACK without a worklist push. Per-type walk strategy
/// for `true` cases lives in mark-phase code (ADR 0185); this
/// table is the single chokepoint for any consumer that needs
/// per-type metadata (mark, future sweep finalizer dispatch,
/// debug / log paths). New `GC_TYPE_*` variants must add a row;
/// the `unreachable!` arm fail-fasts on omission.
#[allow(dead_code)] // first consumer: ADR 0185 (mark phase)
pub(crate) struct GcTypeMeta {
    pub name: &'static str,
    pub has_outgoing_refs: bool,
}

#[allow(dead_code)] // first consumer: ADR 0185 (mark phase)
pub(crate) fn gc_type_meta(type_tag: u8) -> &'static GcTypeMeta {
    match type_tag {
        GC_TYPE_TABLE => &GcTypeMeta {
            name: "table",
            has_outgoing_refs: true,
        },
        GC_TYPE_HASH_BUF => &GcTypeMeta {
            name: "hash_buf",
            has_outgoing_refs: true,
        },
        GC_TYPE_ARRAY_BUF => &GcTypeMeta {
            name: "array_buf",
            has_outgoing_refs: true,
        },
        GC_TYPE_STRING_OBJ => &GcTypeMeta {
            name: "string_obj",
            has_outgoing_refs: false,
        },
        GC_TYPE_CLOSURE_CELL => &GcTypeMeta {
            name: "closure_cell",
            has_outgoing_refs: true,
        },
        GC_TYPE_UPVALUE_BOX => &GcTypeMeta {
            name: "upvalue_box",
            has_outgoing_refs: true,
        },
        GC_TYPE_SCRATCH_BUF => &GcTypeMeta {
            name: "scratch_buf",
            has_outgoing_refs: false,
        },
        _ => unreachable!("unknown GC type tag: {type_tag}"),
    }
}

// =============================================================
// Phase 2.6b-hash-key-validity (ADR 0087): runtime validity
// policy for a hash search-key tag.
//
// Pure decision: maps a tag value to the set of validity
// policies that must run before the key reaches the probe.
// The effectful emitter `emit_hash_key_runtime_validity_gate`
// in `emit.rs` consults this matrix and emits the actual
// MLIR scf.if + trap structure. Trap message globals
// (`s_table_index_nil` / `s_table_index_nan`) are owned by
// `emit.rs`; this module owns the policy decision only.
//
// Splitting decision from emission lets us unit-test the
// policy table with no MLIR Context, and lets future tag
// kinds (WeakKey / ThreadKey) extend coverage by editing the
// table alone — call sites need no change.
// =============================================================
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum HashKeyValidityPolicy {
    /// Tag is TAG_NIL: Lua §3.4.5 forbids nil as a table key.
    /// Emitter exits with `s_table_index_nil`.
    TrapNil,
    /// Tag is TAG_NUMBER: payload is f64 — must be tested for
    /// NaN (Lua §3.4.5 forbids NaN as a key). Emitter loads
    /// the f64 payload and exits with `s_table_index_nan` on
    /// `cmpf Une self self`.
    CheckNaN,
}

/// Pure: which validity policies apply to a hash-key tag.
/// Returns a static slice; future tag kinds (WeakKey /
/// ThreadKey) extend this matrix without changing call sites.
/// Tags with no entry (String / Bool / Function / Table /
/// Deleted) are pass-through — the probe handles them
/// directly.
pub(crate) fn policy_for_tag(tag: i64) -> &'static [HashKeyValidityPolicy] {
    match tag {
        TAG_NIL => &[HashKeyValidityPolicy::TrapNil],
        TAG_NUMBER => &[HashKeyValidityPolicy::CheckNaN],
        _ => &[],
    }
}

// =============================================================
// Phase 2.7p-tagged-arith-coerce (ADR 0089): operand coercion
// plan for a TaggedValue arith / bitwise / unary consumer.
//
// Pure decision: maps a runtime tag to the action the chokepoint
// (`emit_load_tagged_operand_as_number` in `emit.rs`) must take
// to materialise an `f64` for downstream Number-consuming ops.
//
// Lua spec §3.4.3 / §3.4.2 allow String → Number coercion in
// arithmetic and bitwise contexts; this module owns the policy,
// the effectful emitter owns the MLIR scf.if + sscanf + trap
// structure. Trap message globals
// (`s_arith_on_non_numeric` / `s_arith_coerce_failed`) live in
// `emit.rs`. Call sites pick the plan via tag dispatch; the
// chokepoint is `emit_tagged_arith_runtime_dispatch` (BinOp) and
// `emit_tagged_unary_arith_runtime_dispatch` (UnaryOp).
// =============================================================
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaggedArithOperandPlan {
    /// Tag is TAG_NUMBER: load f64 payload at slot+8.
    UseNumberPayload,
    /// Tag is TAG_STRING: load ptr payload at slot+8 and coerce
    /// via `emit_tonumber_for_arith` (ADR 0077). Parse failure
    /// internally traps with `s_arith_coerce_failed`.
    CoerceStringToNumber,
    /// Tag is Bool / Nil / Function / Table / Deleted: emit exit
    /// with `s_arith_on_non_numeric`. Lua spec §3.4.3:
    /// "attempt to perform arithmetic on a {type} value".
    TrapNonNumeric,
}

/// Pure: which operand plan applies to a TaggedValue arith
/// consumer's runtime tag. Total over the i64 tag space —
/// unknown tags fall through to `TrapNonNumeric` (defensive).
/// Future tag kinds extend coverage here without touching the
/// chokepoint shape.
pub(crate) fn policy_for_tagged_arith_operand(tag: i64) -> TaggedArithOperandPlan {
    match tag {
        TAG_NUMBER => TaggedArithOperandPlan::UseNumberPayload,
        TAG_STRING => TaggedArithOperandPlan::CoerceStringToNumber,
        _ => TaggedArithOperandPlan::TrapNonNumeric,
    }
}

/// Allocate a stack slot whose layout matches `kind`. TaggedValue
/// uses 2 × i64 (16 bytes, naturally 8-byte-aligned for the f64
/// payload at offset 8); every other kind is a single element of
/// the natural MLIR type. Phase 2.6c-tag-locals (ADR 0063) added
/// the TaggedValue branch.
pub(crate) fn emit_alloca_slot_for_kind<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    kind: ValueKind,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    use melior::ir::{Identifier, attribute::TypeAttribute};
    let (elem, count) = match kind {
        ValueKind::Number => (types.f64, 1i32),
        // Bool: 1-bit. Nil: nominally 1-bit too — the value is
        // never observed (heterogeneous `==` and print(nil) both
        // bypass the slot via static dispatch), but storage is
        // uniform.
        ValueKind::Bool | ValueKind::Nil => (types.i1, 1),
        // Phase 2.7a (ADR 0024): a String slot stores a `ptr` to
        // a static C-string global.
        ValueKind::String => (types.ptr, 1),
        // Phase 2.5b.3 (ADR 0019): Function values stored as
        // opaque pointers — `!func.func<...>` values are bridged
        // via `unrealized_conversion_cast` at store/load.
        ValueKind::Function(_) => (types.ptr, 1),
        // Phase 2.6a-min (ADR 0053): Table slot stores a heap
        // ptr.
        ValueKind::Table => (types.ptr, 1),
        ValueKind::TaggedValue => (types.i64, 2),
    };

    let count_op = arith::constant(
        context,
        IntegerAttribute::new(types.i32, count as i64).into(),
        loc,
    );
    let count_val: Value<'c, 'a> = block.append_operation(count_op).result(0).unwrap().into();

    let alloca = OperationBuilder::new("llvm.alloca", loc)
        .add_operands(&[count_val])
        .add_attributes(&[(
            Identifier::new(context, "elem_type"),
            TypeAttribute::new(elem).into(),
        )])
        .add_results(&[types.ptr])
        .build()
        .expect("llvm.alloca");
    block.append_operation(alloca).result(0).unwrap().into()
}

/// Phase 2.6c-tag-arr (ADR 0059) / 2.6c-tag-hash (ADR 0060):
/// write `{tag=Number, value}` to a 16-byte tagged value slot.
/// Used by both array element stores and hash entry value
/// stores — the slot layout is identical in both contexts.
pub(crate) fn emit_value_slot_store_number<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    value: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let tag_const = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, TAG_NUMBER).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    // tag at offset 0 = slot_ptr itself
    emit_store(block, tag_const, slot_ptr, loc);
    let value_slot =
        emit_byte_offset_ptr(context, block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
    emit_store(block, value, value_slot, loc);
}

/// Phase 2.6c-tag-hash (ADR 0060): write `{tag=Nil, 0.0}` to a
/// tagged value slot. Used for array hole-fill and for hash
/// `t.k = nil` deletion (soft tombstone — the hash key remains).
pub(crate) fn emit_value_slot_store_nil<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let nil_tag = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, TAG_NIL).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    emit_store(block, nil_tag, slot_ptr, loc);
    let zero_f = block
        .append_operation(arith::constant(
            context,
            FloatAttribute::new(context, types.f64, 0.0).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let value_slot =
        emit_byte_offset_ptr(context, block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
    emit_store(block, zero_f, value_slot, loc);
}

/// Phase 2.6c-tag-hetero (ADR 0064): write `{tag=Bool,
/// payload=value zext-to-i64}` to a tagged value slot. The
/// payload occupies the same 8-byte field as Number / String,
/// just interpreted as i64 (with only the low bit meaningful).
pub(crate) fn emit_value_slot_store_bool<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    value: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let tag = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, TAG_BOOL).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    emit_store(block, tag, slot_ptr, loc);
    // i1 → i64 zext via llvm.zext (arith::extui also works).
    let value_i64: Value<'c, 'a> = block
        .append_operation(
            OperationBuilder::new("llvm.zext", loc)
                .add_operands(&[value])
                .add_results(&[types.i64])
                .build()
                .expect("llvm.zext bool→i64"),
        )
        .result(0)
        .unwrap()
        .into();
    let value_slot =
        emit_byte_offset_ptr(context, block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
    emit_store(block, value_i64, value_slot, loc);
}

/// Phase 2.6c-tag-hetero (ADR 0064): write `{tag=String,
/// payload=ptr}` to a tagged value slot. The payload field
/// stores the string global pointer directly.
pub(crate) fn emit_value_slot_store_string<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    value: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let tag = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, TAG_STRING).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    emit_store(block, tag, slot_ptr, loc);
    let value_slot =
        emit_byte_offset_ptr(context, block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
    emit_store(block, value, value_slot, loc);
}

/// Phase 2.6c-tag-fn-tbl (ADR 0071): write `{tag=Function,
/// payload=ptr}` to a tagged value slot. Phase 2.5c-full Commit 2a
/// (ADR 0083) erased Function-kind values to `!llvm.ptr`, retiring
/// the prior `!func.func<...>`→ptr unrealized_cast bridge. Phase
/// 2.5c-full Commit 2b (ADR 0083) further specialises the payload
/// semantics: `value` is now a closure cell ptr (== `addressof
/// @<fn_sym>_closure` for non-capturing functions); consumers
/// dereference it via `closure::emit_load_closure_fn_ptr` before
/// issuing the actual call.
pub(crate) fn emit_value_slot_store_function<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    value: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let tag = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, TAG_FUNCTION).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    emit_store(block, tag, slot_ptr, loc);
    let value_slot =
        emit_byte_offset_ptr(context, block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
    emit_store(block, value, value_slot, loc);
}

/// Phase 2.6c-tag-fn-tbl (ADR 0071): write `{tag=Table,
/// payload=ptr}` to a tagged value slot. Table values are
/// already `!llvm.ptr` (the stable header pointer, ADR 0056) so
/// no cast is needed.
pub(crate) fn emit_value_slot_store_table<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    value: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let tag = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, TAG_TABLE).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    emit_store(block, tag, slot_ptr, loc);
    let value_slot =
        emit_byte_offset_ptr(context, block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
    emit_store(block, value, value_slot, loc);
}

/// Phase 2.6c-tag-hetero (ADR 0064) / 2.6c-tag-fn-tbl (ADR 0071):
/// kind-dispatched store into a tagged value slot. Mirrors the
/// value-side dispatch used by `Table` constructor /
/// `IndexAssign` / `LocalInit` so each caller has a single
/// entry point regardless of element kind.
pub(crate) fn emit_value_slot_store_dispatched<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    value: Value<'c, 'a>,
    kind: ValueKind,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    match kind {
        ValueKind::Number => {
            emit_value_slot_store_number(context, block, slot_ptr, value, types, loc)
        }
        ValueKind::Bool => emit_value_slot_store_bool(context, block, slot_ptr, value, types, loc),
        ValueKind::String => {
            emit_value_slot_store_string(context, block, slot_ptr, value, types, loc)
        }
        ValueKind::Nil => emit_value_slot_store_nil(context, block, slot_ptr, types, loc),
        ValueKind::Function(_) => {
            emit_value_slot_store_function(context, block, slot_ptr, value, types, loc)
        }
        ValueKind::Table => {
            emit_value_slot_store_table(context, block, slot_ptr, value, types, loc)
        }
        _ => unreachable!("unsupported value kind for tagged slot store: {:?}", kind),
    }
}

/// Phase 2.6c-tag-arr (ADR 0059) / 2.6c-tag-hash (ADR 0060):
/// load the tag at offset 0 of a tagged value slot and trap if
/// it isn't `TAG_NUMBER`. After this returns, the caller can
/// safely load the f64 value at offset 8.
pub(crate) fn emit_value_slot_check_number<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let tag = emit_load(block, slot_ptr, types.i64, loc);
    let expected = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, TAG_NUMBER).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let mismatch: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Ne,
            tag,
            expected,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    let msg_ptr = emit_addressof(context, &then_blk, "s_table_type_mismatch", types, loc);
    emit_exit_with_message(context, &then_blk, msg_ptr, types, loc);
    then_blk.append_operation(scf::r#yield(&[], loc));
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    else_blk.append_operation(scf::r#yield(&[], loc));
    else_region.append_block(else_blk);
    block.append_operation(scf::r#if(mismatch, &[], then_region, else_region, loc));
}

/// Phase 2.6c-tag-defensive-trap (ADR 0069): trap when a tagged
/// slot exposes a runtime tag the current implementation does
/// not handle (Function = 4 / Table = 5 reserved). Reuses the
/// existing `s_table_type_mismatch` global so the diagnostic is
/// consistent with the array/hash trap surface (ADR 0059 / 0060).
pub(crate) fn emit_tagged_unknown_tag_trap<'c>(
    context: &'c Context,
    block: &Block<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let msg_ptr = emit_addressof(context, block, "s_table_type_mismatch", types, loc);
    emit_exit_with_message(context, block, msg_ptr, types, loc);
}

/// Phase 2.6c-tag-dispatch-preamble (post-ADR-0074 Tidy First):
/// load the i64 tag at slot+0 and compute the payload pointer at
/// slot+8 for a 16-byte tagged value slot. Used by the consumer
/// dispatchers (`emit_print_tagged_local`,
/// `emit_tagged_eq_local_local`, `emit_tostring_tagged_local`)
/// which all read both the tag and a per-arm payload. The
/// `emit_type_tagged_local` helper only consults the tag and
/// uses [`emit_load`] directly.
///
/// Codex review (post-ADR-0074) confirmed that a fuller
/// callback-based skeleton extraction is blocked by Rust's
/// borrow-checker against melior's eager region-building.
/// Extracting the preamble alone is the safe, behaviour-
/// preserving win.
pub(crate) fn emit_tag_and_payload_ptr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> (Value<'c, 'a>, Value<'c, 'a>) {
    let tag = emit_load(block, slot_ptr, types.i64, loc);
    let payload_ptr =
        emit_byte_offset_ptr(context, block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
    (tag, payload_ptr)
}

/// Phase 2.6c-tag-hetero (ADR 0064): print a `TaggedValue` local
/// by reading its slot's tag at offset 0 and dispatching on the
/// runtime value: Number → `%g`, Bool → `s_true`/`s_false`,
/// String → `%s`, Nil → `s_nil`. Implemented as a chain of
/// nested scf.if so the codegen output is deterministic and the
/// LLVM optimiser can fold any single-tag case if it ever sees
/// one.
pub(crate) fn emit_print_tagged_local<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let (tag, value_ptr) = emit_tag_and_payload_ptr(context, block, slot_ptr, types, loc);
    let tag_number = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, TAG_NUMBER).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let is_number: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            tag,
            tag_number,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let number_then = Region::new();
    let number_then_blk = Block::new(&[]);
    {
        let f = emit_load(&number_then_blk, value_ptr, types.f64, loc);
        let fmt_ptr = emit_addressof(context, &number_then_blk, "fmt_raw", types, loc);
        emit_printf(context, &number_then_blk, fmt_ptr, f, types, loc);
        number_then_blk.append_operation(scf::r#yield(&[], loc));
    }
    number_then.append_block(number_then_blk);
    let number_else = Region::new();
    let number_else_blk = Block::new(&[]);
    {
        // Inner: Bool branch.
        let tag_bool = number_else_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, TAG_BOOL).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let is_bool: Value<'c, '_> = number_else_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                tag,
                tag_bool,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let bool_then = Region::new();
        let bool_then_blk = Block::new(&[]);
        {
            let payload_i64 = emit_load(&bool_then_blk, value_ptr, types.i64, loc);
            let zero_i64 = bool_then_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, 0).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let payload_i1: Value<'c, '_> = bool_then_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Ne,
                    payload_i64,
                    zero_i64,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let true_ptr = emit_addressof(context, &bool_then_blk, "s_true", types, loc);
            let false_ptr = emit_addressof(context, &bool_then_blk, "s_false", types, loc);
            let chosen: Value<'c, '_> = bool_then_blk
                .append_operation(
                    OperationBuilder::new("llvm.select", loc)
                        .add_operands(&[payload_i1, true_ptr, false_ptr])
                        .add_results(&[types.ptr])
                        .build()
                        .expect("llvm.select"),
                )
                .result(0)
                .unwrap()
                .into();
            // ADR 0112: s_true / s_false are boxed string objects.
            emit_print_string_obj(context, &bool_then_blk, chosen, types, loc);
            bool_then_blk.append_operation(scf::r#yield(&[], loc));
        }
        bool_then.append_block(bool_then_blk);
        let bool_else = Region::new();
        let bool_else_blk = Block::new(&[]);
        {
            // Inner-inner: String branch vs Nil fallback.
            let tag_string = bool_else_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, TAG_STRING).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let is_string: Value<'c, '_> = bool_else_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Eq,
                    tag,
                    tag_string,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let string_then = Region::new();
            let string_then_blk = Block::new(&[]);
            {
                // ADR 0112: payload is a boxed string object ptr.
                let payload_ptr = emit_load(&string_then_blk, value_ptr, types.ptr, loc);
                emit_print_string_obj(context, &string_then_blk, payload_ptr, types, loc);
                string_then_blk.append_operation(scf::r#yield(&[], loc));
            }
            string_then.append_block(string_then_blk);
            let string_else = Region::new();
            let string_else_blk = Block::new(&[]);
            {
                // Phase 2.6c-tag-defensive-trap (ADR 0069): the
                // previous arm collapsed "any tag not in
                // {Number, Bool, String}" into "print nil" — fine
                // for TAG_NIL, but silently masks a future
                // Function/Table tag. Distinguish them: if the
                // tag is TAG_NIL, print "nil"; otherwise trap.
                let tag_nil = string_else_blk
                    .append_operation(arith::constant(
                        context,
                        IntegerAttribute::new(types.i64, TAG_NIL).into(),
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let is_nil: Value<'c, '_> = string_else_blk
                    .append_operation(arith::cmpi(
                        context,
                        arith::CmpiPredicate::Eq,
                        tag,
                        tag_nil,
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let nil_then = Region::new();
                let nil_then_blk = Block::new(&[]);
                {
                    let nil_ptr = emit_addressof(context, &nil_then_blk, "s_nil", types, loc);
                    emit_print_string_obj(context, &nil_then_blk, nil_ptr, types, loc);
                    nil_then_blk.append_operation(scf::r#yield(&[], loc));
                }
                nil_then.append_block(nil_then_blk);
                let nil_else = Region::new();
                let nil_else_blk = Block::new(&[]);
                {
                    // Phase 2.6c-tag-fn-tbl (ADR 0071): print
                    // Function / Table values via the literal
                    // typename string. Address-prefixed forms
                    // are out of scope. Truly-unknown tag falls
                    // through to the ADR 0069 trap.
                    let tag_function_p = nil_else_blk
                        .append_operation(arith::constant(
                            context,
                            IntegerAttribute::new(types.i64, TAG_FUNCTION).into(),
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let is_function: Value<'c, '_> = nil_else_blk
                        .append_operation(arith::cmpi(
                            context,
                            arith::CmpiPredicate::Eq,
                            tag,
                            tag_function_p,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let fn_then = Region::new();
                    let fn_then_blk = Block::new(&[]);
                    {
                        let fn_ptr = emit_addressof(
                            context,
                            &fn_then_blk,
                            "s_typename_function",
                            types,
                            loc,
                        );
                        emit_print_string_obj(context, &fn_then_blk, fn_ptr, types, loc);
                        fn_then_blk.append_operation(scf::r#yield(&[], loc));
                    }
                    fn_then.append_block(fn_then_blk);
                    let fn_else = Region::new();
                    let fn_else_blk = Block::new(&[]);
                    {
                        let tag_table_p = fn_else_blk
                            .append_operation(arith::constant(
                                context,
                                IntegerAttribute::new(types.i64, TAG_TABLE).into(),
                                loc,
                            ))
                            .result(0)
                            .unwrap()
                            .into();
                        let is_table: Value<'c, '_> = fn_else_blk
                            .append_operation(arith::cmpi(
                                context,
                                arith::CmpiPredicate::Eq,
                                tag,
                                tag_table_p,
                                loc,
                            ))
                            .result(0)
                            .unwrap()
                            .into();
                        let tbl_then = Region::new();
                        let tbl_then_blk = Block::new(&[]);
                        {
                            let tbl_ptr = emit_addressof(
                                context,
                                &tbl_then_blk,
                                "s_typename_table",
                                types,
                                loc,
                            );
                            emit_print_string_obj(context, &tbl_then_blk, tbl_ptr, types, loc);
                            tbl_then_blk.append_operation(scf::r#yield(&[], loc));
                        }
                        tbl_then.append_block(tbl_then_blk);
                        let tbl_else = Region::new();
                        let tbl_else_blk = Block::new(&[]);
                        emit_tagged_unknown_tag_trap(context, &tbl_else_blk, types, loc);
                        tbl_else_blk.append_operation(scf::r#yield(&[], loc));
                        tbl_else.append_block(tbl_else_blk);
                        fn_else_blk.append_operation(scf::r#if(
                            is_table,
                            &[],
                            tbl_then,
                            tbl_else,
                            loc,
                        ));
                        fn_else_blk.append_operation(scf::r#yield(&[], loc));
                    }
                    fn_else.append_block(fn_else_blk);
                    nil_else_blk.append_operation(scf::r#if(
                        is_function,
                        &[],
                        fn_then,
                        fn_else,
                        loc,
                    ));
                    nil_else_blk.append_operation(scf::r#yield(&[], loc));
                }
                nil_else.append_block(nil_else_blk);
                string_else_blk.append_operation(scf::r#if(is_nil, &[], nil_then, nil_else, loc));
                string_else_blk.append_operation(scf::r#yield(&[], loc));
            }
            string_else.append_block(string_else_blk);
            bool_else_blk.append_operation(scf::r#if(
                is_string,
                &[],
                string_then,
                string_else,
                loc,
            ));
            bool_else_blk.append_operation(scf::r#yield(&[], loc));
        }
        bool_else.append_block(bool_else_blk);
        number_else_blk.append_operation(scf::r#if(is_bool, &[], bool_then, bool_else, loc));
        number_else_blk.append_operation(scf::r#yield(&[], loc));
    }
    number_else.append_block(number_else_blk);
    block.append_operation(scf::r#if(is_number, &[], number_then, number_else, loc));
}

/// Phase 2.6c-tag-hetero-eq (ADR 0066): `Local(TaggedValue) ==
/// Local(TaggedValue)` runtime dispatch. Loads both slot tags
/// at offset 0 and: if the tags differ, yields `false` (Lua:
/// heterogeneous compare). If the tags match, switches on the
/// shared tag and compares the 8-byte payloads with the kind-
/// appropriate predicate — cmpf for Number, cmpi i64 for Bool's
/// zext payload, strcmp for String, and `true` for both Nil.
/// Function / Table tags fall through to a defensive `false`,
/// reserving room for the follow-up sub-phase that introduces
/// those payloads.
pub(crate) fn emit_tagged_eq_local_local<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_lhs: Value<'c, 'a>,
    slot_rhs: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let (lhs_tag, lhs_payload) = emit_tag_and_payload_ptr(context, block, slot_lhs, types, loc);
    let (rhs_tag, rhs_payload) = emit_tag_and_payload_ptr(context, block, slot_rhs, types, loc);
    let tag_eq: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            lhs_tag,
            rhs_tag,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    // Outer scf.if tag_eq → kind-specific compare, else → false.
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    {
        // Nested chain: TAG_NIL → true; TAG_NUMBER → cmpf;
        // TAG_BOOL → cmpi i64 (zext payload); TAG_STRING →
        // strcmp == 0; else → false (Function/Table reserved).
        let make_const_i64 = |b: &Block<'c>, v: i64| -> Value<'c, '_> {
            b.append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, v).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into()
        };
        let tag_nil = make_const_i64(&then_blk, TAG_NIL);
        let is_nil: Value<'c, '_> = then_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                lhs_tag,
                tag_nil,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let nil_then = Region::new();
        let nil_then_blk = Block::new(&[]);
        let i1_true = nil_then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i1, 1).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        nil_then_blk.append_operation(scf::r#yield(&[i1_true], loc));
        nil_then.append_block(nil_then_blk);
        let nil_else = Region::new();
        let nil_else_blk = Block::new(&[]);
        {
            let tag_number = make_const_i64(&nil_else_blk, TAG_NUMBER);
            let is_number: Value<'c, '_> = nil_else_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Eq,
                    lhs_tag,
                    tag_number,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let num_then = Region::new();
            let num_then_blk = Block::new(&[]);
            {
                let l = emit_load(&num_then_blk, lhs_payload, types.f64, loc);
                let r = emit_load(&num_then_blk, rhs_payload, types.f64, loc);
                let eq: Value<'c, '_> = num_then_blk
                    .append_operation(arith::cmpf(context, arith::CmpfPredicate::Oeq, l, r, loc))
                    .result(0)
                    .unwrap()
                    .into();
                num_then_blk.append_operation(scf::r#yield(&[eq], loc));
            }
            num_then.append_block(num_then_blk);
            let num_else = Region::new();
            let num_else_blk = Block::new(&[]);
            {
                let tag_bool = make_const_i64(&num_else_blk, TAG_BOOL);
                let is_bool: Value<'c, '_> = num_else_blk
                    .append_operation(arith::cmpi(
                        context,
                        arith::CmpiPredicate::Eq,
                        lhs_tag,
                        tag_bool,
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let bool_then = Region::new();
                let bool_then_blk = Block::new(&[]);
                {
                    let l = emit_load(&bool_then_blk, lhs_payload, types.i64, loc);
                    let r = emit_load(&bool_then_blk, rhs_payload, types.i64, loc);
                    let eq: Value<'c, '_> = bool_then_blk
                        .append_operation(arith::cmpi(context, arith::CmpiPredicate::Eq, l, r, loc))
                        .result(0)
                        .unwrap()
                        .into();
                    bool_then_blk.append_operation(scf::r#yield(&[eq], loc));
                }
                bool_then.append_block(bool_then_blk);
                let bool_else = Region::new();
                let bool_else_blk = Block::new(&[]);
                {
                    let tag_string = make_const_i64(&bool_else_blk, TAG_STRING);
                    let is_string: Value<'c, '_> = bool_else_blk
                        .append_operation(arith::cmpi(
                            context,
                            arith::CmpiPredicate::Eq,
                            lhs_tag,
                            tag_string,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let str_then = Region::new();
                    let str_then_blk = Block::new(&[]);
                    {
                        // ADR 0112: payloads are boxed string object
                        // ptrs. emit_string_obj_eq is length+memcmp-
                        // based (no NUL truncation false positives).
                        let lp = emit_load(&str_then_blk, lhs_payload, types.ptr, loc);
                        let rp = emit_load(&str_then_blk, rhs_payload, types.ptr, loc);
                        let eq = emit_string_obj_eq(context, &str_then_blk, lp, rp, types, loc);
                        str_then_blk.append_operation(scf::r#yield(&[eq], loc));
                    }
                    str_then.append_block(str_then_blk);
                    let str_else = Region::new();
                    let str_else_blk = Block::new(&[]);
                    {
                        // Phase 2.6c-tag-fn-tbl (ADR 0071):
                        // Function / Table reference equality
                        // (Lua spec: same object → true). Both
                        // sides have the same tag here (the
                        // outer tag_eq check fired), so a payload
                        // ptr comparison suffices. Phase 2.5c-full
                        // Commit 2b (ADR 0083): for TAG_FUNCTION
                        // the payload is now a closure cell ptr;
                        // non-capturing singletons (1 fn = 1
                        // global) preserve identity equality. Truly
                        // unknown tag stays as the ADR 0069 trap.
                        let tag_function = make_const_i64(&str_else_blk, TAG_FUNCTION);
                        let is_function: Value<'c, '_> = str_else_blk
                            .append_operation(arith::cmpi(
                                context,
                                arith::CmpiPredicate::Eq,
                                lhs_tag,
                                tag_function,
                                loc,
                            ))
                            .result(0)
                            .unwrap()
                            .into();
                        let fn_then = Region::new();
                        let fn_then_blk = Block::new(&[]);
                        {
                            let lp = emit_load(&fn_then_blk, lhs_payload, types.ptr, loc);
                            let rp = emit_load(&fn_then_blk, rhs_payload, types.ptr, loc);
                            let lp_i = fn_then_blk
                                .append_operation(
                                    OperationBuilder::new("llvm.ptrtoint", loc)
                                        .add_operands(&[lp])
                                        .add_results(&[types.i64])
                                        .build()
                                        .expect("llvm.ptrtoint"),
                                )
                                .result(0)
                                .unwrap()
                                .into();
                            let rp_i = fn_then_blk
                                .append_operation(
                                    OperationBuilder::new("llvm.ptrtoint", loc)
                                        .add_operands(&[rp])
                                        .add_results(&[types.i64])
                                        .build()
                                        .expect("llvm.ptrtoint"),
                                )
                                .result(0)
                                .unwrap()
                                .into();
                            let eq: Value<'c, '_> = fn_then_blk
                                .append_operation(arith::cmpi(
                                    context,
                                    arith::CmpiPredicate::Eq,
                                    lp_i,
                                    rp_i,
                                    loc,
                                ))
                                .result(0)
                                .unwrap()
                                .into();
                            fn_then_blk.append_operation(scf::r#yield(&[eq], loc));
                        }
                        fn_then.append_block(fn_then_blk);
                        let fn_else = Region::new();
                        let fn_else_blk = Block::new(&[]);
                        {
                            let tag_table = make_const_i64(&fn_else_blk, TAG_TABLE);
                            let is_table: Value<'c, '_> = fn_else_blk
                                .append_operation(arith::cmpi(
                                    context,
                                    arith::CmpiPredicate::Eq,
                                    lhs_tag,
                                    tag_table,
                                    loc,
                                ))
                                .result(0)
                                .unwrap()
                                .into();
                            let tbl_then = Region::new();
                            let tbl_then_blk = Block::new(&[]);
                            {
                                let lp = emit_load(&tbl_then_blk, lhs_payload, types.ptr, loc);
                                let rp = emit_load(&tbl_then_blk, rhs_payload, types.ptr, loc);
                                let lp_i = tbl_then_blk
                                    .append_operation(
                                        OperationBuilder::new("llvm.ptrtoint", loc)
                                            .add_operands(&[lp])
                                            .add_results(&[types.i64])
                                            .build()
                                            .expect("llvm.ptrtoint"),
                                    )
                                    .result(0)
                                    .unwrap()
                                    .into();
                                let rp_i = tbl_then_blk
                                    .append_operation(
                                        OperationBuilder::new("llvm.ptrtoint", loc)
                                            .add_operands(&[rp])
                                            .add_results(&[types.i64])
                                            .build()
                                            .expect("llvm.ptrtoint"),
                                    )
                                    .result(0)
                                    .unwrap()
                                    .into();
                                let eq: Value<'c, '_> = tbl_then_blk
                                    .append_operation(arith::cmpi(
                                        context,
                                        arith::CmpiPredicate::Eq,
                                        lp_i,
                                        rp_i,
                                        loc,
                                    ))
                                    .result(0)
                                    .unwrap()
                                    .into();
                                tbl_then_blk.append_operation(scf::r#yield(&[eq], loc));
                            }
                            tbl_then.append_block(tbl_then_blk);
                            let tbl_else = Region::new();
                            let tbl_else_blk = Block::new(&[]);
                            emit_tagged_unknown_tag_trap(context, &tbl_else_blk, types, loc);
                            let placeholder = tbl_else_blk
                                .append_operation(arith::constant(
                                    context,
                                    IntegerAttribute::new(types.i1, 0).into(),
                                    loc,
                                ))
                                .result(0)
                                .unwrap()
                                .into();
                            tbl_else_blk.append_operation(scf::r#yield(&[placeholder], loc));
                            tbl_else.append_block(tbl_else_blk);
                            let tbl_op = scf::r#if(is_table, &[types.i1], tbl_then, tbl_else, loc);
                            let tbl_result: Value<'c, '_> = fn_else_blk
                                .append_operation(tbl_op)
                                .result(0)
                                .unwrap()
                                .into();
                            fn_else_blk.append_operation(scf::r#yield(&[tbl_result], loc));
                        }
                        fn_else.append_block(fn_else_blk);
                        let fn_op = scf::r#if(is_function, &[types.i1], fn_then, fn_else, loc);
                        let fn_result: Value<'c, '_> = str_else_blk
                            .append_operation(fn_op)
                            .result(0)
                            .unwrap()
                            .into();
                        str_else_blk.append_operation(scf::r#yield(&[fn_result], loc));
                    }
                    str_else.append_block(str_else_blk);
                    let str_op = scf::r#if(is_string, &[types.i1], str_then, str_else, loc);
                    let str_result: Value<'c, '_> = bool_else_blk
                        .append_operation(str_op)
                        .result(0)
                        .unwrap()
                        .into();
                    bool_else_blk.append_operation(scf::r#yield(&[str_result], loc));
                }
                bool_else.append_block(bool_else_blk);
                let bool_op = scf::r#if(is_bool, &[types.i1], bool_then, bool_else, loc);
                let bool_result: Value<'c, '_> = num_else_blk
                    .append_operation(bool_op)
                    .result(0)
                    .unwrap()
                    .into();
                num_else_blk.append_operation(scf::r#yield(&[bool_result], loc));
            }
            num_else.append_block(num_else_blk);
            let num_op = scf::r#if(is_number, &[types.i1], num_then, num_else, loc);
            let num_result: Value<'c, '_> = nil_else_blk
                .append_operation(num_op)
                .result(0)
                .unwrap()
                .into();
            nil_else_blk.append_operation(scf::r#yield(&[num_result], loc));
        }
        nil_else.append_block(nil_else_blk);
        let nil_op = scf::r#if(is_nil, &[types.i1], nil_then, nil_else, loc);
        let nil_result: Value<'c, '_> = then_blk.append_operation(nil_op).result(0).unwrap().into();
        then_blk.append_operation(scf::r#yield(&[nil_result], loc));
    }
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    {
        let i1_false = else_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i1, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        else_blk.append_operation(scf::r#yield(&[i1_false], loc));
    }
    else_region.append_block(else_blk);
    let if_op = scf::r#if(tag_eq, &[types.i1], then_region, else_region, loc);
    block.append_operation(if_op).result(0).unwrap().into()
}

/// Phase 2.6c-tag-consumers (ADR 0067): runtime tag dispatch for
/// `type(Local(TaggedValue))`. Reads the slot's tag at offset 0
/// and yields a pointer to one of the existing `s_typename_*`
/// global strings. Function/Table tags fall through to the
/// `s_typename_number` default — those payloads aren't yet
/// stored in tagged slots (LIC-2.6c-tag-hetero-fn-tbl-1) so the
/// fallback is unreachable in current programs.
pub(crate) fn emit_type_tagged_local<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let tag = emit_load(block, slot_ptr, types.i64, loc);
    // Build the tag-vs-tag chain: Nil → "nil"; Number → "number";
    // Bool → "boolean"; String → "string"; else → "number"
    // (defensive). Every arm yields a `ptr` into the static
    // typename pool.
    let make_const_i64 = |b: &Block<'c>, v: i64| -> Value<'c, '_> {
        b.append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, v).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into()
    };
    let tag_nil = make_const_i64(block, TAG_NIL);
    let is_nil: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            tag,
            tag_nil,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let nil_then = Region::new();
    let nil_then_blk = Block::new(&[]);
    let nil_str = emit_addressof(context, &nil_then_blk, "s_typename_nil", types, loc);
    nil_then_blk.append_operation(scf::r#yield(&[nil_str], loc));
    nil_then.append_block(nil_then_blk);
    let nil_else = Region::new();
    let nil_else_blk = Block::new(&[]);
    {
        let tag_number = make_const_i64(&nil_else_blk, TAG_NUMBER);
        let is_number: Value<'c, '_> = nil_else_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                tag,
                tag_number,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let num_then = Region::new();
        let num_then_blk = Block::new(&[]);
        let num_str = emit_addressof(context, &num_then_blk, "s_typename_number", types, loc);
        num_then_blk.append_operation(scf::r#yield(&[num_str], loc));
        num_then.append_block(num_then_blk);
        let num_else = Region::new();
        let num_else_blk = Block::new(&[]);
        {
            let tag_bool = make_const_i64(&num_else_blk, TAG_BOOL);
            let is_bool: Value<'c, '_> = num_else_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Eq,
                    tag,
                    tag_bool,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let bool_then = Region::new();
            let bool_then_blk = Block::new(&[]);
            let bool_str =
                emit_addressof(context, &bool_then_blk, "s_typename_boolean", types, loc);
            bool_then_blk.append_operation(scf::r#yield(&[bool_str], loc));
            bool_then.append_block(bool_then_blk);
            let bool_else = Region::new();
            let bool_else_blk = Block::new(&[]);
            {
                let tag_string = make_const_i64(&bool_else_blk, TAG_STRING);
                let is_string: Value<'c, '_> = bool_else_blk
                    .append_operation(arith::cmpi(
                        context,
                        arith::CmpiPredicate::Eq,
                        tag,
                        tag_string,
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let str_then = Region::new();
                let str_then_blk = Block::new(&[]);
                let str_str =
                    emit_addressof(context, &str_then_blk, "s_typename_string", types, loc);
                str_then_blk.append_operation(scf::r#yield(&[str_str], loc));
                str_then.append_block(str_then_blk);
                let str_else = Region::new();
                let str_else_blk = Block::new(&[]);
                {
                    // Phase 2.6c-tag-fn-tbl (ADR 0071): nested
                    // dispatch for Function / Table tags. The
                    // ADR 0069 trap stays as the truly-unknown
                    // arm, kept under the Table check.
                    let tag_function = make_const_i64(&str_else_blk, TAG_FUNCTION);
                    let is_function: Value<'c, '_> = str_else_blk
                        .append_operation(arith::cmpi(
                            context,
                            arith::CmpiPredicate::Eq,
                            tag,
                            tag_function,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let fn_then = Region::new();
                    let fn_then_blk = Block::new(&[]);
                    let fn_str =
                        emit_addressof(context, &fn_then_blk, "s_typename_function", types, loc);
                    fn_then_blk.append_operation(scf::r#yield(&[fn_str], loc));
                    fn_then.append_block(fn_then_blk);
                    let fn_else = Region::new();
                    let fn_else_blk = Block::new(&[]);
                    {
                        let tag_table = make_const_i64(&fn_else_blk, TAG_TABLE);
                        let is_table: Value<'c, '_> = fn_else_blk
                            .append_operation(arith::cmpi(
                                context,
                                arith::CmpiPredicate::Eq,
                                tag,
                                tag_table,
                                loc,
                            ))
                            .result(0)
                            .unwrap()
                            .into();
                        let tbl_then = Region::new();
                        let tbl_then_blk = Block::new(&[]);
                        let tbl_str =
                            emit_addressof(context, &tbl_then_blk, "s_typename_table", types, loc);
                        tbl_then_blk.append_operation(scf::r#yield(&[tbl_str], loc));
                        tbl_then.append_block(tbl_then_blk);
                        let tbl_else = Region::new();
                        let tbl_else_blk = Block::new(&[]);
                        emit_tagged_unknown_tag_trap(context, &tbl_else_blk, types, loc);
                        let placeholder =
                            emit_addressof(context, &tbl_else_blk, "s_typename_number", types, loc);
                        tbl_else_blk.append_operation(scf::r#yield(&[placeholder], loc));
                        tbl_else.append_block(tbl_else_blk);
                        let tbl_op = scf::r#if(is_table, &[types.ptr], tbl_then, tbl_else, loc);
                        let tbl_result: Value<'c, '_> = fn_else_blk
                            .append_operation(tbl_op)
                            .result(0)
                            .unwrap()
                            .into();
                        fn_else_blk.append_operation(scf::r#yield(&[tbl_result], loc));
                    }
                    fn_else.append_block(fn_else_blk);
                    let fn_op = scf::r#if(is_function, &[types.ptr], fn_then, fn_else, loc);
                    let fn_result: Value<'c, '_> = str_else_blk
                        .append_operation(fn_op)
                        .result(0)
                        .unwrap()
                        .into();
                    str_else_blk.append_operation(scf::r#yield(&[fn_result], loc));
                }
                str_else.append_block(str_else_blk);
                let str_op = scf::r#if(is_string, &[types.ptr], str_then, str_else, loc);
                let str_result: Value<'c, '_> = bool_else_blk
                    .append_operation(str_op)
                    .result(0)
                    .unwrap()
                    .into();
                bool_else_blk.append_operation(scf::r#yield(&[str_result], loc));
            }
            bool_else.append_block(bool_else_blk);
            let bool_op = scf::r#if(is_bool, &[types.ptr], bool_then, bool_else, loc);
            let bool_result: Value<'c, '_> = num_else_blk
                .append_operation(bool_op)
                .result(0)
                .unwrap()
                .into();
            num_else_blk.append_operation(scf::r#yield(&[bool_result], loc));
        }
        num_else.append_block(num_else_blk);
        let num_op = scf::r#if(is_number, &[types.ptr], num_then, num_else, loc);
        let num_result: Value<'c, '_> = nil_else_blk
            .append_operation(num_op)
            .result(0)
            .unwrap()
            .into();
        nil_else_blk.append_operation(scf::r#yield(&[num_result], loc));
    }
    nil_else.append_block(nil_else_blk);
    let if_op = scf::r#if(is_nil, &[types.ptr], nil_then, nil_else, loc);
    block.append_operation(if_op).result(0).unwrap().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_for_tag_nil_returns_trap_nil() {
        assert_eq!(
            policy_for_tag(TAG_NIL),
            &[HashKeyValidityPolicy::TrapNil][..],
        );
    }

    #[test]
    fn policy_for_tag_number_returns_check_nan() {
        assert_eq!(
            policy_for_tag(TAG_NUMBER),
            &[HashKeyValidityPolicy::CheckNaN][..],
        );
    }

    #[test]
    fn policy_for_tag_other_tags_pass_through() {
        for tag in [TAG_BOOL, TAG_STRING, TAG_FUNCTION, TAG_TABLE, TAG_DELETED] {
            assert_eq!(
                policy_for_tag(tag),
                &[][..],
                "tag {tag} should have no policy"
            );
        }
    }

    #[test]
    fn policy_for_tagged_arith_operand_number_returns_use_payload() {
        assert_eq!(
            policy_for_tagged_arith_operand(TAG_NUMBER),
            TaggedArithOperandPlan::UseNumberPayload,
        );
    }

    #[test]
    fn policy_for_tagged_arith_operand_string_returns_coerce() {
        assert_eq!(
            policy_for_tagged_arith_operand(TAG_STRING),
            TaggedArithOperandPlan::CoerceStringToNumber,
        );
    }

    #[test]
    fn policy_for_tagged_arith_operand_other_traps() {
        for tag in [TAG_NIL, TAG_BOOL, TAG_FUNCTION, TAG_TABLE, TAG_DELETED] {
            assert_eq!(
                policy_for_tagged_arith_operand(tag),
                TaggedArithOperandPlan::TrapNonNumeric,
                "tag {tag} should trap as non-numeric"
            );
        }
    }
}
