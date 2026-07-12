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
    dialect::{arith, llvm, ods::llvm::AddressOfOperationBuilder, scf},
    ir::{
        Block, BlockLike, Identifier, Location, Region, RegionLike, Type, Value,
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
    // Phase 2.7u-string-abi-refactor (ADR 0112): msg_ptr is now a
    // boxed string object (i64 len + data + compat NUL). Use
    // length-safe println so embedded NUL in diagnostic messages
    // passes through (no current msg has embedded NUL, but the
    // change keeps the API uniform).
    emit_println_string_obj(context, block, msg_ptr, types, loc);
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

/// Generic single-result libc helper — `f64` return type. Used by
/// libm extern calls (`sqrt`, `floor`, `fabs`, etc.) from ADR 0101's
/// `math.*` builtin dispatch.
pub(crate) fn emit_libc_call_f64<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    name: &str,
    args: &[Value<'c, 'a>],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    emit_libc_call_with_result(context, block, name, args, types.f64, loc)
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

/// Phase 2.7u-string-abi-refactor (ADR 0112): malloc + null-check
/// + trap chokepoint. Replaces direct `emit_libc_call_ptr("malloc",
///   ...)` calls at string-alloc sites so OOM has a consistent
///   runtime failure surface (`s_alloc_oom` diagnostic).
///
/// `oom_msg_global_name` is the diagnostic global symbol name —
/// caller passes `"s_alloc_oom"`; parameterising rather than
/// hard-coding keeps `primitive.rs` Lua-semantics-agnostic
/// (consistent with `emit_exit_with_message`'s `msg_ptr` arg).
///
/// Returns the allocated ptr (non-null in successful path; trap
/// diverges on null).
/// ADR 0157 — Phase 3 GC step 1 allocator wrapper. Emits a malloc
/// of `payload_size + GC_HEADER_SIZE`, null-traps via `s_gc_oom`,
/// inits the 16-byte GC header (mark=0 WHITE, type_tag, padding=0,
/// payload_size as u32, next=load(g_gc_head)), atomically pushes
/// to `g_gc_head`, accumulates bytes into `g_gc_total_bytes`, and
/// returns the user-visible payload ptr (`raw + GC_HEADER_SIZE`).
///
/// `g_gc_head` is declared as an i64 module global (holding the
/// ptrtoint of the head ptr) to avoid initializer-region
/// complexity for ptr globals — the helper bridges the
/// ptr ↔ i64 representation locally.
pub(crate) fn emit_gc_alloc<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    payload_size: Value<'c, 'a>,
    type_tag: u8,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    use crate::codegen::tagged::{
        GC_HEADER_OFF_MARK, GC_HEADER_OFF_NEXT, GC_HEADER_OFF_SIZE, GC_HEADER_OFF_TYPE_TAG,
        GC_HEADER_SIZE, GC_THRESHOLD_CAP,
    };
    // ADR 0184 — guard against payload >= 4 GiB silently wrapping
    // when truncated to u32 for GC_HEADER_OFF_SIZE. LLVM constant-
    // folds the branch away when `payload_size` is a small literal.
    let four_gib_const: Value<'c, 'a> = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, 1_i64 << 32).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let too_large: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Uge,
            payload_size,
            four_gib_const,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    {
        let msg = emit_addressof(context, &then_blk, "s_gc_alloc_too_large", types, loc);
        emit_exit_with_message(context, &then_blk, msg, types, loc);
        then_blk.append_operation(scf::r#yield(&[], loc));
    }
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    else_blk.append_operation(scf::r#yield(&[], loc));
    else_region.append_block(else_blk);
    block.append_operation(scf::r#if(too_large, &[], then_region, else_region, loc));

    let header_size_const = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, GC_HEADER_SIZE).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let total_size: Value<'c, 'a> = block
        .append_operation(arith::addi(payload_size, header_size_const, loc))
        .result(0)
        .unwrap()
        .into();
    // ADR 0186 — auto-trigger threshold gate. If this allocation
    // would push `g_gc_total_bytes` past `g_gc_threshold`, call
    // `@gc_mark` + `@gc_sweep` first, then double the threshold
    // against the post-sweep total (capped at `GC_THRESHOLD_CAP`).
    // Doubling lives at the caller, not inside `@gc_sweep`, so
    // explicit `collectgarbage()` does not inflate the threshold.
    let bytes_addr_trig = emit_addressof(context, block, "g_gc_total_bytes", types, loc);
    let cur_total = emit_load(block, bytes_addr_trig, types.i64, loc);
    let projected_total: Value<'c, 'a> = block
        .append_operation(arith::addi(cur_total, total_size, loc))
        .result(0)
        .unwrap()
        .into();
    let thr_addr = emit_addressof(context, block, "g_gc_threshold", types, loc);
    let threshold_pre = emit_load(block, thr_addr, types.i64, loc);
    let should_gc: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Sge,
            projected_total,
            threshold_pre,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let gc_then_region = Region::new();
    let gc_then_blk = Block::new(&[]);
    {
        emit_libc_call_void(context, &gc_then_blk, "gc_mark", &[], loc);
        emit_libc_call_void(context, &gc_then_blk, "gc_sweep", &[], loc);
        // ADR 0200 — threshold doubling uses `g_gc_pause / 100` as
        // the multiplier (was hardcoded `*2` in ADR 0186). Default
        // pause = 200 (Lua 5.4) → identical 2.0× behaviour.
        let bytes_addr_post = emit_addressof(context, &gc_then_blk, "g_gc_total_bytes", types, loc);
        let post_sweep = emit_load(&gc_then_blk, bytes_addr_post, types.i64, loc);
        let pause_addr = emit_addressof(context, &gc_then_blk, "g_gc_pause", types, loc);
        let pause = emit_load(&gc_then_blk, pause_addr, types.i64, loc);
        let mul: Value<'c, '_> = gc_then_blk
            .append_operation(arith::muli(post_sweep, pause, loc))
            .result(0)
            .unwrap()
            .into();
        let hundred_const = gc_then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 100).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let doubled: Value<'c, '_> = gc_then_blk
            .append_operation(arith::divsi(mul, hundred_const, loc))
            .result(0)
            .unwrap()
            .into();
        let thr_addr_inner = emit_addressof(context, &gc_then_blk, "g_gc_threshold", types, loc);
        let cur_thr = emit_load(&gc_then_blk, thr_addr_inner, types.i64, loc);
        let needs_grow: Value<'c, '_> = gc_then_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Sgt,
                doubled,
                cur_thr,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let grow_then = Region::new();
        let grow_then_blk = Block::new(&[]);
        {
            let cap_const = grow_then_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, GC_THRESHOLD_CAP).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let over_cap: Value<'c, '_> = grow_then_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Sgt,
                    doubled,
                    cap_const,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let capped: Value<'c, '_> = grow_then_blk
                .append_operation(arith::select(over_cap, cap_const, doubled, loc))
                .result(0)
                .unwrap()
                .into();
            emit_store(&grow_then_blk, capped, thr_addr_inner, loc);
            grow_then_blk.append_operation(scf::r#yield(&[], loc));
        }
        grow_then.append_block(grow_then_blk);
        let grow_else = Region::new();
        let grow_else_blk = Block::new(&[]);
        grow_else_blk.append_operation(scf::r#yield(&[], loc));
        grow_else.append_block(grow_else_blk);
        gc_then_blk.append_operation(scf::r#if(needs_grow, &[], grow_then, grow_else, loc));
        gc_then_blk.append_operation(scf::r#yield(&[], loc));
    }
    gc_then_region.append_block(gc_then_blk);
    let gc_else_region = Region::new();
    let gc_else_blk = Block::new(&[]);
    gc_else_blk.append_operation(scf::r#yield(&[], loc));
    gc_else_region.append_block(gc_else_blk);
    block.append_operation(scf::r#if(
        should_gc,
        &[],
        gc_then_region,
        gc_else_region,
        loc,
    ));

    let raw = emit_alloc_with_oom_check(context, block, total_size, "s_gc_oom", types, loc);
    // Header init: mark=0, type_tag, padding=0, size, next=load(g_gc_head).
    let mark_ptr = emit_byte_offset_ptr(context, block, raw, GC_HEADER_OFF_MARK, types, loc);
    let mark_zero_i8 = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i8, 0).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    emit_store(block, mark_zero_i8, mark_ptr, loc);
    let type_tag_ptr =
        emit_byte_offset_ptr(context, block, raw, GC_HEADER_OFF_TYPE_TAG, types, loc);
    let type_tag_const = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i8, i64::from(type_tag)).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    emit_store(block, type_tag_const, type_tag_ptr, loc);
    let size_ptr = emit_byte_offset_ptr(context, block, raw, GC_HEADER_OFF_SIZE, types, loc);
    let size_u32: Value<'c, 'a> = block
        .append_operation(arith::trunci(payload_size, types.i32, loc))
        .result(0)
        .unwrap()
        .into();
    emit_store(block, size_u32, size_ptr, loc);
    // Load g_gc_head (as i64), inttoptr → store at next slot.
    let head_addr = emit_addressof(context, block, "g_gc_head", types, loc);
    let head_iv = emit_load(block, head_addr, types.i64, loc);
    let head_ptr_val: Value<'c, 'a> = block
        .append_operation(
            OperationBuilder::new("llvm.inttoptr", loc)
                .add_operands(&[head_iv])
                .add_results(&[types.ptr])
                .build()
                .expect("llvm.inttoptr gc head"),
        )
        .result(0)
        .unwrap()
        .into();
    let next_ptr = emit_byte_offset_ptr(context, block, raw, GC_HEADER_OFF_NEXT, types, loc);
    emit_store(block, head_ptr_val, next_ptr, loc);
    // Push: g_gc_head = ptrtoint(raw).
    let raw_iv: Value<'c, 'a> = block
        .append_operation(
            OperationBuilder::new("llvm.ptrtoint", loc)
                .add_operands(&[raw])
                .add_results(&[types.i64])
                .build()
                .expect("llvm.ptrtoint gc raw"),
        )
        .result(0)
        .unwrap()
        .into();
    emit_store(block, raw_iv, head_addr, loc);
    // total_bytes += total_size.
    let bytes_addr = emit_addressof(context, block, "g_gc_total_bytes", types, loc);
    let old_bytes = emit_load(block, bytes_addr, types.i64, loc);
    let new_bytes: Value<'c, 'a> = block
        .append_operation(arith::addi(old_bytes, total_size, loc))
        .result(0)
        .unwrap()
        .into();
    emit_store(block, new_bytes, bytes_addr, loc);
    // Return payload ptr = raw + GC_HEADER_SIZE.
    emit_byte_offset_ptr(context, block, raw, GC_HEADER_SIZE, types, loc)
}

pub(crate) fn emit_alloc_with_oom_check<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    size_i64: Value<'c, 'a>,
    oom_msg_global_name: &str,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let ptr = emit_libc_call_ptr(context, block, "malloc", &[size_i64], types, loc);
    // ptrtoint to i64, then `arith::cmpi(Eq, _, 0)` for null check.
    let ptr_as_i64: Value<'c, '_> = block
        .append_operation(
            OperationBuilder::new("llvm.ptrtoint", loc)
                .add_operands(&[ptr])
                .add_results(&[types.i64])
                .build()
                .expect("llvm.ptrtoint malloc result"),
        )
        .result(0)
        .unwrap()
        .into();
    let zero_i64: Value<'c, '_> = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, 0).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let is_null: Value<'c, '_> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            ptr_as_i64,
            zero_i64,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    {
        let msg = emit_addressof(context, &then_blk, oom_msg_global_name, types, loc);
        emit_exit_with_message(context, &then_blk, msg, types, loc);
        then_blk.append_operation(scf::r#yield(&[], loc));
    }
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    else_blk.append_operation(scf::r#yield(&[], loc));
    else_region.append_block(else_blk);
    block.append_operation(scf::r#if(is_null, &[], then_region, else_region, loc));
    ptr
}

// =============================================================================
// Phase 2.7u-string-abi-refactor (ADR 0112): boxed string object helpers.
// =============================================================================
//
// Lua spec mandates byte-sequence string semantics including
// embedded NUL bytes. The pre-0112 C-string ABI (NUL-terminated
// `!llvm.ptr`, ADR 0024) truncates at the first NUL, breaking
// `#"\x00" == 1` and many other length-aware operations.
//
// ADR 0112 introduces a **boxed string object** layout:
//
//   String = !llvm.ptr to { i64 len, i8 data[len + 1] }
//
// - offset 0: i64 length (truth source for byte count)
// - offset 8: i8 data[len] (raw bytes, may contain NUL)
// - offset 8+len: i8 0 (compat NUL terminator, lets legacy
//                       printf %s / strcmp partially work for
//                       NUL-free callers; all post-0112 consumers
//                       use length-aware paths)
//
// Storage:
// - Heap (most strings): emit_string_obj_alloc → emit_alloc_with_oom_check
// - Static global (literals / diagnostic msgs / type names):
//   `emit_string_global` (in emit.rs) emits the same layout via
//   an i8 byte-array initialiser containing little-endian len
//   followed by data + NUL.
//
// Helpers are placed in primitive.rs (not emit.rs) so both
// emit.rs and tagged.rs can use them without circular `pub(crate)`
// dependencies. The layout itself is a generic byte-string-with-
// length pattern (not Lua-specific), keeping primitive.rs's
// "low-level wrapper" character.

/// Offset of the i8 data bytes within a string object. The
/// i64 length field sits at offset 0 (no constant: `emit_load`
/// at the object ptr reads it directly).
pub(crate) const STRING_OBJ_OFF_DATA: i64 = 8;
/// Constant header size in bytes (`STRING_OBJ_OFF_DATA`).
pub(crate) const STRING_OBJ_HEADER_SIZE: i64 = 8;
/// Bytes added beyond `len` when allocating a string object:
/// 8-byte header + 1-byte compat NUL terminator.
pub(crate) const STRING_OBJ_ALLOC_OVERHEAD: i64 = STRING_OBJ_HEADER_SIZE + 1;

/// Load the i64 length field from a string object.
pub(crate) fn emit_string_obj_len<'a, 'c>(
    block: &'a Block<'c>,
    s_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    // String object header sits at offset 0; direct load i64.
    emit_load(block, s_ptr, types.i64, loc)
}

/// Get a ptr to the data bytes (offset 8) of a string object.
pub(crate) fn emit_string_obj_data<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    s_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    emit_byte_offset_ptr(context, block, s_ptr, STRING_OBJ_OFF_DATA, types, loc)
}

/// Allocate a fresh heap string object with `len` bytes of
/// payload. Returns ptr to header. The data bytes are
/// uninitialised; caller must memcpy/store into the data region
/// then call `emit_string_obj_finalize_nul` (or write the NUL
/// byte manually) before publishing the object.
///
/// Internally uses `emit_alloc_with_oom_check` so OOM produces
/// the `s_alloc_oom` runtime trap.
pub(crate) fn emit_string_obj_alloc<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    len_i64: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let overhead = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, STRING_OBJ_ALLOC_OVERHEAD).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let total_size: Value<'c, '_> = block
        .append_operation(arith::addi(len_i64, overhead, loc))
        .result(0)
        .unwrap()
        .into();
    // ADR 0157 — Phase 3 GC step 1: route through `emit_gc_alloc`
    // so every string object is tracked in `g_gc_head`. Returns the
    // user-visible payload ptr (i.e. raw + GC_HEADER_SIZE); the
    // string-object layout offsets (len at +0, data at +8) stay
    // correct relative to this ptr.
    let s_ptr = emit_gc_alloc(
        context,
        block,
        total_size,
        crate::codegen::tagged::GC_TYPE_STRING_OBJ,
        types,
        loc,
    );
    // Store len at offset 0 (header).
    emit_store(block, len_i64, s_ptr, loc);
    s_ptr
}

