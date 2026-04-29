//! Emit MLIR from a resolved HIR chunk.
//!
//! Produces an MLIR `Module` using standard dialects (`arith`, `func`, `llvm`).
//! All statements run in `func.func @main`. Locals are stack slots created
//! with `llvm.alloca` at function entry.

use melior::{
    Context,
    dialect::{
        DialectRegistry, arith,
        arith::CmpfPredicate,
        func, llvm,
        ods::llvm::{AddressOfOperationBuilder, GlobalOperationBuilder, LLVMFuncOperationBuilder},
        scf,
    },
    ir::{
        Block, BlockLike, Identifier, Location, Module, Region, RegionLike, Type, Value,
        attribute::{
            DenseI32ArrayAttribute, FlatSymbolRefAttribute, FloatAttribute, IntegerAttribute,
            StringAttribute, TypeAttribute,
        },
        operation::{OperationBuilder, OperationLike},
        r#type::{FunctionType, IntegerType},
    },
    utility::register_all_dialects,
};

use crate::hir::{
    Builtin, HirChunk, HirExpr, HirExprKind, HirStmt, HirStmtKind, LocalId, LocalInfo, ValueKind,
    infer_kind,
};
use crate::parser::{BinOp, UnaryOp};

use super::error::CodegenError;

struct Types<'c> {
    i1: Type<'c>,
    i32: Type<'c>,
    i64: Type<'c>,
    f64: Type<'c>,
    ptr: Type<'c>,
}

/// Emit a verified MLIR module from a resolved HIR chunk.
pub fn emit_module<'c>(context: &'c Context, chunk: &HirChunk) -> Result<Module<'c>, CodegenError> {
    let module = emit_module_unverified(context, chunk)?;
    if !module.as_operation().verify() {
        return Err(CodegenError::VerificationFailed);
    }
    Ok(module)
}

/// Emit an MLIR module **without** the final `verify()` gate — used by
/// codegen unit tests that want to inspect the IR for ops that depend on
/// later-Phase-2.2b machinery (e.g. comparing ops emitted before
/// `print(bool)` is wired up).
pub(crate) fn emit_module_unverified<'c>(
    context: &'c Context,
    chunk: &HirChunk,
) -> Result<Module<'c>, CodegenError> {
    let loc = Location::unknown(context);
    let module = Module::new(loc);

    let i8_type = IntegerType::new(context, 8).into();
    let types = Types {
        i1: IntegerType::new(context, 1).into(),
        i32: IntegerType::new(context, 32).into(),
        i64: IntegerType::new(context, 64).into(),
        f64: Type::float64(context),
        ptr: llvm::r#type::pointer(context, 0),
    };

    emit_fmt_global(context, &module, i8_type, loc);
    emit_printf_decl(context, &module, &types, loc);
    emit_libm_decls(context, &module, &types, loc);
    emit_main(context, &module, chunk, &types, loc)?;

    Ok(module)
}

pub fn new_context() -> Context {
    let context = Context::new();
    let registry = DialectRegistry::new();
    register_all_dialects(&registry);
    context.append_dialect_registry(&registry);
    context.load_all_available_dialects();
    context
}

fn emit_fmt_global<'c>(
    context: &'c Context,
    module: &Module<'c>,
    i8_type: Type<'c>,
    loc: Location<'c>,
) {
    emit_string_global(context, module, i8_type, "fmt", "%g\n\0", loc);
    // Phase 2.2b: format and string pool for `print(bool)`.
    emit_string_global(context, module, i8_type, "fmt_str", "%s\n\0", loc);
    emit_string_global(context, module, i8_type, "s_true", "true\0", loc);
    emit_string_global(context, module, i8_type, "s_false", "false\0", loc);
    // Phase 2.3a: nil string for `print(nil)`.
    emit_string_global(context, module, i8_type, "s_nil", "nil\0", loc);
}

fn emit_string_global<'c>(
    context: &'c Context,
    module: &Module<'c>,
    i8_type: Type<'c>,
    name: &str,
    value: &str,
    loc: Location<'c>,
) {
    let array_type = llvm::r#type::array(i8_type, value.len() as u32);
    let global_op = GlobalOperationBuilder::new(context, loc)
        .initializer(Region::new())
        .global_type(TypeAttribute::new(array_type))
        .sym_name(StringAttribute::new(context, name))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::Internal,
        ))
        .constant(melior::ir::attribute::Attribute::unit(context))
        .value(StringAttribute::new(context, value).into())
        .build();
    module.body().append_operation(global_op.into());
}

