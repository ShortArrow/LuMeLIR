//! Emit MLIR from a resolved HIR chunk.
//!
//! Produces an MLIR `Module` using standard dialects (`arith`, `func`, `llvm`).
//! All statements run in `func.func @main`. Locals are stack slots created
//! with `llvm.alloca` at function entry.

use melior::{
    Context,
    dialect::{
        DialectRegistry, arith, func, llvm,
        ods::llvm::{AddressOfOperationBuilder, GlobalOperationBuilder, LLVMFuncOperationBuilder},
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

use crate::hir::{Builtin, HirChunk, HirExpr, HirExprKind, HirStmtKind, LocalId};
use crate::parser::BinOp;

use super::error::CodegenError;

struct Types<'c> {
    i32: Type<'c>,
    i64: Type<'c>,
    f64: Type<'c>,
    ptr: Type<'c>,
}

/// Emit an MLIR module from a resolved HIR chunk.
pub fn emit_module<'c>(context: &'c Context, chunk: &HirChunk) -> Result<Module<'c>, CodegenError> {
    let loc = Location::unknown(context);
    let module = Module::new(loc);

    let i8_type = IntegerType::new(context, 8).into();
    let types = Types {
        i32: IntegerType::new(context, 32).into(),
        i64: IntegerType::new(context, 64).into(),
        f64: Type::float64(context),
        ptr: llvm::r#type::pointer(context, 0),
    };

    emit_fmt_global(context, &module, i8_type, loc);
    emit_printf_decl(context, &module, &types, loc);
    emit_main(context, &module, chunk, &types, loc)?;

    if !module.as_operation().verify() {
        return Err(CodegenError::VerificationFailed);
    }

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
    let array_type = llvm::r#type::array(i8_type, 4);
    let global_op = GlobalOperationBuilder::new(context, loc)
        .initializer(Region::new())
        .global_type(TypeAttribute::new(array_type))
        .sym_name(StringAttribute::new(context, "fmt"))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::Internal,
        ))
        .constant(melior::ir::attribute::Attribute::unit(context))
        .value(StringAttribute::new(context, "%g\n\0").into())
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
        .map(|_| emit_alloca_slot(context, &main_block, types, loc))
        .collect();

    for stmt in &chunk.stmts {
        match &stmt.kind {
            HirStmtKind::LocalInit { id, value } => {
                let v = emit_expr(context, &main_block, value, &slots, types, loc)?;
                emit_store(&main_block, v, slots[id.0], loc);
            }
            HirStmtKind::ExprStmt(expr) => {
                emit_expr_stmt(context, &main_block, expr, &slots, types, loc)?;
            }
        }
    }

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

fn emit_alloca_slot<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let one = arith::constant(context, IntegerAttribute::new(types.i32, 1).into(), loc);
    let one_val: Value<'c, 'a> = block.append_operation(one).result(0).unwrap().into();

    let alloca = OperationBuilder::new("llvm.alloca", loc)
        .add_operands(&[one_val])
        .add_attributes(&[(
            Identifier::new(context, "elem_type"),
            TypeAttribute::new(types.f64).into(),
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
        HirExprKind::Local(LocalId(idx)) => Ok(emit_load(block, slots[*idx], types.f64, loc)),
        HirExprKind::BinOp { op, lhs, rhs } => {
            let lhs_val = emit_expr(context, block, lhs, slots, types, loc)?;
            let rhs_val = emit_expr(context, block, rhs, slots, types, loc)?;
            emit_binop(block, *op, lhs_val, rhs_val, loc)
        }
        HirExprKind::Call { builtin, args } => match builtin {
            Builtin::Print => {
                let arg_val = emit_expr(context, block, &args[0], slots, types, loc)?;
                emit_printf_call(context, block, arg_val, types, loc);
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
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    emit_expr(context, block, expr, slots, types, loc)?;
    Ok(())
}

fn emit_binop<'a, 'c>(
    block: &'a Block<'c>,
    op: BinOp,
    lhs: Value<'c, 'a>,
    rhs: Value<'c, 'a>,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    match op {
        BinOp::Add => {
            let add_op = arith::addf(lhs, rhs, loc);
            Ok(block.append_operation(add_op).result(0).unwrap().into())
        }
    }
}

fn emit_printf_call<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    value: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let addr_op = AddressOfOperationBuilder::new(context, loc)
        .res(types.ptr)
        .global_name(FlatSymbolRefAttribute::new(context, "fmt"))
        .build();
    let fmt_ptr = block
        .append_operation(addr_op.into())
        .result(0)
        .unwrap()
        .into();

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
