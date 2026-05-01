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
    Builtin, Callee, FuncId, HirChunk, HirExpr, HirExprKind, HirFunction, HirStmt, HirStmtKind,
    LocalId, LocalInfo, ValueKind, infer_kind,
};
use crate::parser::{BinOp, UnaryOp};

use super::error::CodegenError;

struct Types<'c> {
    i1: Type<'c>,
    i32: Type<'c>,
    i64: Type<'c>,
    f64: Type<'c>,
    ptr: Type<'c>,
    /// Phase 2.7a (ADR 0024): string literal payload → MLIR global
    /// symbol name. Built by [`collect_string_pool`] before any
    /// `emit_*` runs and read at every `HirExprKind::Str` use site.
    string_pool: std::collections::HashMap<String, String>,
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
    // Phase 2.7a: collect unique string literals across the chunk so
    // the codegen can emit one global per payload up front, before
    // any `HirExprKind::Str` use site asks for an `addressof`.
    let string_pool = collect_string_pool(chunk);
    let types = Types {
        i1: IntegerType::new(context, 1).into(),
        i32: IntegerType::new(context, 32).into(),
        i64: IntegerType::new(context, 64).into(),
        f64: Type::float64(context),
        ptr: llvm::r#type::pointer(context, 0),
        string_pool,
    };

    emit_fmt_global(context, &module, i8_type, loc);
    emit_user_string_globals(context, &module, i8_type, &types, loc);
    emit_printf_decl(context, &module, &types, loc);
    emit_libm_decls(context, &module, &types, loc);
    emit_strlen_decl(context, &module, &types, loc);
    emit_string_runtime_decls(context, &module, &types, loc);
    for hir_fn in &chunk.functions {
        emit_function(context, &module, hir_fn, &chunk.functions, &types, loc)?;
    }
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
    // Phase 2.7c (ADR 0026): `%.14g` is the Lua-spec format for
    // `tostring(number)`. The trailing `\0` lets snprintf consume it
    // as a C string.
    emit_string_global(context, module, i8_type, "fmt_tostring_g", "%.14g\0", loc);
    // Phase 2.7e (ADR 0028): `%lf` parses an f64 from a string for
    // `tonumber(string)` via `sscanf`.
    emit_string_global(context, module, i8_type, "fmt_tonumber_lf", "%lf\0", loc);
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

/// Phase 2.7a (ADR 0024): walk the HIR, collect every `HirExprKind::Str`
/// payload, deduplicate, and assign a stable `lstr_<i>` global symbol
/// name to each. The order is the BTreeSet's natural sort so the
/// emitted IR is deterministic regardless of source order.
fn collect_string_pool(chunk: &HirChunk) -> std::collections::HashMap<String, String> {
    use std::collections::BTreeSet;
    fn visit_expr(e: &HirExpr, set: &mut BTreeSet<String>) {
        match &e.kind {
            HirExprKind::Str(s) => {
                set.insert(s.clone());
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                visit_expr(lhs, set);
                visit_expr(rhs, set);
            }
            HirExprKind::UnaryOp { operand, .. } => visit_expr(operand, set),
            HirExprKind::Call { args, .. } => {
                for a in args {
                    visit_expr(a, set);
                }
            }
            _ => {}
        }
    }
    fn visit_stmt(s: &HirStmt, set: &mut BTreeSet<String>) {
        match &s.kind {
            HirStmtKind::LocalInit { value, .. } | HirStmtKind::Assign { value, .. } => {
                visit_expr(value, set);
            }
            HirStmtKind::Block { stmts } => {
                for st in stmts {
                    visit_stmt(st, set);
                }
            }
            HirStmtKind::If {
                cond,
                then_body,
                elifs,
                else_body,
            } => {
                visit_expr(cond, set);
                for st in then_body {
                    visit_stmt(st, set);
                }
                for (c, b) in elifs {
                    visit_expr(c, set);
                    for st in b {
                        visit_stmt(st, set);
                    }
                }
                if let Some(b) = else_body {
                    for st in b {
                        visit_stmt(st, set);
                    }
                }
            }
            HirStmtKind::While { cond, body, .. } => {
                visit_expr(cond, set);
                for st in body {
                    visit_stmt(st, set);
                }
            }
            HirStmtKind::ForNumeric {
                start,
                stop,
                step,
                body,
                ..
            } => {
                visit_expr(start, set);
                visit_expr(stop, set);
                visit_expr(step, set);
                for st in body {
                    visit_stmt(st, set);
                }
            }
            HirStmtKind::ExprStmt(e) => visit_expr(e, set),
            HirStmtKind::MultiAssignFromCall { args, .. } => {
                for a in args {
                    visit_expr(a, set);
                }
            }
        }
    }
    let mut set: BTreeSet<String> = BTreeSet::new();
    for st in &chunk.stmts {
        visit_stmt(st, &mut set);
    }
    for f in &chunk.functions {
        for st in &f.body {
            visit_stmt(st, &mut set);
        }
    }
    set.into_iter()
        .enumerate()
        .map(|(i, s)| (s, format!("lstr_{i}")))
        .collect()
}

/// Phase 2.7a (ADR 0024): emit one `llvm.mlir.global` per entry in
/// the string pool, terminating each payload with a NUL byte so it
/// can be passed to libc `printf`/`strlen` directly.
fn emit_user_string_globals<'c>(
    context: &'c Context,
    module: &Module<'c>,
    i8_type: Type<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let mut entries: Vec<(&String, &String)> = types.string_pool.iter().collect();
    entries.sort_by(|a, b| a.1.cmp(b.1));
    for (payload, name) in entries {
        let mut value = payload.clone();
        value.push('\0');
        emit_string_global(context, module, i8_type, name, &value, loc);
    }
}

/// Phase 2.7a (ADR 0024): declare libc `strlen(ptr) -> i64` so the
/// `#s` length operator can compile to a single call without
/// statically tracking each string's byte length.
fn emit_strlen_decl<'c>(
    context: &'c Context,
    module: &Module<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let strlen_ty = llvm::r#type::function(types.i64, &[types.ptr], false);
    let strlen_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "strlen"))
        .function_type(TypeAttribute::new(strlen_ty))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(strlen_op.into());
}

/// Phase 2.7b (ADR 0025): declare libc `malloc`, `memcpy`, and
/// `strcmp` for the runtime side of `..` concat and String equality.
fn emit_string_runtime_decls<'c>(
    context: &'c Context,
    module: &Module<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let malloc_ty = llvm::r#type::function(types.ptr, &[types.i64], false);
    let malloc_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "malloc"))
        .function_type(TypeAttribute::new(malloc_ty))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(malloc_op.into());

    let memcpy_ty = llvm::r#type::function(types.ptr, &[types.ptr, types.ptr, types.i64], false);
    let memcpy_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "memcpy"))
        .function_type(TypeAttribute::new(memcpy_ty))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(memcpy_op.into());

    let strcmp_ty = llvm::r#type::function(types.i32, &[types.ptr, types.ptr], false);
    let strcmp_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "strcmp"))
        .function_type(TypeAttribute::new(strcmp_ty))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(strcmp_op.into());

    // snprintf(ptr, i64, ptr, ...) -> i32  (variadic, Phase 2.7c)
    let snprintf_ty = llvm::r#type::function(types.i32, &[types.ptr, types.i64, types.ptr], true);
    let snprintf_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "snprintf"))
        .function_type(TypeAttribute::new(snprintf_ty))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(snprintf_op.into());

    // sscanf(ptr, ptr, ...) -> i32  (variadic, Phase 2.7e)
    let sscanf_ty = llvm::r#type::function(types.i32, &[types.ptr, types.ptr], true);
    let sscanf_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "sscanf"))
        .function_type(TypeAttribute::new(sscanf_ty))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(sscanf_op.into());
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

/// MLIR type for a function parameter of static [`ValueKind`]. Number
/// is f64; Function(arity) is `!func.func<(f64, ...) -> f64>` (arity
/// f64 params, single Number return — Phase 2.5b.2 fixes the return
/// to Number).
fn param_mlir_type<'c>(context: &'c Context, kind: ValueKind, types: &Types<'c>) -> Type<'c> {
    match kind {
        ValueKind::Number => types.f64,
        ValueKind::Function(arity) => {
            let p_types: Vec<Type<'c>> = (0..arity).map(|_| types.f64).collect();
            FunctionType::new(context, &p_types, &[types.f64]).into()
        }
        ValueKind::Bool | ValueKind::Nil => types.i1,
        // Phase 2.7a (ADR 0024): a String value is a `!llvm.ptr`
        // into the static C-string pool.
        ValueKind::String => types.ptr,
    }
}