fn emit_printf_decl<'c>(
    context: &'c Context,
    module: &Module<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let printf_fn_type = llvm::r#type::function(types.i32, &[types.ptr], true);
    let printf_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "printf"))
        .function_type(TypeAttribute::new(printf_fn_type))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(printf_op.into());
}

fn emit_libm_decls<'c>(
    context: &'c Context,
    module: &Module<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    // pow(f64, f64) -> f64
    let pow_ty = llvm::r#type::function(types.f64, &[types.f64, types.f64], false);
    let pow_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "pow"))
        .function_type(TypeAttribute::new(pow_ty))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(pow_op.into());

    // floor(f64) -> f64
    let floor_ty = llvm::r#type::function(types.f64, &[types.f64], false);
    let floor_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "floor"))
        .function_type(TypeAttribute::new(floor_ty))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(floor_op.into());
}

fn emit_main<'c>(
    context: &'c Context,
    module: &Module<'c>,
    chunk: &HirChunk,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let main_region = Region::new();
    let main_block = Block::new(&[]);

    let slots: Vec<Value<'c, '_>> = chunk
        .locals
        .iter()
        .map(|info| emit_alloca_slot_for_kind(context, &main_block, info.kind, types, loc))
        .collect();

    emit_stmts(
        context,
        &main_block,
        &chunk.stmts,
        &slots,
        &chunk.locals,
        types,
        loc,
    )?;

    let zero = arith::constant(context, IntegerAttribute::new(types.i64, 0).into(), loc);
    let zero_val = main_block.append_operation(zero).result(0).unwrap().into();
    main_block.append_operation(func::r#return(&[zero_val], loc));

    main_region.append_block(main_block);

    let main_fn_type = FunctionType::new(context, &[], &[types.i64]);
    let main_op = func::func(
        context,
        StringAttribute::new(context, "main"),
        TypeAttribute::new(main_fn_type.into()),
        main_region,
        &[],
        loc,
    );
    module.body().append_operation(main_op);
    Ok(())
}

fn emit_stmts<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    stmts: &[HirStmt],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    for stmt in stmts {
        emit_stmt(context, block, stmt, slots, locals, types, loc)?;
    }
    Ok(())
}

fn emit_stmt<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    stmt: &HirStmt,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    match &stmt.kind {
        HirStmtKind::LocalInit { id, value } | HirStmtKind::Assign { id, value } => {
            let v = emit_expr(context, block, value, slots, locals, types, loc)?;
            emit_store(block, v, slots[id.0], loc);
        }
        HirStmtKind::Block { stmts } => {
            emit_stmts(context, block, stmts, slots, locals, types, loc)?;
        }
        HirStmtKind::If {
            cond,
            then_body,
            elifs,
            else_body,
        } => {
            emit_if(
                context, block, cond, then_body, elifs, else_body, slots, locals, types, loc,
            )?;
        }
        HirStmtKind::While { cond, body } => {
            emit_while(context, block, cond, body, slots, locals, types, loc)?;
        }
        HirStmtKind::ExprStmt(expr) => {
            emit_expr_stmt(context, block, expr, slots, locals, types, loc)?;
        }
    }
    Ok(())
}

fn emit_alloca_slot_for_kind<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    kind: ValueKind,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let elem = match kind {
        ValueKind::Number => types.f64,
        // Bool: 1-bit. Nil: nominally 1-bit too — the value is never
        // observed (heterogeneous `==` and print(nil) both bypass
        // the slot via static dispatch), but storage is uniform.
        ValueKind::Bool | ValueKind::Nil => types.i1,
    };

    let one = arith::constant(context, IntegerAttribute::new(types.i32, 1).into(), loc);
    let one_val: Value<'c, 'a> = block.append_operation(one).result(0).unwrap().into();

    let alloca = OperationBuilder::new("llvm.alloca", loc)
        .add_operands(&[one_val])
        .add_attributes(&[(
            Identifier::new(context, "elem_type"),
            TypeAttribute::new(elem).into(),
        )])
        .add_results(&[types.ptr])
        .build()
        .expect("llvm.alloca");
    block.append_operation(alloca).result(0).unwrap().into()
}