/// Write the compat NUL terminator at `s_ptr + 8 + len`. Call
/// after populating the data region (the alloc helper does not
/// store the NUL automatically — caller controls fill order).
pub(crate) fn emit_string_obj_finalize_nul<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    s_ptr: Value<'c, 'a>,
    len_i64: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let data_ptr = emit_string_obj_data(context, block, s_ptr, types, loc);
    let term_ptr = emit_byte_offset_ptr_dynamic(context, block, data_ptr, len_i64, types, loc);
    let zero_i8 = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i8, 0).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    emit_store(block, zero_i8, term_ptr, loc);
}

/// Build a fresh string object by copying `len` bytes from
/// `src_data_ptr` (a raw i8 ptr — e.g., from snprintf or a libc
/// result). Convenience wrapper for the common
/// `alloc + memcpy + nul-term` pattern.
pub(crate) fn emit_string_obj_from_bytes<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    src_data_ptr: Value<'c, 'a>,
    len_i64: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let s_ptr = emit_string_obj_alloc(context, block, len_i64, types, loc);
    let dst_data = emit_string_obj_data(context, block, s_ptr, types, loc);
    let _ = emit_libc_call_ptr(
        context,
        block,
        "memcpy",
        &[dst_data, src_data_ptr, len_i64],
        types,
        loc,
    );
    emit_string_obj_finalize_nul(context, block, s_ptr, len_i64, types, loc);
    s_ptr
}

