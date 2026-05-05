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
    Types, emit_addressof, emit_byte_offset_ptr, emit_exit_with_message, emit_load, emit_store,
    emit_unrealized_cast,
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
// Phase 2.6c-tag-fn-tbl (ADR 0071): Function (closure-less) and
// Table values now occupy tags 4 and 5. Closures with upvalues
// remain HIR-rejected (LIC-2.6c-tag-hetero-closure-escape-1).
pub(crate) const TAG_FUNCTION: i64 = 4;
pub(crate) const TAG_TABLE: i64 = 5;

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
/// payload=ptr}` to a tagged value slot. The function value
/// arrives as `!func.func<...>` from `emit_expr`; bridge it to
/// `!llvm.ptr` via `emit_unrealized_cast` (ADR 0019) before the
/// 8-byte store.
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
    let value_ptr = emit_unrealized_cast(block, value, types.ptr, loc);
    let value_slot =
        emit_byte_offset_ptr(context, block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
    emit_store(block, value_ptr, value_slot, loc);
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

/// Phase 2.6c-tag-fn-tbl-call (ADR 0072): assert a tagged-value
/// slot's tag is `TAG_FUNCTION`, exiting with the standard
/// type-mismatch diagnostic otherwise. Mirrors
/// [`emit_value_slot_check_number`] (ADR 0063) for the function-
/// callee path used when invoking a function value retrieved
/// through a tagged slot (`local g = t[k]; g()`).
pub(crate) fn emit_value_slot_check_function<'a, 'c>(
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
            IntegerAttribute::new(types.i64, TAG_FUNCTION).into(),
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