fn emit_store<'a, 'c>(
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

fn emit_load<'a, 'c>(
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

fn emit_expr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    expr: &HirExpr,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    match &expr.kind {
        HirExprKind::Number(n) => {
            let op = arith::constant(
                context,
                FloatAttribute::new(context, types.f64, *n).into(),
                loc,
            );
            Ok(block.append_operation(op).result(0).unwrap().into())
        }
        HirExprKind::Bool(b) => {
            let attr = IntegerAttribute::new(types.i1, i64::from(*b));
            let op = arith::constant(context, attr.into(), loc);
            Ok(block.append_operation(op).result(0).unwrap().into())
        }
        HirExprKind::Nil => {
            // Step 3 stub — per-kind slot allocation lands in Step 4.
            // Bool(0) stand-in keeps the IR shape valid so we can
            // exercise the HIR fold logic in tests.
            let attr = IntegerAttribute::new(types.i1, 0);
            let op = arith::constant(context, attr.into(), loc);
            Ok(block.append_operation(op).result(0).unwrap().into())
        }
        HirExprKind::Local(LocalId(idx)) => {
            let slot_ty = match locals[*idx].kind {
                ValueKind::Number => types.f64,
                ValueKind::Bool | ValueKind::Nil => types.i1,
            };
            Ok(emit_load(block, slots[*idx], slot_ty, loc))
        }
        HirExprKind::BinOp { op, lhs, rhs } => {
            let lhs_val = emit_expr(context, block, lhs, slots, locals, types, loc)?;
            let rhs_val = emit_expr(context, block, rhs, slots, locals, types, loc)?;
            emit_binop(context, block, *op, lhs_val, rhs_val, types, loc)
        }
        HirExprKind::UnaryOp { op, operand } => {
            let v = emit_expr(context, block, operand, slots, locals, types, loc)?;
            emit_unary(block, *op, v, loc)
        }
        HirExprKind::Call { builtin, args } => match builtin {
            Builtin::Print => {
                let kind = infer_kind(&args[0], locals);
                let arg_val = emit_expr(context, block, &args[0], slots, locals, types, loc)?;
                emit_print_value(context, block, arg_val, kind, types, loc);
                Ok(arg_val)
            }
        },
    }
}

fn emit_expr_stmt<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    expr: &HirExpr,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    emit_expr(context, block, expr, slots, locals, types, loc)?;
    Ok(())
}

fn emit_binop<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    op: BinOp,
    lhs: Value<'c, 'a>,
    rhs: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    let result = match op {
        BinOp::Add => block
            .append_operation(arith::addf(lhs, rhs, loc))
            .result(0)
            .unwrap()
            .into(),
        BinOp::Sub => block
            .append_operation(arith::subf(lhs, rhs, loc))
            .result(0)
            .unwrap()
            .into(),
        BinOp::Mul => block
            .append_operation(arith::mulf(lhs, rhs, loc))
            .result(0)
            .unwrap()
            .into(),
        BinOp::Div => block
            .append_operation(arith::divf(lhs, rhs, loc))
            .result(0)
            .unwrap()
            .into(),
        BinOp::Mod => emit_lua_mod(context, block, lhs, rhs, types, loc),
        BinOp::Pow => emit_libm_pow(context, block, lhs, rhs, types, loc),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => {
            let predicate = cmpf_predicate(op);
            block
                .append_operation(arith::cmpf(context, predicate, lhs, rhs, loc))
                .result(0)
                .unwrap()
                .into()
        }
    };
    Ok(result)
}

/// Map a comparison [`BinOp`] to its `arith.cmpf` predicate. Always
/// **ordered** — `NaN <op> x` is `false`, including `NaN == NaN`,
/// matching IEEE 754 and Lua 5.4 semantics.
fn cmpf_predicate(op: BinOp) -> CmpfPredicate {
    match op {
        BinOp::Lt => CmpfPredicate::Olt,
        BinOp::Le => CmpfPredicate::Ole,
        BinOp::Gt => CmpfPredicate::Ogt,
        BinOp::Ge => CmpfPredicate::Oge,
        BinOp::Eq => CmpfPredicate::Oeq,
        BinOp::Ne => CmpfPredicate::One,
        _ => unreachable!("cmpf_predicate called with non-comparison BinOp"),
    }
}