/// Byte-equal comparison of two string objects. Returns i1
/// (`true` iff `len_a == len_b && memcmp(data_a, data_b, len_a)
/// == 0`). Short-circuits on length mismatch.
pub(crate) fn emit_string_obj_eq<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    a_ptr: Value<'c, 'a>,
    b_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let len_a = emit_string_obj_len(block, a_ptr, types, loc);
    let len_b = emit_string_obj_len(block, b_ptr, types, loc);
    let len_eq: Value<'c, '_> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            len_a,
            len_b,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    // scf::r#if (len_eq): then memcmp == 0; else false.
    let false_i1 = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i1, 0).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    {
        let data_a = emit_string_obj_data(context, &then_blk, a_ptr, types, loc);
        let data_b = emit_string_obj_data(context, &then_blk, b_ptr, types, loc);
        let cmp = emit_libc_call_i32(
            context,
            &then_blk,
            "memcmp",
            &[data_a, data_b, len_a],
            types,
            loc,
        );
        let zero_i32 = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i32, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let mem_eq: Value<'c, '_> = then_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                cmp,
                zero_i32,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        then_blk.append_operation(scf::r#yield(&[mem_eq], loc));
    }
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    else_blk.append_operation(scf::r#yield(&[false_i1], loc));
    else_region.append_block(else_blk);
    block
        .append_operation(scf::r#if(
            len_eq,
            &[types.i1],
            then_region,
            else_region,
            loc,
        ))
        .result(0)
        .unwrap()
        .into()
}