/// MLIR result types for a function whose HIR return-kinds list is
/// `ret_kinds`. Phase 2.5b.3 (ADR 0019) added Function returns; 2.5e
/// (ADR 0020) added Bool and Nil; 2.5d (ADR 0021) generalises to a
/// `Vec` so multi-result functions emit a result type per position.
fn ret_mlir_types<'c>(
    context: &'c Context,
    ret_kinds: &[ValueKind],
    types: &Types<'c>,
) -> Vec<Type<'c>> {
    ret_kinds
        .iter()
        .map(|k| match k {
            ValueKind::Number => types.f64,
            ValueKind::Bool | ValueKind::Nil => types.i1,
            ValueKind::Function(arity) => {
                let p_types: Vec<Type<'c>> = (0..*arity).map(|_| types.f64).collect();
                FunctionType::new(context, &p_types, &[types.f64]).into()
            }
            ValueKind::String => types.ptr,
        })
        .collect()
}

/// `builtin.unrealized_conversion_cast` — used to bridge `!func.func`
/// values to/from `!llvm.ptr` so the value can be stored in an LLVM
/// alloca slot. Subsequent `--convert-func-to-llvm` lowers `!func.func`
/// to `!llvm.ptr`, and `--reconcile-unrealized-casts` then erases the
/// pair as a no-op. Phase 2.5b.3 (ADR 0019).
fn emit_unrealized_cast<'a, 'c>(
    block: &'a Block<'c>,
    value: Value<'c, 'a>,
    target_ty: Type<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let op = OperationBuilder::new("builtin.unrealized_conversion_cast", loc)
        .add_operands(&[value])
        .add_results(&[target_ty])
        .build()
        .expect("builtin.unrealized_conversion_cast");
    block.append_operation(op).result(0).unwrap().into()
}

