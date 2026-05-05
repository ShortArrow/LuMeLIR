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

// =============================================================
// Phase 2.6a-norm (ADR 0056): stable table header layout.
//
// `ValueKind::Table` is `!llvm.ptr` pointing at a heap-allocated
// header. The header is 32 bytes and **never moves** for the
// lifetime of the table value — `local b = a` aliases stay valid
// even when the inner buffers are reallocated by future grow /
// hash sub-phases. The contract:
//
// ```text
// table value (!llvm.ptr)
//   → header [32 bytes, malloc'd, stable]
//        offset 0  : i64 length          ← `#t` reads here (frozen)
//        offset 8  : i64 capacity        ← grow updates (2.6a-grow)
//        offset 16 : !llvm.ptr array_buf ← f64 elem buffer (frozen offset)
//        offset 24 : !llvm.ptr hash_buf  ← null today; 2.6b lights it up
//
//   array_buf (!llvm.ptr) → [f64 elem₀][f64 elem₁]…[f64 elem_{cap-1}]
// ```
//
// **Frozen offsets** (will never move; depend on these):
//   - 0  (length)
//   - 16 (array_buf)
//
// **Mutable offsets** (may relocate as the layout grows):
//   - 8  (capacity)
//   - 24 (hash_buf)
//
// =============================================================
const TABLE_OFF_LEN: i64 = 0;
const TABLE_OFF_CAP: i64 = 8;
const TABLE_OFF_ARRAY_BUF: i64 = 16;
const TABLE_OFF_HASH_BUF: i64 = 24;
const TABLE_HEADER_SIZE: i64 = 32;

// =============================================================
// Phase 2.6b-hash (ADR 0058): hash_buf layout for string-keyed
// fields. Lazy-allocated on first `t.k = v`. Open addressing
// with linear probing.
//
// ```text
// hash_buf (!llvm.ptr)
//   [0..8)   : i64 hash_cap     # bucket count (always power of two)
//   [8..16)  : i64 hash_count   # occupied buckets
//   [16..)   : entry array      # hash_cap × 16-byte entries
//      entry layout (16 bytes):
//        [0..8)  : !llvm.ptr key_str   (null = empty bucket)
//        [8..16) : f64 value
// ```
//
// Initial capacity 8 (= 128 bytes for entries + 16 for header).
// Resize at load factor 0.75 (`count * 4 ≥ cap * 3`).
// =============================================================
const HASH_OFF_CAP: i64 = 0;
const HASH_OFF_COUNT: i64 = 8;
const HASH_OFF_ENTRIES: i64 = 16;
// Phase 2.6c-tag-hash (ADR 0060): hash entry layout is now
// `{ptr key, 16-byte tagged value slot}` = 24 bytes. The value
// slot starts at offset 8 inside the entry; its internal
// `{tag, value}` layout matches array_buf's element slot exactly,
// so `emit_value_slot_*` helpers work unchanged on it.
const HASH_ENTRY_SIZE: i64 = 24;
const HASH_ENTRY_OFF_KEY: i64 = 0;
const HASH_ENTRY_OFF_VALUE_SLOT: i64 = 8;
const HASH_INITIAL_CAP: i64 = 8;
// Phase 2.6c-tag-hash-hard (ADR 0062): a deleted bucket has its
// key replaced with this i64-comparable sentinel. Both `.data`-
// section string globals and `malloc`'d buffers have alignment
// well above 1, so the sentinel is unambiguously distinguishable
// from any live key pointer (and from null = empty bucket).
const HASH_DELETED_KEY: i64 = 1;

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
const ARRAY_ELEM_SIZE: i64 = 16;
// `ARRAY_ELEM_OFF_TAG = 0`: tag at slot start. Implicit in the
// `emit_load(elem_ptr, types.i64, …)` calls; not referenced by
// name to avoid an unused-const warning.
const ARRAY_ELEM_OFF_VALUE: i64 = 8;
const TAG_NIL: i64 = 0;
const TAG_NUMBER: i64 = 1;
// Phase 2.6c-tag-hetero (ADR 0064): widen the tagged slot to
// carry Bool and String values. Function (4) and Table (5)
// tags remain reserved for a follow-up sub-phase.
const TAG_BOOL: i64 = 2;
const TAG_STRING: i64 = 3;