/// 3-way lexicographic compare of two boxed string objects.
/// Returns a signed i32:
/// - negative if `a < b`
/// - 0 if `a == b`
/// - positive if `a > b`
///
/// Semantics match `strcmp` for the NUL-free byte slice but
/// also handle embedded NULs correctly (memcmp over min(la, lb),
/// then length-difference tiebreak so the shorter is "less"
/// when one is a strict prefix of the other).
pub(crate) fn emit_string_obj_compare<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    a_ptr: Value<'c, 'a>,
    b_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let len_a = emit_string_obj_len(block, a_ptr, types, loc);
    let len_b = emit_string_obj_len(block, b_ptr, types, loc);
    let data_a = emit_string_obj_data(context, block, a_ptr, types, loc);
    let data_b = emit_string_obj_data(context, block, b_ptr, types, loc);
    let a_shorter: Value<'c, '_> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Slt,
            len_a,
            len_b,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let min_len: Value<'c, '_> = block
        .append_operation(arith::select(a_shorter, len_a, len_b, loc))
        .result(0)
        .unwrap()
        .into();
    let mem = emit_libc_call_i32(
        context,
        block,
        "memcmp",
        &[data_a, data_b, min_len],
        types,
        loc,
    );
    let zero_i32 = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i32, 0).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let mem_eq: Value<'c, '_> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            mem,
            zero_i32,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    // Tiebreak when prefix bytes match: trunci(len_a - len_b) → i32.
    let len_diff_i64: Value<'c, '_> = block
        .append_operation(arith::subi(len_a, len_b, loc))
        .result(0)
        .unwrap()
        .into();
    let len_diff_i32: Value<'c, '_> = block
        .append_operation(arith::trunci(len_diff_i64, types.i32, loc))
        .result(0)
        .unwrap()
        .into();
    block
        .append_operation(arith::select(mem_eq, len_diff_i32, mem, loc))
        .result(0)
        .unwrap()
        .into()
}

