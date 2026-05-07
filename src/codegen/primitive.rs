//! Low-level MLIR / LLVM-dialect helpers shared across `codegen`.
//!
//! Phase 2.6c-tag-rs-split (ADR 0073) extracted the pure-MLIR
//! primitive surface from `emit.rs` so that both `emit.rs` and
//! `tagged.rs` can depend on the same low-level layer without
//! `pub(crate)` cycles. Nothing here knows about Lua semantics —
//! every function is a thin wrapper over a single MLIR / LLVM
//! dialect operation (load / store / GEP / unrealized cast /
//! addressof / printf / exit) plus the shared [`Types`] struct
//! holding the MLIR types reused across the codegen pass.

use melior::{
    Context,
    dialect::{arith, llvm, ods::llvm::AddressOfOperationBuilder},
    ir::{
        Block, BlockLike, Identifier, Location, Type, Value,
        attribute::{
            DenseI32ArrayAttribute, FlatSymbolRefAttribute, IntegerAttribute, TypeAttribute,
        },
        operation::OperationBuilder,
    },
};

/// Cached MLIR types reused across the codegen pass.
///
/// Phase 2.7a (ADR 0024): `string_pool` maps a Lua string literal
/// payload to the MLIR global symbol name carrying it. Built by
/// `collect_string_pool` in `emit.rs` before any `emit_*` runs and
/// read at every `HirExprKind::Str` use site.
pub(crate) struct Types<'c> {
    pub(crate) i1: Type<'c>,
    /// Phase 2.6a-arr (ADR 0054): i8 used as the GEP element type
    /// for byte-offset pointer arithmetic on table heap regions.
    pub(crate) i8: Type<'c>,
    pub(crate) i32: Type<'c>,
    pub(crate) i64: Type<'c>,
    pub(crate) f64: Type<'c>,
    pub(crate) ptr: Type<'c>,
    pub(crate) string_pool: std::collections::HashMap<String, String>,
}

/// `llvm.store value, slot`. Type-erased through the operand types.
pub(crate) fn emit_store<'a, 'c>(
    block: &'a Block<'c>,
    value: Value<'c, 'a>,
    slot: Value<'c, 'a>,
    loc: Location<'c>,
) {
    let store = OperationBuilder::new("llvm.store", loc)
        .add_operands(&[value, slot])
        .build()
        .expect("llvm.store");
    block.append_operation(store);
}

/// `llvm.load slot : <result_ty>`. `result_ty` (the second arg
/// historically named `f64_type`) names the destination MLIR type.
pub(crate) fn emit_load<'a, 'c>(
    block: &'a Block<'c>,
    slot: Value<'c, 'a>,
    f64_type: Type<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let load = OperationBuilder::new("llvm.load", loc)
        .add_operands(&[slot])
        .add_results(&[f64_type])
        .build()
        .expect("llvm.load");
    block.append_operation(load).result(0).unwrap().into()
}

/// `llvm.getelementptr` over an `i8` element type — interpret
/// `offset_bytes` as a raw byte offset from `base`. Phase 2.6a-arr
/// (ADR 0054).
pub(crate) fn emit_byte_offset_ptr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    base: Value<'c, 'a>,
    offset_bytes: i64,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let offset_const = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, offset_bytes).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    emit_byte_offset_ptr_dynamic(context, block, base, offset_const, types, loc)
}

/// Same as [`emit_byte_offset_ptr`] but the offset is a runtime
/// `i64` value (used for variable-key indexing). Phase 2.6a-arr
/// (ADR 0054).
pub(crate) fn emit_byte_offset_ptr_dynamic<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    base: Value<'c, 'a>,
    offset_i64: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let gep = OperationBuilder::new("llvm.getelementptr", loc)
        .add_operands(&[base, offset_i64])
        .add_attributes(&[
            (
                Identifier::new(context, "rawConstantIndices"),
                DenseI32ArrayAttribute::new(context, &[i32::MIN]).into(),
            ),
            (
                Identifier::new(context, "elem_type"),
                TypeAttribute::new(types.i8).into(),
            ),
        ])
        .add_results(&[types.ptr])
        .build()
        .expect("llvm.getelementptr (i8 byte offset)");
    block.append_operation(gep).result(0).unwrap().into()
}

/// `llvm.mlir.addressof @<global_name> : !llvm.ptr` — load the
/// address of a module-level global string / function symbol.
pub(crate) fn emit_addressof<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    global_name: &str,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let addr_op = AddressOfOperationBuilder::new(context, loc)
        .res(types.ptr)
        .global_name(FlatSymbolRefAttribute::new(context, global_name))
        .build();
    block
        .append_operation(addr_op.into())
        .result(0)
        .unwrap()
        .into()
}

