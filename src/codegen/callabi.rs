//! Function call ABI helpers — kind-vector ↔ MLIR result-layout
//! conversions used by both direct `Callee::User` and indirect
//! `Callee::IndirectDispatch` paths (ADR 0082).
//!
//! The split (Phase 2.5x-callee-dispatch, ADR 0082 Tidy First) is
//! responsibility-driven: `tagged.rs` owns the slot-level
//! `{i64 tag, i64 payload}` layout; this module owns the **return
//! ABI flattening** — a `Vec<ValueKind>` of logical return positions
//! becomes a flat `Vec<Type>` of MLIR result types, and an index
//! walker maps logical positions to MLIR result indices. The TaggedValue
//! kind is the only one that contributes >1 MLIR result per position
//! today (ADR 0076: `(tag: i64, payload_raw: i64)`).
//!
//! These helpers were originally inline in `emit.rs`. Moving them
//! here is preparation for ADR 0082's per-call-site dispatch chain,
//! which calls them from a new emission helper that doesn't fit
//! `emit.rs`'s existing call-arm shape.

use melior::{
    Context,
    ir::{Type, r#type::FunctionType},
};

use crate::hir::ValueKind;

use super::primitive::Types;

/// MLIR result types for a function whose HIR return-kinds list is
/// `ret_kinds`. Phase 2.5b.3 (ADR 0019) added Function returns; 2.5e
/// (ADR 0020) added Bool and Nil; 2.5d (ADR 0021) generalises to a
/// `Vec` so multi-result functions emit a result type per position;
/// 2.6c-tag-locals-fn (ADR 0074) makes TaggedValue contribute 2 MLIR
/// results (tag + payload_raw).
pub(crate) fn ret_mlir_types<'c>(
    context: &'c Context,
    ret_kinds: &[ValueKind],
    types: &Types<'c>,
) -> Vec<Type<'c>> {
    ret_kinds
        .iter()
        .flat_map(|k| match k {
            ValueKind::Number => vec![types.f64],
            ValueKind::Bool | ValueKind::Nil => vec![types.i1],
            ValueKind::Function(arity) => {
                let p_types: Vec<Type<'c>> = (0..*arity).map(|_| types.f64).collect();
                vec![FunctionType::new(context, &p_types, &[types.f64]).into()]
            }
            ValueKind::String | ValueKind::Table => vec![types.ptr],
            // Phase 2.6c-tag-locals-fn (ADR 0074): a TaggedValue
            // return becomes 2 MLIR i64 results: (tag, payload_raw).
            // The caller reassembles them into a 16-byte tagged
            // slot. payload_raw is i64 because slot copies (ADR
            // 0064) already round-trip any payload kind through
            // raw i64.
            ValueKind::TaggedValue => vec![types.i64, types.i64],
        })
        .collect()
}

/// Phase 2.6c-tag-locals-fn-multi (ADR 0076): MLIR `func.call`
/// result count produced by a single logical return position of
/// the given kind. Mirrors [`ret_mlir_types`]'s `flat_map`
/// expansion: TaggedValue is 2 (tag + payload_raw), every other
/// supported return kind is 1.
pub(crate) fn ret_kind_result_width(kind: ValueKind) -> usize {
    match kind {
        ValueKind::TaggedValue => 2,
        _ => 1,
    }
}

/// Phase 2.6c-tag-locals-fn-multi (ADR 0076): flat MLIR result
/// index where logical return position `pos` starts. For
/// `ret_kinds = [Number, TaggedValue, Bool]`, position 0 starts
/// at MLIR result 0, position 1 at MLIR result 1, position 2 at
/// MLIR result 3 (because the TaggedValue at position 1 took 2
/// MLIR results).
pub(crate) fn flat_result_index(ret_kinds: &[ValueKind], pos: usize) -> usize {
    ret_kinds[..pos]
        .iter()
        .map(|k| ret_kind_result_width(*k))
        .sum()
}