/// FNV-1a 64-bit hash of a string object, iterating over `len`
/// bytes from the data region (not strlen-bounded; NUL is part
/// of the hashed sequence).
pub(crate) fn emit_string_obj_hash<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    s_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    // FNV-1a 64-bit constants (offset basis as signed i64 bit pattern).
    const FNV_OFFSET_BASIS: i64 = -3750763034362895579;
    const FNV_PRIME: i64 = 1099511628211;
    let len_i64 = emit_string_obj_len(block, s_ptr, types, loc);
    let data_ptr = emit_string_obj_data(context, block, s_ptr, types, loc);
    let init_hash = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, FNV_OFFSET_BASIS).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let init_idx = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, 0).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let before_region = Region::new();
    let before_blk = Block::new(&[(types.i64, loc), (types.i64, loc)]);
    {
        let hash_arg: Value<'c, '_> = before_blk.argument(0).unwrap().into();
        let idx_arg: Value<'c, '_> = before_blk.argument(1).unwrap().into();
        let cond: Value<'c, '_> = before_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Slt,
                idx_arg,
                len_i64,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        before_blk.append_operation(scf::condition(cond, &[hash_arg, idx_arg], loc));
    }
    before_region.append_block(before_blk);
    let after_region = Region::new();
    let after_blk = Block::new(&[(types.i64, loc), (types.i64, loc)]);
    {
        let hash_arg: Value<'c, '_> = after_blk.argument(0).unwrap().into();
        let idx_arg: Value<'c, '_> = after_blk.argument(1).unwrap().into();
        let byte_ptr =
            emit_byte_offset_ptr_dynamic(context, &after_blk, data_ptr, idx_arg, types, loc);
        let byte_i8 = emit_load(&after_blk, byte_ptr, types.i8, loc);
        let byte_i64: Value<'c, '_> = after_blk
            .append_operation(
                OperationBuilder::new("arith.extui", loc)
                    .add_operands(&[byte_i8])
                    .add_results(&[types.i64])
                    .build()
                    .expect("arith.extui i8 -> i64"),
            )
            .result(0)
            .unwrap()
            .into();
        let xored: Value<'c, '_> = after_blk
            .append_operation(arith::xori(hash_arg, byte_i64, loc))
            .result(0)
            .unwrap()
            .into();
        let prime = after_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, FNV_PRIME).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let new_hash: Value<'c, '_> = after_blk
            .append_operation(arith::muli(xored, prime, loc))
            .result(0)
            .unwrap()
            .into();
        let one = after_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 1).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let next_idx: Value<'c, '_> = after_blk
            .append_operation(arith::addi(idx_arg, one, loc))
            .result(0)
            .unwrap()
            .into();
        after_blk.append_operation(scf::r#yield(&[new_hash, next_idx], loc));
    }
    after_region.append_block(after_blk);
    let while_op = scf::r#while(
        &[init_hash, init_idx],
        &[types.i64, types.i64],
        before_region,
        after_region,
        loc,
    );
    block.append_operation(while_op).result(0).unwrap().into()
}