struct Types<'c> {
    i1: Type<'c>,
    /// Phase 2.6a-arr (ADR 0054): i8 used as the GEP element type
    /// for byte-offset pointer arithmetic on table heap regions.
    i8: Type<'c>,
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
        i8: i8_type,
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
    // Phase 2.8b (ADR 0032): no-newline siblings of `fmt`/`fmt_str`,
    // plus literal `\t`/`\n` payloads, for multi-arg `print(a, b, ...)`.
    emit_string_global(context, module, i8_type, "fmt_raw", "%g\0", loc);
    emit_string_global(context, module, i8_type, "fmt_str_raw", "%s\0", loc);
    emit_string_global(context, module, i8_type, "s_tab", "\t\0", loc);
    emit_string_global(context, module, i8_type, "s_newline", "\n\0", loc);
    // Phase 2.7c (ADR 0026): `%.14g` is the Lua-spec format for
    // `tostring(number)`. The trailing `\0` lets snprintf consume it
    // as a C string.
    emit_string_global(context, module, i8_type, "fmt_tostring_g", "%.14g\0", loc);
    // Phase 2.7e (ADR 0028): `%lf` parses an f64 from a string for
    // `tonumber(string)` via `sscanf`.
    emit_string_global(context, module, i8_type, "fmt_tonumber_lf", "%lf\0", loc);
    // Phase 2.7f (ADR 0029): per-kind Lua type-name strings for the
    // `type(x)` builtin. Each is NUL-terminated so they can flow
    // straight through `print(type(x))` / concat without any copy.
    emit_string_global(
        context,
        module,
        i8_type,
        "s_typename_number",
        "number\0",
        loc,
    );
    emit_string_global(
        context,
        module,
        i8_type,
        "s_typename_string",
        "string\0",
        loc,
    );
    emit_string_global(
        context,
        module,
        i8_type,
        "s_typename_boolean",
        "boolean\0",
        loc,
    );
    emit_string_global(context, module, i8_type, "s_typename_nil", "nil\0", loc);
    emit_string_global(
        context,
        module,
        i8_type,
        "s_typename_function",
        "function\0",
        loc,
    );
    // Phase 2.6a-min (ADR 0053): typename for `type(t)` on tables.
    emit_string_global(context, module, i8_type, "s_typename_table", "table\0", loc);
    // Phase 2.7g (ADR 0030): diagnostic for the `assert` failure
    // path. The `printf("%s\n", _)` caller already adds the line
    // terminator, so the payload itself ends with NUL only.
    emit_string_global(
        context,
        module,
        i8_type,
        "s_assert_failed",
        "assertion failed!\0",
        loc,
    );
    // Phase 2.6a-arr (ADR 0054): out-of-bounds diagnostic.
    emit_string_global(
        context,
        module,
        i8_type,
        "s_table_oob",
        "table index out of bounds\0",
        loc,
    );
    // Phase 2.6b-hash (ADR 0058): missing-key diagnostic for
    // `t.k` / `t["k"]` reads.
    emit_string_global(
        context,
        module,
        i8_type,
        "s_table_missing_key",
        "table missing key\0",
        loc,
    );
    // Phase 2.6c-tag-arr (ADR 0059): tag-mismatch diagnostic
    // when reading a slot whose tag differs from the expected
    // kind (currently always Number on the read API surface).
    emit_string_global(
        context,
        module,
        i8_type,
        "s_table_type_mismatch",
        "table value type mismatch\0",
        loc,
    );
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
            // Phase 2.6a-arr (ADR 0054) / 2.6b-hash (ADR 0058):
            // recurse into table constructors and index expressions
            // so any string literal used as a table element or hash
            // key is seeded into the global pool.
            HirExprKind::Table(elems) => {
                for elem in elems {
                    visit_expr(elem, set);
                }
            }
            HirExprKind::Index { target, key } => {
                visit_expr(target, set);
                visit_expr(key, set);
            }
            HirExprKind::IsNilQuery { target, key } => {
                visit_expr(target, set);
                visit_expr(key, set);
            }
            HirExprKind::IndexTagged { target, key } => {
                visit_expr(target, set);
                visit_expr(key, set);
            }
            HirExprKind::IsNilLocal { .. } => {}
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
            HirStmtKind::Repeat { body, cond, .. } => {
                for st in body {
                    visit_stmt(st, set);
                }
                visit_expr(cond, set);
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
            HirStmtKind::IndexAssign { target, key, value } => {
                visit_expr(target, set);
                visit_expr(key, set);
                visit_expr(value, set);
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

    // Phase 2.6a-grow (ADR 0057): free(ptr) -> void. Used to
    // release the old `array_buf` after a doubling realloc; safe
    // on the null pointer (C99 §7.20.3.2) so callers can free
    // unconditionally if cap was 0.
    let free_ty = llvm::r#type::function(llvm::r#type::void(context), &[types.ptr], false);
    let free_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "free"))
        .function_type(TypeAttribute::new(free_ty))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(free_op.into());

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

    // exit(i32) -> void  (Phase 2.7g, ADR 0030)
    let exit_ty = llvm::r#type::function(llvm::r#type::void(context), &[types.i32], false);
    let exit_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "exit"))
        .function_type(TypeAttribute::new(exit_ty))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(exit_op.into());
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
        // Phase 2.6a-min (ADR 0053): a Table value is a `!llvm.ptr`
        // to a heap-allocated `[length: i64]`-prefixed region.
        ValueKind::Table => types.ptr,
        // Phase 2.6c-tag-locals (ADR 0063): a TaggedValue slot
        // is a 16-byte tagged region. Function params and returns
        // do not currently carry TaggedValue (function-return
        // widening is a follow-up sub-phase) — `unreachable!`
        // guards against that until the next phase relaxes it.
        ValueKind::TaggedValue => {
            unreachable!("TaggedValue is not yet a function param/return kind")
        }
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
            ValueKind::String | ValueKind::Table => types.ptr,
            ValueKind::TaggedValue => {
                unreachable!("TaggedValue is not yet a function return kind")
            }
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
    // Phase 2.5c-min (ADR 0037): upvalues become extra MLIR
    // parameters appended after the Lua params; the callsite-side
    // `lower_call` already passes their values as extra args.
    let param_types: Vec<Type<'c>> = hir_fn
        .params
        .iter()
        .map(|info| param_mlir_type(context, info.kind, types))
        .collect();
    let upvalue_types: Vec<Type<'c>> = hir_fn
        .upvalues
        .iter()
        .map(|uv| param_mlir_type(context, uv.kind, types))
        .collect();
    let all_param_types: Vec<Type<'c>> = param_types
        .iter()
        .chain(upvalue_types.iter())
        .copied()
        .collect();
    let block_args: Vec<(Type<'c>, Location<'c>)> =
        all_param_types.iter().map(|t| (*t, loc)).collect();
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

    // Phase 2.5c-min: store each upvalue's incoming block argument
    // into the slot allocated for its inner local. Body code loads
    // from `slots[uv.inner_local_id.0]` via the standard
    // `HirExprKind::Local` path — no special-case in body codegen.
    for (i, uv) in hir_fn.upvalues.iter().enumerate() {
        let block_arg_idx = hir_fn.params.len() + i;
        let arg: Value<'c, '_> = block.argument(block_arg_idx).unwrap().into();
        emit_store(&block, arg, slots[uv.inner_local_id.0], loc);
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
            // Phase 2.7a / 2.6a-min: String and Table returns
            // surface the slot's `ptr` value directly — no cast is
            // needed since the slot type already matches the
            // function signature.
            ValueKind::String | ValueKind::Table => {
                emit_load(&block, slots[ret_value_idx], types.ptr, loc)
            }
            ValueKind::TaggedValue => {
                unreachable!("TaggedValue is not yet a function return kind")
            }
        };
        ret_values.push(v);
    }
    block.append_operation(func::r#return(&ret_values, loc));
    region.append_block(block);

    let ret_types = ret_mlir_types(context, &hir_fn.ret_kinds, types);
    // Phase 2.5c-min: function signature widens to include upvalue
    // params. `all_param_types` is `param_types ++ upvalue_types`.
    let fn_type = FunctionType::new(context, &all_param_types, &ret_types);
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
                ValueKind::TaggedValue => {
                    // Phase 2.6c-tag-locals (ADR 0063): the dst is
                    // a 16-byte tagged slot. Source dispatch:
                    //   - IndexTagged: non-trapping table read +
                    //     tagged store (emit_local_init_tagged).
                    //   - Local(TaggedValue): copy the 16-byte
                    //     slot verbatim (preserves Nil tag).
                    //   - else (Number-kind expr): wrap in
                    //     `{TAG_NUMBER, value}`.
                    match &value.kind {
                        HirExprKind::IndexTagged { target, key } => {
                            emit_local_init_tagged(
                                context,
                                block,
                                slots[id.0],
                                target,
                                key,
                                slots,
                                locals,
                                functions,
                                types,
                                params_len,
                                loc,
                            )?;
                        }
                        HirExprKind::Local(LocalId(src_idx))
                            if matches!(locals[*src_idx].kind, ValueKind::TaggedValue) =>
                        {
                            let src_slot = slots[*src_idx];
                            let dst_slot = slots[id.0];
                            // tag (i64 at offset 0)
                            let tag = emit_load(block, src_slot, types.i64, loc);
                            emit_store(block, tag, dst_slot, loc);
                            // payload (f64 at offset 8)
                            let src_value_ptr = emit_byte_offset_ptr(
                                context,
                                block,
                                src_slot,
                                ARRAY_ELEM_OFF_VALUE,
                                types,
                                loc,
                            );
                            let dst_value_ptr = emit_byte_offset_ptr(
                                context,
                                block,
                                dst_slot,
                                ARRAY_ELEM_OFF_VALUE,
                                types,
                                loc,
                            );
                            // Phase 2.6c-tag-hetero (ADR 0064):
                            // copy as raw i64 so any tag's payload
                            // (f64 / i64-zext bool / ptr) survives.
                            let payload = emit_load(block, src_value_ptr, types.i64, loc);
                            emit_store(block, payload, dst_value_ptr, loc);
                        }
                        _ => {
                            let v = emit_expr(
                                context, block, value, slots, locals, functions, types, params_len,
                                loc,
                            )?;
                            // Wrap as a Number-tagged slot.
                            emit_value_slot_store_number(
                                context,
                                block,
                                slots[id.0],
                                v,
                                types,
                                loc,
                            );
                        }
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
        HirStmtKind::Repeat {
            body,
            cond,
            break_id,
        } => {
            emit_repeat(
                context, block, body, cond, *break_id, slots, locals, functions, types, params_len,
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
        // Phase 2.6a-wr (ADR 0055) / 2.6a-norm (ADR 0056) /
        // 2.6a-grow (ADR 0057): `target[key] = value` —
        // - bounds-check: key in [1, length+1] (write allows the
        //   one-past-end push slot)
        // - if key > cap: realloc array_buf (memcpy + free old),
        //   update header.cap and header.array_buf
        // - if key > length: update header.length to key
        // - reload array_buf (might have been just swapped) and
        //   store the value at offset (key-1)*8
        HirStmtKind::IndexAssign { target, key, value } => {
            let target_ptr = emit_expr(
                context, block, target, slots, locals, functions, types, params_len, loc,
            )?;
            let key_kind = infer_kind(key, locals, functions);
            let value_v = emit_expr(
                context, block, value, slots, locals, functions, types, params_len, loc,
            )?;
            match key_kind {
                ValueKind::Number => {
                    // Phase 2.6c-tag-arr (ADR 0059): hole-write
                    // enabled. Bound check is now `key >= 1`
                    // only; grow handles capacity, gap-fill marks
                    // intermediate slots as Nil-tagged.
                    let key_f = emit_expr(
                        context, block, key, slots, locals, functions, types, params_len, loc,
                    )?;
                    let key_i = emit_f2i(block, key_f, types, loc);
                    let length_i = emit_load(block, target_ptr, types.i64, loc);
                    // Lower bound: key >= 1 (else trap; same
                    // diagnostic as before via s_table_oob).
                    let one = block
                        .append_operation(arith::constant(
                            context,
                            IntegerAttribute::new(types.i64, 1).into(),
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let too_small: Value<'c, 'a> = block
                        .append_operation(arith::cmpi(
                            context,
                            arith::CmpiPredicate::Slt,
                            key_i,
                            one,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let lower_then = Region::new();
                    let lower_then_blk = Block::new(&[]);
                    {
                        let msg =
                            emit_addressof(context, &lower_then_blk, "s_table_oob", types, loc);
                        emit_exit_with_message(context, &lower_then_blk, msg, types, loc);
                        lower_then_blk.append_operation(scf::r#yield(&[], loc));
                    }
                    lower_then.append_block(lower_then_blk);
                    let lower_else = Region::new();
                    let lower_else_blk = Block::new(&[]);
                    lower_else_blk.append_operation(scf::r#yield(&[], loc));
                    lower_else.append_block(lower_else_blk);
                    block.append_operation(scf::r#if(too_small, &[], lower_then, lower_else, loc));
                    // Grow array_buf to cover key (uses cap*16
                    // sizing internally — 2.6a-grow updated below).
                    emit_table_grow_if_needed(
                        context, block, target_ptr, key_i, length_i, types, loc,
                    );
                    // Gap fill: emit_array_fill_nil iterates
                    // `[length+1, key)` (1-based, half-open) marking
                    // each slot Nil-tagged. No-op when key ≤ length+1
                    // (the runtime cmpi inside its loop is the
                    // base-case guard).
                    let array_buf = emit_table_array_buf(context, block, target_ptr, types, loc);
                    let length_plus_one: Value<'c, 'a> = block
                        .append_operation(arith::addi(length_i, one, loc))
                        .result(0)
                        .unwrap()
                        .into();
                    emit_array_fill_nil(
                        context,
                        block,
                        array_buf,
                        length_plus_one,
                        key_i,
                        types,
                        loc,
                    );
                    // Length update: if key > length, length = key.
                    let push_grew_len: Value<'c, 'a> = block
                        .append_operation(arith::cmpi(
                            context,
                            arith::CmpiPredicate::Sgt,
                            key_i,
                            length_i,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let len_then_region = Region::new();
                    let len_then_blk = Block::new(&[]);
                    {
                        let len_slot_inner = emit_byte_offset_ptr(
                            context,
                            &len_then_blk,
                            target_ptr,
                            TABLE_OFF_LEN,
                            types,
                            loc,
                        );
                        emit_store(&len_then_blk, key_i, len_slot_inner, loc);
                        len_then_blk.append_operation(scf::r#yield(&[], loc));
                    }
                    len_then_region.append_block(len_then_blk);
                    let len_else_region = Region::new();
                    let len_else_blk = Block::new(&[]);
                    len_else_blk.append_operation(scf::r#yield(&[], loc));
                    len_else_region.append_block(len_else_blk);
                    block.append_operation(scf::r#if(
                        push_grew_len,
                        &[],
                        len_then_region,
                        len_else_region,
                        loc,
                    ));
                    // Phase 2.6c-tag-hetero (ADR 0064): tagged
                    // store dispatched on value kind. Number /
                    // Bool / String land in the same 16-byte slot.
                    let value_kind = infer_kind(value, locals, functions);
                    let elem_ptr =
                        emit_array_elem_ptr(context, block, array_buf, key_i, types, loc);
                    emit_value_slot_store_dispatched(
                        context, block, elem_ptr, value_v, value_kind, types, loc,
                    );
                }
                ValueKind::String => {
                    // Phase 2.6b-hash (ADR 0058): hash insert.
                    // ensure_buf → grow_if_needed → probe → store
                    // (handle both "new key" → count++ and
                    // "existing key" → just overwrite value).
                    let key_str = emit_expr(
                        context, block, key, slots, locals, functions, types, params_len, loc,
                    )?;
                    emit_hash_ensure_buf(context, block, target_ptr, types, loc);
                    emit_hash_grow_if_needed(context, block, target_ptr, types, loc);
                    let hash_buf_slot = emit_byte_offset_ptr(
                        context,
                        block,
                        target_ptr,
                        TABLE_OFF_HASH_BUF,
                        types,
                        loc,
                    );
                    let hash_buf = emit_load(block, hash_buf_slot, types.ptr, loc);
                    let cap_slot =
                        emit_byte_offset_ptr(context, block, hash_buf, HASH_OFF_CAP, types, loc);
                    let cap = emit_load(block, cap_slot, types.i64, loc);
                    let bucket = emit_hash_probe_for_insert(
                        context, block, hash_buf, cap, key_str, types, loc,
                    );
                    let entry_size = block
                        .append_operation(arith::constant(
                            context,
                            IntegerAttribute::new(types.i64, HASH_ENTRY_SIZE).into(),
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let entries_off = block
                        .append_operation(arith::constant(
                            context,
                            IntegerAttribute::new(types.i64, HASH_OFF_ENTRIES).into(),
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let bucket_off: Value<'c, 'a> = block
                        .append_operation(arith::muli(bucket, entry_size, loc))
                        .result(0)
                        .unwrap()
                        .into();
                    let entry_off: Value<'c, 'a> = block
                        .append_operation(arith::addi(bucket_off, entries_off, loc))
                        .result(0)
                        .unwrap()
                        .into();
                    let entry_ptr = emit_byte_offset_ptr_dynamic(
                        context, block, hash_buf, entry_off, types, loc,
                    );
                    // Check whether the bucket was empty (= new key)
                    // — if so, increment count and store the key.
                    let cur_key = emit_load(block, entry_ptr, types.ptr, loc);
                    let cur_key_i = block
                        .append_operation(
                            OperationBuilder::new("llvm.ptrtoint", loc)
                                .add_operands(&[cur_key])
                                .add_results(&[types.i64])
                                .build()
                                .expect("llvm.ptrtoint"),
                        )
                        .result(0)
                        .unwrap()
                        .into();
                    let zero_i64 = block
                        .append_operation(arith::constant(
                            context,
                            IntegerAttribute::new(types.i64, 0).into(),
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let was_empty: Value<'c, 'a> = block
                        .append_operation(arith::cmpi(
                            context,
                            arith::CmpiPredicate::Eq,
                            cur_key_i,
                            zero_i64,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let value_slot_ptr = emit_byte_offset_ptr(
                        context,
                        block,
                        entry_ptr,
                        HASH_ENTRY_OFF_VALUE_SLOT,
                        types,
                        loc,
                    );
                    let value_kind = infer_kind(value, locals, functions);
                    match value_kind {
                        // Phase 2.6c-tag-hetero (ADR 0064): Number /
                        // Bool / String all share the new-key + count++
                        // path; only the final tagged store helper
                        // differs. Dispatch via
                        // `emit_value_slot_store_dispatched`.
                        ValueKind::Number | ValueKind::Bool | ValueKind::String => {
                            let new_key_then = Region::new();
                            let new_key_blk = Block::new(&[]);
                            {
                                emit_store(&new_key_blk, key_str, entry_ptr, loc);
                                let count_slot = emit_byte_offset_ptr(
                                    context,
                                    &new_key_blk,
                                    hash_buf,
                                    HASH_OFF_COUNT,
                                    types,
                                    loc,
                                );
                                let count = emit_load(&new_key_blk, count_slot, types.i64, loc);
                                let one_inner = new_key_blk
                                    .append_operation(arith::constant(
                                        context,
                                        IntegerAttribute::new(types.i64, 1).into(),
                                        loc,
                                    ))
                                    .result(0)
                                    .unwrap()
                                    .into();
                                let new_count: Value<'c, '_> = new_key_blk
                                    .append_operation(arith::addi(count, one_inner, loc))
                                    .result(0)
                                    .unwrap()
                                    .into();
                                emit_store(&new_key_blk, new_count, count_slot, loc);
                                new_key_blk.append_operation(scf::r#yield(&[], loc));
                            }
                            new_key_then.append_block(new_key_blk);
                            let new_key_else = Region::new();
                            let new_key_else_blk = Block::new(&[]);
                            new_key_else_blk.append_operation(scf::r#yield(&[], loc));
                            new_key_else.append_block(new_key_else_blk);
                            block.append_operation(scf::r#if(
                                was_empty,
                                &[],
                                new_key_then,
                                new_key_else,
                                loc,
                            ));
                            emit_value_slot_store_dispatched(
                                context,
                                block,
                                value_slot_ptr,
                                value_v,
                                value_kind,
                                types,
                                loc,
                            );
                        }
                        ValueKind::Nil => {
                            // Phase 2.6c-tag-hash-hard (ADR 0062):
                            // hard delete. When the key is present
                            // (was_empty == false), overwrite the
                            // key slot with HASH_DELETED_KEY and
                            // mark the value slot Nil. Probe will
                            // skip past the sentinel and rehash
                            // drops it physically. When the key is
                            // absent (was_empty == true), this is a
                            // no-op (Lua spec: deleting a missing
                            // key is harmless). count is unchanged
                            // either way — sentinel stays counted
                            // against load factor until rehash.
                            let del_then = Region::new();
                            let del_then_blk = Block::new(&[]);
                            {
                                let sentinel_i = del_then_blk
                                    .append_operation(arith::constant(
                                        context,
                                        IntegerAttribute::new(types.i64, HASH_DELETED_KEY).into(),
                                        loc,
                                    ))
                                    .result(0)
                                    .unwrap()
                                    .into();
                                let sentinel_ptr: Value<'c, '_> = del_then_blk
                                    .append_operation(
                                        OperationBuilder::new("llvm.inttoptr", loc)
                                            .add_operands(&[sentinel_i])
                                            .add_results(&[types.ptr])
                                            .build()
                                            .expect("llvm.inttoptr"),
                                    )
                                    .result(0)
                                    .unwrap()
                                    .into();
                                emit_store(&del_then_blk, sentinel_ptr, entry_ptr, loc);
                                emit_value_slot_store_nil(
                                    context,
                                    &del_then_blk,
                                    value_slot_ptr,
                                    types,
                                    loc,
                                );
                                del_then_blk.append_operation(scf::r#yield(&[], loc));
                            }
                            del_then.append_block(del_then_blk);
                            let del_else = Region::new();
                            let del_else_blk = Block::new(&[]);
                            del_else_blk.append_operation(scf::r#yield(&[], loc));
                            del_else.append_block(del_else_blk);
                            // del happens only when !was_empty.
                            let i1_one = block
                                .append_operation(arith::constant(
                                    context,
                                    IntegerAttribute::new(types.i1, 1).into(),
                                    loc,
                                ))
                                .result(0)
                                .unwrap()
                                .into();
                            let not_empty: Value<'c, 'a> = block
                                .append_operation(arith::xori(was_empty, i1_one, loc))
                                .result(0)
                                .unwrap()
                                .into();
                            block.append_operation(scf::r#if(
                                not_empty,
                                &[],
                                del_then,
                                del_else,
                                loc,
                            ));
                        }
                        _ => unreachable!(
                            "HIR rejects non-Number/Bool/String/Nil values for hash insert"
                        ),
                    }
                }
                _ => unreachable!("HIR rejects non-Number/String key kinds"),
            }
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
    // (elem_type, count) — most kinds are a single-element slot.
    // Phase 2.6c-tag-locals (ADR 0063): TaggedValue needs a
    // 16-byte slot laid out as `{i64 tag, f64 value}`. Allocating
    // 2 × i64 gives the required size and 8-byte alignment so the
    // f64 store/load at offset 8 is naturally aligned.
    let (elem, count) = match kind {
        ValueKind::Number => (types.f64, 1i32),
        // Bool: 1-bit. Nil: nominally 1-bit too — the value is never
        // observed (heterogeneous `==` and print(nil) both bypass
        // the slot via static dispatch), but storage is uniform.
        ValueKind::Bool | ValueKind::Nil => (types.i1, 1),
        // Phase 2.7a (ADR 0024): a String slot stores a `ptr` to a
        // static C-string global.
        ValueKind::String => (types.ptr, 1),
        // Phase 2.5b.3 (ADR 0019): Function values stored as opaque
        // pointers — `!func.func<...>` values are bridged via
        // `unrealized_conversion_cast` at store/load.
        ValueKind::Function(_) => (types.ptr, 1),
        // Phase 2.6a-min (ADR 0053): Table slot stores a heap ptr.
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

/// Phase 2.6a-norm (ADR 0056): emit a null `!llvm.ptr` constant
/// via `llvm.mlir.zero`. Used to initialise the `array_buf` field
/// for empty tables and the `hash_buf` field (always null until
/// 2.6b lights it up).
fn emit_null_ptr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let _ = context;
    let zero_op = OperationBuilder::new("llvm.mlir.zero", loc)
        .add_results(&[types.ptr])
        .build()
        .expect("llvm.mlir.zero");
    block.append_operation(zero_op).result(0).unwrap().into()
}

/// Phase 2.6a-norm (ADR 0056): load the `array_buf` ptr from a
/// table header. Used by index read, index write, and 2.6a-grow's
/// post-realloc reload. Three call sites — rule of three paid.
fn emit_table_array_buf<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    table_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let slot = emit_byte_offset_ptr(context, block, table_ptr, TABLE_OFF_ARRAY_BUF, types, loc);
    emit_load(block, slot, types.ptr, loc)
}

/// Phase 2.6c-tag-arr (ADR 0059): compute the byte address of the
/// 16-byte slot at 1-based `key_i` inside `array_buf`. The slot
/// holds `{i64 tag, f64 value}`; callers add `+0` for tag and
/// `+8` for value via the offset constants.
fn emit_array_elem_ptr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    array_buf: Value<'c, 'a>,
    key_i: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let one = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, 1).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let zero_idx: Value<'c, 'a> = block
        .append_operation(arith::subi(key_i, one, loc))
        .result(0)
        .unwrap()
        .into();
    let elem_size = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, ARRAY_ELEM_SIZE).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let offset: Value<'c, 'a> = block
        .append_operation(arith::muli(zero_idx, elem_size, loc))
        .result(0)
        .unwrap()
        .into();
    emit_byte_offset_ptr_dynamic(context, block, array_buf, offset, types, loc)
}

/// Phase 2.6c-tag-arr (ADR 0059) / 2.6c-tag-hash (ADR 0060):
/// write `{tag=Number, value}` to a 16-byte tagged value slot.
/// Used by both array element stores and hash entry value
/// stores — the slot layout is identical in both contexts.
fn emit_value_slot_store_number<'a, 'c>(
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
fn emit_value_slot_store_nil<'a, 'c>(
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
fn emit_value_slot_store_bool<'a, 'c>(
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
fn emit_value_slot_store_string<'a, 'c>(
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

/// Phase 2.6c-tag-hetero (ADR 0064): kind-dispatched store into a
/// tagged value slot. Mirrors the value-side dispatch used by
/// `Table` constructor / `IndexAssign` / `LocalInit` so each
/// caller has a single entry point regardless of element kind.
fn emit_value_slot_store_dispatched<'a, 'c>(
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
        _ => unreachable!("unsupported value kind for tagged slot store: {:?}", kind),
    }
}

/// Phase 2.6c-tag-arr (ADR 0059) / 2.6c-tag-hash (ADR 0060):
/// load the tag at offset 0 of a tagged value slot and trap if
/// it isn't `TAG_NUMBER`. After this returns, the caller can
/// safely load the f64 value at offset 8.
fn emit_value_slot_check_number<'a, 'c>(
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

/// Phase 2.6c-tag-locals (ADR 0063): non-trapping `IndexTagged`
/// read that writes the resulting `{tag, value}` directly to a
/// `TaggedValue` local slot. Mirror of the `IsNilQuery`
/// codegen — same bounds / probe / null-buf shape — but instead
/// of yielding an i1 we write either `{TAG_NIL, 0.0}` or
/// `{TAG_NUMBER, value}` to `dst_slot`.
#[allow(clippy::too_many_arguments)]
fn emit_local_init_tagged<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    dst_slot: Value<'c, 'a>,
    target: &HirExpr,
    key: &HirExpr,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let target_ptr = emit_expr(
        context, block, target, slots, locals, functions, types, params_len, loc,
    )?;
    let key_kind = infer_kind(key, locals, functions);
    match key_kind {
        ValueKind::Number => {
            let key_f = emit_expr(
                context, block, key, slots, locals, functions, types, params_len, loc,
            )?;
            let key_i = emit_f2i(block, key_f, types, loc);
            let length_i = emit_load(block, target_ptr, types.i64, loc);
            let one = block
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let too_small: Value<'c, 'a> = block
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Slt,
                    key_i,
                    one,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let too_big: Value<'c, 'a> = block
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Sgt,
                    key_i,
                    length_i,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let oob: Value<'c, 'a> = block
                .append_operation(arith::ori(too_small, too_big, loc))
                .result(0)
                .unwrap()
                .into();
            let then_region = Region::new();
            let then_blk = Block::new(&[]);
            emit_value_slot_store_nil(context, &then_blk, dst_slot, types, loc);
            then_blk.append_operation(scf::r#yield(&[], loc));
            then_region.append_block(then_blk);
            let else_region = Region::new();
            let else_blk = Block::new(&[]);
            {
                let array_buf = emit_table_array_buf(context, &else_blk, target_ptr, types, loc);
                let elem_ptr =
                    emit_array_elem_ptr(context, &else_blk, array_buf, key_i, types, loc);
                // Copy the 16-byte tagged slot verbatim — the
                // source already lives as `{tag, value}` (ADR
                // 0059), and Nil-tagged source maps to Nil-
                // tagged local slot directly.
                let tag = emit_load(&else_blk, elem_ptr, types.i64, loc);
                emit_store(&else_blk, tag, dst_slot, loc);
                let src_value_ptr = emit_byte_offset_ptr(
                    context,
                    &else_blk,
                    elem_ptr,
                    ARRAY_ELEM_OFF_VALUE,
                    types,
                    loc,
                );
                let dst_value_ptr = emit_byte_offset_ptr(
                    context,
                    &else_blk,
                    dst_slot,
                    ARRAY_ELEM_OFF_VALUE,
                    types,
                    loc,
                );
                // Phase 2.6c-tag-hetero (ADR 0064): raw i64 copy.
                let payload = emit_load(&else_blk, src_value_ptr, types.i64, loc);
                emit_store(&else_blk, payload, dst_value_ptr, loc);
                else_blk.append_operation(scf::r#yield(&[], loc));
            }
            else_region.append_block(else_blk);
            block.append_operation(scf::r#if(oob, &[], then_region, else_region, loc));
        }
        ValueKind::String => {
            let key_str = emit_expr(
                context, block, key, slots, locals, functions, types, params_len, loc,
            )?;
            let hash_buf_slot =
                emit_byte_offset_ptr(context, block, target_ptr, TABLE_OFF_HASH_BUF, types, loc);
            let hash_buf = emit_load(block, hash_buf_slot, types.ptr, loc);
            let hash_buf_i: Value<'c, 'a> = block
                .append_operation(
                    OperationBuilder::new("llvm.ptrtoint", loc)
                        .add_operands(&[hash_buf])
                        .add_results(&[types.i64])
                        .build()
                        .expect("llvm.ptrtoint"),
                )
                .result(0)
                .unwrap()
                .into();
            let zero_i64 = block
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, 0).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let hash_buf_null: Value<'c, 'a> = block
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Eq,
                    hash_buf_i,
                    zero_i64,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let then_region = Region::new();
            let then_blk = Block::new(&[]);
            emit_value_slot_store_nil(context, &then_blk, dst_slot, types, loc);
            then_blk.append_operation(scf::r#yield(&[], loc));
            then_region.append_block(then_blk);
            let else_region = Region::new();
            let else_blk = Block::new(&[]);
            {
                let cap_slot =
                    emit_byte_offset_ptr(context, &else_blk, hash_buf, HASH_OFF_CAP, types, loc);
                let cap = emit_load(&else_blk, cap_slot, types.i64, loc);
                let bucket = emit_hash_probe_for_insert(
                    context, &else_blk, hash_buf, cap, key_str, types, loc,
                );
                let entry_size = else_blk
                    .append_operation(arith::constant(
                        context,
                        IntegerAttribute::new(types.i64, HASH_ENTRY_SIZE).into(),
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let bucket_off: Value<'c, '_> = else_blk
                    .append_operation(arith::muli(bucket, entry_size, loc))
                    .result(0)
                    .unwrap()
                    .into();
                let entries_base = else_blk
                    .append_operation(arith::constant(
                        context,
                        IntegerAttribute::new(types.i64, HASH_OFF_ENTRIES).into(),
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let entry_total_off: Value<'c, '_> = else_blk
                    .append_operation(arith::addi(bucket_off, entries_base, loc))
                    .result(0)
                    .unwrap()
                    .into();
                let entry_ptr = emit_byte_offset_ptr_dynamic(
                    context,
                    &else_blk,
                    hash_buf,
                    entry_total_off,
                    types,
                    loc,
                );
                let key_at = emit_load(&else_blk, entry_ptr, types.ptr, loc);
                let key_at_i: Value<'c, '_> = else_blk
                    .append_operation(
                        OperationBuilder::new("llvm.ptrtoint", loc)
                            .add_operands(&[key_at])
                            .add_results(&[types.i64])
                            .build()
                            .expect("llvm.ptrtoint"),
                    )
                    .result(0)
                    .unwrap()
                    .into();
                let zero_inner = else_blk
                    .append_operation(arith::constant(
                        context,
                        IntegerAttribute::new(types.i64, 0).into(),
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let key_at_null: Value<'c, '_> = else_blk
                    .append_operation(arith::cmpi(
                        context,
                        arith::CmpiPredicate::Eq,
                        key_at_i,
                        zero_inner,
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                // Inner scf.if: missing key → store Nil; else
                // copy the 16-byte tagged value slot.
                let inner_then = Region::new();
                let inner_then_blk = Block::new(&[]);
                emit_value_slot_store_nil(context, &inner_then_blk, dst_slot, types, loc);
                inner_then_blk.append_operation(scf::r#yield(&[], loc));
                inner_then.append_block(inner_then_blk);
                let inner_else = Region::new();
                let inner_else_blk = Block::new(&[]);
                {
                    let value_slot_ptr = emit_byte_offset_ptr(
                        context,
                        &inner_else_blk,
                        entry_ptr,
                        HASH_ENTRY_OFF_VALUE_SLOT,
                        types,
                        loc,
                    );
                    let tag = emit_load(&inner_else_blk, value_slot_ptr, types.i64, loc);
                    emit_store(&inner_else_blk, tag, dst_slot, loc);
                    let src_value_ptr = emit_byte_offset_ptr(
                        context,
                        &inner_else_blk,
                        value_slot_ptr,
                        ARRAY_ELEM_OFF_VALUE,
                        types,
                        loc,
                    );
                    let dst_value_ptr = emit_byte_offset_ptr(
                        context,
                        &inner_else_blk,
                        dst_slot,
                        ARRAY_ELEM_OFF_VALUE,
                        types,
                        loc,
                    );
                    // Phase 2.6c-tag-hetero (ADR 0064): raw i64 copy.
                    let payload = emit_load(&inner_else_blk, src_value_ptr, types.i64, loc);
                    emit_store(&inner_else_blk, payload, dst_value_ptr, loc);
                    inner_else_blk.append_operation(scf::r#yield(&[], loc));
                }
                inner_else.append_block(inner_else_blk);
                else_blk.append_operation(scf::r#if(key_at_null, &[], inner_then, inner_else, loc));
                else_blk.append_operation(scf::r#yield(&[], loc));
            }
            else_region.append_block(else_blk);
            block.append_operation(scf::r#if(hash_buf_null, &[], then_region, else_region, loc));
        }
        _ => unreachable!("IndexTagged key must be Number or String"),
    }
    Ok(())
}

/// Phase 2.6c-tag-arr (ADR 0059): fill slots `[from_idx, to_idx)`
/// (1-based, half-open) with TAG_NIL. Used by hole-write to make
/// gap slots deterministically Nil rather than UB. No-op when
/// `from_idx >= to_idx` (covered by the runtime cmpi inside the
/// while loop).
fn emit_array_fill_nil<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    array_buf: Value<'c, 'a>,
    from_idx: Value<'c, 'a>,
    to_idx: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let before_region = Region::new();
    let before_blk = Block::new(&[(types.i64, loc)]);
    {
        let i_arg: Value<'c, '_> = before_blk.argument(0).unwrap().into();
        let cond: Value<'c, '_> = before_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Slt,
                i_arg,
                to_idx,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        before_blk.append_operation(scf::condition(cond, &[i_arg], loc));
    }
    before_region.append_block(before_blk);
    let after_region = Region::new();
    let after_blk = Block::new(&[(types.i64, loc)]);
    {
        let i_arg: Value<'c, '_> = after_blk.argument(0).unwrap().into();
        let elem_ptr = emit_array_elem_ptr(context, &after_blk, array_buf, i_arg, types, loc);
        emit_value_slot_store_nil(context, &after_blk, elem_ptr, types, loc);
        let one = after_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 1).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let next: Value<'c, '_> = after_blk
            .append_operation(arith::addi(i_arg, one, loc))
            .result(0)
            .unwrap()
            .into();
        after_blk.append_operation(scf::r#yield(&[next], loc));
    }
    after_region.append_block(after_blk);
    let while_op = scf::r#while(&[from_idx], &[types.i64], before_region, after_region, loc);
    block.append_operation(while_op);
}

/// Phase 2.6a-grow (ADR 0057): if `key_i > capacity`, allocate a
/// new array_buf with `max(cap*2, key_i)` slots, memcpy the
/// existing `length_i` elements (only when cap > 0; gates against
/// null source UB), free the old buffer, and update the header's
/// capacity (offset 8) and array_buf (offset 16) fields. Must be
/// called *after* the bounds check so we know `key_i ≥ 1` and we
/// won't grow unboundedly.
fn emit_table_grow_if_needed<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    table_ptr: Value<'c, 'a>,
    key_i: Value<'c, 'a>,
    length_i: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    // cap = load i64, header + 8
    let cap_slot = emit_byte_offset_ptr(context, block, table_ptr, TABLE_OFF_CAP, types, loc);
    let cap = emit_load(block, cap_slot, types.i64, loc);
    // need_grow = key_i > cap
    let need_grow: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Sgt,
            key_i,
            cap,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    {
        let two = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 2).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let cap_doubled: Value<'c, '_> = then_blk
            .append_operation(arith::muli(cap, two, loc))
            .result(0)
            .unwrap()
            .into();
        let new_cap: Value<'c, '_> = then_blk
            .append_operation(arith::maxsi(cap_doubled, key_i, loc))
            .result(0)
            .unwrap()
            .into();
        // Phase 2.6c-tag-arr (ADR 0059): each slot is now 16
        // bytes (`{i64 tag, f64 value}`).
        let elem_size = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, ARRAY_ELEM_SIZE).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let new_size: Value<'c, '_> = then_blk
            .append_operation(arith::muli(new_cap, elem_size, loc))
            .result(0)
            .unwrap()
            .into();
        let new_buf = emit_libc_call_ptr(context, &then_blk, "malloc", &[new_size], types, loc);
        // Conditional copy + free: only when cap > 0 (i.e. there
        // was a previous non-null array_buf).
        let zero_i64 = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let had_old: Value<'c, '_> = then_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Sgt,
                cap,
                zero_i64,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let copy_then = Region::new();
        let copy_blk = Block::new(&[]);
        {
            let array_buf_slot = emit_byte_offset_ptr(
                context,
                &copy_blk,
                table_ptr,
                TABLE_OFF_ARRAY_BUF,
                types,
                loc,
            );
            let old_buf = emit_load(&copy_blk, array_buf_slot, types.ptr, loc);
            // Phase 2.6c-tag-arr (ADR 0059): copy 16 bytes per
            // existing slot (was 8 bytes pre-tagging).
            let elem_size_inner = copy_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, ARRAY_ELEM_SIZE).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let copy_size: Value<'c, '_> = copy_blk
                .append_operation(arith::muli(length_i, elem_size_inner, loc))
                .result(0)
                .unwrap()
                .into();
            let _ = emit_libc_call_ptr(
                context,
                &copy_blk,
                "memcpy",
                &[new_buf, old_buf, copy_size],
                types,
                loc,
            );
            emit_libc_call_void(context, &copy_blk, "free", &[old_buf], loc);
            copy_blk.append_operation(scf::r#yield(&[], loc));
        }
        copy_then.append_block(copy_blk);
        let copy_else = Region::new();
        let copy_else_blk = Block::new(&[]);
        copy_else_blk.append_operation(scf::r#yield(&[], loc));
        copy_else.append_block(copy_else_blk);
        then_blk.append_operation(scf::r#if(had_old, &[], copy_then, copy_else, loc));
        // Update header.capacity and header.array_buf.
        let cap_slot_inner =
            emit_byte_offset_ptr(context, &then_blk, table_ptr, TABLE_OFF_CAP, types, loc);
        emit_store(&then_blk, new_cap, cap_slot_inner, loc);
        let array_buf_slot_inner = emit_byte_offset_ptr(
            context,
            &then_blk,
            table_ptr,
            TABLE_OFF_ARRAY_BUF,
            types,
            loc,
        );
        emit_store(&then_blk, new_buf, array_buf_slot_inner, loc);
        then_blk.append_operation(scf::r#yield(&[], loc));
    }
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    else_blk.append_operation(scf::r#yield(&[], loc));
    else_region.append_block(else_blk);
    block.append_operation(scf::r#if(need_grow, &[], then_region, else_region, loc));
}

/// Phase 2.6b-hash (ADR 0058): FNV-1a 64-bit hash of a NUL-
/// terminated C string. `strlen` gives the byte count, then a
/// `scf.while` loop folds each byte into the running hash.
/// Returns the i64 hash value (bit pattern interpreted as
/// unsigned for bucket masking).
fn emit_string_hash<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    str_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    // FNV-1a 64-bit constants: offset basis and prime. The offset
    // basis 14695981039346656037 doesn't fit in i64 as positive,
    // but as bit pattern it equals -3750763034362895579 i64 —
    // store via the wrapping reinterpretation.
    const FNV_OFFSET_BASIS: i64 = -3750763034362895579;
    const FNV_PRIME: i64 = 1099511628211;
    let len_i64 = emit_libc_call_i64(context, block, "strlen", &[str_ptr], types, loc);
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
    // scf.while [hash, idx] -> hash:
    //   before: yield (idx < len) carrying hash, idx
    //   after : load byte, hash = (hash xor byte) * prime; idx++; yield hash, idx
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
        // byte_ptr = str_ptr + idx (gep i8)
        let byte_ptr =
            emit_byte_offset_ptr_dynamic(context, &after_blk, str_ptr, idx_arg, types, loc);
        // i8 byte → i64 zero-extended
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
        let new_idx: Value<'c, '_> = after_blk
            .append_operation(arith::addi(idx_arg, one, loc))
            .result(0)
            .unwrap()
            .into();
        after_blk.append_operation(scf::r#yield(&[new_hash, new_idx], loc));
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

/// Phase 2.6b-hash (ADR 0058): if `header.hash_buf` is null,
/// allocate the initial hash region (header for cap+count, then
/// `cap` zeroed entries), and store the buffer pointer in the
/// table header. After this returns, `header.hash_buf` is
/// guaranteed non-null with cap=HASH_INITIAL_CAP and count=0.
fn emit_hash_ensure_buf<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    table_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let hash_buf_slot =
        emit_byte_offset_ptr(context, block, table_ptr, TABLE_OFF_HASH_BUF, types, loc);
    let hash_buf = emit_load(block, hash_buf_slot, types.ptr, loc);
    // is_null = ptrtoint(hash_buf) == 0
    let hash_buf_i = block
        .append_operation(
            OperationBuilder::new("llvm.ptrtoint", loc)
                .add_operands(&[hash_buf])
                .add_results(&[types.i64])
                .build()
                .expect("llvm.ptrtoint"),
        )
        .result(0)
        .unwrap()
        .into();
    let zero = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, 0).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let is_null: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            hash_buf_i,
            zero,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    {
        let total_size = HASH_OFF_ENTRIES + HASH_INITIAL_CAP * HASH_ENTRY_SIZE;
        let size_const = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, total_size).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let new_buf = emit_libc_call_ptr(context, &then_blk, "malloc", &[size_const], types, loc);
        // Initialize header.cap and header.count
        let cap_const = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, HASH_INITIAL_CAP).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let cap_slot = emit_byte_offset_ptr(context, &then_blk, new_buf, HASH_OFF_CAP, types, loc);
        emit_store(&then_blk, cap_const, cap_slot, loc);
        let zero_const = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let count_slot =
            emit_byte_offset_ptr(context, &then_blk, new_buf, HASH_OFF_COUNT, types, loc);
        emit_store(&then_blk, zero_const, count_slot, loc);
        // Zero-init each entry's key field (value field will be
        // written on first occupancy; only key=null marks empty).
        let null_key = emit_null_ptr(context, &then_blk, types, loc);
        for i in 0..HASH_INITIAL_CAP {
            let entry_off = HASH_OFF_ENTRIES + i * HASH_ENTRY_SIZE + HASH_ENTRY_OFF_KEY;
            let key_slot = emit_byte_offset_ptr(context, &then_blk, new_buf, entry_off, types, loc);
            emit_store(&then_blk, null_key, key_slot, loc);
        }
        // Store buf into header.
        let hash_buf_slot_inner = emit_byte_offset_ptr(
            context,
            &then_blk,
            table_ptr,
            TABLE_OFF_HASH_BUF,
            types,
            loc,
        );
        emit_store(&then_blk, new_buf, hash_buf_slot_inner, loc);
        then_blk.append_operation(scf::r#yield(&[], loc));
    }
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    else_blk.append_operation(scf::r#yield(&[], loc));
    else_region.append_block(else_blk);
    block.append_operation(scf::r#if(is_null, &[], then_region, else_region, loc));
}

/// Phase 2.6c-tag-hash-tidy (Tidy First): the unified scf.while
/// skeleton shared by `emit_hash_probe_lookup` and
/// `emit_hash_probe_for_insert`. Walks `hash_buf` from
/// `hash(key_str) & mask` forward, skipping deleted-tombstone
/// buckets (`HASH_DELETED_KEY`), until a stop condition fires.
///
/// `trap_on_null = true` (lookup):
///   - empty bucket (key == null) traps with `s_table_missing_key`
///   - matching key terminates the loop and the bucket is returned
///
/// `trap_on_null = false` (insert / rehash placement):
///   - empty bucket terminates the loop (caller treats as
///     "insert here", `was_empty == true`)
///   - matching key terminates the loop (caller overwrites)
///
/// Sentinel buckets are never the terminating answer in either
/// mode; the probe walks past them. Insert with sentinel reuse
/// is intentionally not implemented — sentinels are dropped at
/// the next rehash (ADR 0062).
#[allow(clippy::too_many_arguments)]
fn emit_hash_probe_loop<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    hash_buf: Value<'c, 'a>,
    cap: Value<'c, 'a>,
    key_str: Value<'c, 'a>,
    trap_on_null: bool,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let one = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, 1).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let mask: Value<'c, 'a> = block
        .append_operation(arith::subi(cap, one, loc))
        .result(0)
        .unwrap()
        .into();
    let hash = emit_string_hash(context, block, key_str, types, loc);
    let initial_bucket: Value<'c, 'a> = block
        .append_operation(arith::andi(hash, mask, loc))
        .result(0)
        .unwrap()
        .into();
    let before_region = Region::new();
    let before_blk = Block::new(&[(types.i64, loc)]);
    {
        let bucket_arg: Value<'c, '_> = before_blk.argument(0).unwrap().into();
        let entry_size = before_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, HASH_ENTRY_SIZE).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let entries_base = before_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, HASH_OFF_ENTRIES).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let bucket_off: Value<'c, '_> = before_blk
            .append_operation(arith::muli(bucket_arg, entry_size, loc))
            .result(0)
            .unwrap()
            .into();
        let entry_off: Value<'c, '_> = before_blk
            .append_operation(arith::addi(bucket_off, entries_base, loc))
            .result(0)
            .unwrap()
            .into();
        let entry_ptr =
            emit_byte_offset_ptr_dynamic(context, &before_blk, hash_buf, entry_off, types, loc);
        let key_at_slot = emit_load(&before_blk, entry_ptr, types.ptr, loc);
        let key_at_slot_i = before_blk
            .append_operation(
                OperationBuilder::new("llvm.ptrtoint", loc)
                    .add_operands(&[key_at_slot])
                    .add_results(&[types.i64])
                    .build()
                    .expect("llvm.ptrtoint"),
            )
            .result(0)
            .unwrap()
            .into();
        let zero_i64 = before_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let is_null: Value<'c, '_> = before_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                key_at_slot_i,
                zero_i64,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let sentinel_const = before_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, HASH_DELETED_KEY).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let is_sentinel: Value<'c, '_> = before_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                key_at_slot_i,
                sentinel_const,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let is_skip: Value<'c, '_> = before_blk
            .append_operation(arith::ori(is_null, is_sentinel, loc))
            .result(0)
            .unwrap()
            .into();
        // Lookup-only: trap on empty bucket before strcmp. Insert
        // mode treats empty as a successful terminator instead.
        if trap_on_null {
            let null_trap_then = Region::new();
            let null_trap_blk = Block::new(&[]);
            let msg_ptr =
                emit_addressof(context, &null_trap_blk, "s_table_missing_key", types, loc);
            emit_exit_with_message(context, &null_trap_blk, msg_ptr, types, loc);
            null_trap_blk.append_operation(scf::r#yield(&[], loc));
            null_trap_then.append_block(null_trap_blk);
            let null_trap_else = Region::new();
            let null_trap_else_blk = Block::new(&[]);
            null_trap_else_blk.append_operation(scf::r#yield(&[], loc));
            null_trap_else.append_block(null_trap_else_blk);
            before_blk.append_operation(scf::r#if(
                is_null,
                &[],
                null_trap_then,
                null_trap_else,
                loc,
            ));
        }
        // strcmp gated on is_skip (null OR sentinel): yield false
        // directly when we cannot legally compare, otherwise call.
        let eq_then = Region::new();
        let eq_then_blk = Block::new(&[]);
        {
            let i1_zero = eq_then_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i1, 0).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            eq_then_blk.append_operation(scf::r#yield(&[i1_zero], loc));
        }
        eq_then.append_block(eq_then_blk);
        let eq_else = Region::new();
        let eq_else_blk = Block::new(&[]);
        {
            let cmp = emit_libc_call_i32(
                context,
                &eq_else_blk,
                "strcmp",
                &[key_at_slot, key_str],
                types,
                loc,
            );
            let zero_i32 = eq_else_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i32, 0).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let eq_inner: Value<'c, '_> = eq_else_blk
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
            eq_else_blk.append_operation(scf::r#yield(&[eq_inner], loc));
        }
        eq_else.append_block(eq_else_blk);
        let eq_op = scf::r#if(is_skip, &[types.i1], eq_then, eq_else, loc);
        let eq: Value<'c, '_> = before_blk.append_operation(eq_op).result(0).unwrap().into();
        // Lookup: terminate on eq. Insert: terminate on (is_null OR eq).
        let terminate: Value<'c, '_> = if trap_on_null {
            eq
        } else {
            before_blk
                .append_operation(arith::ori(is_null, eq, loc))
                .result(0)
                .unwrap()
                .into()
        };
        let i1_one = before_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i1, 1).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let keep_going: Value<'c, '_> = before_blk
            .append_operation(arith::xori(terminate, i1_one, loc))
            .result(0)
            .unwrap()
            .into();
        before_blk.append_operation(scf::condition(keep_going, &[bucket_arg], loc));
    }
    before_region.append_block(before_blk);
    let after_region = Region::new();
    let after_blk = Block::new(&[(types.i64, loc)]);
    {
        let bucket_arg: Value<'c, '_> = after_blk.argument(0).unwrap().into();
        let one_inner = after_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 1).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let plus1: Value<'c, '_> = after_blk
            .append_operation(arith::addi(bucket_arg, one_inner, loc))
            .result(0)
            .unwrap()
            .into();
        let next: Value<'c, '_> = after_blk
            .append_operation(arith::andi(plus1, mask, loc))
            .result(0)
            .unwrap()
            .into();
        after_blk.append_operation(scf::r#yield(&[next], loc));
    }
    after_region.append_block(after_blk);
    let while_op = scf::r#while(
        &[initial_bucket],
        &[types.i64],
        before_region,
        after_region,
        loc,
    );
    block.append_operation(while_op).result(0).unwrap().into()
}

/// Phase 2.6b-hash (ADR 0058): probe `hash_buf` for the bucket
/// matching `key_str`. Returns the i64 bucket index on success
/// or traps with `s_table_missing_key` if the probe walks into
/// an empty bucket (= key not present). Sentinel buckets (ADR
/// 0062) are skipped.
///
/// Caller must guarantee the buffer is non-null; for lookup that
/// means `header.hash_buf != null` (else trap before calling).
fn emit_hash_probe_lookup<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    hash_buf: Value<'c, 'a>,
    cap: Value<'c, 'a>,
    key_str: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    emit_hash_probe_loop(context, block, hash_buf, cap, key_str, true, types, loc)
}

/// Phase 2.6b-hash (ADR 0058): walk `hash_buf` from a starting
/// bucket until either an empty slot OR a slot whose key equals
/// `key_str` is found, and return that bucket. Sentinel buckets
/// (ADR 0062) are skipped — no reuse. Used by the insert path,
/// the rehash placement path, and the IsNilQuery hash arm.
fn emit_hash_probe_for_insert<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    hash_buf: Value<'c, 'a>,
    cap: Value<'c, 'a>,
    key_str: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    emit_hash_probe_loop(context, block, hash_buf, cap, key_str, false, types, loc)
}

/// Phase 2.6b-hash (ADR 0058): if load factor ≥ 0.75, double the
/// `hash_cap` and rehash all existing entries into a fresh
/// buffer. Old buffer is freed; header.hash_buf is updated.
/// Caller must have already ensured `hash_buf` is non-null.
fn emit_hash_grow_if_needed<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    table_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let hash_buf_slot =
        emit_byte_offset_ptr(context, block, table_ptr, TABLE_OFF_HASH_BUF, types, loc);
    let hash_buf = emit_load(block, hash_buf_slot, types.ptr, loc);
    let cap_slot = emit_byte_offset_ptr(context, block, hash_buf, HASH_OFF_CAP, types, loc);
    let cap = emit_load(block, cap_slot, types.i64, loc);
    let count_slot = emit_byte_offset_ptr(context, block, hash_buf, HASH_OFF_COUNT, types, loc);
    let count = emit_load(block, count_slot, types.i64, loc);
    // Need rehash if count*4 >= cap*3.
    let four = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, 4).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let three = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, 3).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let count4: Value<'c, 'a> = block
        .append_operation(arith::muli(count, four, loc))
        .result(0)
        .unwrap()
        .into();
    let cap3: Value<'c, 'a> = block
        .append_operation(arith::muli(cap, three, loc))
        .result(0)
        .unwrap()
        .into();
    let need_grow: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Sge,
            count4,
            cap3,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    {
        let two = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 2).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let new_cap: Value<'c, '_> = then_blk
            .append_operation(arith::muli(cap, two, loc))
            .result(0)
            .unwrap()
            .into();
        let entry_size = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, HASH_ENTRY_SIZE).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let entries_size: Value<'c, '_> = then_blk
            .append_operation(arith::muli(new_cap, entry_size, loc))
            .result(0)
            .unwrap()
            .into();
        let header_off = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, HASH_OFF_ENTRIES).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let total_size: Value<'c, '_> = then_blk
            .append_operation(arith::addi(entries_size, header_off, loc))
            .result(0)
            .unwrap()
            .into();
        let new_buf = emit_libc_call_ptr(context, &then_blk, "malloc", &[total_size], types, loc);
        // header init (cap and count=count; count stays since
        // every old entry will be reinserted)
        let new_cap_slot =
            emit_byte_offset_ptr(context, &then_blk, new_buf, HASH_OFF_CAP, types, loc);
        emit_store(&then_blk, new_cap, new_cap_slot, loc);
        let new_count_slot =
            emit_byte_offset_ptr(context, &then_blk, new_buf, HASH_OFF_COUNT, types, loc);
        emit_store(&then_blk, count, new_count_slot, loc);
        // Null-init each entry's key in new_buf via scf.while [i].
        let null_key = emit_null_ptr(context, &then_blk, types, loc);
        let zero_idx = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let init_before = Region::new();
        let init_before_blk = Block::new(&[(types.i64, loc)]);
        {
            let i_arg: Value<'c, '_> = init_before_blk.argument(0).unwrap().into();
            let cond: Value<'c, '_> = init_before_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Slt,
                    i_arg,
                    new_cap,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            init_before_blk.append_operation(scf::condition(cond, &[i_arg], loc));
        }
        init_before.append_block(init_before_blk);
        let init_after = Region::new();
        let init_after_blk = Block::new(&[(types.i64, loc)]);
        {
            let i_arg: Value<'c, '_> = init_after_blk.argument(0).unwrap().into();
            let entry_size_inner = init_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, HASH_ENTRY_SIZE).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let entries_off_inner = init_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, HASH_OFF_ENTRIES).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let off_part: Value<'c, '_> = init_after_blk
                .append_operation(arith::muli(i_arg, entry_size_inner, loc))
                .result(0)
                .unwrap()
                .into();
            let off: Value<'c, '_> = init_after_blk
                .append_operation(arith::addi(off_part, entries_off_inner, loc))
                .result(0)
                .unwrap()
                .into();
            let key_slot =
                emit_byte_offset_ptr_dynamic(context, &init_after_blk, new_buf, off, types, loc);
            emit_store(&init_after_blk, null_key, key_slot, loc);
            let one_i = init_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let next_i: Value<'c, '_> = init_after_blk
                .append_operation(arith::addi(i_arg, one_i, loc))
                .result(0)
                .unwrap()
                .into();
            init_after_blk.append_operation(scf::r#yield(&[next_i], loc));
        }
        init_after.append_block(init_after_blk);
        let init_while = scf::r#while(&[zero_idx], &[types.i64], init_before, init_after, loc);
        then_blk.append_operation(init_while);
        // Rehash: walk old buf, for each non-null key probe in
        // new_buf and store. scf.while [i].
        let rehash_zero = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let rehash_before = Region::new();
        let rehash_before_blk = Block::new(&[(types.i64, loc)]);
        {
            let i_arg: Value<'c, '_> = rehash_before_blk.argument(0).unwrap().into();
            let cond: Value<'c, '_> = rehash_before_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Slt,
                    i_arg,
                    cap,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            rehash_before_blk.append_operation(scf::condition(cond, &[i_arg], loc));
        }
        rehash_before.append_block(rehash_before_blk);
        let rehash_after = Region::new();
        let rehash_after_blk = Block::new(&[(types.i64, loc)]);
        {
            let i_arg: Value<'c, '_> = rehash_after_blk.argument(0).unwrap().into();
            let entry_size_inner = rehash_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, HASH_ENTRY_SIZE).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let entries_off_inner = rehash_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, HASH_OFF_ENTRIES).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let off_part: Value<'c, '_> = rehash_after_blk
                .append_operation(arith::muli(i_arg, entry_size_inner, loc))
                .result(0)
                .unwrap()
                .into();
            let off: Value<'c, '_> = rehash_after_blk
                .append_operation(arith::addi(off_part, entries_off_inner, loc))
                .result(0)
                .unwrap()
                .into();
            let old_entry_ptr =
                emit_byte_offset_ptr_dynamic(context, &rehash_after_blk, hash_buf, off, types, loc);
            let old_key = emit_load(&rehash_after_blk, old_entry_ptr, types.ptr, loc);
            let old_key_i = rehash_after_blk
                .append_operation(
                    OperationBuilder::new("llvm.ptrtoint", loc)
                        .add_operands(&[old_key])
                        .add_results(&[types.i64])
                        .build()
                        .expect("llvm.ptrtoint"),
                )
                .result(0)
                .unwrap()
                .into();
            let zero_i64_inner = rehash_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, 0).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let not_null: Value<'c, '_> = rehash_after_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Ne,
                    old_key_i,
                    zero_i64_inner,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            // Phase 2.6c-tag-hash-hard (ADR 0062): treat sentinel
            // entries as unoccupied so rehash physically drops
            // them. Live = (key != null && key != sentinel).
            let sentinel_const_inner = rehash_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, HASH_DELETED_KEY).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let not_sentinel: Value<'c, '_> = rehash_after_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Ne,
                    old_key_i,
                    sentinel_const_inner,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let occupied: Value<'c, '_> = rehash_after_blk
                .append_operation(arith::andi(not_null, not_sentinel, loc))
                .result(0)
                .unwrap()
                .into();
            let migrate_then = Region::new();
            let migrate_blk = Block::new(&[]);
            {
                // Probe new_buf for empty slot using old_key.
                let new_bucket = emit_hash_probe_for_insert(
                    context,
                    &migrate_blk,
                    new_buf,
                    new_cap,
                    old_key,
                    types,
                    loc,
                );
                let entry_size_m = migrate_blk
                    .append_operation(arith::constant(
                        context,
                        IntegerAttribute::new(types.i64, HASH_ENTRY_SIZE).into(),
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let entries_off_m = migrate_blk
                    .append_operation(arith::constant(
                        context,
                        IntegerAttribute::new(types.i64, HASH_OFF_ENTRIES).into(),
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let new_off_part: Value<'c, '_> = migrate_blk
                    .append_operation(arith::muli(new_bucket, entry_size_m, loc))
                    .result(0)
                    .unwrap()
                    .into();
                let new_off: Value<'c, '_> = migrate_blk
                    .append_operation(arith::addi(new_off_part, entries_off_m, loc))
                    .result(0)
                    .unwrap()
                    .into();
                let new_entry_ptr = emit_byte_offset_ptr_dynamic(
                    context,
                    &migrate_blk,
                    new_buf,
                    new_off,
                    types,
                    loc,
                );
                emit_store(&migrate_blk, old_key, new_entry_ptr, loc);
                // Phase 2.6c-tag-hash (ADR 0060): migrate the
                // entire 16-byte tagged value slot (tag at +0,
                // value at +8) so Nil-tagged entries (`t.k = nil`
                // soft-deletes) re-rehash with their tag intact.
                let value_slot_off_const = migrate_blk
                    .append_operation(arith::constant(
                        context,
                        IntegerAttribute::new(types.i64, HASH_ENTRY_OFF_VALUE_SLOT).into(),
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let old_value_slot_ptr = emit_byte_offset_ptr_dynamic(
                    context,
                    &migrate_blk,
                    old_entry_ptr,
                    value_slot_off_const,
                    types,
                    loc,
                );
                let new_value_slot_ptr = emit_byte_offset_ptr_dynamic(
                    context,
                    &migrate_blk,
                    new_entry_ptr,
                    value_slot_off_const,
                    types,
                    loc,
                );
                // tag at slot+0
                let old_tag = emit_load(&migrate_blk, old_value_slot_ptr, types.i64, loc);
                emit_store(&migrate_blk, old_tag, new_value_slot_ptr, loc);
                // value at slot+8
                let old_value_inner_ptr = emit_byte_offset_ptr(
                    context,
                    &migrate_blk,
                    old_value_slot_ptr,
                    ARRAY_ELEM_OFF_VALUE,
                    types,
                    loc,
                );
                let old_value = emit_load(&migrate_blk, old_value_inner_ptr, types.f64, loc);
                let new_value_inner_ptr = emit_byte_offset_ptr(
                    context,
                    &migrate_blk,
                    new_value_slot_ptr,
                    ARRAY_ELEM_OFF_VALUE,
                    types,
                    loc,
                );
                emit_store(&migrate_blk, old_value, new_value_inner_ptr, loc);
                migrate_blk.append_operation(scf::r#yield(&[], loc));
            }
            migrate_then.append_block(migrate_blk);
            let migrate_else = Region::new();
            let migrate_else_blk = Block::new(&[]);
            migrate_else_blk.append_operation(scf::r#yield(&[], loc));
            migrate_else.append_block(migrate_else_blk);
            rehash_after_blk.append_operation(scf::r#if(
                occupied,
                &[],
                migrate_then,
                migrate_else,
                loc,
            ));
            let one_i = rehash_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let next_i: Value<'c, '_> = rehash_after_blk
                .append_operation(arith::addi(i_arg, one_i, loc))
                .result(0)
                .unwrap()
                .into();
            rehash_after_blk.append_operation(scf::r#yield(&[next_i], loc));
        }
        rehash_after.append_block(rehash_after_blk);
        let rehash_while = scf::r#while(
            &[rehash_zero],
            &[types.i64],
            rehash_before,
            rehash_after,
            loc,
        );
        then_blk.append_operation(rehash_while);
        // Free old buf, install new buf in header.
        emit_libc_call_void(context, &then_blk, "free", &[hash_buf], loc);
        let header_hash_slot = emit_byte_offset_ptr(
            context,
            &then_blk,
            table_ptr,
            TABLE_OFF_HASH_BUF,
            types,
            loc,
        );
        emit_store(&then_blk, new_buf, header_hash_slot, loc);
        then_blk.append_operation(scf::r#yield(&[], loc));
    }
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    else_blk.append_operation(scf::r#yield(&[], loc));
    else_region.append_block(else_blk);
    block.append_operation(scf::r#if(need_grow, &[], then_region, else_region, loc));
}

/// Phase 2.6a-arr (ADR 0054): produce a `!llvm.ptr` pointing at
fn emit_byte_offset_ptr<'a, 'c>(
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

/// Phase 2.6a-arr (ADR 0054): same as `emit_byte_offset_ptr` but
/// the offset is a runtime `i64` value (used for variable-key
/// indexing). `llvm.getelementptr` with `elem_type = i8` so the
/// offset is interpreted in raw bytes.
fn emit_byte_offset_ptr_dynamic<'a, 'c>(
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

/// Phase 2.6a-arr (ADR 0054): runtime bounds check for `t[i]`.
/// `key_i < 1 || key_i > length` triggers `exit(1)` with the
/// "table index out of bounds" diagnostic. Lua spec returns nil
/// for OOB; until tagged values arrive we trap to keep the
/// failure observable (security over compatibility).
fn emit_table_bounds_check<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    key_i: Value<'c, 'a>,
    length_i: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let one = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, 1).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    // key_i < 1
    let too_small: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Slt,
            key_i,
            one,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    // key_i > length
    let too_big: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Sgt,
            key_i,
            length_i,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let oob: Value<'c, 'a> = block
        .append_operation(arith::ori(too_small, too_big, loc))
        .result(0)
        .unwrap()
        .into();
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    let msg_ptr = emit_addressof(context, &then_blk, "s_table_oob", types, loc);
    emit_exit_with_message(context, &then_blk, msg_ptr, types, loc);
    then_blk.append_operation(scf::r#yield(&[], loc));
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    else_blk.append_operation(scf::r#yield(&[], loc));
    else_region.append_block(else_blk);
    block.append_operation(scf::r#if(oob, &[], then_region, else_region, loc));
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
        // Phase 2.6a-norm (ADR 0056): stable header layout.
        // Allocate the 32-byte header first (header ptr is the
        // table value and never moves), then a separate buffer for
        // the f64 elements. Empty tables get a null array_buf —
        // bounds check excludes any read/write before it touches
        // the null. Future grow / hash phases swap the inner
        // buffers without disturbing the header.
        HirExprKind::Table(elems) => {
            let elem_count = elems.len();
            // Step 1: header malloc (fixed 32 bytes).
            let header_size = block
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, TABLE_HEADER_SIZE).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let header_ptr =
                emit_libc_call_ptr(context, block, "malloc", &[header_size], types, loc);
            // length + capacity (both = elem_count for a freshly-
            // constructed array; capacity tracking lands in 2.6a-grow).
            let len_const = block
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, elem_count as i64).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let len_slot =
                emit_byte_offset_ptr(context, block, header_ptr, TABLE_OFF_LEN, types, loc);
            emit_store(block, len_const, len_slot, loc);
            let cap_slot =
                emit_byte_offset_ptr(context, block, header_ptr, TABLE_OFF_CAP, types, loc);
            emit_store(block, len_const, cap_slot, loc);
            // Step 2: array_buf malloc (or null for empty). Each
            // element is now a 16-byte tagged slot (ADR 0059).
            let array_buf: Value<'c, '_> = if elem_count == 0 {
                emit_null_ptr(context, block, types, loc)
            } else {
                let buf_bytes = block
                    .append_operation(arith::constant(
                        context,
                        IntegerAttribute::new(types.i64, (elem_count as i64) * ARRAY_ELEM_SIZE)
                            .into(),
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                emit_libc_call_ptr(context, block, "malloc", &[buf_bytes], types, loc)
            };
            let array_buf_slot =
                emit_byte_offset_ptr(context, block, header_ptr, TABLE_OFF_ARRAY_BUF, types, loc);
            emit_store(block, array_buf, array_buf_slot, loc);
            // Step 3: hash_buf = null (2.6b will populate).
            let null_hash = emit_null_ptr(context, block, types, loc);
            let hash_buf_slot =
                emit_byte_offset_ptr(context, block, header_ptr, TABLE_OFF_HASH_BUF, types, loc);
            emit_store(block, null_hash, hash_buf_slot, loc);
            // Phase 2.6c-tag-hetero (ADR 0064): store each element
            // as a tagged slot, dispatching on its static kind.
            // Number → {Number, f64}, Bool → {Bool, i64-zext},
            // String → {String, ptr}, Nil → {Nil, 0}.
            for (i, elem) in elems.iter().enumerate() {
                let v = emit_expr(
                    context, block, elem, slots, locals, functions, types, params_len, loc,
                )?;
                let elem_kind = infer_kind(elem, locals, functions);
                let offset_bytes = (i as i64) * ARRAY_ELEM_SIZE;
                let elem_ptr =
                    emit_byte_offset_ptr(context, block, array_buf, offset_bytes, types, loc);
                emit_value_slot_store_dispatched(
                    context, block, elem_ptr, v, elem_kind, types, loc,
                );
            }
            Ok(header_ptr)
        }
        // Phase 2.6a-arr (ADR 0054) / 2.6a-norm (ADR 0056): `t[i]`
        // — load length from header (offset 0, frozen contract),
        // bounds-check, fetch array_buf from header (offset 16,
        // frozen contract), then GEP `(key-1)*8` bytes into
        // array_buf and load f64.
        HirExprKind::Index { target, key } => {
            let target_ptr = emit_expr(
                context, block, target, slots, locals, functions, types, params_len, loc,
            )?;
            let key_kind = infer_kind(key, locals, functions);
            match key_kind {
                ValueKind::Number => {
                    // Phase 2.6c-tag-arr (ADR 0059): each slot is
                    // a tagged 16-byte struct. Bounds-check, then
                    // load tag and trap on non-Number, then load
                    // the f64 value at offset 8.
                    let key_f = emit_expr(
                        context, block, key, slots, locals, functions, types, params_len, loc,
                    )?;
                    let key_i = emit_f2i(block, key_f, types, loc);
                    let length_i = emit_load(block, target_ptr, types.i64, loc);
                    emit_table_bounds_check(context, block, key_i, length_i, types, loc);
                    let array_buf = emit_table_array_buf(context, block, target_ptr, types, loc);
                    let elem_ptr =
                        emit_array_elem_ptr(context, block, array_buf, key_i, types, loc);
                    emit_value_slot_check_number(context, block, elem_ptr, types, loc);
                    let value_slot = emit_byte_offset_ptr(
                        context,
                        block,
                        elem_ptr,
                        ARRAY_ELEM_OFF_VALUE,
                        types,
                        loc,
                    );
                    Ok(emit_load(block, value_slot, types.f64, loc))
                }
                ValueKind::String => {
                    // Phase 2.6b-hash (ADR 0058): hash-keyed lookup.
                    // header.hash_buf null → trap (no entries). Else
                    // probe to find the bucket and load value.
                    let key_str = emit_expr(
                        context, block, key, slots, locals, functions, types, params_len, loc,
                    )?;
                    let hash_buf_slot = emit_byte_offset_ptr(
                        context,
                        block,
                        target_ptr,
                        TABLE_OFF_HASH_BUF,
                        types,
                        loc,
                    );
                    let hash_buf = emit_load(block, hash_buf_slot, types.ptr, loc);
                    // Trap if hash_buf is null (no string keys ever
                    // inserted).
                    let hash_buf_i = block
                        .append_operation(
                            OperationBuilder::new("llvm.ptrtoint", loc)
                                .add_operands(&[hash_buf])
                                .add_results(&[types.i64])
                                .build()
                                .expect("llvm.ptrtoint"),
                        )
                        .result(0)
                        .unwrap()
                        .into();
                    let zero_i64 = block
                        .append_operation(arith::constant(
                            context,
                            IntegerAttribute::new(types.i64, 0).into(),
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let null_buf: Value<'c, 'a> = block
                        .append_operation(arith::cmpi(
                            context,
                            arith::CmpiPredicate::Eq,
                            hash_buf_i,
                            zero_i64,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let null_then = Region::new();
                    let null_then_blk = Block::new(&[]);
                    {
                        let msg = emit_addressof(
                            context,
                            &null_then_blk,
                            "s_table_missing_key",
                            types,
                            loc,
                        );
                        emit_exit_with_message(context, &null_then_blk, msg, types, loc);
                        null_then_blk.append_operation(scf::r#yield(&[], loc));
                    }
                    null_then.append_block(null_then_blk);
                    let null_else = Region::new();
                    let null_else_blk = Block::new(&[]);
                    null_else_blk.append_operation(scf::r#yield(&[], loc));
                    null_else.append_block(null_else_blk);
                    block.append_operation(scf::r#if(null_buf, &[], null_then, null_else, loc));
                    let cap_slot =
                        emit_byte_offset_ptr(context, block, hash_buf, HASH_OFF_CAP, types, loc);
                    let cap = emit_load(block, cap_slot, types.i64, loc);
                    let bucket =
                        emit_hash_probe_lookup(context, block, hash_buf, cap, key_str, types, loc);
                    // Phase 2.6c-tag-hash (ADR 0060): tag-checked
                    // value load. value slot starts at
                    // HASH_OFF_ENTRIES + bucket*HASH_ENTRY_SIZE +
                    // HASH_ENTRY_OFF_VALUE_SLOT.
                    let entry_size = block
                        .append_operation(arith::constant(
                            context,
                            IntegerAttribute::new(types.i64, HASH_ENTRY_SIZE).into(),
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let bucket_off: Value<'c, 'a> = block
                        .append_operation(arith::muli(bucket, entry_size, loc))
                        .result(0)
                        .unwrap()
                        .into();
                    let value_slot_total_off = block
                        .append_operation(arith::constant(
                            context,
                            IntegerAttribute::new(
                                types.i64,
                                HASH_OFF_ENTRIES + HASH_ENTRY_OFF_VALUE_SLOT,
                            )
                            .into(),
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let total_off: Value<'c, 'a> = block
                        .append_operation(arith::addi(bucket_off, value_slot_total_off, loc))
                        .result(0)
                        .unwrap()
                        .into();
                    let value_slot_ptr = emit_byte_offset_ptr_dynamic(
                        context, block, hash_buf, total_off, types, loc,
                    );
                    emit_value_slot_check_number(context, block, value_slot_ptr, types, loc);
                    let value_ptr = emit_byte_offset_ptr(
                        context,
                        block,
                        value_slot_ptr,
                        ARRAY_ELEM_OFF_VALUE,
                        types,
                        loc,
                    );
                    Ok(emit_load(block, value_ptr, types.f64, loc))
                }
                _ => unreachable!("HIR rejects non-Number/String key kinds"),
            }
        }
        // Phase 2.6c-isnil-query (ADR 0061): non-trapping nil
        // probe. OOB array reads, missing hash keys, and Nil-
        // tagged slots all yield `true`; Number-tagged slots
        // yield `false`. Mirrors the Index arm's two key-kind
        // branches but never traps.
        HirExprKind::IsNilQuery { target, key } => {
            let target_ptr = emit_expr(
                context, block, target, slots, locals, functions, types, params_len, loc,
            )?;
            let key_kind = infer_kind(key, locals, functions);
            let i1_true = block
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i1, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            match key_kind {
                ValueKind::Number => {
                    let key_f = emit_expr(
                        context, block, key, slots, locals, functions, types, params_len, loc,
                    )?;
                    let key_i = emit_f2i(block, key_f, types, loc);
                    let length_i = emit_load(block, target_ptr, types.i64, loc);
                    let one = block
                        .append_operation(arith::constant(
                            context,
                            IntegerAttribute::new(types.i64, 1).into(),
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let too_small: Value<'c, 'a> = block
                        .append_operation(arith::cmpi(
                            context,
                            arith::CmpiPredicate::Slt,
                            key_i,
                            one,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let too_big: Value<'c, 'a> = block
                        .append_operation(arith::cmpi(
                            context,
                            arith::CmpiPredicate::Sgt,
                            key_i,
                            length_i,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let oob: Value<'c, 'a> = block
                        .append_operation(arith::ori(too_small, too_big, loc))
                        .result(0)
                        .unwrap()
                        .into();
                    // scf.if oob -> yield true; else load tag and
                    // compare with TAG_NIL.
                    let then_region = Region::new();
                    let then_blk = Block::new(&[]);
                    then_blk.append_operation(scf::r#yield(&[i1_true], loc));
                    then_region.append_block(then_blk);
                    let else_region = Region::new();
                    let else_blk = Block::new(&[]);
                    {
                        let array_buf =
                            emit_table_array_buf(context, &else_blk, target_ptr, types, loc);
                        let elem_ptr =
                            emit_array_elem_ptr(context, &else_blk, array_buf, key_i, types, loc);
                        let tag = emit_load(&else_blk, elem_ptr, types.i64, loc);
                        let tag_nil_const = else_blk
                            .append_operation(arith::constant(
                                context,
                                IntegerAttribute::new(types.i64, TAG_NIL).into(),
                                loc,
                            ))
                            .result(0)
                            .unwrap()
                            .into();
                        let is_nil: Value<'c, '_> = else_blk
                            .append_operation(arith::cmpi(
                                context,
                                arith::CmpiPredicate::Eq,
                                tag,
                                tag_nil_const,
                                loc,
                            ))
                            .result(0)
                            .unwrap()
                            .into();
                        else_blk.append_operation(scf::r#yield(&[is_nil], loc));
                    }
                    else_region.append_block(else_blk);
                    let if_op = scf::r#if(oob, &[types.i1], then_region, else_region, loc);
                    Ok(block.append_operation(if_op).result(0).unwrap().into())
                }
                ValueKind::String => {
                    let key_str = emit_expr(
                        context, block, key, slots, locals, functions, types, params_len, loc,
                    )?;
                    let hash_buf_slot = emit_byte_offset_ptr(
                        context,
                        block,
                        target_ptr,
                        TABLE_OFF_HASH_BUF,
                        types,
                        loc,
                    );
                    let hash_buf = emit_load(block, hash_buf_slot, types.ptr, loc);
                    let hash_buf_i: Value<'c, 'a> = block
                        .append_operation(
                            OperationBuilder::new("llvm.ptrtoint", loc)
                                .add_operands(&[hash_buf])
                                .add_results(&[types.i64])
                                .build()
                                .expect("llvm.ptrtoint"),
                        )
                        .result(0)
                        .unwrap()
                        .into();
                    let zero_i64 = block
                        .append_operation(arith::constant(
                            context,
                            IntegerAttribute::new(types.i64, 0).into(),
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let hash_buf_null: Value<'c, 'a> = block
                        .append_operation(arith::cmpi(
                            context,
                            arith::CmpiPredicate::Eq,
                            hash_buf_i,
                            zero_i64,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    // scf.if hash_buf_null -> yield true; else
                    // probe and inspect bucket.
                    let then_region = Region::new();
                    let then_blk = Block::new(&[]);
                    then_blk.append_operation(scf::r#yield(&[i1_true], loc));
                    then_region.append_block(then_blk);
                    let else_region = Region::new();
                    let else_blk = Block::new(&[]);
                    {
                        let cap_slot = emit_byte_offset_ptr(
                            context,
                            &else_blk,
                            hash_buf,
                            HASH_OFF_CAP,
                            types,
                            loc,
                        );
                        let cap = emit_load(&else_blk, cap_slot, types.i64, loc);
                        // emit_hash_probe_for_insert returns a
                        // bucket whose key is either null OR
                        // matches `key_str` — exactly what we
                        // need for non-trapping lookup.
                        let bucket = emit_hash_probe_for_insert(
                            context, &else_blk, hash_buf, cap, key_str, types, loc,
                        );
                        let entry_size = else_blk
                            .append_operation(arith::constant(
                                context,
                                IntegerAttribute::new(types.i64, HASH_ENTRY_SIZE).into(),
                                loc,
                            ))
                            .result(0)
                            .unwrap()
                            .into();
                        let bucket_off: Value<'c, '_> = else_blk
                            .append_operation(arith::muli(bucket, entry_size, loc))
                            .result(0)
                            .unwrap()
                            .into();
                        let entries_base = else_blk
                            .append_operation(arith::constant(
                                context,
                                IntegerAttribute::new(types.i64, HASH_OFF_ENTRIES).into(),
                                loc,
                            ))
                            .result(0)
                            .unwrap()
                            .into();
                        let entry_total_off: Value<'c, '_> = else_blk
                            .append_operation(arith::addi(bucket_off, entries_base, loc))
                            .result(0)
                            .unwrap()
                            .into();
                        let entry_ptr = emit_byte_offset_ptr_dynamic(
                            context,
                            &else_blk,
                            hash_buf,
                            entry_total_off,
                            types,
                            loc,
                        );
                        let key_at = emit_load(&else_blk, entry_ptr, types.ptr, loc);
                        let key_at_i: Value<'c, '_> = else_blk
                            .append_operation(
                                OperationBuilder::new("llvm.ptrtoint", loc)
                                    .add_operands(&[key_at])
                                    .add_results(&[types.i64])
                                    .build()
                                    .expect("llvm.ptrtoint"),
                            )
                            .result(0)
                            .unwrap()
                            .into();
                        let zero_i64_inner = else_blk
                            .append_operation(arith::constant(
                                context,
                                IntegerAttribute::new(types.i64, 0).into(),
                                loc,
                            ))
                            .result(0)
                            .unwrap()
                            .into();
                        let key_at_null: Value<'c, '_> = else_blk
                            .append_operation(arith::cmpi(
                                context,
                                arith::CmpiPredicate::Eq,
                                key_at_i,
                                zero_i64_inner,
                                loc,
                            ))
                            .result(0)
                            .unwrap()
                            .into();
                        // Inner scf.if: missing key -> true; else
                        // load value tag and check TAG_NIL.
                        let inner_then = Region::new();
                        let inner_then_blk = Block::new(&[]);
                        let inner_true = inner_then_blk
                            .append_operation(arith::constant(
                                context,
                                IntegerAttribute::new(types.i1, 1).into(),
                                loc,
                            ))
                            .result(0)
                            .unwrap()
                            .into();
                        inner_then_blk.append_operation(scf::r#yield(&[inner_true], loc));
                        inner_then.append_block(inner_then_blk);
                        let inner_else = Region::new();
                        let inner_else_blk = Block::new(&[]);
                        {
                            let value_slot_ptr = emit_byte_offset_ptr(
                                context,
                                &inner_else_blk,
                                entry_ptr,
                                HASH_ENTRY_OFF_VALUE_SLOT,
                                types,
                                loc,
                            );
                            let tag = emit_load(&inner_else_blk, value_slot_ptr, types.i64, loc);
                            let tag_nil_const = inner_else_blk
                                .append_operation(arith::constant(
                                    context,
                                    IntegerAttribute::new(types.i64, TAG_NIL).into(),
                                    loc,
                                ))
                                .result(0)
                                .unwrap()
                                .into();
                            let is_nil: Value<'c, '_> = inner_else_blk
                                .append_operation(arith::cmpi(
                                    context,
                                    arith::CmpiPredicate::Eq,
                                    tag,
                                    tag_nil_const,
                                    loc,
                                ))
                                .result(0)
                                .unwrap()
                                .into();
                            inner_else_blk.append_operation(scf::r#yield(&[is_nil], loc));
                        }
                        inner_else.append_block(inner_else_blk);
                        let inner_if =
                            scf::r#if(key_at_null, &[types.i1], inner_then, inner_else, loc);
                        let inner_result: Value<'c, '_> = else_blk
                            .append_operation(inner_if)
                            .result(0)
                            .unwrap()
                            .into();
                        else_blk.append_operation(scf::r#yield(&[inner_result], loc));
                    }
                    else_region.append_block(else_blk);
                    let if_op =
                        scf::r#if(hash_buf_null, &[types.i1], then_region, else_region, loc);
                    Ok(block.append_operation(if_op).result(0).unwrap().into())
                }
                _ => unreachable!("IsNilQuery key must be Number or String"),
            }
        }
        HirExprKind::Local(LocalId(idx)) => {
            let info = &locals[*idx];
            match info.kind {
                ValueKind::Number => Ok(emit_load(block, slots[*idx], types.f64, loc)),
                ValueKind::Bool | ValueKind::Nil => {
                    Ok(emit_load(block, slots[*idx], types.i1, loc))
                }
                ValueKind::String => Ok(emit_load(block, slots[*idx], types.ptr, loc)),
                ValueKind::Table => Ok(emit_load(block, slots[*idx], types.ptr, loc)),
                ValueKind::TaggedValue => {
                    // Phase 2.6c-tag-locals (ADR 0063): tag-checked
                    // extract. Nil-tagged → trap. Number-tagged →
                    // load f64 at offset +8. Mirrors the array
                    // element read in ADR 0059.
                    let slot_ptr = slots[*idx];
                    emit_value_slot_check_number(context, block, slot_ptr, types, loc);
                    let value_ptr = emit_byte_offset_ptr(
                        context,
                        block,
                        slot_ptr,
                        ARRAY_ELEM_OFF_VALUE,
                        types,
                        loc,
                    );
                    Ok(emit_load(block, value_ptr, types.f64, loc))
                }
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
            // Phase 2.6c-tag-hetero-fix (ADR 0065): `TaggedValue ==
            // <typed>` / `~= <typed>` lower to a runtime tag-
            // dispatch compare. Without this, `emit_expr` on the
            // `Local(TaggedValue)` side would extract f64 with
            // trap-on-non-Number and the comparison would never
            // see the runtime String / Bool tag — silent
            // miscompile flagged by codex review.
            if matches!(op, BinOp::Eq | BinOp::Ne) {
                if let Some(v) = emit_tagged_eq_runtime_dispatch(
                    context, block, *op, lhs, rhs, slots, locals, functions, types, params_len, loc,
                )? {
                    return Ok(v);
                }
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
            // first, then flip with xori. `Len` dispatches on operand
            // kind (String → strlen; Table → load i64 from header).
            // Other unaries (currently `-`, `~`) pass through
            // emit_unary unchanged.
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
            if matches!(op, UnaryOp::Len) {
                let kind = infer_kind(operand, locals, functions);
                let v = emit_expr(
                    context, block, operand, slots, locals, functions, types, params_len, loc,
                )?;
                let len_i64 = match kind {
                    ValueKind::String => {
                        emit_libc_call_i64(context, block, "strlen", &[v], types, loc)
                    }
                    ValueKind::Table => emit_load(block, v, types.i64, loc),
                    _ => unreachable!("HIR rejects #x for non-String/Table kinds"),
                };
                return Ok(emit_i2f(block, len_i64, types, loc));
            }
            let v = emit_expr(
                context, block, operand, slots, locals, functions, types, params_len, loc,
            )?;
            emit_unary(context, block, *op, v, types, loc)
        }
        HirExprKind::Call { callee, args } => match callee {
            Callee::Builtin(Builtin::Print) => {
                // Phase 2.8b (ADR 0032): variadic. `print()` outputs
                // a bare newline; `print(a, b, ...)` outputs each
                // value (kind-dispatched, no inline `\n`), with a
                // tab between values and a single `\n` at the end.
                if args.is_empty() {
                    emit_print_literal(context, block, "s_newline", types, loc);
                } else {
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            emit_print_literal(context, block, "s_tab", types, loc);
                        }
                        let kind = infer_kind(a, locals, functions);
                        // Phase 2.6c-tag-hetero (ADR 0064):
                        // `print(Local(TaggedValue))` dispatches at
                        // runtime on the slot's tag.
                        if kind == ValueKind::TaggedValue {
                            if let HirExprKind::Local(LocalId(idx)) = &a.kind {
                                emit_print_tagged_local(context, block, slots[*idx], types, loc);
                                continue;
                            }
                        }
                        // Phase 2.6c-tag-hetero-fix (ADR 0065):
                        // inline `print(t[k])` / `print(t.k)` is
                        // also a tagged read — materialise it into
                        // a tmp 16-byte slot via the existing
                        // non-trapping `emit_local_init_tagged`
                        // path, then dispatch print on the slot.
                        // Without this, Bool / String / Nil
                        // payloads abort with a tag-mismatch trap
                        // when the source is `HirExprKind::Index`
                        // directly (codex review P1, ADR 0065).
                        if let HirExprKind::Index { target, key } = &a.kind {
                            let tmp_slot = emit_alloca_slot_for_kind(
                                context,
                                block,
                                ValueKind::TaggedValue,
                                types,
                                loc,
                            );
                            emit_local_init_tagged(
                                context, block, tmp_slot, target, key, slots, locals, functions,
                                types, params_len, loc,
                            )?;
                            emit_print_tagged_local(context, block, tmp_slot, types, loc);
                            continue;
                        }
                        let v = emit_expr(
                            context, block, a, slots, locals, functions, types, params_len, loc,
                        )?;
                        emit_print_value_raw(context, block, v, kind, types, loc);
                    }
                    emit_print_literal(context, block, "s_newline", types, loc);
                }
                // print() returns nothing useful — yield a placeholder
                // f64 0.0 for the expression-position contract.
                let zero = arith::constant(
                    context,
                    FloatAttribute::new(context, types.f64, 0.0).into(),
                    loc,
                );
                Ok(block.append_operation(zero).result(0).unwrap().into())
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
            // Phase 2.7g (ADR 0030) / 2.7m (ADR 0051): `assert(cond,
            // [msg])` — pass through when truthy; otherwise print
            // the (custom or default) message and `exit(1)`. The
            // second arg, when present, is evaluated unconditionally
            // (it's a regular call argument); the conditional fork
            // only chooses which message ptr feeds the failure path.
            Callee::Builtin(Builtin::Assert) => {
                let cond_val = emit_expr(
                    context, block, &args[0], slots, locals, functions, types, params_len, loc,
                )?;
                let custom_msg = if args.len() == 2 {
                    Some(emit_expr(
                        context, block, &args[1], slots, locals, functions, types, params_len, loc,
                    )?)
                } else {
                    None
                };
                emit_assert(context, block, cond_val, custom_msg, types, loc);
                Ok(cond_val)
            }
            // Phase 2.7h (ADR 0033): `error(msg)` — unconditional
            // exit. Print the message and `exit(1)` via the shared
            // helper extracted in the preceding Tidy First commit.
            // The placeholder f64 0.0 result satisfies the
            // expression-position contract; control never reaches it.
            Callee::Builtin(Builtin::Error) => {
                let msg_val = emit_expr(
                    context, block, &args[0], slots, locals, functions, types, params_len, loc,
                )?;
                emit_exit_with_message(context, block, msg_val, types, loc);
                let zero = arith::constant(
                    context,
                    FloatAttribute::new(context, types.f64, 0.0).into(),
                    loc,
                );
                Ok(block.append_operation(zero).result(0).unwrap().into())
            }
            // Phase 2.7f (ADR 0029): `type(x)` is pure static
            // dispatch — the arg's value is irrelevant, only its
            // kind matters. We still emit_expr the arg because Lua
            // semantics evaluate it for side effects (e.g. when the
            // arg is itself a function call that prints).
            Callee::Builtin(Builtin::Type) => {
                let kind = infer_kind(&args[0], locals, functions);
                if !matches!(
                    args[0].kind,
                    HirExprKind::FunctionRef(_) | HirExprKind::Local(_)
                ) {
                    let _ = emit_expr(
                        context, block, &args[0], slots, locals, functions, types, params_len, loc,
                    )?;
                }
                let global = match kind {
                    ValueKind::Number => "s_typename_number",
                    ValueKind::String => "s_typename_string",
                    ValueKind::Bool => "s_typename_boolean",
                    ValueKind::Nil => "s_typename_nil",
                    ValueKind::Function(_) => "s_typename_function",
                    ValueKind::Table => "s_typename_table",
                    // Phase 2.6c-tag-locals (ADR 0063): static
                    // dispatch picks the inner kind's name. A
                    // runtime nil-tagged read of `type(x)` will
                    // mis-report "number" instead of "nil"; that
                    // is logged as a follow-up LIC since the
                    // widening tests in this sub-phase do not
                    // exercise it.
                    ValueKind::TaggedValue => "s_typename_number",
                };
                Ok(emit_addressof(context, block, global, types, loc))
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
        // Phase 2.6c-tag-locals (ADR 0063): IndexTagged is consumed
        // inline by `emit_local_init_tagged` inside `emit_stmt` —
        // it never surfaces in value-context emit.
        HirExprKind::IndexTagged { .. } => {
            unreachable!("IndexTagged is consumed by emit_stmt(LocalInit/Assign)")
        }
        // Phase 2.6c-tag-locals (ADR 0063): non-trapping nil probe
        // on a TaggedValue local — read the tag at slot+0 and
        // compare against TAG_NIL. Returns i1.
        HirExprKind::IsNilLocal { local_id } => {
            let slot_ptr = slots[local_id.0];
            let tag = emit_load(block, slot_ptr, types.i64, loc);
            let tag_nil = block
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, TAG_NIL).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            Ok(block
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Eq,
                    tag,
                    tag_nil,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into())
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
        // Phase 2.7a (ADR 0024) / 2.6a-min (ADR 0053): `#x` is now
        // dispatched in emit_expr where the operand kind is
        // available — String → strlen; Table → load header. This
        // arm is unreachable.
        UnaryOp::Len => unreachable!(
            "UnaryOp::Len is dispatched in emit_expr where the \
             operand kind is available (String → strlen, Table → \
             header load)"
        ),
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
        // Phase 2.7n (ADR 0052): `tostring(f)` returns the literal
        // "function". Reuses the global `s_typename_function`
        // already registered for `type(f)` so we don't bloat the
        // module with a near-duplicate string.
        ValueKind::Function(_) => emit_addressof(context, block, "s_typename_function", types, loc),
        // Phase 2.6a-min (ADR 0053): tables don't yet flow through
        // `tostring` — HIR rejects this path. The arm exists only
        // for exhaustiveness.
        ValueKind::Table => unreachable!(
            "Table-kind tostring is rejected at HIR-time \
             (FunctionUsedAsValue analogue) — should not reach codegen"
        ),
        // Phase 2.6c-tag-locals (ADR 0063): the upstream Local
        // read already extracted f64 (trapping on Nil) before
        // tostring sees the value, so dispatch as Number.
        ValueKind::TaggedValue => {
            emit_tostring(context, block, value, ValueKind::Number, types, loc)
        }
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
        ValueKind::Bool | ValueKind::Nil | ValueKind::Function(_) | ValueKind::Table => {
            unreachable!(
                "tonumber on Bool/Nil/Function/Table is rejected at HIR-time \
                 (TypeMismatch / FunctionUsedAsValue) — should not reach codegen"
            )
        }
        // Phase 2.6c-tag-locals (ADR 0063): Local read produced f64
        // (trapping on Nil) before tonumber sees it.
        ValueKind::TaggedValue => value,
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

/// Phase 2.6a-grow (ADR 0057): void-returning libc helper. Used for
/// `free(ptr)`. Mirrors `emit_libc_call_with_result` but skips
/// `add_results` since the callee returns nothing.
fn emit_libc_call_void<'a, 'c>(
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

/// Phase 2.8b (ADR 0032): per-arg print path. Multi-arg
/// `print(a, b, ...)` calls this once per argument with the
/// no-newline format globals (`fmt_raw`, `fmt_str_raw`) and
/// composes tab separators / a single trailing newline around the
/// loop. Single-arg `print(x)` is the same loop with N = 1, so
/// every print call funnels through here.
fn emit_print_value_raw<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    value: Value<'c, 'a>,
    kind: ValueKind,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    match kind {
        ValueKind::Number => {
            let fmt_ptr = emit_addressof(context, block, "fmt_raw", types, loc);
            emit_printf(context, block, fmt_ptr, value, types, loc);
        }
        ValueKind::Bool => {
            let true_ptr = emit_addressof(context, block, "s_true", types, loc);
            let false_ptr = emit_addressof(context, block, "s_false", types, loc);
            let select_op = OperationBuilder::new("llvm.select", loc)
                .add_operands(&[value, true_ptr, false_ptr])
                .add_results(&[types.ptr])
                .build()
                .expect("llvm.select");
            let chosen: Value<'c, 'a> = block.append_operation(select_op).result(0).unwrap().into();
            let fmt_ptr = emit_addressof(context, block, "fmt_str_raw", types, loc);
            emit_printf(context, block, fmt_ptr, chosen, types, loc);
        }
        ValueKind::Nil => {
            let nil_ptr = emit_addressof(context, block, "s_nil", types, loc);
            let fmt_ptr = emit_addressof(context, block, "fmt_str_raw", types, loc);
            emit_printf(context, block, fmt_ptr, nil_ptr, types, loc);
        }
        ValueKind::String => {
            let fmt_ptr = emit_addressof(context, block, "fmt_str_raw", types, loc);
            emit_printf(context, block, fmt_ptr, value, types, loc);
        }
        ValueKind::Function(_) | ValueKind::Table => unreachable!(
            "Function/Table args are rejected at HIR-time \
             (FunctionUsedAsValue) — should not reach codegen"
        ),
        // Phase 2.6c-tag-locals (ADR 0063): the upstream Local
        // read already produced the f64 (trapping on Nil). Print
        // it as Number.
        ValueKind::TaggedValue => {
            let fmt_ptr = emit_addressof(context, block, "fmt_raw", types, loc);
            emit_printf(context, block, fmt_ptr, value, types, loc);
        }
    }
}

/// Phase 2.6c-tag-hetero (ADR 0064): print a `TaggedValue` local
/// by reading its slot's tag at offset 0 and dispatching on the
/// runtime value: Number → `%g`, Bool → `s_true`/`s_false`,
/// String → `%s`, Nil → `s_nil`. Implemented as a chain of
/// nested scf.if so the codegen output is deterministic and the
/// LLVM optimiser can fold any single-tag case if it ever sees
/// one.
fn emit_print_tagged_local<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let tag = emit_load(block, slot_ptr, types.i64, loc);
    let value_ptr =
        emit_byte_offset_ptr(context, block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
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
            let fmt_ptr = emit_addressof(context, &bool_then_blk, "fmt_str_raw", types, loc);
            emit_printf(context, &bool_then_blk, fmt_ptr, chosen, types, loc);
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
                let payload_ptr = emit_load(&string_then_blk, value_ptr, types.ptr, loc);
                let fmt_ptr = emit_addressof(context, &string_then_blk, "fmt_str_raw", types, loc);
                emit_printf(context, &string_then_blk, fmt_ptr, payload_ptr, types, loc);
                string_then_blk.append_operation(scf::r#yield(&[], loc));
            }
            string_then.append_block(string_then_blk);
            let string_else = Region::new();
            let string_else_blk = Block::new(&[]);
            {
                // Fallback: TAG_NIL (or any other tag — print "nil").
                let nil_ptr = emit_addressof(context, &string_else_blk, "s_nil", types, loc);
                let fmt_ptr = emit_addressof(context, &string_else_blk, "fmt_str_raw", types, loc);
                emit_printf(context, &string_else_blk, fmt_ptr, nil_ptr, types, loc);
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

/// Phase 2.6c-tag-hetero-fix (ADR 0065): `TaggedValue` operand
/// `==` / `~=` runtime tag dispatch. When at least one side is
/// `Local(TaggedValue)` and the other side is a Number / Bool /
/// String typed operand, emit a tag check + per-kind compare
/// instead of folding the heterogeneous-kind result. Returns
/// `Some(i1)` when the dispatch fires, `None` to fall through
/// to the existing BinOp path.
///
/// The TaggedValue ↔ TaggedValue case is intentionally left to
/// the fall-through path — it lands as a Number-only extract and
/// traps for non-Number operands (LIC-2.6c-tag-hetero-eq-1).
#[allow(clippy::too_many_arguments)]
fn emit_tagged_eq_runtime_dispatch<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    op: BinOp,
    lhs: &HirExpr,
    rhs: &HirExpr,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<Option<Value<'c, 'a>>, CodegenError> {
    fn tagged_local_idx(e: &HirExpr, locals: &[LocalInfo]) -> Option<LocalId> {
        if let HirExprKind::Local(LocalId(idx)) = &e.kind {
            if matches!(locals[*idx].kind, ValueKind::TaggedValue) {
                return Some(LocalId(*idx));
            }
        }
        None
    }
    // Identify which side carries the tagged local. Skip when
    // both sides or neither side are TaggedValue locals.
    let (tagged_id, typed_expr, typed_kind) =
        match (tagged_local_idx(lhs, locals), tagged_local_idx(rhs, locals)) {
            (Some(_), Some(_)) => return Ok(None), // both tagged — fall through
            (Some(id), None) => {
                let k = infer_kind(rhs, locals, functions);
                (id, rhs, k)
            }
            (None, Some(id)) => {
                let k = infer_kind(lhs, locals, functions);
                (id, lhs, k)
            }
            (None, None) => return Ok(None),
        };
    // Only Number / Bool / String typed sides dispatch here. Nil
    // is already handled via IsNilLocal in HIR lowering.
    let (expected_tag, payload_ty) = match typed_kind {
        ValueKind::Number => (TAG_NUMBER, types.f64),
        ValueKind::Bool => (TAG_BOOL, types.i64),
        ValueKind::String => (TAG_STRING, types.ptr),
        _ => return Ok(None),
    };
    let slot_ptr = slots[tagged_id.0];
    let typed_val = emit_expr(
        context, block, typed_expr, slots, locals, functions, types, params_len, loc,
    )?;
    let tag = emit_load(block, slot_ptr, types.i64, loc);
    let expected_const = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, expected_tag).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let tag_match: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            tag,
            expected_const,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    // scf.if tag_match → compare payload; else → false.
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    {
        let value_ptr = emit_byte_offset_ptr(
            context,
            &then_blk,
            slot_ptr,
            ARRAY_ELEM_OFF_VALUE,
            types,
            loc,
        );
        let payload = emit_load(&then_blk, value_ptr, payload_ty, loc);
        let eq_inner: Value<'c, '_> = match typed_kind {
            ValueKind::Number => then_blk
                .append_operation(arith::cmpf(
                    context,
                    arith::CmpfPredicate::Oeq,
                    payload,
                    typed_val,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into(),
            ValueKind::Bool => {
                // Slot stores i1 zero-extended to i64; the typed
                // RHS comes in as i1. Truncate the slot payload
                // back to i1 before the cmpi, otherwise the
                // operands don't share a width.
                let payload_i1: Value<'c, '_> = then_blk
                    .append_operation(
                        OperationBuilder::new("llvm.trunc", loc)
                            .add_operands(&[payload])
                            .add_results(&[types.i1])
                            .build()
                            .expect("llvm.trunc i64→i1"),
                    )
                    .result(0)
                    .unwrap()
                    .into();
                then_blk
                    .append_operation(arith::cmpi(
                        context,
                        arith::CmpiPredicate::Eq,
                        payload_i1,
                        typed_val,
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into()
            }
            ValueKind::String => {
                // strcmp(payload_ptr, typed_val) == 0
                let cmp = emit_libc_call_i32(
                    context,
                    &then_blk,
                    "strcmp",
                    &[payload, typed_val],
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
                then_blk
                    .append_operation(arith::cmpi(
                        context,
                        arith::CmpiPredicate::Eq,
                        cmp,
                        zero_i32,
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into()
            }
            _ => unreachable!(),
        };
        then_blk.append_operation(scf::r#yield(&[eq_inner], loc));
    }
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    {
        let i1_zero = else_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i1, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        else_blk.append_operation(scf::r#yield(&[i1_zero], loc));
    }
    else_region.append_block(else_blk);
    let if_op = scf::r#if(tag_match, &[types.i1], then_region, else_region, loc);
    let eq_result: Value<'c, 'a> = block.append_operation(if_op).result(0).unwrap().into();
    let final_val = match op {
        BinOp::Eq => eq_result,
        BinOp::Ne => {
            let i1_one = block
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i1, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            block
                .append_operation(arith::xori(eq_result, i1_one, loc))
                .result(0)
                .unwrap()
                .into()
        }
        _ => unreachable!(),
    };
    Ok(Some(final_val))
}

/// Phase 2.8b (ADR 0032): emit `printf("%s", @<global_name>)` for
/// the literal tab / newline separators in multi-arg print.
fn emit_print_literal<'c>(
    context: &'c Context,
    block: &Block<'c>,
    global_name: &str,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let fmt_ptr = emit_addressof(context, block, "fmt_str_raw", types, loc);
    let payload = emit_addressof(context, block, global_name, types, loc);
    emit_printf(context, block, fmt_ptr, payload, types, loc);
}

/// Phase 2.7h (ADR 0033, Tidy First): emit `printf("%s\n", msg_ptr)`
/// followed by `exit(1)` into `block`. Pure-by-construction —
/// only depends on its arguments — so it composes cleanly into
/// both the conditional `assert` failure path and the
/// unconditional `error(msg)` builtin (Phase 2.7h follow-up).
fn emit_exit_with_message<'a, 'c>(
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

/// Phase 2.7g (ADR 0030): runtime check for `assert(cond)`. When
/// `cond` is false, prints "assertion failed!" and `exit(1)`s via
/// the shared `emit_exit_with_message` helper. The `scf.if` here
/// uses the void form — the assert call's return value is `cond`
/// itself, yielded by the caller in `emit_expr`.
fn emit_assert<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    cond: Value<'c, 'a>,
    custom_msg: Option<Value<'c, 'a>>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let one = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i1, 1).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let not_cond: Value<'c, 'a> = block
        .append_operation(arith::xori(cond, one, loc))
        .result(0)
        .unwrap()
        .into();

    // Then-region: emit the failure path via the shared helper,
    // then a structurally-required (but unreachable) yield. exit()
    // is noreturn so control never falls through. Phase 2.7m
    // (ADR 0051): if the user supplied a 2nd-arg message, it's
    // already in scope as `custom_msg` from the outer block — feed
    // that ptr into the failure path instead of the default global.
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    let msg_ptr = match custom_msg {
        Some(v) => v,
        None => emit_addressof(context, &then_blk, "s_assert_failed", types, loc),
    };
    emit_exit_with_message(context, &then_blk, msg_ptr, types, loc);
    then_blk.append_operation(scf::r#yield(&[], loc));
    then_region.append_block(then_blk);

    // Else-region: empty yield (assertion passed, continue normally).
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    else_blk.append_operation(scf::r#yield(&[], loc));
    else_region.append_block(else_blk);

    block.append_operation(scf::r#if(not_cond, &[], then_region, else_region, loc));
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
        // including the empty string. Same fold as Number. Phase
        // 2.6a-min: every table is truthy too (Lua's truthy/falsy
        // rule: only `nil` and `false` are falsy).
        ValueKind::String | ValueKind::Table => {
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
        // Phase 2.6c-tag-locals (ADR 0063): Local read produced
        // f64 (trapping on Nil). Truthy by Number rule.
        ValueKind::TaggedValue => {
            let attr = IntegerAttribute::new(types.i1, 1);
            block
                .append_operation(arith::constant(context, attr.into(), loc))
                .result(0)
                .unwrap()
                .into()
        }
    }
}

/// Map a static [`ValueKind`] to the MLIR type used for slots / yields
/// of values of that kind. Phase 2.3a fixes the layout: Number → f64,
/// Bool/Nil → i1.
fn kind_to_mlir_type<'c>(kind: ValueKind, types: &Types<'c>) -> Type<'c> {
    match kind {
        ValueKind::Number => types.f64,
        ValueKind::Bool | ValueKind::Nil | ValueKind::Function(_) => types.i1,
        ValueKind::String | ValueKind::Table => types.ptr,
        // Phase 2.6c-tag-locals (ADR 0063): the slot is 16 bytes
        // but the *yielded* value once extracted is f64 (Local
        // read traps on Nil and unwraps the tagged payload).
        ValueKind::TaggedValue => types.f64,
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

/// Phase 2.4b (ADR 0035): `repeat body until cond`. Lowers to
/// `scf.while` with body and cond evaluation packed into the
/// `before` region and an empty `after` region — yielding the
/// do-while semantics where the body executes before the cond is
/// tested. `cond_i1` is **inverted** (we continue while `not
/// cond`) and AND-extended with `not _broken` when a break flag
/// was allocated, matching the existing `emit_while` discipline.
#[allow(clippy::too_many_arguments)]
fn emit_repeat<'a, 'c>(
    context: &'c Context,
    parent: &'a Block<'c>,
    body: &[HirStmt],
    cond: &HirExpr,
    break_id: Option<LocalId>,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    // Before region: body, then evaluate cond → not_cond [AND not_broken].
    let before = Region::new();
    let before_blk = Block::new(&[]);
    let blk_slots = transmute_slots(slots);
    emit_stmts(
        context,
        &before_blk,
        body,
        blk_slots,
        locals,
        functions,
        types,
        params_len,
        loc,
    )?;
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
    let cond_i1 = emit_truthiness(context, &before_blk, cond_val, cond_kind, types, loc);
    let one = before_blk
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i1, 1).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let mut continue_i1: Value<'c, '_> = before_blk
        .append_operation(arith::xori(cond_i1, one, loc))
        .result(0)
        .unwrap()
        .into();
    if let Some(id) = break_id {
        continue_i1 = and_not_broken(context, &before_blk, continue_i1, blk_slots, id, types, loc);
    }
    before_blk.append_operation(scf::condition(continue_i1, &[], loc));
    before.append_block(before_blk);

    // After region: empty — body already ran in `before`. The yield
    // is structurally required by scf.while.
    let after = Region::new();
    let after_blk = Block::new(&[]);
    after_blk.append_operation(scf::r#yield(&[], loc));
    after.append_block(after_blk);

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