fn emit_unary<'a, 'c>(
    block: &'a Block<'c>,
    op: UnaryOp,
    operand: Value<'c, 'a>,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    match op {
        UnaryOp::Neg => Ok(block
            .append_operation(arith::negf(operand, loc))
            .result(0)
            .unwrap()
            .into()),
    }
}

/// `a % b` per Lua 5.4: floor modulo, sign follows divisor.
/// `a - floor(a/b) * b`
fn emit_lua_mod<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    lhs: Value<'c, 'a>,
    rhs: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let div = block
        .append_operation(arith::divf(lhs, rhs, loc))
        .result(0)
        .unwrap()
        .into();
    let floored = emit_libm_call(context, block, "floor", &[div], types, loc);
    let scaled = block
        .append_operation(arith::mulf(floored, rhs, loc))
        .result(0)
        .unwrap()
        .into();
    block
        .append_operation(arith::subf(lhs, scaled, loc))
        .result(0)
        .unwrap()
        .into()
}

fn emit_libm_pow<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    base: Value<'c, 'a>,
    exp: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    emit_libm_call(context, block, "pow", &[base, exp], types, loc)
}

/// Emit `llvm.call @<name>(args) : (f64, ...) -> f64`. The extern decl
/// must already exist in the module (see [`emit_libm_decls`]).
///
/// `var_callee_type` is omitted for non-variadic callees — the call-site
/// type is recovered from the symbol declaration. (printf needs it
/// because it *is* variadic.)
fn emit_libm_call<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    name: &str,
    args: &[Value<'c, 'a>],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let n = i32::try_from(args.len()).expect("libm arity fits in i32");
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
        .add_results(&[types.f64])
        .build()
        .unwrap_or_else(|_| panic!("llvm.call @{name}"));
    block.append_operation(call_op).result(0).unwrap().into()
}

/// Dispatch print to the right libc path based on the static value
/// kind of the argument. Numbers go through `printf("%g\n", v)`;
/// booleans select between `s_true`/`s_false` via `llvm.select` and
/// print through `printf("%s\n", ptr)`.
fn emit_print_value<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    value: Value<'c, 'a>,
    kind: ValueKind,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    match kind {
        ValueKind::Number => emit_printf_g(context, block, value, types, loc),
        ValueKind::Bool => emit_print_bool(context, block, value, types, loc),
        ValueKind::Nil => emit_print_nil(context, block, types, loc),
    }
}

fn emit_print_nil<'c>(
    context: &'c Context,
    block: &Block<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let nil_ptr = emit_addressof(context, block, "s_nil", types, loc);
    let fmt_ptr = emit_addressof(context, block, "fmt_str", types, loc);
    emit_printf(context, block, fmt_ptr, nil_ptr, types, loc);
}

fn emit_printf_g<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    value: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let fmt_ptr = emit_addressof(context, block, "fmt", types, loc);
    emit_printf(context, block, fmt_ptr, value, types, loc);
}

fn emit_print_bool<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    cond_i1: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let true_ptr = emit_addressof(context, block, "s_true", types, loc);
    let false_ptr = emit_addressof(context, block, "s_false", types, loc);
    let select_op = OperationBuilder::new("llvm.select", loc)
        .add_operands(&[cond_i1, true_ptr, false_ptr])
        .add_results(&[types.ptr])
        .build()
        .expect("llvm.select i1, ptr, ptr");
    let selected = block.append_operation(select_op).result(0).unwrap().into();
    let fmt_ptr = emit_addressof(context, block, "fmt_str", types, loc);
    emit_printf(context, block, fmt_ptr, selected, types, loc);
}