/// Phase 2.devinfra-stdout-fwrite (ADR 0117): write a string
/// object to stdout via `fwrite(data, 1, len, stdout)`. Binary-
/// safe — embedded NUL bytes pass through.
///
/// Supersedes the pre-0117 implementation that used
/// `printf("%.*s", len_i32, data)`, which per POSIX `%s`
/// semantics truncates at the first NUL byte. Lua §2.4 mandates
/// 8-bit clean strings (any byte, including NUL), so the printf
/// path defeated ADR 0112's boxed-string ABI promise at the
/// stdout chokepoint.
///
/// Lookup the libc `extern FILE *stdout;` symbol, load the
/// FILE pointer, and pass it as the 4th argument to fwrite.
/// The return value (count of items written) is ignored —
/// matches Lua spec where `io.write` does not surface short-
/// write errors.
pub(crate) fn emit_print_string_obj<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    s_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let len = emit_string_obj_len(block, s_ptr, types, loc);
    let data = emit_string_obj_data(context, block, s_ptr, types, loc);
    let one_i64 = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, 1).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let stdout_addr =
        emit_addressof(context, block, super::emit::host_stdio_symbol("stdout"), types, loc);
    let stdout_ptr = emit_load(block, stdout_addr, types.ptr, loc);
    // fwrite(data, 1, len, stdout) — non-variadic, 4 args.
    let call_op = OperationBuilder::new("llvm.call", loc)
        .add_operands(&[data, one_i64, len, stdout_ptr])
        .add_attributes(&[
            (
                Identifier::new(context, "callee"),
                FlatSymbolRefAttribute::new(context, "fwrite").into(),
            ),
            (
                Identifier::new(context, "operandSegmentSizes"),
                DenseI32ArrayAttribute::new(context, &[4, 0]).into(),
            ),
            (
                Identifier::new(context, "op_bundle_sizes"),
                DenseI32ArrayAttribute::new(context, &[]).into(),
            ),
        ])
        .add_results(&[types.i64])
        .build()
        .expect("llvm.call @fwrite");
    block.append_operation(call_op);
}

/// Phase 2.devinfra-stdout-fwrite (ADR 0117): print a string
/// object followed by `\n`. Implemented as two fwrite calls —
/// once for the string data, once for the global `s_newline`
/// boxed object — so both reuse the binary-safe chokepoint.
pub(crate) fn emit_println_string_obj<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    s_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    emit_print_string_obj(context, block, s_ptr, types, loc);
    // Inline equivalent of `emit_print_literal("s_newline")` —
    // primitive.rs does not depend on emit.rs.
    let newline_ptr = emit_addressof(context, block, "s_newline", types, loc);
    emit_print_string_obj(context, block, newline_ptr, types, loc);
}
