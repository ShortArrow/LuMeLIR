//! Emit MLIR from a parsed AST expression.
//!
//! Produces an MLIR `Module` using standard dialects (`arith`, `func`, `llvm`).
//! The module wraps the expression in a `func.func @main` that calls `printf`
//! to print the result.

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

use crate::parser::{BinOp, Expr, ExprKind};

use super::error::CodegenError;

/// Cached MLIR types used throughout code generation.
struct Types<'c> {
    i32: Type<'c>,
    i64: Type<'c>,
    f64: Type<'c>,
    ptr: Type<'c>,
}

/// Emit an MLIR module from an AST expression.
///
/// The emitted module wraps the expression in `func.func @main`, prints
/// the result via `printf("%g\n", <result>)`, and returns 0.
pub fn emit_module<'c>(context: &'c Context, expr: &Expr) -> Result<Module<'c>, CodegenError> {
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
    emit_main(context, &module, expr, &types, loc)?;

    if !module.as_operation().verify() {
        return Err(CodegenError::VerificationFailed);
    }

    Ok(module)
}

/// Create a new MLIR context with all dialects registered.
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
    expr: &Expr,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let main_region = Region::new();
    let main_block = Block::new(&[]);

    let result = emit_expr(context, &main_block, expr, types, loc)?;

    emit_printf_call(context, &main_block, result, types, loc);

    // return 0 : i64
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

fn emit_expr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    expr: &Expr,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    match &expr.kind {
        ExprKind::Number(n) => {
            let op = arith::constant(
                context,
                FloatAttribute::new(context, types.f64, *n).into(),
                loc,
            );
            Ok(block.append_operation(op).result(0).unwrap().into())
        }
        ExprKind::BinOp { op, lhs, rhs } => {
            let lhs_val = emit_expr(context, block, lhs, types, loc)?;
            let rhs_val = emit_expr(context, block, rhs, types, loc)?;
            emit_binop(block, *op, lhs_val, rhs_val, loc)
        }
        ExprKind::Call { callee, args } => match &callee.kind {
            ExprKind::Ident(name) if name == "print" && args.len() == 1 => {
                emit_expr(context, block, &args[0], types, loc)
            }
            _ => Err(CodegenError::UnsupportedExpr(format!(
                "unsupported call: {:?}",
                callee.kind
            ))),
        },
        ExprKind::Ident(name) => Err(CodegenError::UnsupportedExpr(format!(
            "bare identifier '{name}' not supported in Phase 1"
        ))),
    }
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
    use crate::lexer::Span;
    use crate::parser::{BinOp, Expr, ExprKind};

    fn span() -> Span {
        Span { start: 0, end: 0 }
    }

    fn number(n: f64) -> Expr {
        Expr::new(ExprKind::Number(n), span())
    }

    fn addition(lhs: Expr, rhs: Expr) -> Expr {
        Expr::new(
            ExprKind::BinOp {
                op: BinOp::Add,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            span(),
        )
    }

    fn print_call(arg: Expr) -> Expr {
        Expr::new(
            ExprKind::Call {
                callee: Box::new(Expr::new(ExprKind::Ident("print".into()), span())),
                args: vec![arg],
            },
            span(),
        )
    }

    #[test]
    fn emit_number_constant_produces_arith_constant() {
        let ctx = new_context();
        let module = emit_module(&ctx, &print_call(number(42.0))).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("arith.constant 4.200000e+01"),
            "expected arith.constant 42.0, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_addition_produces_arith_addf() {
        let ctx = new_context();
        let module = emit_module(&ctx, &print_call(addition(number(1.0), number(2.0)))).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("arith.addf"),
            "expected arith.addf, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_print_call_produces_printf_call() {
        let ctx = new_context();
        let module = emit_module(&ctx, &print_call(number(7.0))).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("llvm.call @printf"),
            "expected llvm.call @printf, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_print_1_plus_2_verifies() {
        let ctx = new_context();
        let module = emit_module(&ctx, &print_call(addition(number(1.0), number(2.0)))).unwrap();
        assert!(
            module.as_operation().verify(),
            "module should verify for print(1 + 2)"
        );
    }

    #[test]
    fn emit_from_parsed_source_verifies() {
        let ctx = new_context();
        let expr = crate::parser::parse("print(1 + 2)").unwrap();
        let module = emit_module(&ctx, &expr).unwrap();
        assert!(
            module.as_operation().verify(),
            "module from parsed source should verify"
        );
    }

    #[test]
    fn emit_unsupported_bare_ident_errors() {
        let ctx = new_context();
        let expr = Expr::new(ExprKind::Ident("x".into()), span());
        assert!(emit_module(&ctx, &expr).is_err());
    }
}