/// Emit a `func.func @<mangled_name>(...) -> (...)` for a user-defined
/// function. Phase 2.5a constrains every parameter and the optional
/// return value to `Number` (f64). The `_returned` / `_ret_value`
/// slots that the HIR's return-desugar relies on live as ordinary
/// locals at the head of `hir_fn.locals` (declared by `lower_function_body`).
fn emit_function<'c>(
    context: &'c Context,
    module: &Module<'c>,
    hir_fn: &HirFunction,
    functions: &[HirFunction],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let region = Region::new();
    // Phase 2.5b.2: each parameter's MLIR type is determined by its
    // ValueKind. Number → f64, Function(arity) → !func.func<...>.
    let param_types: Vec<Type<'c>> = hir_fn
        .params
        .iter()
        .map(|info| param_mlir_type(context, info.kind, types))
        .collect();
    let block_args: Vec<(Type<'c>, Location<'c>)> = param_types.iter().map(|t| (*t, loc)).collect();
    let block = Block::new(&block_args);

    // Allocate slots for every local. Function-kind params are special:
    // their `slot` is the block argument value itself (not a pointer);
    // we never load/store those. Non-param Function-kind locals get an
    // i1 placeholder slot — their value is reproduced at every use site
    // via `func.constant` from `LocalInfo.func_id`.
    let slots: Vec<Value<'c, '_>> = hir_fn
        .locals
        .iter()
        .enumerate()
        .map(|(i, info)| match info.kind {
            ValueKind::Function(_) if i < hir_fn.params.len() => block.argument(i).unwrap().into(),
            _ => emit_alloca_slot_for_kind(context, &block, info.kind, types, loc),
        })
        .collect();

    // Store each Number parameter (block argument) into its alloca slot.
    // Function-kind params are already wired up in `slots` above.
    for (i, info) in hir_fn.params.iter().enumerate() {
        if !matches!(info.kind, ValueKind::Function(_)) {
            let arg: Value<'c, '_> = block.argument(i).unwrap().into();
            emit_store(&block, arg, slots[i], loc);
        }
    }

    emit_stmts(
        context,
        &block,
        &hir_fn.body,
        &slots,
        &hir_fn.locals,
        functions,
        types,
        hir_fn.params.len(),
        loc,
    )?;

    // Trailing return: load each `_ret_value_N` slot (Phase 2.5d
    // ADR 0021) — slots are placed sequentially after `_returned`,
    // i.e. at `params.len() + 1 + i` for the i-th return position.
    // Function-kind returns load the slot as `ptr` and bridge back
    // to the function type via unrealized_conversion_cast (ADR 0019).
    let mut ret_values: Vec<Value<'c, '_>> = Vec::with_capacity(hir_fn.ret_kinds.len());
    for (i, k) in hir_fn.ret_kinds.iter().enumerate() {
        let ret_value_idx = hir_fn.params.len() + 1 + i;
        let v = match k {
            ValueKind::Number => emit_load(&block, slots[ret_value_idx], types.f64, loc),
            ValueKind::Bool | ValueKind::Nil => {
                emit_load(&block, slots[ret_value_idx], types.i1, loc)
            }
            ValueKind::Function(arity) => {
                let ptr_val = emit_load(&block, slots[ret_value_idx], types.ptr, loc);
                let p_types: Vec<Type<'c>> = (0..*arity).map(|_| types.f64).collect();
                let fn_ty: Type<'c> = FunctionType::new(context, &p_types, &[types.f64]).into();
                emit_unrealized_cast(&block, ptr_val, fn_ty, loc)
            }
            // Phase 2.7a: String returns surface the slot's `ptr`
            // value directly — no cast is needed since the slot type
            // already matches the function signature.
            ValueKind::String => emit_load(&block, slots[ret_value_idx], types.ptr, loc),
        };
        ret_values.push(v);
    }
    block.append_operation(func::r#return(&ret_values, loc));
    region.append_block(block);

    let ret_types = ret_mlir_types(context, &hir_fn.ret_kinds, types);
    let fn_type = FunctionType::new(context, &param_types, &ret_types);
    let func_op = func::func(
        context,
        StringAttribute::new(context, &hir_fn.mangled_name),
        TypeAttribute::new(fn_type.into()),
        region,
        &[],
        loc,
    );
    module.body().append_operation(func_op);
    Ok(())
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
        &chunk.functions,
        types,
        0,
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

#[allow(clippy::too_many_arguments)]
fn emit_stmts<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    stmts: &[HirStmt],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    for stmt in stmts {
        emit_stmt(
            context, block, stmt, slots, locals, functions, types, params_len, loc,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_stmt<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    stmt: &HirStmt,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    match &stmt.kind {
        HirStmtKind::LocalInit { id, value } | HirStmtKind::Assign { id, value } => {
            // Phase 2.5b.3: Function-kind locals fall into three buckets:
            //   - Param slot (id < params_len): the slot already holds
            //     the block argument SSA value; never write.
            //   - Known FuncId (`local f = function() end` or alias):
            //     the value is reproducible via `func.constant` at use
            //     sites; we skip the store as a small optimisation.
            //   - Otherwise (e.g. `local g = get_f()` or the synthetic
            //     `_ret_value` slot): evaluate the rhs, bridge the
            //     `!func.func` value to `!llvm.ptr`, and store.
            let info = &locals[id.0];
            match info.kind {
                ValueKind::Function(_) => {
                    let is_param = id.0 < params_len;
                    let known_fid = info.func_id.is_some();
                    if !is_param && !known_fid {
                        let v = emit_expr(
                            context, block, value, slots, locals, functions, types, params_len, loc,
                        )?;
                        let ptr_val = emit_unrealized_cast(block, v, types.ptr, loc);
                        emit_store(block, ptr_val, slots[id.0], loc);
                    }
                }
                _ => {
                    let v = emit_expr(
                        context, block, value, slots, locals, functions, types, params_len, loc,
                    )?;
                    emit_store(block, v, slots[id.0], loc);
                }
            }
        }
        HirStmtKind::Block { stmts } => {
            emit_stmts(
                context, block, stmts, slots, locals, functions, types, params_len, loc,
            )?;
        }
        HirStmtKind::If {
            cond,
            then_body,
            elifs,
            else_body,
        } => {
            emit_if(
                context, block, cond, then_body, elifs, else_body, slots, locals, functions, types,
                params_len, loc,
            )?;
        }
        HirStmtKind::While {
            cond,
            body,
            break_id,
        } => {
            emit_while(
                context, block, cond, body, *break_id, slots, locals, functions, types, params_len,
                loc,
            )?;
        }
        HirStmtKind::ForNumeric {
            var_id,
            start,
            stop,
            step,
            body,
            break_id,
        } => {
            emit_for_numeric(
                context, block, *var_id, start, stop, step, body, *break_id, slots, locals,
                functions, types, params_len, loc,
            )?;
        }
        HirStmtKind::ExprStmt(expr) => {
            emit_expr_stmt(
                context, block, expr, slots, locals, functions, types, params_len, loc,
            )?;
        }
        HirStmtKind::MultiAssignFromCall {
            dst_ids,
            callee,
            args,
        } => {
            emit_multi_assign_from_call(
                context, block, dst_ids, *callee, args, slots, locals, functions, types,
                params_len, loc,
            )?;
        }
    }
    Ok(())
}

/// Phase 2.5d (ADR 0021): emit a multi-result `func.call` and store
/// each result into the matching destination slot. Currently only
/// `Callee::User` is supported — `Callee::Indirect` lacks
/// statically-tracked ret arity, and builtins have a fixed shape.
#[allow(clippy::too_many_arguments)]
fn emit_multi_assign_from_call<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    dst_ids: &[LocalId],
    callee: Callee,
    args: &[HirExpr],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let Callee::User(FuncId(fid)) = callee else {
        unreachable!("HIR rejects non-User callees in MultiAssignFromCall");
    };
    let target = &functions[fid];
    let mut arg_vals: Vec<Value<'c, 'a>> = Vec::with_capacity(args.len());
    for a in args {
        arg_vals.push(emit_expr(
            context, block, a, slots, locals, functions, types, params_len, loc,
        )?);
    }
    let ret_types = ret_mlir_types(context, &target.ret_kinds, types);
    let call_op = func::call(
        context,
        FlatSymbolRefAttribute::new(context, &target.mangled_name),
        &arg_vals,
        &ret_types,
        loc,
    );
    let op_ref = block.append_operation(call_op);
    for (i, dst) in dst_ids.iter().enumerate() {
        let v: Value<'c, 'a> = op_ref.result(i).unwrap().into();
        let info = &locals[dst.0];
        // Function-kind slots need the same ucast bridge used by
        // ordinary LocalInit (Phase 2.5b.3).
        let store_val = if matches!(info.kind, ValueKind::Function(_)) {
            emit_unrealized_cast(block, v, types.ptr, loc)
        } else {
            v
        };
        emit_store(block, store_val, slots[dst.0], loc);
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
        // Phase 2.7a (ADR 0024): a String slot stores a `ptr` to a
        // static C-string global.
        ValueKind::String => types.ptr,
        // Phase 2.5b.3 (ADR 0019): Function values stored as opaque
        // pointers — `!func.func<...>` values are bridged via
        // `unrealized_conversion_cast` at store/load.
        ValueKind::Function(_) => types.ptr,
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

#[allow(clippy::too_many_arguments)]
fn emit_expr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    expr: &HirExpr,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
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
        // Phase 2.7a (ADR 0024): a string literal materialises as
        // `llvm.mlir.addressof @lstr_<i>`. The corresponding global
        // was emitted by `emit_user_string_globals` at module top.
        HirExprKind::Str(s) => {
            let global_name = types
                .string_pool
                .get(s)
                .expect("collect_string_pool seeds every literal before codegen");
            Ok(emit_addressof(context, block, global_name, types, loc))
        }
        HirExprKind::Local(LocalId(idx)) => {
            let info = &locals[*idx];
            match info.kind {
                ValueKind::Number => Ok(emit_load(block, slots[*idx], types.f64, loc)),
                ValueKind::Bool | ValueKind::Nil => {
                    Ok(emit_load(block, slots[*idx], types.i1, loc))
                }
                ValueKind::String => Ok(emit_load(block, slots[*idx], types.ptr, loc)),
                ValueKind::Function(arity) => {
                    // Three buckets (Phase 2.5b.3, ADR 0019):
                    //   - Function param: slots[idx] is the block arg.
                    //   - Known FuncId: re-emit `func.constant`; the slot
                    //     value is never read on this path.
                    //   - Otherwise: alloca'd `ptr` slot — load it and
                    //     bridge back to `!func.func` via ucast.
                    if *idx < params_len {
                        Ok(slots[*idx])
                    } else if let Some(fid) = info.func_id {
                        let target = &functions[fid.0];
                        let p_types: Vec<Type<'c>> = target
                            .params
                            .iter()
                            .map(|p| param_mlir_type(context, p.kind, types))
                            .collect();
                        let r_types = ret_mlir_types(context, &target.ret_kinds, types);
                        let fn_type = FunctionType::new(context, &p_types, &r_types);
                        let constant = func::constant(
                            context,
                            FlatSymbolRefAttribute::new(context, &target.mangled_name),
                            fn_type,
                            loc,
                        );
                        Ok(block.append_operation(constant).result(0).unwrap().into())
                    } else {
                        let ptr_val = emit_load(block, slots[*idx], types.ptr, loc);
                        let p_types: Vec<Type<'c>> = (0..arity).map(|_| types.f64).collect();
                        let fn_ty: Type<'c> =
                            FunctionType::new(context, &p_types, &[types.f64]).into();
                        Ok(emit_unrealized_cast(block, ptr_val, fn_ty, loc))
                    }
                }
            }
        }
        HirExprKind::BinOp { op, lhs, rhs } => {
            // `and`/`or` short-circuit — must NOT eagerly evaluate rhs.
            if matches!(op, BinOp::And | BinOp::Or) {
                return emit_short_circuit(
                    context, block, *op, lhs, rhs, slots, locals, functions, types, params_len, loc,
                );
            }
            let lhs_val = emit_expr(
                context, block, lhs, slots, locals, functions, types, params_len, loc,
            )?;
            let rhs_val = emit_expr(
                context, block, rhs, slots, locals, functions, types, params_len, loc,
            )?;
            // Phase 2.7b (ADR 0025): String operands need a different
            // backend than the f64-typed `arith.cmpf` / arithmetic
            // path. Concat is always String; Eq/Ne dispatches on the
            // operand kind so the runtime path is `strcmp`.
            if matches!(op, BinOp::Concat) {
                return Ok(emit_concat(context, block, lhs_val, rhs_val, types, loc));
            }
            if matches!(
                op,
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
            ) && infer_kind(lhs, locals, functions) == ValueKind::String
            {
                // Phase 2.7b/d: equality and ordering on String
                // operands both go through `strcmp` + integer
                // compare. Same helper, different `arith.cmpi`
                // predicate per BinOp.
                return Ok(emit_string_cmp(
                    context, block, *op, lhs_val, rhs_val, types, loc,
                ));
            }
            emit_binop(context, block, *op, lhs_val, rhs_val, types, loc)
        }
        HirExprKind::UnaryOp { op, operand } => {
            // `not` accepts any kind: convert operand to truthiness i1
            // first, then flip with xori. Other unaries (currently `-`)
            // pass through emit_unary unchanged.
            if matches!(op, UnaryOp::Not) {
                let kind = infer_kind(operand, locals, functions);
                let v = emit_expr(
                    context, block, operand, slots, locals, functions, types, params_len, loc,
                )?;
                let truth = emit_truthiness(context, block, v, kind, types, loc);
                let one = arith::constant(context, IntegerAttribute::new(types.i1, 1).into(), loc);
                let one_val: Value<'c, 'a> = block.append_operation(one).result(0).unwrap().into();
                return Ok(block
                    .append_operation(arith::xori(truth, one_val, loc))
                    .result(0)
                    .unwrap()
                    .into());
            }
            let v = emit_expr(
                context, block, operand, slots, locals, functions, types, params_len, loc,
            )?;
            emit_unary(context, block, *op, v, types, loc)
        }
        HirExprKind::Call { callee, args } => match callee {
            Callee::Builtin(Builtin::Print) => {
                let kind = infer_kind(&args[0], locals, functions);
                let arg_val = emit_expr(
                    context, block, &args[0], slots, locals, functions, types, params_len, loc,
                )?;
                emit_print_value(context, block, arg_val, kind, types, loc);
                Ok(arg_val)
            }
            // Phase 2.7c (ADR 0026): `tostring(x)` materialises a
            // String for the four supported kinds. Function values
            // are rejected at HIR-time and never reach codegen.
            Callee::Builtin(Builtin::ToString) => {
                let kind = infer_kind(&args[0], locals, functions);
                let arg_val = emit_expr(
                    context, block, &args[0], slots, locals, functions, types, params_len, loc,
                )?;
                Ok(emit_tostring(context, block, arg_val, kind, types, loc))
            }
            // Phase 2.7e (ADR 0028): `tonumber(x)`. Number identity
            // path; String dispatched through `sscanf("%lf")` with a
            // NaN sentinel on parse failure.
            Callee::Builtin(Builtin::ToNumber) => {
                let kind = infer_kind(&args[0], locals, functions);
                let arg_val = emit_expr(
                    context, block, &args[0], slots, locals, functions, types, params_len, loc,
                )?;
                Ok(emit_tonumber(context, block, arg_val, kind, types, loc))
            }
            Callee::User(FuncId(fid)) => {
                let target = &functions[*fid];
                let mut arg_vals: Vec<Value<'c, 'a>> = Vec::with_capacity(args.len());
                for a in args {
                    arg_vals.push(emit_expr(
                        context, block, a, slots, locals, functions, types, params_len, loc,
                    )?);
                }
                let ret_types = ret_mlir_types(context, &target.ret_kinds, types);
                let call_op = func::call(
                    context,
                    FlatSymbolRefAttribute::new(context, &target.mangled_name),
                    &arg_vals,
                    &ret_types,
                    loc,
                );
                let op_ref = block.append_operation(call_op);
                if !target.ret_kinds.is_empty() {
                    // Multi-result calls in expression position
                    // truncate to the first value (Lua semantics).
                    Ok(op_ref.result(0).unwrap().into())
                } else {
                    // Void call — synthesise a placeholder f64 0.0
                    // value. It is never read; caller's `infer_kind`
                    // returned Number, so consistent with our type
                    // model.
                    let zero = arith::constant(
                        context,
                        FloatAttribute::new(context, types.f64, 0.0).into(),
                        loc,
                    );
                    Ok(block.append_operation(zero).result(0).unwrap().into())
                }
            }
            Callee::Indirect(LocalId(idx)) => {
                // Phase 2.5b.2/b.3: a Function-kind local with no
                // statically known FuncId. Either it is a function
                // parameter (slot is the block argument) or it was
                // bound from a call/expression and lives in an
                // alloca'd `ptr` slot.
                let callee_val = if *idx < params_len {
                    slots[*idx]
                } else {
                    let info = &locals[*idx];
                    let arity = match info.kind {
                        ValueKind::Function(a) => a,
                        _ => unreachable!("Callee::Indirect on non-Function local"),
                    };
                    let ptr_val = emit_load(block, slots[*idx], types.ptr, loc);
                    let p_types: Vec<Type<'c>> = (0..arity).map(|_| types.f64).collect();
                    let fn_ty: Type<'c> = FunctionType::new(context, &p_types, &[types.f64]).into();
                    emit_unrealized_cast(block, ptr_val, fn_ty, loc)
                };
                let mut arg_vals: Vec<Value<'c, 'a>> = Vec::with_capacity(args.len());
                for a in args {
                    arg_vals.push(emit_expr(
                        context, block, a, slots, locals, functions, types, params_len, loc,
                    )?);
                }
                let call_op = func::call_indirect(callee_val, &arg_vals, &[types.f64], loc);
                let op_ref = block.append_operation(call_op);
                Ok(op_ref.result(0).unwrap().into())
            }
        },
        HirExprKind::FunctionRef(FuncId(id)) => {
            // Phase 2.5b.3: a function reference materialises as a
            // `func.constant` SSA value so it can flow through stores
            // (e.g. into the synthetic `_ret_value` slot when a
            // function body does `return function() ... end`) and into
            // call-site arguments. Storage-skipping LocalInit paths
            // never reach here, so this is always a real use.
            let target = &functions[*id];
            let p_types: Vec<Type<'c>> = target
                .params
                .iter()
                .map(|p| param_mlir_type(context, p.kind, types))
                .collect();
            let r_types = ret_mlir_types(context, &target.ret_kinds, types);
            let fn_type = FunctionType::new(context, &p_types, &r_types);
            let constant = func::constant(
                context,
                FlatSymbolRefAttribute::new(context, &target.mangled_name),
                fn_type,
                loc,
            );
            Ok(block.append_operation(constant).result(0).unwrap().into())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_expr_stmt<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    expr: &HirExpr,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    emit_expr(
        context, block, expr, slots, locals, functions, types, params_len, loc,
    )?;
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
        // Phase 2.2c (ADR 0022): `a // b == floor(a / b)`.
        BinOp::FloorDiv => {
            let div = block
                .append_operation(arith::divf(lhs, rhs, loc))
                .result(0)
                .unwrap()
                .into();
            emit_libm_call(context, block, "floor", &[div], types, loc)
        }
        // Phase 2.2c bitwise — convert each f64 operand to i64 via
        // `arith.fptosi`, do the integer op, then convert back via
        // `arith.sitofp`. Lua 5.4 bitwise ops are integer-typed; we
        // truncate the f64 representation just like `math.tointeger`.
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
            let li = emit_f2i(block, lhs, types, loc);
            let ri = emit_f2i(block, rhs, types, loc);
            let res_i = match op {
                BinOp::BitAnd => block
                    .append_operation(arith::andi(li, ri, loc))
                    .result(0)
                    .unwrap()
                    .into(),
                BinOp::BitOr => block
                    .append_operation(arith::ori(li, ri, loc))
                    .result(0)
                    .unwrap()
                    .into(),
                BinOp::BitXor => block
                    .append_operation(arith::xori(li, ri, loc))
                    .result(0)
                    .unwrap()
                    .into(),
                BinOp::Shl => block
                    .append_operation(arith::shli(li, ri, loc))
                    .result(0)
                    .unwrap()
                    .into(),
                BinOp::Shr => block
                    .append_operation(arith::shrsi(li, ri, loc))
                    .result(0)
                    .unwrap()
                    .into(),
                _ => unreachable!(),
            };
            emit_i2f(block, res_i, types, loc)
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => {
            let predicate = cmpf_predicate(op);
            block
                .append_operation(arith::cmpf(context, predicate, lhs, rhs, loc))
                .result(0)
                .unwrap()
                .into()
        }
        BinOp::And | BinOp::Or => {
            // Step 2 stub — short-circuit codegen lands in Step 4.
            // emit_expr should bypass emit_binop for these via the
            // emit_short_circuit special case once that exists.
            unimplemented!("BinOp::And/Or — Phase 2.3c Step 4");
        }
        BinOp::Concat => {
            unreachable!("BinOp::Concat is intercepted in emit_expr — should not reach emit_binop")
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
    context: &'c Context,
    block: &'a Block<'c>,
    op: UnaryOp,
    operand: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    match op {
        UnaryOp::Neg => Ok(block
            .append_operation(arith::negf(operand, loc))
            .result(0)
            .unwrap()
            .into()),
        UnaryOp::Not => {
            // Caller (emit_expr) is responsible for converting `operand`
            // to its truthiness `i1` before reaching here, since `not`
            // accepts any kind. We then flip the bit with xori.
            unreachable!(
                "UnaryOp::Not should be handled in emit_expr where the \
                 operand kind is available for truthiness conversion"
            );
        }
        // Phase 2.2c (ADR 0022): `~x = x XOR -1` after f64→i64.
        UnaryOp::BitNot => {
            let xi = emit_f2i(block, operand, types, loc);
            let neg_one = block
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, -1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let r_i = block
                .append_operation(arith::xori(xi, neg_one, loc))
                .result(0)
                .unwrap()
                .into();
            Ok(emit_i2f(block, r_i, types, loc))
        }
        // Phase 2.7a (ADR 0024): `#s` calls libc strlen on the
        // string ptr, then promotes the i64 byte count to f64 to
        // match our Number-typed expression world.
        UnaryOp::Len => {
            let n = i32::try_from(1usize).unwrap();
            let call_op = OperationBuilder::new("llvm.call", loc)
                .add_operands(&[operand])
                .add_attributes(&[
                    (
                        Identifier::new(context, "callee"),
                        FlatSymbolRefAttribute::new(context, "strlen").into(),
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
                .add_results(&[types.i64])
                .build()
                .expect("llvm.call @strlen");
            let len_i64: Value<'c, 'a> = block.append_operation(call_op).result(0).unwrap().into();
            Ok(emit_i2f(block, len_i64, types, loc))
        }
    }
}

/// Phase 2.2c (ADR 0022): convert an `f64` operand to a 64-bit signed
/// integer via `arith.fptosi` for use in the bitwise ops, which are
/// integer-typed in Lua 5.4 (truncates the float).
fn emit_f2i<'a, 'c>(
    block: &'a Block<'c>,
    v: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let op = OperationBuilder::new("arith.fptosi", loc)
        .add_operands(&[v])
        .add_results(&[types.i64])
        .build()
        .expect("arith.fptosi");
    block.append_operation(op).result(0).unwrap().into()
}

/// Phase 2.2c (ADR 0022): convert a 64-bit signed integer back to
/// `f64` via `arith.sitofp`. Used after a bitwise op so the result
/// flows back into the existing Number-typed expression world.
fn emit_i2f<'a, 'c>(
    block: &'a Block<'c>,
    v: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let op = OperationBuilder::new("arith.sitofp", loc)
        .add_operands(&[v])
        .add_results(&[types.f64])
        .build()
        .expect("arith.sitofp");
    block.append_operation(op).result(0).unwrap().into()
}

/// Phase 2.7b (ADR 0025): runtime concat via libc `malloc` + `memcpy`.
/// Allocates `strlen(a) + strlen(b) + 1` bytes, copies `a` then `b`
/// (the second copy includes `b`'s NUL terminator). Returned `ptr`
/// is the freshly-allocated buffer; intentionally leaks since we
/// have no GC yet.
fn emit_concat<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    lhs: Value<'c, 'a>,
    rhs: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let la = emit_libc_call_i64(context, block, "strlen", &[lhs], types, loc);
    let lb = emit_libc_call_i64(context, block, "strlen", &[rhs], types, loc);
    let one = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, 1).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let lab = block
        .append_operation(arith::addi(la, lb, loc))
        .result(0)
        .unwrap()
        .into();
    let total = block
        .append_operation(arith::addi(lab, one, loc))
        .result(0)
        .unwrap()
        .into();
    let buf = emit_libc_call_ptr(context, block, "malloc", &[total], types, loc);
    // memcpy(buf, lhs, la)
    let _ = emit_libc_call_ptr(context, block, "memcpy", &[buf, lhs, la], types, loc);
    // memcpy(buf + la, rhs, lb + 1)
    let dst2 = block
        .append_operation(
            OperationBuilder::new("llvm.getelementptr", loc)
                .add_operands(&[buf, la])
                .add_attributes(&[
                    (
                        Identifier::new(context, "elem_type"),
                        TypeAttribute::new(IntegerType::new(context, 8).into()).into(),
                    ),
                    (
                        Identifier::new(context, "rawConstantIndices"),
                        DenseI32ArrayAttribute::new(context, &[i32::MIN]).into(),
                    ),
                ])
                .add_results(&[types.ptr])
                .build()
                .expect("llvm.getelementptr"),
        )
        .result(0)
        .unwrap()
        .into();
    let lb_plus_one = block
        .append_operation(arith::addi(lb, one, loc))
        .result(0)
        .unwrap()
        .into();
    let _ = emit_libc_call_ptr(
        context,
        block,
        "memcpy",
        &[dst2, rhs, lb_plus_one],
        types,
        loc,
    );
    buf
}

/// Phase 2.7c (ADR 0026): convert `value` (of static `kind`) into a
/// String. Number → `snprintf("%.14g", n)` into a malloc'd buffer.
/// Bool / Nil reuse the existing `s_true` / `s_false` / `s_nil`
/// global strings. String values are returned as-is.
fn emit_tostring<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    value: Value<'c, 'a>,
    kind: ValueKind,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    match kind {
        ValueKind::String => value,
        ValueKind::Bool => {
            // Materialise an i1 → ptr select between `s_true` /
            // `s_false`. Mirrors the existing emit_print_bool path
            // but yields a ptr instead of consuming it directly.
            let true_ptr = emit_addressof(context, block, "s_true", types, loc);
            let false_ptr = emit_addressof(context, block, "s_false", types, loc);
            let select = OperationBuilder::new("llvm.select", loc)
                .add_operands(&[value, true_ptr, false_ptr])
                .add_results(&[types.ptr])
                .build()
                .expect("llvm.select");
            block.append_operation(select).result(0).unwrap().into()
        }
        ValueKind::Nil => emit_addressof(context, block, "s_nil", types, loc),
        ValueKind::Number => {
            // Allocate a fixed-size buffer (32 bytes — covers any f64
            // formatted with %.14g plus sign, dot, exponent, NUL),
            // then `snprintf(buf, 32, "%.14g", n)`. Returns the buf
            // ptr; intentionally leaks (no GC, ADR 0025).
            let buf_size = block
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, 32).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let buf = emit_libc_call_ptr(context, block, "malloc", &[buf_size], types, loc);
            let fmt_ptr = emit_addressof(context, block, "fmt_tostring_g", types, loc);
            // snprintf is variadic — needs the same callee_type
            // attribute pattern as printf.
            let snprintf_callee_ty =
                llvm::r#type::function(types.i32, &[types.ptr, types.i64, types.ptr], true);
            let n = i32::try_from(4usize).unwrap();
            let call_op = OperationBuilder::new("llvm.call", loc)
                .add_operands(&[buf, buf_size, fmt_ptr, value])
                .add_attributes(&[
                    (
                        Identifier::new(context, "callee"),
                        FlatSymbolRefAttribute::new(context, "snprintf").into(),
                    ),
                    (
                        Identifier::new(context, "var_callee_type"),
                        TypeAttribute::new(snprintf_callee_ty).into(),
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
                .add_results(&[types.i32])
                .build()
                .expect("llvm.call @snprintf");
            block.append_operation(call_op);
            buf
        }
        ValueKind::Function(_) => unreachable!(
            "Function-kind tostring is rejected at HIR-time \
             (FunctionUsedAsValue) — should not reach codegen"
        ),
    }
}

/// Phase 2.7e (ADR 0028): convert `value` (of static `kind`) into a
/// Number. Number → identity. String → `sscanf("%lf", buf, &out)`
/// into an alloca'd f64; if the call returned 1 the parsed value
/// is yielded, otherwise NaN. Other kinds reject at HIR time and
/// never reach codegen.
fn emit_tonumber<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    value: Value<'c, 'a>,
    kind: ValueKind,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    match kind {
        ValueKind::Number => value,
        ValueKind::String => {
            // alloca an f64 receiver — `sscanf` writes through the
            // pointer when the conversion succeeds.
            let one = block
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i32, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let receiver: Value<'c, 'a> = block
                .append_operation(
                    OperationBuilder::new("llvm.alloca", loc)
                        .add_operands(&[one])
                        .add_attributes(&[(
                            Identifier::new(context, "elem_type"),
                            TypeAttribute::new(types.f64).into(),
                        )])
                        .add_results(&[types.ptr])
                        .build()
                        .expect("llvm.alloca f64 (tonumber receiver)"),
                )
                .result(0)
                .unwrap()
                .into();
            let fmt_ptr = emit_addressof(context, block, "fmt_tonumber_lf", types, loc);
            let sscanf_callee_ty =
                llvm::r#type::function(types.i32, &[types.ptr, types.ptr, types.ptr], true);
            let n = i32::try_from(3usize).unwrap();
            let call_op = OperationBuilder::new("llvm.call", loc)
                .add_operands(&[value, fmt_ptr, receiver])
                .add_attributes(&[
                    (
                        Identifier::new(context, "callee"),
                        FlatSymbolRefAttribute::new(context, "sscanf").into(),
                    ),
                    (
                        Identifier::new(context, "var_callee_type"),
                        TypeAttribute::new(sscanf_callee_ty).into(),
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
                .add_results(&[types.i32])
                .build()
                .expect("llvm.call @sscanf");
            let ret_i32: Value<'c, 'a> = block.append_operation(call_op).result(0).unwrap().into();
            // If sscanf consumed one field, load the parsed f64;
            // otherwise yield NaN. `arith.select` joins both arms.
            let one_i32 = block
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i32, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let ok = block
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Eq,
                    ret_i32,
                    one_i32,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let parsed = emit_load(block, receiver, types.f64, loc);
            let nan = block
                .append_operation(arith::constant(
                    context,
                    FloatAttribute::new(context, types.f64, f64::NAN).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            block
                .append_operation(arith::select(ok, parsed, nan, loc))
                .result(0)
                .unwrap()
                .into()
        }
        ValueKind::Bool | ValueKind::Nil | ValueKind::Function(_) => unreachable!(
            "tonumber on Bool/Nil/Function is rejected at HIR-time \
             (TypeMismatch / FunctionUsedAsValue) — should not reach codegen"
        ),
    }
}

/// Phase 2.7b/d (ADRs 0025, 0027): String equality and ordering via
/// `strcmp`. `strcmp(a, b)` returns negative / zero / positive; the
/// per-op predicate then projects that to an i1. Lexicographic
/// ordering uses **signed** comparisons (`Slt` / `Sle` / `Sgt` /
/// `Sge`) since `strcmp` returns a signed `int`.
fn emit_string_cmp<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    op: BinOp,
    lhs: Value<'c, 'a>,
    rhs: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let cmp = emit_libc_call_i32(context, block, "strcmp", &[lhs, rhs], types, loc);
    let zero = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i32, 0).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let pred = match op {
        BinOp::Eq => arith::CmpiPredicate::Eq,
        BinOp::Ne => arith::CmpiPredicate::Ne,
        BinOp::Lt => arith::CmpiPredicate::Slt,
        BinOp::Le => arith::CmpiPredicate::Sle,
        BinOp::Gt => arith::CmpiPredicate::Sgt,
        BinOp::Ge => arith::CmpiPredicate::Sge,
        _ => unreachable!("emit_string_cmp called with non-comparison op"),
    };
    block
        .append_operation(arith::cmpi(context, pred, cmp, zero, loc))
        .result(0)
        .unwrap()
        .into()
}

/// Generic single-result libc helper — `i64` return type.
fn emit_libc_call_i64<'a, 'c>(
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
fn emit_libc_call_i32<'a, 'c>(
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
fn emit_libc_call_ptr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    name: &str,
    args: &[Value<'c, 'a>],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    emit_libc_call_with_result(context, block, name, args, types.ptr, loc)
}

fn emit_libc_call_with_result<'a, 'c>(
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
        // Phase 2.7a (ADR 0024): a String value is already a `ptr`
        // into the static C-string pool; printf with `%s\n` is the
        // direct path.
        ValueKind::String => {
            let fmt_ptr = emit_addressof(context, block, "fmt_str", types, loc);
            emit_printf(context, block, fmt_ptr, value, types, loc);
        }
        ValueKind::Function(_) => unreachable!(
            "Function-kind args are rejected at HIR-time \
             (HirError::FunctionUsedAsValue) — should not reach codegen"
        ),
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
        // Phase 2.7a (ADR 0024): every string is truthy in Lua,
        // including the empty string. Same fold as Number.
        ValueKind::String => {
            let attr = IntegerAttribute::new(types.i1, 1);
            block
                .append_operation(arith::constant(context, attr.into(), loc))
                .result(0)
                .unwrap()
                .into()
        }
        ValueKind::Function(_) => unreachable!(
            "Function-kind values do not flow through truthiness \
             (HIR rejects them at use sites) — ADR 0017"
        ),
    }
}

/// Map a static [`ValueKind`] to the MLIR type used for slots / yields
/// of values of that kind. Phase 2.3a fixes the layout: Number → f64,
/// Bool/Nil → i1.
fn kind_to_mlir_type<'c>(kind: ValueKind, types: &Types<'c>) -> Type<'c> {
    match kind {
        ValueKind::Number => types.f64,
        ValueKind::Bool | ValueKind::Nil | ValueKind::Function(_) => types.i1,
        ValueKind::String => types.ptr,
    }
}

/// Lower `a and b` / `a or b` as an expression-form `scf.if`.
///
/// `a and b`: if `truthy(a)` yield `b`, else yield `a` (Lua: value-
/// preserving). `a or b` is the dual. The two-arm `scf.if` joins both
/// values via its result, so RHS is only evaluated when needed.
///
/// Same-kind enforcement happens at HIR-time, so `kind_to_mlir_type`
/// works for both arms.
#[allow(clippy::too_many_arguments)]
fn emit_short_circuit<'a, 'c>(
    context: &'c Context,
    parent: &'a Block<'c>,
    op: BinOp,
    lhs: &HirExpr,
    rhs: &HirExpr,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    let kind = infer_kind(lhs, locals, functions);
    let result_ty = kind_to_mlir_type(kind, types);

    let lhs_val = emit_expr(
        context, parent, lhs, slots, locals, functions, types, params_len, loc,
    )?;
    let cond = emit_truthiness(context, parent, lhs_val, kind, types, loc);

    // For `and`: then = rhs, else = lhs. For `or`: then = lhs, else = rhs.
    let (then_yield_lhs, else_yield_lhs) = match op {
        BinOp::And => (false, true),
        BinOp::Or => (true, false),
        _ => unreachable!("emit_short_circuit called with non-and/or BinOp"),
    };

    let then_region = build_yield_region(
        context,
        parent,
        rhs,
        lhs_val,
        then_yield_lhs,
        slots,
        locals,
        functions,
        types,
        params_len,
        loc,
    )?;
    let else_region = build_yield_region(
        context,
        parent,
        rhs,
        lhs_val,
        else_yield_lhs,
        slots,
        locals,
        functions,
        types,
        params_len,
        loc,
    )?;

    let if_op = scf::r#if(cond, &[result_ty], then_region, else_region, loc);
    let result = parent.append_operation(if_op).result(0).unwrap().into();
    Ok(result)
}

/// Build a Region with one Block that either yields `lhs_val` directly
/// (when `yield_lhs == true`) or evaluates `rhs` inside the block and
/// yields the result.
#[allow(clippy::too_many_arguments)]
fn build_yield_region<'a, 'c>(
    context: &'c Context,
    _parent: &'a Block<'c>,
    rhs: &HirExpr,
    lhs_val: Value<'c, 'a>,
    yield_lhs: bool,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<Region<'c>, CodegenError> {
    let region = Region::new();
    let blk = Block::new(&[]);
    let blk_slots = transmute_slots(slots);
    let yielded: Value<'c, '_> = if yield_lhs {
        // `lhs_val` was produced in the parent block; its lifetime is
        // tied to the caller's block. Re-borrow at the inner block's
        // lifetime — sound because the parent block dominates the
        // scf.if's regions.
        unsafe { std::mem::transmute::<Value<'c, 'a>, Value<'c, '_>>(lhs_val) }
    } else {
        emit_expr(
            context, &blk, rhs, blk_slots, locals, functions, types, params_len, loc,
        )?
    };
    blk.append_operation(scf::r#yield(&[yielded], loc));
    region.append_block(blk);
    Ok(region)
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
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let cond_kind = infer_kind(cond, locals, functions);
    let cond_val = emit_expr(
        context, parent, cond, slots, locals, functions, types, params_len, loc,
    )?;
    let cond_i1 = emit_truthiness(context, parent, cond_val, cond_kind, types, loc);

    let then_region = build_body_region(
        context, then_body, slots, locals, functions, types, params_len, loc,
    )?;
    let else_region = build_else_region(
        context, elifs, else_body, slots, locals, functions, types, params_len, loc,
    )?;

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
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<Region<'c>, CodegenError> {
    if let Some(((cond, body), rest)) = elifs.split_first() {
        // Nested scf.if for the next elseif/else arm.
        let region = Region::new();
        let blk = Block::new(&[]);
        let cond_kind = infer_kind(cond, locals, functions);
        // Re-bind slots and emit into the new block.
        let blk_slots = transmute_slots(slots);
        let cond_val = emit_expr(
            context, &blk, cond, blk_slots, locals, functions, types, params_len, loc,
        )?;
        let cond_i1 = emit_truthiness(context, &blk, cond_val, cond_kind, types, loc);
        let nested_then = build_body_region(
            context, body, blk_slots, locals, functions, types, params_len, loc,
        )?;
        let nested_else = build_else_region(
            context, rest, else_body, blk_slots, locals, functions, types, params_len, loc,
        )?;
        blk.append_operation(scf::r#if(cond_i1, &[], nested_then, nested_else, loc));
        blk.append_operation(scf::r#yield(&[], loc));
        region.append_block(blk);
        Ok(region)
    } else {
        // Leaf: either else_body or empty `scf.yield` for absent else.
        match else_body {
            Some(body) => build_body_region(
                context, body, slots, locals, functions, types, params_len, loc,
            ),
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
#[allow(clippy::too_many_arguments)]
fn build_body_region<'c>(
    context: &'c Context,
    body: &[HirStmt],
    slots: &[Value<'c, '_>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<Region<'c>, CodegenError> {
    let region = Region::new();
    let blk = Block::new(&[]);
    let blk_slots = transmute_slots(slots);
    emit_stmts(
        context, &blk, body, blk_slots, locals, functions, types, params_len, loc,
    )?;
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
    break_id: Option<LocalId>,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    // before region: evaluate cond, optionally AND with `not _broken`.
    let before = Region::new();
    let before_blk = Block::new(&[]);
    let blk_slots = transmute_slots(slots);
    let cond_kind = infer_kind(cond, locals, functions);
    let cond_val = emit_expr(
        context,
        &before_blk,
        cond,
        blk_slots,
        locals,
        functions,
        types,
        params_len,
        loc,
    )?;
    let mut cond_i1 = emit_truthiness(context, &before_blk, cond_val, cond_kind, types, loc);
    if let Some(id) = break_id {
        cond_i1 = and_not_broken(context, &before_blk, cond_i1, blk_slots, id, types, loc);
    }
    before_blk.append_operation(scf::condition(cond_i1, &[], loc));
    before.append_block(before_blk);

    // after region: body, scf.yield.
    let after = build_body_region(
        context, body, slots, locals, functions, types, params_len, loc,
    )?;

    parent.append_operation(scf::r#while(&[], &[], before, after, loc));
    Ok(())
}

/// `cond AND (NOT load(slots[broken_id]))` as a single i1 in `block`.
/// Used by emit_while/emit_for_numeric to honour `break` flags.
fn and_not_broken<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    cond_i1: Value<'c, 'a>,
    slots: &[Value<'c, 'a>],
    broken_id: LocalId,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let broken = emit_load(block, slots[broken_id.0], types.i1, loc);
    let one = arith::constant(context, IntegerAttribute::new(types.i1, 1).into(), loc);
    let one_val: Value<'c, 'a> = block.append_operation(one).result(0).unwrap().into();
    let not_broken: Value<'c, 'a> = block
        .append_operation(arith::xori(broken, one_val, loc))
        .result(0)
        .unwrap()
        .into();
    block
        .append_operation(arith::andi(cond_i1, not_broken, loc))
        .result(0)
        .unwrap()
        .into()
}

/// Lower a numeric `for var = start, stop, step do body end` to a
/// `scf.while` loop. start/stop/step are evaluated **once** before the
/// loop (Lua 5.4 §3.3.5). Auxiliary `stop` and `step` slots are
/// allocated in the parent block; the loop variable already has its
/// own slot in `slots[var_id.0]`.
///
/// Sign dispatch is **runtime**: an inner `scf.if` in the before
/// region picks `arith.cmpf ole` (step > 0) or `oge` (step ≤ 0).
/// LLVM constant-folds when `step` is a literal.
#[allow(clippy::too_many_arguments)]
fn emit_for_numeric<'a, 'c>(
    context: &'c Context,
    parent: &'a Block<'c>,
    var_id: LocalId,
    start: &HirExpr,
    stop: &HirExpr,
    step: &HirExpr,
    body: &[HirStmt],
    break_id: Option<LocalId>,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    // Evaluate start/stop/step once in the parent block, store into
    // dedicated slots (stop/step) and the loop variable's own slot.
    let start_val = emit_expr(
        context, parent, start, slots, locals, functions, types, params_len, loc,
    )?;
    let stop_val = emit_expr(
        context, parent, stop, slots, locals, functions, types, params_len, loc,
    )?;
    let step_val = emit_expr(
        context, parent, step, slots, locals, functions, types, params_len, loc,
    )?;

    let stop_slot = emit_alloca_slot_for_kind(context, parent, ValueKind::Number, types, loc);
    let step_slot = emit_alloca_slot_for_kind(context, parent, ValueKind::Number, types, loc);
    emit_store(parent, start_val, slots[var_id.0], loc);
    emit_store(parent, stop_val, stop_slot, loc);
    emit_store(parent, step_val, step_slot, loc);

    // Before region: load i/stop/step, runtime-dispatch on step sign,
    // emit scf.condition.
    let before = Region::new();
    let before_blk = Block::new(&[]);
    let blk_slots = transmute_slots(slots);
    let var_slot = blk_slots[var_id.0];
    let stop_slot_b = unsafe { std::mem::transmute::<Value<'c, 'a>, Value<'c, '_>>(stop_slot) };
    let step_slot_b = unsafe { std::mem::transmute::<Value<'c, 'a>, Value<'c, '_>>(step_slot) };
    let i_val = emit_load(&before_blk, var_slot, types.f64, loc);
    let stop_now = emit_load(&before_blk, stop_slot_b, types.f64, loc);
    let step_now = emit_load(&before_blk, step_slot_b, types.f64, loc);

    let zero_attr = FloatAttribute::new(context, types.f64, 0.0);
    let zero = before_blk
        .append_operation(arith::constant(context, zero_attr.into(), loc))
        .result(0)
        .unwrap()
        .into();
    let step_pos = before_blk
        .append_operation(arith::cmpf(
            context,
            CmpfPredicate::Ogt,
            step_now,
            zero,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();

    // Inner scf.if -> i1 yields the right ordered comparison.
    let then_region =
        build_for_cond_region(context, CmpfPredicate::Ole, i_val, stop_now, types, loc);
    let else_region =
        build_for_cond_region(context, CmpfPredicate::Oge, i_val, stop_now, types, loc);
    let cond = before_blk
        .append_operation(scf::r#if(
            step_pos,
            &[types.i1],
            then_region,
            else_region,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let final_cond = if let Some(id) = break_id {
        and_not_broken(context, &before_blk, cond, blk_slots, id, types, loc)
    } else {
        cond
    };
    before_blk.append_operation(scf::condition(final_cond, &[], loc));
    before.append_block(before_blk);

    // After region: body, then i = i + step, scf.yield.
    let after = Region::new();
    let after_blk = Block::new(&[]);
    let after_slots = transmute_slots(slots);
    emit_stmts(
        context,
        &after_blk,
        body,
        after_slots,
        locals,
        functions,
        types,
        params_len,
        loc,
    )?;
    let i_now = emit_load(&after_blk, after_slots[var_id.0], types.f64, loc);
    let step_now_after = emit_load(
        &after_blk,
        unsafe { std::mem::transmute::<Value<'c, 'a>, Value<'c, '_>>(step_slot) },
        types.f64,
        loc,
    );
    let i_next = after_blk
        .append_operation(arith::addf(i_now, step_now_after, loc))
        .result(0)
        .unwrap()
        .into();
    emit_store(&after_blk, i_next, after_slots[var_id.0], loc);
    after_blk.append_operation(scf::r#yield(&[], loc));
    after.append_block(after_blk);

    parent.append_operation(scf::r#while(&[], &[], before, after, loc));
    Ok(())
}

/// Build a single-block Region that yields `arith.cmpf <pred> i, stop : i1`.
/// Used as the then/else region of the for-loop sign dispatch.
fn build_for_cond_region<'c, 'a>(
    context: &'c Context,
    pred: CmpfPredicate,
    i: Value<'c, 'a>,
    stop: Value<'c, 'a>,
    _types: &Types<'c>,
    loc: Location<'c>,
) -> Region<'c> {
    let region = Region::new();
    let blk = Block::new(&[]);
    let i_inner = unsafe { std::mem::transmute::<Value<'c, 'a>, Value<'c, '_>>(i) };
    let stop_inner = unsafe { std::mem::transmute::<Value<'c, 'a>, Value<'c, '_>>(stop) };
    let cmp = blk
        .append_operation(arith::cmpf(context, pred, i_inner, stop_inner, loc))
        .result(0)
        .unwrap()
        .into();
    blk.append_operation(scf::r#yield(&[cmp], loc));
    region.append_block(blk);
    region
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
    fn emit_not_uses_arith_xori() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(not true)")).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("arith.xori"),
            "expected arith.xori for `not`, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_not_with_number_emits_truthiness_then_xor() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(not 1)")).unwrap();
        let mlir = module.as_operation().to_string();
        // Number truthiness folds to constant true at compile time;
        // xori turns that into false.
        assert!(
            mlir.contains("arith.xori") && mlir.contains("arith.constant true"),
            "expected truthiness + xori for `not 1`, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_and_uses_scf_if_with_yield() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(true and false)")).unwrap();
        let mlir = module.as_operation().to_string();
        // Expression-form scf.if: pretty-prints `-> (i1)` and yields i1.
        assert!(
            mlir.contains("scf.if") && mlir.contains("-> (i1)") && mlir.contains("scf.yield"),
            "expected expression-form scf.if for `and`, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_or_uses_scf_if_with_yield() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("print(false or true)")).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("scf.if") && mlir.contains("-> (i1)") && mlir.contains("scf.yield"),
            "expected expression-form scf.if for `or`, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_module_with_short_circuit_verifies() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("if (1 < 2) and (2 < 3) then print(true) end"),
        )
        .expect("short-circuit module must verify");
        assert!(module.as_operation().verify());
    }

    #[test]
    fn emit_function_arg_param_uses_func_func_type() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function apply(g, x) return g(x) end\nlocal f = function(x) return x*2 end\nprint(apply(f, 7))",
            ),
        )
        .expect("apply pattern must verify");
        let mlir = module.as_operation().to_string();
        // Pretty-printer prints `(f64) -> f64` rather than the
        // longer `!func.func<(f64) -> f64>` form for params.
        assert!(
            mlir.contains("%arg0: (f64) -> f64"),
            "expected an `(f64) -> f64` param, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_indirect_call_via_local_uses_call_indirect() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function apply(g, x) return g(x) end\nlocal f = function(x) return x*2 end\nprint(apply(f, 7))",
            ),
        )
        .unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("call_indirect"),
            "expected `call_indirect`, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_module_with_apply_pattern_verifies() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function apply(g, x) return g(x) end\nlocal f = function(x) return x*2 end\nprint(apply(f, 7))",
            ),
        )
        .expect("apply module must verify");
        assert!(module.as_operation().verify());
    }

    #[test]
    fn emit_anonymous_function_creates_user_anon_symbol() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("local f = function() return 1 end\nprint(f())"),
        )
        .unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("func.func @user_anon_0"),
            "expected mangled `@user_anon_0` symbol, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_local_function_value_module_verifies() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("local f = function(x) return x * 2 end\nprint(f(7))"),
        )
        .expect("function-value module must verify");
        assert!(module.as_operation().verify());
    }

    #[test]
    fn emit_alias_call_resolves_to_anon_symbol() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("local f = function() return 42 end\nlocal g = f\nprint(g())"),
        )
        .expect("alias module must verify");
        let mlir = module.as_operation().to_string();
        // The call site for g() should still resolve to @user_anon_0
        // — alias is a HIR-only kind copy.
        assert!(
            mlir.contains("call @user_anon_0"),
            "expected alias call to @user_anon_0, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_function_def_creates_func_func_symbol() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("local function f() end")).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("func.func @user_f_0"),
            "expected mangled `@user_f_0` symbol, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_user_function_call_uses_func_call() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("local function f() return 1 end\nprint(f())"),
        )
        .unwrap();
        let mlir = module.as_operation().to_string();
        // Pretty-printer drops the `func.` prefix in context (it's
        // unambiguous after dialect resolution); accept either form.
        assert!(
            mlir.contains("call @user_f_0"),
            "expected a call to `@user_f_0`, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_recursive_function_module_verifies() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function fact(n) if n == 0 then return 1 end\nreturn n * fact(n - 1) end\nprint(fact(5))",
            ),
        )
        .expect("recursive module must verify");
        assert!(module.as_operation().verify());
    }

    #[test]
    fn emit_void_function_module_verifies() {
        let ctx = new_context();
        let module =
            emit_module(&ctx, &lower_src("local function f() end\nf()")).expect("void must verify");
        assert!(module.as_operation().verify());
    }

    #[test]
    fn emit_return_via_returned_flag_emits_func_return() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("local function f() return 42 end\nprint(f())"),
        )
        .unwrap();
        let mlir = module.as_operation().to_string();
        // The function's body wraps post-return stmts with the
        // `_returned` flag and emits a single trailing `return`.
        assert!(
            mlir.contains("return"),
            "expected `return` op in MLIR, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_while_with_break_extends_cond_with_andi() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("while true do break end")).unwrap();
        let mlir = module.as_operation().to_string();
        // `and_not_broken` emits xori + andi against the broken flag.
        assert!(
            mlir.contains("arith.andi") && mlir.contains("arith.xori"),
            "expected andi+xori for break-aware cond, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_module_with_break_in_for_verifies() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("for i = 1, 5 do if i == 3 then break end end"),
        )
        .expect("for+break module must verify");
        assert!(module.as_operation().verify());
    }

    #[test]
    fn emit_for_numeric_uses_scf_while() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("for i = 1, 3 do print(i) end")).unwrap();
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("scf.while") && mlir.contains("scf.condition"),
            "expected scf.while + scf.condition for `for`, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_for_numeric_evaluates_start_once() {
        // start = 1 should appear exactly once in the IR (not inside
        // the loop body) — proving it isn't re-evaluated each iter.
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("for i = 1, 3 do print(i) end")).unwrap();
        let mlir = module.as_operation().to_string();
        // arith.constant 1.000000e+00 should appear once (the loop init);
        // the addf in the after region uses load+constant-step, not 1.
        let starts = mlir.matches("1.000000e+00").count();
        // 1 from `start`, plus possibly 1 from the synthetic step constant.
        // Either way, less than 4 (we don't re-evaluate `start` each iter).
        assert!(
            starts <= 3,
            "expected start evaluated once, got {starts} occurrences"
        );
    }

    #[test]
    fn emit_for_with_negative_step_uses_runtime_dispatch() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("for i = 10, 1, -2 do print(i) end")).unwrap();
        let mlir = module.as_operation().to_string();
        // Runtime sign dispatch: scf.if step > 0 yields ole, else oge.
        assert!(
            mlir.contains("scf.if") && (mlir.contains("ole") || mlir.contains("oge")),
            "expected runtime sign dispatch via scf.if + ole/oge, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_module_with_for_verifies() {
        let ctx = new_context();
        let module = emit_module(&ctx, &lower_src("for i = 1, 3 do print(i) end"))
            .expect("for module must verify");
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

    // -----------------------------------------------------------
    // Phase 2.5b.3 — function return values (ADR 0019).
    // -----------------------------------------------------------

    #[test]
    fn emit_function_with_function_return_type() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function d(x) return x*2 end\nlocal function gd() return d end\nlocal f = gd()\nprint(f(5))",
            ),
        )
        .expect("get_doubler module must verify");
        let mlir = module.as_operation().to_string();
        // The signature of `gd` should declare a function-typed return.
        // Pretty-printer renders this as `() -> ((f64) -> f64)` (the
        // outer `() -> ...` is gd's signature; the inner is its return).
        assert!(
            mlir.contains("@user_gd_") && mlir.contains("(f64) -> f64"),
            "expected function-typed return for gd, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_call_with_function_result_threads_value_to_caller() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function d(x) return x*2 end\nlocal function gd() return d end\nlocal f = gd()\nprint(f(5))",
            ),
        )
        .unwrap();
        let mlir = module.as_operation().to_string();
        // The call-site to `gd` must produce a function-typed value
        // and the subsequent `f(5)` must dispatch via call_indirect.
        assert!(
            mlir.contains("call @user_gd_") && mlir.contains("call_indirect"),
            "expected `call @user_gd_*` then `call_indirect`, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_module_with_get_doubler_pattern_verifies() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function d(x) return x*2 end\nlocal function gd() return d end\nlocal f = gd()\nprint(f(5))",
            ),
        )
        .expect("get_doubler full module must verify");
        assert!(module.as_operation().verify());
    }

    // -----------------------------------------------------------
    // Phase 2.5e — Bool/Nil signatures (ADR 0020).
    // -----------------------------------------------------------

    #[test]
    fn emit_function_with_bool_return_uses_i1_result() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("local function pos(x) return x > 0 end\nprint(pos(5))"),
        )
        .expect("Bool-return module must verify");
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("@user_pos_") && mlir.contains("-> i1"),
            "expected `pos` to return i1, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_function_with_nil_return_uses_i1_result() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("local function n() return nil end\nprint(n())"),
        )
        .expect("Nil-return module must verify");
        let mlir = module.as_operation().to_string();
        assert!(
            mlir.contains("@user_n_") && mlir.contains("-> i1"),
            "expected `n` to return i1 (for nil), got:\n{mlir}"
        );
    }

    #[test]
    fn emit_module_with_bool_predicate_verifies() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function is_zero(n) return n == 0 end\nlocal b = is_zero(0)\nprint(b)",
            ),
        )
        .expect("Bool predicate module must verify");
        assert!(module.as_operation().verify());
    }
}