/// `printf(fmt_ptr, value)` — variadic libc printf with a single
/// payload argument. The variadic descriptor declares `(ptr) →
/// i32` plus `...`, matching the `extern "C" int printf(const char
/// *, ...)` signature.
pub(crate) fn emit_printf<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    fmt_ptr: Value<'c, 'a>,
    value: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let call_op = OperationBuilder::new("llvm.call", loc)
        .add_operands(&[fmt_ptr, value])
        .add_attributes(&[
            (
                Identifier::new(context, "callee"),
                FlatSymbolRefAttribute::new(context, "printf").into(),
            ),
            (
                Identifier::new(context, "operandSegmentSizes"),
                DenseI32ArrayAttribute::new(context, &[2, 0]).into(),
            ),
            (
                Identifier::new(context, "op_bundle_sizes"),
                DenseI32ArrayAttribute::new(context, &[]).into(),
            ),
            (
                Identifier::new(context, "var_callee_type"),
                TypeAttribute::new(llvm::r#type::function(types.i32, &[types.ptr], true)).into(),
            ),
        ])
        .add_results(&[types.i32])
        .build()
        .expect("llvm.call @printf");
    block.append_operation(call_op);
}

/// Phase 2.7h (ADR 0033, Tidy First): emit `printf("%s\n", msg_ptr)`
/// followed by `exit(1)` into `block`. Pure-by-construction — only
/// depends on its arguments — so it composes cleanly into both the
/// conditional `assert` failure path and the unconditional
/// `error(msg)` builtin.
pub(crate) fn emit_exit_with_message<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    msg_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let fmt_ptr = emit_addressof(context, block, "fmt_str", types, loc);
    emit_printf(context, block, fmt_ptr, msg_ptr, types, loc);
    let one_i32 = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i32, 1).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let exit_call = OperationBuilder::new("llvm.call", loc)
        .add_operands(&[one_i32])
        .add_attributes(&[
            (
                Identifier::new(context, "callee"),
                FlatSymbolRefAttribute::new(context, "exit").into(),
            ),
            (
                Identifier::new(context, "operandSegmentSizes"),
                DenseI32ArrayAttribute::new(context, &[1, 0]).into(),
            ),
            (
                Identifier::new(context, "op_bundle_sizes"),
                DenseI32ArrayAttribute::new(context, &[]).into(),
            ),
        ])
        .build()
        .expect("llvm.call @exit");
    block.append_operation(exit_call);
}

/// Generic single-result libc helper — `i64` return type.
pub(crate) fn emit_libc_call_i64<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    name: &str,
    args: &[Value<'c, 'a>],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    emit_libc_call_with_result(context, block, name, args, types.i64, loc)
}

/// Generic single-result libc helper — `i32` return type.
pub(crate) fn emit_libc_call_i32<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    name: &str,
    args: &[Value<'c, 'a>],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    emit_libc_call_with_result(context, block, name, args, types.i32, loc)
}

/// Generic single-result libc helper — `ptr` return type.
pub(crate) fn emit_libc_call_ptr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    name: &str,
    args: &[Value<'c, 'a>],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    emit_libc_call_with_result(context, block, name, args, types.ptr, loc)
}

/// Phase 2.6a-grow (ADR 0057): void-returning libc helper. Used for
/// `free(ptr)`. Mirrors `emit_libc_call_with_result` but skips
/// `add_results` since the callee returns nothing.
pub(crate) fn emit_libc_call_void<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    name: &str,
    args: &[Value<'c, 'a>],
    loc: Location<'c>,
) {
    let n = i32::try_from(args.len()).expect("call arity fits in i32");
    let call_op = OperationBuilder::new("llvm.call", loc)
        .add_operands(args)
        .add_attributes(&[
            (
                Identifier::new(context, "callee"),
                FlatSymbolRefAttribute::new(context, name).into(),
            ),
            (
                Identifier::new(context, "operandSegmentSizes"),
                DenseI32ArrayAttribute::new(context, &[n, 0]).into(),
            ),
            (
                Identifier::new(context, "op_bundle_sizes"),
                DenseI32ArrayAttribute::new(context, &[]).into(),
            ),
        ])
        .build()
        .unwrap_or_else(|_| panic!("llvm.call @{name}"));
    block.append_operation(call_op);
}

/// Generic single-result libc helper — base implementation.
pub(crate) fn emit_libc_call_with_result<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    name: &str,
    args: &[Value<'c, 'a>],
    result_ty: Type<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let n = i32::try_from(args.len()).expect("call arity fits in i32");
    let call_op = OperationBuilder::new("llvm.call", loc)
        .add_operands(args)
        .add_attributes(&[
            (
                Identifier::new(context, "callee"),
                FlatSymbolRefAttribute::new(context, name).into(),
            ),
            (
                Identifier::new(context, "operandSegmentSizes"),
                DenseI32ArrayAttribute::new(context, &[n, 0]).into(),
            ),
            (
                Identifier::new(context, "op_bundle_sizes"),
                DenseI32ArrayAttribute::new(context, &[]).into(),
            ),
        ])
        .add_results(&[result_ty])
        .build()
        .unwrap_or_else(|_| panic!("llvm.call @{name}"));
    block.append_operation(call_op).result(0).unwrap().into()
}