fn emit_addressof<'a, 'c>(
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

fn emit_printf<'a, 'c>(
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

/// Convert a Lua value to its truthiness `i1`.
///
/// Lua 5.4: only `nil` and `false` are falsy; everything else (including
/// `0` and the empty string) is truthy. Because slot kinds are static
/// in Phase 2.3b, the conversion is a compile-time three-way switch and
/// emits at most one `arith.constant`.
fn emit_truthiness<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    value: Value<'c, 'a>,
    kind: ValueKind,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    match kind {
        ValueKind::Number => {
            let attr = IntegerAttribute::new(types.i1, 1);
            block
                .append_operation(arith::constant(context, attr.into(), loc))
                .result(0)
                .unwrap()
                .into()
        }
        ValueKind::Bool => value,
        ValueKind::Nil => {
            let attr = IntegerAttribute::new(types.i1, 0);
            block
                .append_operation(arith::constant(context, attr.into(), loc))
                .result(0)
                .unwrap()
                .into()
        }
    }
}

/// Lower an `if/elseif*/else?/end` to a (possibly nested) `scf.if`.
///
/// The chain `if c1 then b1 elseif c2 then b2 else b3 end` becomes
/// `scf.if c1 { b1 } else { scf.if c2 { b2 } else { b3 } }`.
#[allow(clippy::too_many_arguments)]
fn emit_if<'a, 'c>(
    context: &'c Context,
    parent: &'a Block<'c>,
    cond: &HirExpr,
    then_body: &[HirStmt],
    elifs: &[(HirExpr, Vec<HirStmt>)],
    else_body: &Option<Vec<HirStmt>>,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let cond_kind = infer_kind(cond, locals);
    let cond_val = emit_expr(context, parent, cond, slots, locals, types, loc)?;
    let cond_i1 = emit_truthiness(context, parent, cond_val, cond_kind, types, loc);

    let then_region = build_body_region(context, then_body, slots, locals, types, loc)?;
    let else_region = build_else_region(context, elifs, else_body, slots, locals, types, loc)?;

    let if_op = scf::r#if(cond_i1, &[], then_region, else_region, loc);
    parent.append_operation(if_op);
    Ok(())
}

/// Build the `else` region for an If, recursing on the elseif chain.
#[allow(clippy::too_many_arguments)]
fn build_else_region<'c>(
    context: &'c Context,
    elifs: &[(HirExpr, Vec<HirStmt>)],
    else_body: &Option<Vec<HirStmt>>,
    slots: &[Value<'c, '_>],
    locals: &[LocalInfo],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<Region<'c>, CodegenError> {
    if let Some(((cond, body), rest)) = elifs.split_first() {
        // Nested scf.if for the next elseif/else arm.
        let region = Region::new();
        let blk = Block::new(&[]);
        let cond_kind = infer_kind(cond, locals);
        // Re-bind slots and emit into the new block.
        let blk_slots = transmute_slots(slots);
        let cond_val = emit_expr(context, &blk, cond, blk_slots, locals, types, loc)?;
        let cond_i1 = emit_truthiness(context, &blk, cond_val, cond_kind, types, loc);
        let nested_then = build_body_region(context, body, blk_slots, locals, types, loc)?;
        let nested_else =
            build_else_region(context, rest, else_body, blk_slots, locals, types, loc)?;
        blk.append_operation(scf::r#if(cond_i1, &[], nested_then, nested_else, loc));
        blk.append_operation(scf::r#yield(&[], loc));
        region.append_block(blk);
        Ok(region)
    } else {
        // Leaf: either else_body or empty `scf.yield` for absent else.
        match else_body {
            Some(body) => build_body_region(context, body, slots, locals, types, loc),
            None => {
                let region = Region::new();
                let blk = Block::new(&[]);
                blk.append_operation(scf::r#yield(&[], loc));
                region.append_block(blk);
                Ok(region)
            }
        }
    }
}

/// Build a Region containing one Block: the body, terminated by `scf.yield`.
fn build_body_region<'c>(
    context: &'c Context,
    body: &[HirStmt],
    slots: &[Value<'c, '_>],
    locals: &[LocalInfo],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<Region<'c>, CodegenError> {
    let region = Region::new();
    let blk = Block::new(&[]);
    let blk_slots = transmute_slots(slots);
    emit_stmts(context, &blk, body, blk_slots, locals, types, loc)?;
    blk.append_operation(scf::r#yield(&[], loc));
    region.append_block(blk);
    Ok(region)
}

/// Re-borrow the slot table at the inner block's lifetime. The slot
/// pointers are produced by allocas in the function entry block, which
/// dominates every inner region — so the values themselves are valid
/// inside the new block. The lifetime parameter `'a` on `Value` is just
/// the borrow lifetime of the caller's block, not the value's actual
/// scope, so a transmute is sound here.
fn transmute_slots<'a, 'c>(slots: &[Value<'c, '_>]) -> &'a [Value<'c, 'a>] {
    // SAFETY: Value<'c, '_> is covariant in the second lifetime; the
    // alloca'd pointers dominate every inner region, so referencing them
    // from inner blocks is well-typed at the MLIR level. Rust's type
    // system can't see this dominance relation, so we cast.
    unsafe { std::mem::transmute(slots) }
}

#[allow(clippy::too_many_arguments)]
fn emit_while<'a, 'c>(
    context: &'c Context,
    parent: &'a Block<'c>,
    cond: &HirExpr,
    body: &[HirStmt],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    // before region: evaluate cond, scf.condition.
    let before = Region::new();
    let before_blk = Block::new(&[]);
    let blk_slots = transmute_slots(slots);
    let cond_kind = infer_kind(cond, locals);
    let cond_val = emit_expr(context, &before_blk, cond, blk_slots, locals, types, loc)?;
    let cond_i1 = emit_truthiness(context, &before_blk, cond_val, cond_kind, types, loc);
    before_blk.append_operation(scf::condition(cond_i1, &[], loc));
    before.append_block(before_blk);

    // after region: body, scf.yield.
    let after = build_body_region(context, body, slots, locals, types, loc)?;

    parent.append_operation(scf::r#while(&[], &[], before, after, loc));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir;
    use crate::parser;

    fn lower_src(src: &str) -> HirChunk {
        let chunk = parser::parse(src).expect("parse");
        hir::lower(&chunk).expect("lower")
    }

    #[test]
    fn emit_number_constant_produces_arith_constant() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(42)")).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("arith.constant 4.200000e+01"),
            "expected arith.constant 42.0, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_addition_produces_arith_addf() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(1 + 2)")).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("arith.addf"),
            "expected arith.addf, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_print_call_produces_printf_call() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(7)")).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("llvm.call @printf"),
            "expected llvm.call @printf, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_print_1_plus_2_verifies() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(1 + 2)")).unwrap();
        assert!(
            module.as_operation().verify(),
            "module should verify for print(1 + 2)"
        );
    }

    #[test]
    fn emit_local_then_print_verifies() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("local x = 1\nprint(x + 2)")).unwrap();
        assert!(
            module.as_operation().verify(),
            "Phase 2.0 target should verify"
        );
    }

    #[test]
    fn emit_assignment_verifies_and_emits_store() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("local x = 1\nx = 2\nprint(x)")).unwrap();
        assert!(
            module.as_operation().verify(),
            "assign module should verify"
        );
        let mlir = module.as_operation().to_string();
        // Two stores expected: one for the init, one for the reassignment.
        let stores = mlir.matches("llvm.store").count();
        assert!(
            stores >= 2,
            "expected ≥2 llvm.store ops for init+assign, got {stores} in:\n{mlir}"
        );
    }

    #[test]
    fn emit_block_with_inner_local_verifies() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("local x = 1\ndo local x = 99\nprint(x) end\nprint(x)"),
        )
        .unwrap();
        assert!(
            module.as_operation().verify(),
            "block-shadowing module should verify"
        );
    }

    #[test]
    fn emit_lt_uses_arith_cmpf_olt() {
        let ctx = new_context();
        // Use the unverified variant: print(bool) lands in Step 5,
        // so the module would fail verify until then. We're only
        // checking the comparison op shape here.
        let module = emit_module_unverified(&ctx, &lower_src("print(1 < 2)")).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("arith.cmpf olt"),
            "expected arith.cmpf olt, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_eq_uses_arith_cmpf_oeq() {
        let ctx = new_context();
        let module = emit_module_unverified(&ctx, &lower_src("print(1 == 2)")).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("arith.cmpf oeq"),
            "expected arith.cmpf oeq, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_true_literal_produces_i1_constant() {
        let ctx = new_context();
        let module = emit_module_unverified(&ctx, &lower_src("print(true == true)")).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("value = true") && mlir.contains("-> i1"),
            "expected an i1 `value = true` constant, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_print_true_uses_string_format() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(true)"))
            .expect("print(true) module must verify after Step 5");
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("@s_true") && mlir.contains("@s_false"),
            "expected string globals @s_true/@s_false, got:\n{mlir}"
        );
        assert!(
            mlir.contains("@fmt_str"),
            "expected `%s\\n` format global @fmt_str, got:\n{mlir}"
        );
        assert!(
            mlir.contains("llvm.select"),
            "expected llvm.select to choose between true/false ptrs, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_print_comparison_module_verifies() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(2 >= 2)"))
            .expect("print(comparison) module must verify after Step 5");
        assert!(module.as_operation().verify());
    }

    #[test]
    fn emit_if_uses_scf_if() {
        let ctx = new_context();
        let module =
            emit_module(&ctx, &lower_src("if 1 then print(1) end")).expect("if module must verify");
        let mlir = module.as_operation().to_string();
        assert!(mlir.contains("scf.if"), "expected scf.if op, got:\n{mlir}");
    }

    #[test]
    fn emit_if_with_else_emits_both_arms() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("if 1 then print(1) else print(2) end")).unwrap();
        let mlir = module.as_operation().to_string();
        // scf.if with both arms is pretty-printed as `scf.if ... { ... } else { ... }`.
        // (Empty-yield-only regions don't print `scf.yield` explicitly.)
        assert!(
            mlir.contains("scf.if") && mlir.contains("} else {"),
            "expected scf.if with else arm, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_while_uses_scf_while() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("local i = 0\nwhile i < 3 do i = i + 1 end"),
        )
        .expect("while module must verify");
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("scf.while") && mlir.contains("scf.condition"),
            "expected scf.while + scf.condition, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_truthiness_for_number_emits_constant_true() {
        // `if 1 then ... end`: number-typed cond becomes a static `true`
        // constant before scf.if (Lua: every number is truthy).
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("if 1 then print(1) end")).unwrap();
        let mlir = module.as_operation().to_string();
        // Pretty-printer: `%true = arith.constant true` (no `value =` prefix).
        assert!(
            mlir.contains("arith.constant true"),
            "expected i1 `true` from number-truthiness, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_module_with_if_verifies() {
        let ctx = new_context();
        let module =
            emit_module(&ctx, &lower_src("if nil then print(1) else print(2) end")).unwrap();
        assert!(module.as_operation().verify());
    }

    #[test]
    fn emit_local_bool_uses_i1_slot() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("local b = true\nprint(b)"))
            .expect("local-bool module must verify");
        let mlir = module.as_operation().to_string();
        // Expect an i1 alloca, not f64. Pretty-printed shape is
        // `llvm.alloca %c1 x i1 : (i32) -> !llvm.ptr`.
        assert!(
            mlir.contains("llvm.alloca") && mlir.contains("x i1"),
            "expected i1 alloca slot for bool local, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_local_nil_module_verifies() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("local n = nil\nprint(n)"))
            .expect("local-nil module must verify");
        assert!(module.as_operation().verify());
    }

    #[test]
    fn emit_print_nil_uses_s_nil_global() {
        let ctx = new_context();
        let module =
            emit_module(&ctx, &lower_src("print(nil)")).expect("print(nil) module must verify");
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("@s_nil"),
            "expected @s_nil string global, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_subtraction_uses_arith_subf() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(3 - 1)")).unwrap();
        assert!(module.as_operation().verify());
        assert!(module.as_operation().to_string().contains("arith.subf"));
    }

    #[test]
    fn emit_multiplication_uses_arith_mulf() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(2 * 3)")).unwrap();
        assert!(module.as_operation().verify());
        assert!(module.as_operation().to_string().contains("arith.mulf"));
    }

    #[test]
    fn emit_division_uses_arith_divf() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(8 / 2)")).unwrap();
        assert!(module.as_operation().verify());
        assert!(module.as_operation().to_string().contains("arith.divf"));
    }

    #[test]
    fn emit_pow_calls_libm_pow() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(2 ^ 10)")).unwrap();
        assert!(module.as_operation().verify());
        assert!(module.as_operation().to_string().contains("llvm.call @pow"));
    }

    #[test]
    fn emit_modulo_uses_floor_for_lua_semantics() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(5 % 3)")).unwrap();
        assert!(module.as_operation().verify());
        let mlir = module.as_operation().to_string();
        assert!(mlir.contains("llvm.call @floor"), "got:\n{mlir}");
    }

    #[test]
    fn emit_unary_minus_uses_arith_negf() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(-5)")).unwrap();
        assert!(module.as_operation().verify());
        assert!(module.as_operation().to_string().contains("arith.negf"));
    }

    #[test]
    fn emit_local_emits_alloca() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("local x = 1\nprint(x)")).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("llvm.alloca"),
            "expected llvm.alloca for local, got:\n{mlir}"
        );
        assert!(
            mlir.contains("llvm.store") && mlir.contains("llvm.load"),
            "expected llvm.store/load for local, got:\n{mlir}"
        );
    }
}
