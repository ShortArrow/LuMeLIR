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
        llvm,
        ods::llvm::{GlobalOperationBuilder, LLVMFuncOperationBuilder},
        scf,
    },
    ir::{
        Block, BlockLike, Identifier, Location, Module, Region, RegionLike, Type, Value,
        attribute::{
            DenseI32ArrayAttribute, FlatSymbolRefAttribute, FloatAttribute, IntegerAttribute,
            StringAttribute, TypeAttribute,
        },
        operation::{OperationBuilder, OperationLike},
        r#type::IntegerType,
    },
    utility::register_all_dialects,
};

use crate::hir::{
    Builtin, Callee, FuncId, HirChunk, HirExpr, HirExprKind, HirFunction, HirStmt, HirStmtKind,
    IndirectSig, LocalId, LocalInfo, ValueKind, infer_kind,
};
use crate::parser::{BinOp, UnaryOp};

use super::callabi::{
    emit_extract_struct_field, emit_pack_struct, flat_result_index, ret_mlir_types,
    struct_return_type,
};
use super::error::CodegenError;
use super::primitive::{
    Types, emit_addressof, emit_byte_offset_ptr, emit_byte_offset_ptr_dynamic,
    emit_exit_with_message, emit_libc_call_i32, emit_libc_call_i64, emit_libc_call_ptr,
    emit_libc_call_void, emit_load, emit_printf, emit_store,
};
use super::tagged::{
    ARRAY_ELEM_OFF_VALUE, ARRAY_ELEM_SIZE, HashKeyValidityPolicy, TAG_BOOL, TAG_DELETED,
    TAG_FUNCTION, TAG_NIL, TAG_NUMBER, TAG_STRING, TAG_TABLE, TaggedArithOperandPlan,
    emit_alloca_slot_for_kind, emit_print_tagged_local, emit_tag_and_payload_ptr,
    emit_tagged_eq_local_local, emit_tagged_unknown_tag_trap, emit_type_tagged_local,
    emit_value_slot_check_number, emit_value_slot_store_dispatched, emit_value_slot_store_nil,
    policy_for_tag, policy_for_tagged_arith_operand,
};

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
// Phase 2.6b-hash-keys (ADR 0079): hash entry layout widens to
// `{16-byte tagged key slot, 16-byte tagged value slot}` = 32
// bytes. Both slots share the array element layout (ADR 0064)
// so `emit_value_slot_*` helpers work unchanged on each. The
// previous ptr-only `HASH_DELETED_KEY=1` sentinel (ADR 0062) is
// retired; tombstones are now expressed by `TAG_DELETED` (= 6)
// in the key slot's tag word, and empty buckets by `TAG_NIL`
// (= 0) which `malloc` zero-init makes automatic.
const HASH_ENTRY_SIZE: i64 = 32;
const HASH_ENTRY_OFF_KEY_SLOT: i64 = 0;
const HASH_ENTRY_OFF_VALUE_SLOT: i64 = 16;
const HASH_INITIAL_CAP: i64 = 8;

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
    // Phase 2.5c-full Commit 2b (ADR 0083): one closure singleton
    // global per user fn. The producer sites (`HirExprKind::FunctionRef`,
    // known-FuncId Local) materialise the cell ptr via
    // `addressof @user_fn_NN_closure` instead of the raw fn ptr.
    for hir_fn in &chunk.functions {
        super::closure::emit_user_fn_closure_global(
            context,
            &module,
            &hir_fn.mangled_name,
            &types,
            loc,
        );
    }
    // Phase 2.8e-iter-next (ADR 0081): always emit the `next(t, k)`
    // helper. The function is small and unused calls are eliminated
    // by the LLVM backend's dead-code pass; emitting it
    // unconditionally avoids threading "is `next` referenced" usage
    // signal through every HIR pass.
    emit_lumelir_next_function(context, &module, &types, loc);
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
    // `t.k` / `t["k"]` reads. ADR 0088 narrowed the user-reachable
    // surface — Index hash arms now route through
    // `emit_hash_lookup_into_tagged_slot` which materialises Nil on
    // missing instead of trapping; this global is reserved for the
    // future `HashLookupOutcome::TrapMissing` consumer (`next()`-
    // strict-key style).
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
    // Phase 2.7p-arith-string-coerce (ADR 0077): runtime trap
    // diagnostic when a String operand of an arithmetic /
    // bitwise BinOp fails to parse as a number. Lua spec
    // §3.4.1: `"abc" + 1` is a runtime error, not a silent
    // NaN propagation.
    emit_string_global(
        context,
        module,
        i8_type,
        "s_arith_coerce_failed",
        "attempt to perform arithmetic on a string value\0",
        loc,
    );
    // Phase 2.7p-tagged-arith-coerce (ADR 0089): runtime trap for
    // arith / bitwise / unary on a TaggedValue whose tag is
    // Bool / Nil / Function / Table / Deleted (= non-coercible).
    // Distinct from `s_arith_coerce_failed` (parse-fail on a
    // String operand): different error condition, different
    // diagnostic. Lua spec §3.4.3:
    // "attempt to perform arithmetic on a {type} value".
    emit_string_global(
        context,
        module,
        i8_type,
        "s_arith_on_non_numeric",
        "attempt to perform arithmetic on a non-numeric value\0",
        loc,
    );
    // Phase 2.5x-callee-dispatch (ADR 0082): runtime traps for the
    // indirect-dispatch chain. `s_call_non_function` fires when the
    // tagged-slot's tag is not TAG_FUNCTION (e.g. `local g = "s";
    // g()`). `s_call_unknown_fn_ptr` fires when the loaded fn
    // pointer doesn't match any compile-time-known user function
    // (impossible from Lua source today; defensive backstop for
    // future FFI / dynamic loaders).
    emit_string_global(
        context,
        module,
        i8_type,
        "s_call_non_function",
        "attempt to call a non-function value\0",
        loc,
    );
    emit_string_global(
        context,
        module,
        i8_type,
        "s_call_unknown_fn_ptr",
        "attempt to call an unknown function pointer\0",
        loc,
    );
    // Phase 2.8e-iter-tk (ADR 0084): runtime trap for `t[k] = v`
    // when `k` is a TaggedValue local whose dynamic tag is
    // TAG_NIL. Lua spec §3.4.5 — `nil` values cannot be used as
    // table keys. ADR 0087: this global is fired by the probe
    // chokepoint's validity gate, not by the call sites.
    emit_string_global(
        context,
        module,
        i8_type,
        "s_table_index_nil",
        "table index is nil\0",
        loc,
    );
    // Phase 2.6b-hash-key-nan (ADR 0086): runtime trap for `t[k]`
    // when `k` is NaN. Lua spec §3.4.5 forbids NaN as a table
    // index. Surface is wider than ADR 0084's nil case: NaN can
    // arrive via the static Number-key array path (`t[0/0]`) too.
    // Trap message globals are owned by emit.rs; the validity
    // policy that decides which globals fire on which tag lives
    // in `tagged.rs::policy_for_tag` (ADR 0087).
    emit_string_global(
        context,
        module,
        i8_type,
        "s_table_index_nan",
        "table index is NaN\0",
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

/// Phase 2.6b-hash-key-nan (ADR 0086): emit `if cond then exit
/// "table index is NaN" end`. Reuses the ADR 0084 nil-trap shape:
/// scf.if → printf("%s\n", s_table_index_nan) → exit(1) on the
/// then branch, scf.yield no-op on the else branch.
fn emit_table_index_nan_trap_if<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    cond_i1: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    {
        let msg = emit_addressof(context, &then_blk, "s_table_index_nan", types, loc);
        emit_exit_with_message(context, &then_blk, msg, types, loc);
        then_blk.append_operation(scf::r#yield(&[], loc));
    }
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    else_blk.append_operation(scf::r#yield(&[], loc));
    else_region.append_block(else_blk);
    block.append_operation(scf::r#if(cond_i1, &[], then_region, else_region, loc));
}

/// Phase 2.6b-hash-key-validity (ADR 0087): runtime tag-validity
/// gate at the hash probe entry. Replaces the ADR 0086
/// `emit_hash_key_nan_preflight` and folds in ADR 0084's per-call
/// inline `s_table_index_nil` traps so every hash search-key
/// carrier — `IndexAssign`, `Index`, and the iterator-internal
/// probes that `pairs` / `next` use — converges on a single
/// chokepoint.
///
/// The decision matrix (which tags need which checks) lives in
/// `tagged.rs::policy_for_tag` as a pure function; this helper
/// is the effectful emitter that consults that matrix and lays
/// down the MLIR scf.if + trap structure. Trap message globals
/// (`s_table_index_nil`, `s_table_index_nan`) and the underlying
/// `emit_table_index_nan_trap_if` shape stay owned here in
/// emit.rs.
///
/// TAG_NIL must be tested before TAG_NUMBER — the nil slot has
/// no f64 payload, so the NaN load must not run on it. Other
/// tags (Bool / String / Function / Table / Deleted) are
/// pass-through; the probe handles them directly.
fn emit_hash_key_runtime_validity_gate<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    search_key_slot: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    // Tags whose payloads or values trigger a runtime-validity
    // check. Order is load-bearing — see doc-comment.
    const POLICED_TAGS: &[i64] = &[TAG_NIL, TAG_NUMBER];
    let tag = emit_load(block, search_key_slot, types.i64, loc);
    for &policed_tag in POLICED_TAGS {
        let policies = policy_for_tag(policed_tag);
        if policies.is_empty() {
            continue;
        }
        let tag_const = block
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, policed_tag).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let is_match: Value<'c, 'a> = block
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                tag,
                tag_const,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let then_region = Region::new();
        let then_blk = Block::new(&[]);
        for policy in policies {
            match policy {
                HashKeyValidityPolicy::TrapNil => {
                    let msg = emit_addressof(context, &then_blk, "s_table_index_nil", types, loc);
                    emit_exit_with_message(context, &then_blk, msg, types, loc);
                }
                HashKeyValidityPolicy::CheckNaN => {
                    let payload_ptr = emit_byte_offset_ptr(
                        context,
                        &then_blk,
                        search_key_slot,
                        ARRAY_ELEM_OFF_VALUE,
                        types,
                        loc,
                    );
                    let payload = emit_load(&then_blk, payload_ptr, types.f64, loc);
                    let is_nan: Value<'c, '_> = then_blk
                        .append_operation(arith::cmpf(
                            context,
                            CmpfPredicate::Une,
                            payload,
                            payload,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    emit_table_index_nan_trap_if(context, &then_blk, is_nan, types, loc);
                }
            }
        }
        then_blk.append_operation(scf::r#yield(&[], loc));
        then_region.append_block(then_blk);
        let else_region = Region::new();
        let else_blk = Block::new(&[]);
        else_blk.append_operation(scf::r#yield(&[], loc));
        else_region.append_block(else_blk);
        block.append_operation(scf::r#if(is_match, &[], then_region, else_region, loc));
    }
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
            HirExprKind::IndexTagged { target, key } => {
                visit_expr(target, set);
                visit_expr(key, set);
            }
            // Phase 2.6c-tag-hetero-eq (ADR 0066): IsNil unifies
            // the previous IsNilQuery/IsNilLocal pair. Walking
            // the operand covers both the Index source (which has
            // its own string-pool entries) and the Local source
            // (which has none, but recursing is harmless).
            HirExprKind::IsNil(operand) => visit_expr(operand, set),
            // Phase 2.7p-arith-string-coerce (ADR 0077): the
            // wrapper holds a String operand; recurse so its pool
            // entry is registered.
            HirExprKind::ArithStringCoerce(operand) => visit_expr(operand, set),
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
    let _ = context;
    match kind {
        ValueKind::Number => types.f64,
        // Phase 2.5c-full Commit 2a (ADR 0083): Function values
        // become `!llvm.ptr` once user fns are emitted in the
        // LLVM dialect. The prior `!func.func<(f64, ...) -> f64>`
        // representation was incompatible with `llvm.func` callees
        // (spike B3 verifier rejection).
        ValueKind::Function(_) => types.ptr,
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

/// Phase 2.5c-full Commit 2a (ADR 0083): pick the LLVM-dialect
/// callee return type for a flattened MLIR-result list. 0 →
/// `!llvm.void`, 1 → that single type, ≥2 → `!llvm.struct<(...)>`.
fn llvm_callee_ret_ty<'c>(context: &'c Context, ret_types: &[Type<'c>]) -> Type<'c> {
    match ret_types.len() {
        0 => llvm::r#type::void(context),
        1 => ret_types[0],
        _ => struct_return_type(context, ret_types).expect("non-empty multi-return"),
    }
}

/// Phase 2.5c-full Commit 2a (ADR 0083): emit `llvm.return` for
/// a callee whose HIR return positions flattened to `ret_values`.
/// Single-value matches LLVM's `ret %v` directly; multi-value
/// packs the values into the matching `!llvm.struct<(...)>` first.
fn emit_llvm_return<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    ret_types: &[Type<'c>],
    ret_values: &[Value<'c, 'a>],
    loc: Location<'c>,
) {
    debug_assert_eq!(ret_types.len(), ret_values.len());
    let mut op = OperationBuilder::new("llvm.return", loc);
    match ret_values.len() {
        0 => {}
        1 => op = op.add_operands(&[ret_values[0]]),
        _ => {
            let st_ty = struct_return_type(context, ret_types).expect("multi ret_types");
            let packed = emit_pack_struct(context, block, st_ty, ret_values, loc);
            op = op.add_operands(&[packed]);
        }
    }
    block.append_operation(op.build().expect("llvm.return"));
}

/// Phase 2.5c-full Commit 2a (ADR 0083): emit a top-level
/// `llvm.func @sym(...) -> ret_ty { region }`. Used for user
/// fns, `main`, and `__lumelir_next` after the migration off
/// the `func` dialect.
fn emit_llvm_func<'c>(
    context: &'c Context,
    module: &Module<'c>,
    sym_name: &str,
    region: Region<'c>,
    param_types: &[Type<'c>],
    ret_ty: Type<'c>,
    loc: Location<'c>,
) {
    let fn_type = llvm::r#type::function(ret_ty, param_types, false);
    let func_op = LLVMFuncOperationBuilder::new(context, loc)
        .body(region)
        .sym_name(StringAttribute::new(context, sym_name))
        .function_type(TypeAttribute::new(fn_type))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::External,
        ))
        .build();
    module.body().append_operation(func_op.into());
}

/// Phase 2.5c-full Commit 2a (ADR 0083): emit `llvm.call
/// @callee(args) : (...) -> ret_ty` (callee-attribute form). The
/// LLVM dialect's call op expects `operandSegmentSizes` =
/// `[args.len(), 0]` and an `op_bundle_sizes` empty array; this
/// helper centralises those boilerplate attributes.
fn emit_llvm_call_user<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    callee_name: &str,
    args: &[Value<'c, 'a>],
    result_ty: Option<Type<'c>>,
    loc: Location<'c>,
) -> melior::ir::operation::OperationRef<'c, 'a> {
    let n = i32::try_from(args.len()).expect("call arity fits in i32");
    let mut builder = OperationBuilder::new("llvm.call", loc)
        .add_operands(args)
        .add_attributes(&[
            (
                Identifier::new(context, "callee"),
                FlatSymbolRefAttribute::new(context, callee_name).into(),
            ),
            (
                Identifier::new(context, "operandSegmentSizes"),
                DenseI32ArrayAttribute::new(context, &[n, 0]).into(),
            ),
            (
                Identifier::new(context, "op_bundle_sizes"),
                DenseI32ArrayAttribute::new(context, &[]).into(),
            ),
        ]);
    if let Some(t) = result_ty {
        builder = builder.add_results(&[t]);
    }
    block.append_operation(
        builder
            .build()
            .unwrap_or_else(|_| panic!("llvm.call @{callee_name}")),
    )
}

/// Phase 2.5c-full Commit 2a (ADR 0083): emit `llvm.call
/// %callee_ptr(args) : (...) -> ret_ty` (operand-callee form).
/// Used for `Callee::Indirect` where the callee is an
/// SSA `!llvm.ptr` value rather than a known symbol. The LLVM
/// dialect signals operand-callee mode by omitting the `callee`
/// attribute and threading the callee ptr as the first operand
/// in the same `callee_operands` group as the args; the
/// non-variadic pointee type is inferred from the operand and
/// result MLIR types, so no `var_callee_type` attribute is
/// emitted (it is reserved for variadic callees only).
fn emit_llvm_call_indirect<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    callee_ptr: Value<'c, 'a>,
    args: &[Value<'c, 'a>],
    result_ty: Option<Type<'c>>,
    loc: Location<'c>,
) -> melior::ir::operation::OperationRef<'c, 'a> {
    let total = i32::try_from(args.len() + 1).expect("indirect call arity fits in i32");
    let mut all_operands: Vec<Value<'c, 'a>> = Vec::with_capacity(args.len() + 1);
    all_operands.push(callee_ptr);
    all_operands.extend_from_slice(args);
    let mut builder = OperationBuilder::new("llvm.call", loc)
        .add_operands(&all_operands)
        .add_attributes(&[
            (
                Identifier::new(context, "operandSegmentSizes"),
                DenseI32ArrayAttribute::new(context, &[total, 0]).into(),
            ),
            (
                Identifier::new(context, "op_bundle_sizes"),
                DenseI32ArrayAttribute::new(context, &[]).into(),
            ),
        ]);
    if let Some(t) = result_ty {
        builder = builder.add_results(&[t]);
    }
    block.append_operation(builder.build().expect("llvm.call indirect"))
}

/// Phase 2.6c-tag-locals-fn-multi (ADR 0076): pack the two i64
/// MLIR results that materialise a TaggedValue return position
/// into a 16-byte tagged destination slot. Position-aware via
/// `flat_result_index` (ADR 0076) so multi-position TaggedValue
/// returns resolve to non-overlapping MLIR result ranges.
///
/// Phase 2.5c-full Commit 2a (ADR 0083): now takes the call's
/// flattened result vector directly (extracted from a single
/// `!llvm.struct<>` result up front by [`extract_unpacked_results`])
/// rather than reading off an `OperationRef`.
#[allow(clippy::too_many_arguments)]
fn emit_pack_tagged_result_at_pos<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    flat_results: &[Value<'c, 'a>],
    dst_slot: Value<'c, 'a>,
    ret_kinds: &[ValueKind],
    pos: usize,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    debug_assert!(matches!(ret_kinds[pos], ValueKind::TaggedValue));
    let start = flat_result_index(ret_kinds, pos);
    let tag = flat_results[start];
    let payload = flat_results[start + 1];
    emit_store(block, tag, dst_slot, loc);
    let payload_ptr =
        emit_byte_offset_ptr(context, block, dst_slot, ARRAY_ELEM_OFF_VALUE, types, loc);
    emit_store(block, payload, payload_ptr, loc);
}

/// Phase 2.5c-full Commit 2a (ADR 0083): unpack the result of an
/// `llvm.call` whose callee return shape was determined by
/// [`llvm_callee_ret_ty`]. Yields one `Value` per flattened MLIR
/// result type — `extractvalue` for the multi-return struct case,
/// the single SSA result for the 1-arity case, and an empty Vec
/// for void.
fn extract_unpacked_results<'a, 'c, 'op>(
    context: &'c Context,
    block: &'a Block<'c>,
    op_ref: &melior::ir::operation::OperationRef<'c, 'op>,
    ret_types: &[Type<'c>],
    loc: Location<'c>,
) -> Vec<Value<'c, 'a>>
where
    'op: 'a,
{
    match ret_types.len() {
        0 => Vec::new(),
        1 => vec![op_ref.result(0).unwrap().into()],
        _ => {
            let struct_val: Value<'c, 'a> = op_ref.result(0).unwrap().into();
            (0..ret_types.len())
                .map(|i| {
                    emit_extract_struct_field(
                        context,
                        block,
                        struct_val,
                        ret_types[i],
                        i as i64,
                        loc,
                    )
                })
                .collect()
        }
    }
}

/// Phase 2.5c-full Commit 2a (ADR 0083): emit `llvm.call @callee(args)`
/// and immediately unpack the call's flattened MLIR results. This is
/// the canonical replacement for `func::call` callsites whose result
/// vector was previously read directly off the `OperationRef`.
fn emit_call_user_unpacked<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    callee_name: &str,
    args: &[Value<'c, 'a>],
    ret_types: &[Type<'c>],
    loc: Location<'c>,
) -> Vec<Value<'c, 'a>> {
    let result_ty = if ret_types.is_empty() {
        None
    } else {
        Some(llvm_callee_ret_ty(context, ret_types))
    };
    let op_ref = emit_llvm_call_user(context, block, callee_name, args, result_ty, loc);
    extract_unpacked_results(context, block, &op_ref, ret_types, loc)
}

/// Phase 2.5c-full Commit 3b (ADR 0083): emit `llvm.call
/// @<callee_name>(%cell_ptr, lua_args...)` and unpack the result.
/// This is the single canonical helper used by every direct-user-call
/// codegen path (4 sites: Call expr arm, multi-assign, dispatch
/// chain then-branch, TaggedValue pack helper) — Codex review of
/// Commit 3a flagged that scattered cell-ptr threading across these
/// sites was the most likely source of ABI cutover bugs.
///
/// **Status**: defined now (Commit 3b prep) but the 4 direct-user-call
/// sites still use the legacy `emit_call_user_unpacked` until the
/// atomic ABI cutover lands. `#[allow(dead_code)]` hides the warning
/// for the prep window only.
///
/// `cell_ptr` selection follows ADR 0083 §3, with the HIR-level
/// invariants established by Commit 3b prep fix
/// (synthetic FunctionDef-locals + mutual-capturing-recursion reject):
/// 1. non-capturing target → `addressof @<callee_name>_closure`
///    (the static singleton; identity-stable across aliases)
/// 2. capturing + `holding_local = Some(idx)` → load the cell ptr
///    from `slots[idx]`. After the prep fix, every chunk-level and
///    nested forward call to a capturing fn resolves to the
///    synthetic FunctionDef-local in scope, so this arm covers
///    Test A (`tests/phase2_5c1_top_level_capture.rs`) and Test B
///    (`tests/phase2_5f_nested_function.rs`). The LocalInit storage
///    rule guarantees the slot was populated with the right cell
///    ptr (fresh malloc at closure-creation site, or alias copy
///    for `local g = f`).
/// 3. capturing + `holding_local = None` → use `in_function_cell_ptr`
///    (the entry block's `block.argument(0)`). After the prep fix
///    the only way to land here is **self-recursion** inside the
///    capturing fn's own body (Function-kind upvalues are still
///    rejected, so `lookup_or_capture_upvalue` falls through to
///    `function_names` which sees the fn's own name). The HIR
///    post-pass `check_mutual_capturing_recursion_in_stmts`
///    rejects every other case before reaching codegen, so
///    `in_function_cell_ptr` here is exactly the caller-supplied
///    "current self instance".
/// 4. capturing + `holding_local = None` + `in_function_cell_ptr =
///    None` → unreachable (would mean a top-level capturing fn,
///    which has no enclosing scope to capture from; the HIR upvalue
///    pass cannot construct such a function).
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
fn emit_call_user_with_cell<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    target: &HirFunction,
    holding_local: Option<LocalId>,
    lua_args: &[Value<'c, 'a>],
    ret_types: &[Type<'c>],
    slots: &[Value<'c, 'a>],
    types: &Types<'c>,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Vec<Value<'c, 'a>> {
    let cell_ptr: Value<'c, 'a> = if target.upvalues.is_empty() {
        let closure_sym = format!("{}_closure", target.mangled_name);
        emit_addressof(context, block, &closure_sym, types, loc)
    } else if let Some(LocalId(idx)) = holding_local {
        emit_load(block, slots[idx], types.ptr, loc)
    } else if let Some(cp) = in_function_cell_ptr {
        cp
    } else {
        unreachable!(
            "capturing target {} reached with no holding_local and no in-function cell_ptr — \
             ADR 0083 invariant violated (top-level capturing fn is impossible)",
            target.mangled_name
        )
    };
    let mut all_args: Vec<Value<'c, 'a>> = Vec::with_capacity(lua_args.len() + 1);
    all_args.push(cell_ptr);
    all_args.extend_from_slice(lua_args);
    emit_call_user_unpacked(
        context,
        block,
        &target.mangled_name,
        &all_args,
        ret_types,
        loc,
    )
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
    // Phase 2.5c-full Commit 3b body atomic Step 5 (ADR 0083): every
    // user `llvm.func` accepts a `!llvm.ptr` closure cell ptr as
    // its first argument. Lua params shift to args 1..N. The body
    // unpacks `cell.upvalue_box[i]` for each `uv` at function entry
    // and uses the box ptr as the slot for the captured local —
    // see `closure.rs` ABI contract for full detail.
    let param_types: Vec<Type<'c>> = hir_fn
        .params
        .iter()
        .map(|info| param_mlir_type(context, info.kind, types))
        .collect();
    let mut all_param_types: Vec<Type<'c>> = Vec::with_capacity(1 + param_types.len());
    all_param_types.push(types.ptr); // %arg0 = closure cell ptr
    all_param_types.extend(param_types.iter().copied());
    let block_args: Vec<(Type<'c>, Location<'c>)> =
        all_param_types.iter().map(|t| (*t, loc)).collect();
    let block = Block::new(&block_args);

    // Cell ptr is the entry block's first argument. Threaded down
    // through `emit_stmts` so direct user calls and capturing
    // closure creation sites can recover it.
    let cell_ptr: Value<'c, '_> = block.argument(0).unwrap().into();

    // Allocate slots for every local. Function-kind params are special:
    // their `slot` is the block argument value itself (not a pointer);
    // we never load/store those. Non-param Function-kind locals get an
    // alloca slot whose value is reproduced at every use site via
    // `func.constant` (or, post-Commit-2b, via `addressof @<sym>_closure`).
    let mut slots: Vec<Value<'c, '_>> = hir_fn
        .locals
        .iter()
        .enumerate()
        .map(|(i, info)| match info.kind {
            // Lua params shift: param i is at block_arg(i+1) since
            // block_arg(0) is the cell ptr.
            ValueKind::Function(_) if i < hir_fn.params.len() => {
                block.argument(i + 1).unwrap().into()
            }
            _ => emit_alloca_slot_for_kind(context, &block, info.kind, types, loc),
        })
        .collect();

    // Store each non-Function parameter (block_arg(i+1)) into its
    // alloca slot. Function-kind params keep their block_arg as the
    // slot value directly (no store).
    for (i, info) in hir_fn.params.iter().enumerate() {
        if !matches!(info.kind, ValueKind::Function(_)) {
            let arg: Value<'c, '_> = block.argument(i + 1).unwrap().into();
            // Phase 2.5c-full Commit 3b body atomic Step 6: when the
            // captured-local post-pass marked this param as
            // `is_captured`, allocate a heap upvalue box and store
            // through it instead of the alloca, then override the
            // slot with the box ptr so subsequent reads/writes flow
            // through the heap box.
            if hir_fn.locals[i].is_captured {
                let box_ptr =
                    super::closure::emit_allocate_upvalue_box(context, &block, types, loc);
                super::closure::emit_store_upvalue_box_value(
                    context, &block, box_ptr, arg, info.kind, types, loc,
                );
                slots[i] = box_ptr;
            } else {
                emit_store(&block, arg, slots[i], loc);
            }
        }
    }

    // Phase 2.5c-full Commit 3b body atomic Step 5 (ADR 0083): unpack
    // each captured upvalue from the cell. The box ptr is the slot
    // for the inner local, so `HirExprKind::Local(inner_id)` reads /
    // writes flow through `closure::emit_load/store_upvalue_box_value`.
    for (i, uv) in hir_fn.upvalues.iter().enumerate() {
        let box_ptr =
            super::closure::emit_load_closure_upvalue_box(context, &block, cell_ptr, i, types, loc);
        slots[uv.inner_local_id.0] = box_ptr;
    }

    // Phase 2.5c-full Commit 3b body atomic Step 6 (ADR 0083): for
    // every body-introduced local marked `is_captured` by the HIR
    // post-pass, allocate a heap upvalue box at function entry so
    // its slot is already a box ptr when the LocalInit /
    // Assign / Local-read paths fire. Non-Function kinds only —
    // Function-kind captured locals would need a 16-byte box and
    // are out of ADR 0083 Commit 3 scope. Params are already
    // handled above; upvalue inner_locals already point at the
    // caller-supplied cell's box ptr.
    let upvalue_inner_local_ids: std::collections::HashSet<usize> = hir_fn
        .upvalues
        .iter()
        .map(|uv| uv.inner_local_id.0)
        .collect();
    for (i, info) in hir_fn.locals.iter().enumerate() {
        if !info.is_captured {
            continue;
        }
        if i < hir_fn.params.len() {
            continue; // params already handled above
        }
        if upvalue_inner_local_ids.contains(&i) {
            continue; // upvalue inner_local already pointed at the cell's box
        }
        if matches!(info.kind, ValueKind::Function(_) | ValueKind::TaggedValue) {
            // Function-kind / TaggedValue captures are out of
            // Commit 3 scope; the post-pass should have left
            // them rejected upstream, so the slot stays as the
            // alloca it was given.
            continue;
        }
        let box_ptr = super::closure::emit_allocate_upvalue_box(context, &block, types, loc);
        slots[i] = box_ptr;
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
        Some(cell_ptr),
        loc,
    )?;

    // Trailing return: load each `_ret_value_N` slot (Phase 2.5d
    // ADR 0021) — slots are placed sequentially after `_returned`,
    // i.e. at `params.len() + 1 + i` for the i-th return position.
    // Phase 2.5c-full Commit 2a (ADR 0083): Function-kind returns
    // load the slot as `ptr` directly — the prior unrealized_cast
    // bridge to `!func.func<...>` is gone now that user fns live
    // in the LLVM dialect (function values are first-class ptrs).
    let ret_types = ret_mlir_types(context, &hir_fn.ret_kinds, types);
    let mut ret_values: Vec<Value<'c, '_>> = Vec::with_capacity(ret_types.len());
    for (i, k) in hir_fn.ret_kinds.iter().enumerate() {
        let ret_value_idx = hir_fn.params.len() + 1 + i;
        match k {
            ValueKind::Number => {
                ret_values.push(emit_load(&block, slots[ret_value_idx], types.f64, loc))
            }
            ValueKind::Bool | ValueKind::Nil => {
                ret_values.push(emit_load(&block, slots[ret_value_idx], types.i1, loc))
            }
            ValueKind::Function(_) => {
                ret_values.push(emit_load(&block, slots[ret_value_idx], types.ptr, loc));
            }
            // Phase 2.7a / 2.6a-min: String and Table returns
            // surface the slot's `ptr` value directly — no cast is
            // needed since the slot type already matches the
            // function signature.
            ValueKind::String | ValueKind::Table => {
                ret_values.push(emit_load(&block, slots[ret_value_idx], types.ptr, loc));
            }
            // Phase 2.6c-tag-locals-fn (ADR 0074): a TaggedValue
            // return position emits 2 MLIR results — the i64 tag
            // at slot+0 and the raw i64 payload at slot+8.
            ValueKind::TaggedValue => {
                let slot_ptr = slots[ret_value_idx];
                let tag = emit_load(&block, slot_ptr, types.i64, loc);
                let payload_ptr = emit_byte_offset_ptr(
                    context,
                    &block,
                    slot_ptr,
                    ARRAY_ELEM_OFF_VALUE,
                    types,
                    loc,
                );
                let payload = emit_load(&block, payload_ptr, types.i64, loc);
                ret_values.push(tag);
                ret_values.push(payload);
            }
        };
    }
    emit_llvm_return(context, &block, &ret_types, &ret_values, loc);
    region.append_block(block);

    // Phase 2.5c-min: function signature widens to include upvalue
    // params. `all_param_types` is `param_types ++ upvalue_types`.
    // Phase 2.5c-full Commit 2a (ADR 0083): emit as `llvm.func`,
    // wrapping multi-return (≥2 MLIR results) in `!llvm.struct<>`
    // since LLVM IR's `ret` instruction is single-value.
    let llvm_ret_ty = llvm_callee_ret_ty(context, &ret_types);
    emit_llvm_func(
        context,
        module,
        &hir_fn.mangled_name,
        region,
        &all_param_types,
        llvm_ret_ty,
        loc,
    );
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

    // Chunk-level (main fn) has no enclosing closure cell.
    emit_stmts(
        context,
        &main_block,
        &chunk.stmts,
        &slots,
        &chunk.locals,
        &chunk.functions,
        types,
        0,
        None,
        loc,
    )?;

    let zero = arith::constant(context, IntegerAttribute::new(types.i64, 0).into(), loc);
    let zero_val: Value<'c, '_> = main_block.append_operation(zero).result(0).unwrap().into();
    emit_llvm_return(context, &main_block, &[types.i64], &[zero_val], loc);

    main_region.append_block(main_block);

    // Phase 2.5c-full Commit 2a (ADR 0083): main is now `llvm.func
    // @main() -> i64`. The i64 exit-code shape is preserved — only
    // the dialect of the enclosing function flips.
    emit_llvm_func(context, module, "main", main_region, &[], types.i64, loc);
    Ok(())
}

/// Phase 2.8e-iter-next (ADR 0081): emit the `@__lumelir_next`
/// helper function used by `Builtin::Next` calls.
///
/// Lua spec §3.7.3: `next(t, prev_k)` returns `(next_k, next_v)`,
/// the next non-nil entry in `t`'s iteration order after `prev_k`,
/// or `(nil, nil)` when exhausted. Order is unspecified; we walk
/// the array part first then the hash part. The function is
/// stateless — each call recomputes the position from `prev_k`.
///
/// **Resume strategy** (naive linear scan, O(N) per call): we
/// walk the entire table once, using a `found` flag to record
/// whether we've passed `prev_k`. The first live entry we visit
/// while `found=true` is the answer. The flag starts true when
/// `prev_k == nil`. This avoids the bucket-probe-then-resume
/// implementation; the O(N²) total cost across a full pairs
/// loop is acceptable for typical small tables and matches the
/// codex pre-review's request for "tag check first" safety.
///
/// **Signature**: `(t: ptr, prev_tag: i64, prev_payload: i64) →
/// (next_k_tag: i64, next_k_payload: i64, next_v_tag: i64,
/// next_v_payload: i64)`. The 4-i64 result follows ADR 0076's
/// flattened TaggedValue ABI for two consecutive TaggedValue
/// return positions.
fn emit_lumelir_next_function<'c>(
    context: &'c Context,
    module: &Module<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let region = Region::new();
    let block = Block::new(&[(types.ptr, loc), (types.i64, loc), (types.i64, loc)]);
    let t_arg: Value<'c, '_> = block.argument(0).unwrap().into();
    let prev_tag_arg: Value<'c, '_> = block.argument(1).unwrap().into();
    let prev_payload_arg: Value<'c, '_> = block.argument(2).unwrap().into();

    // Constants used throughout.
    let i64_const = |b: &Block<'c>, v: i64| -> Value<'c, '_> {
        b.append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, v).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into()
    };
    let i1_const = |b: &Block<'c>, v: bool| -> Value<'c, '_> {
        b.append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i1, if v { 1 } else { 0 }).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into()
    };

    // === Header reads ===
    let len_slot = emit_byte_offset_ptr(context, &block, t_arg, TABLE_OFF_LEN, types, loc);
    let len = emit_load(&block, len_slot, types.i64, loc);
    let arr_buf_slot =
        emit_byte_offset_ptr(context, &block, t_arg, TABLE_OFF_ARRAY_BUF, types, loc);
    let arr_buf = emit_load(&block, arr_buf_slot, types.ptr, loc);
    let hash_buf_slot =
        emit_byte_offset_ptr(context, &block, t_arg, TABLE_OFF_HASH_BUF, types, loc);
    let hash_buf = emit_load(&block, hash_buf_slot, types.ptr, loc);

    // === State alloca ===
    // Result fields are i64, initially NIL/0 so the "exhausted"
    // case requires no explicit write.
    let alloca_i64 = |b: &Block<'c>| -> Value<'c, '_> {
        let one = i64_const(b, 1);
        let op = OperationBuilder::new("llvm.alloca", loc)
            .add_operands(&[one])
            .add_attributes(&[(
                Identifier::new(context, "elem_type"),
                TypeAttribute::new(types.i64).into(),
            )])
            .add_results(&[types.ptr])
            .build()
            .expect("llvm.alloca i64");
        b.append_operation(op).result(0).unwrap().into()
    };
    let alloca_i1 = |b: &Block<'c>| -> Value<'c, '_> {
        let one = i64_const(b, 1);
        let op = OperationBuilder::new("llvm.alloca", loc)
            .add_operands(&[one])
            .add_attributes(&[(
                Identifier::new(context, "elem_type"),
                TypeAttribute::new(types.i1).into(),
            )])
            .add_results(&[types.ptr])
            .build()
            .expect("llvm.alloca i1");
        b.append_operation(op).result(0).unwrap().into()
    };
    let res_k_tag = alloca_i64(&block);
    let res_k_pay = alloca_i64(&block);
    let res_v_tag = alloca_i64(&block);
    let res_v_pay = alloca_i64(&block);
    let done_slot = alloca_i1(&block);
    let found_slot = alloca_i1(&block);
    // Initial: result fields = (NIL, 0, NIL, 0); done = false;
    // found = (prev_tag == TAG_NIL) — when the user passes nil we
    // are already "past" the imaginary slot before slot 1.
    let nil_const = i64_const(&block, TAG_NIL);
    let zero_i64 = i64_const(&block, 0);
    emit_store(&block, nil_const, res_k_tag, loc);
    emit_store(&block, zero_i64, res_k_pay, loc);
    emit_store(&block, nil_const, res_v_tag, loc);
    emit_store(&block, zero_i64, res_v_pay, loc);
    let false_i1 = i1_const(&block, false);
    emit_store(&block, false_i1, done_slot, loc);
    let prev_is_nil: Value<'c, '_> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            prev_tag_arg,
            nil_const,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    emit_store(&block, prev_is_nil, found_slot, loc);

    // === Array phase: scf.while %i = 1..=len, gated on !done ===
    let one_i64 = i64_const(&block, 1);
    let arr_before = Region::new();
    let arr_before_blk = Block::new(&[(types.i64, loc)]);
    {
        let i_arg: Value<'c, '_> = arr_before_blk.argument(0).unwrap().into();
        let len_inner = unsafe { std::mem::transmute::<Value<'c, '_>, Value<'c, '_>>(len) };
        let i_le_len: Value<'c, '_> = arr_before_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Sle,
                i_arg,
                len_inner,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let done_inner = emit_load(&arr_before_blk, done_slot, types.i1, loc);
        let one_i1 = arr_before_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i1, 1).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let not_done: Value<'c, '_> = arr_before_blk
            .append_operation(arith::xori(done_inner, one_i1, loc))
            .result(0)
            .unwrap()
            .into();
        let cond: Value<'c, '_> = arr_before_blk
            .append_operation(arith::andi(i_le_len, not_done, loc))
            .result(0)
            .unwrap()
            .into();
        arr_before_blk.append_operation(scf::condition(cond, &[i_arg], loc));
    }
    arr_before.append_block(arr_before_blk);

    let arr_after = Region::new();
    let arr_after_blk = Block::new(&[(types.i64, loc)]);
    {
        let i_arg: Value<'c, '_> = arr_after_blk.argument(0).unwrap().into();
        let arr_buf_inner = unsafe { std::mem::transmute::<Value<'c, '_>, Value<'c, '_>>(arr_buf) };

        // slot_off = (i - 1) * 16
        let one_inner = arr_after_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 1).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let i_minus_one: Value<'c, '_> = arr_after_blk
            .append_operation(arith::subi(i_arg, one_inner, loc))
            .result(0)
            .unwrap()
            .into();
        let elem_size = arr_after_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, ARRAY_ELEM_SIZE).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let slot_off: Value<'c, '_> = arr_after_blk
            .append_operation(arith::muli(i_minus_one, elem_size, loc))
            .result(0)
            .unwrap()
            .into();
        let slot_ptr = emit_byte_offset_ptr_dynamic(
            context,
            &arr_after_blk,
            arr_buf_inner,
            slot_off,
            types,
            loc,
        );
        let slot_tag = emit_load(&arr_after_blk, slot_ptr, types.i64, loc);
        let nil_inner = arr_after_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, TAG_NIL).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let is_live: Value<'c, '_> = arr_after_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Ne,
                slot_tag,
                nil_inner,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let found_now = emit_load(&arr_after_blk, found_slot, types.i1, loc);

        // Branch 1: if !found && live && (prev was Number-equal-to-i)
        // → mark found = true. We test `prev_tag == TAG_NUMBER` and
        // `bitcast(prev_payload, f64) == sitofp(i)` for the array-
        // index case. Wrong matches (e.g. user passed prev_k as a
        // hash key with TAG_STRING) fall through unchanged.
        let prev_tag_inner =
            unsafe { std::mem::transmute::<Value<'c, '_>, Value<'c, '_>>(prev_tag_arg) };
        let prev_payload_inner =
            unsafe { std::mem::transmute::<Value<'c, '_>, Value<'c, '_>>(prev_payload_arg) };
        let tag_number_const = arr_after_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, TAG_NUMBER).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let prev_is_number: Value<'c, '_> = arr_after_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                prev_tag_inner,
                tag_number_const,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        // i_as_f64 = sitofp i64 i → f64
        let i_as_f64: Value<'c, '_> = arr_after_blk
            .append_operation(
                OperationBuilder::new("arith.sitofp", loc)
                    .add_operands(&[i_arg])
                    .add_results(&[types.f64])
                    .build()
                    .expect("arith.sitofp"),
            )
            .result(0)
            .unwrap()
            .into();
        // prev_payload_f64 = bitcast i64 → f64
        let prev_payload_f64: Value<'c, '_> = arr_after_blk
            .append_operation(
                OperationBuilder::new("arith.bitcast", loc)
                    .add_operands(&[prev_payload_inner])
                    .add_results(&[types.f64])
                    .build()
                    .expect("arith.bitcast f64"),
            )
            .result(0)
            .unwrap()
            .into();
        let prev_value_eq_i: Value<'c, '_> = arr_after_blk
            .append_operation(arith::cmpf(
                context,
                CmpfPredicate::Oeq,
                i_as_f64,
                prev_payload_f64,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let prev_is_this_i: Value<'c, '_> = arr_after_blk
            .append_operation(arith::andi(prev_is_number, prev_value_eq_i, loc))
            .result(0)
            .unwrap()
            .into();
        let one_i1 = arr_after_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i1, 1).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let not_found: Value<'c, '_> = arr_after_blk
            .append_operation(arith::xori(found_now, one_i1, loc))
            .result(0)
            .unwrap()
            .into();
        let mark_found_cond: Value<'c, '_> = arr_after_blk
            .append_operation(arith::andi(not_found, prev_is_this_i, loc))
            .result(0)
            .unwrap()
            .into();
        // if mark_found_cond: store true → found_slot
        let mark_then = Region::new();
        let mark_then_blk = Block::new(&[]);
        {
            let true_i1 = mark_then_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i1, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            emit_store(&mark_then_blk, true_i1, found_slot, loc);
            mark_then_blk.append_operation(scf::r#yield(&[], loc));
        }
        mark_then.append_block(mark_then_blk);
        let mark_else = Region::new();
        let mark_else_blk = Block::new(&[]);
        mark_else_blk.append_operation(scf::r#yield(&[], loc));
        mark_else.append_block(mark_else_blk);
        arr_after_blk.append_operation(scf::r#if(mark_found_cond, &[], mark_then, mark_else, loc));

        // Branch 2: if found && live && i != prev_index → record
        // result and mark done. We re-load `found_slot` so the
        // mark-found branch above flows into here.
        let found_now2 = emit_load(&arr_after_blk, found_slot, types.i1, loc);
        let live_and_found: Value<'c, '_> = arr_after_blk
            .append_operation(arith::andi(is_live, found_now2, loc))
            .result(0)
            .unwrap()
            .into();
        // We need to skip the slot we JUST marked as the resume
        // point. The mark-found branch sets found_slot above; that
        // same slot's live entry should not be returned. Use:
        // record_cond = live && found && !prev_is_this_i
        let one_i1_b = arr_after_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i1, 1).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let not_resume: Value<'c, '_> = arr_after_blk
            .append_operation(arith::xori(prev_is_this_i, one_i1_b, loc))
            .result(0)
            .unwrap()
            .into();
        let record_cond: Value<'c, '_> = arr_after_blk
            .append_operation(arith::andi(live_and_found, not_resume, loc))
            .result(0)
            .unwrap()
            .into();
        let rec_then = Region::new();
        let rec_then_blk = Block::new(&[]);
        {
            // res_k_tag = TAG_NUMBER
            let n_const = rec_then_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, TAG_NUMBER).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            emit_store(&rec_then_blk, n_const, res_k_tag, loc);
            // res_k_pay = bitcast(sitofp(i), i64)
            let i_inner_rec = unsafe { std::mem::transmute::<Value<'c, '_>, Value<'c, '_>>(i_arg) };
            let i_f64: Value<'c, '_> = rec_then_blk
                .append_operation(
                    OperationBuilder::new("arith.sitofp", loc)
                        .add_operands(&[i_inner_rec])
                        .add_results(&[types.f64])
                        .build()
                        .expect("arith.sitofp rec"),
                )
                .result(0)
                .unwrap()
                .into();
            let i_as_i64: Value<'c, '_> = rec_then_blk
                .append_operation(
                    OperationBuilder::new("arith.bitcast", loc)
                        .add_operands(&[i_f64])
                        .add_results(&[types.i64])
                        .build()
                        .expect("arith.bitcast i64"),
                )
                .result(0)
                .unwrap()
                .into();
            emit_store(&rec_then_blk, i_as_i64, res_k_pay, loc);
            // res_v_tag = slot_tag (already loaded above; materialise inside region)
            let slot_ptr_inner =
                unsafe { std::mem::transmute::<Value<'c, '_>, Value<'c, '_>>(slot_ptr) };
            let v_tag = emit_load(&rec_then_blk, slot_ptr_inner, types.i64, loc);
            emit_store(&rec_then_blk, v_tag, res_v_tag, loc);
            let v_pay_ptr = emit_byte_offset_ptr(
                context,
                &rec_then_blk,
                slot_ptr_inner,
                ARRAY_ELEM_OFF_VALUE,
                types,
                loc,
            );
            let v_pay = emit_load(&rec_then_blk, v_pay_ptr, types.i64, loc);
            emit_store(&rec_then_blk, v_pay, res_v_pay, loc);
            // done = true
            let true_i1_rec = rec_then_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i1, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            emit_store(&rec_then_blk, true_i1_rec, done_slot, loc);
            rec_then_blk.append_operation(scf::r#yield(&[], loc));
        }
        rec_then.append_block(rec_then_blk);
        let rec_else = Region::new();
        let rec_else_blk = Block::new(&[]);
        rec_else_blk.append_operation(scf::r#yield(&[], loc));
        rec_else.append_block(rec_else_blk);
        arr_after_blk.append_operation(scf::r#if(record_cond, &[], rec_then, rec_else, loc));

        // i_next = i + 1
        let one_inner2 = arr_after_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 1).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let i_next: Value<'c, '_> = arr_after_blk
            .append_operation(arith::addi(i_arg, one_inner2, loc))
            .result(0)
            .unwrap()
            .into();
        arr_after_blk.append_operation(scf::r#yield(&[i_next], loc));
    }
    arr_after.append_block(arr_after_blk);
    block.append_operation(scf::r#while(
        &[one_i64],
        &[types.i64],
        arr_before,
        arr_after,
        loc,
    ));

    // === Hash phase: scf.if hash_buf != null && !done, then scf.while ===
    let hash_buf_iv: Value<'c, '_> = block
        .append_operation(
            OperationBuilder::new("llvm.ptrtoint", loc)
                .add_operands(&[hash_buf])
                .add_results(&[types.i64])
                .build()
                .expect("llvm.ptrtoint hash_buf"),
        )
        .result(0)
        .unwrap()
        .into();
    let zero_i64_outer = i64_const(&block, 0);
    let hash_not_null: Value<'c, '_> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Ne,
            hash_buf_iv,
            zero_i64_outer,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let done_outer = emit_load(&block, done_slot, types.i1, loc);
    let one_i1_outer = i1_const(&block, true);
    let not_done_outer: Value<'c, '_> = block
        .append_operation(arith::xori(done_outer, one_i1_outer, loc))
        .result(0)
        .unwrap()
        .into();
    let hash_guard: Value<'c, '_> = block
        .append_operation(arith::andi(hash_not_null, not_done_outer, loc))
        .result(0)
        .unwrap()
        .into();
    let hash_then = Region::new();
    let hash_then_blk = Block::new(&[]);
    {
        let hash_buf_inner =
            unsafe { std::mem::transmute::<Value<'c, '_>, Value<'c, '_>>(hash_buf) };
        let cap_slot = emit_byte_offset_ptr(
            context,
            &hash_then_blk,
            hash_buf_inner,
            HASH_OFF_CAP,
            types,
            loc,
        );
        let cap = emit_load(&hash_then_blk, cap_slot, types.i64, loc);

        let h_before = Region::new();
        let h_before_blk = Block::new(&[(types.i64, loc)]);
        {
            let bi_arg: Value<'c, '_> = h_before_blk.argument(0).unwrap().into();
            let cap_inner = unsafe { std::mem::transmute::<Value<'c, '_>, Value<'c, '_>>(cap) };
            let bi_lt_cap: Value<'c, '_> = h_before_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Slt,
                    bi_arg,
                    cap_inner,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let done_inner = emit_load(&h_before_blk, done_slot, types.i1, loc);
            let one_i1_b = h_before_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i1, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let not_done: Value<'c, '_> = h_before_blk
                .append_operation(arith::xori(done_inner, one_i1_b, loc))
                .result(0)
                .unwrap()
                .into();
            let cond: Value<'c, '_> = h_before_blk
                .append_operation(arith::andi(bi_lt_cap, not_done, loc))
                .result(0)
                .unwrap()
                .into();
            h_before_blk.append_operation(scf::condition(cond, &[bi_arg], loc));
        }
        h_before.append_block(h_before_blk);

        let h_after = Region::new();
        let h_after_blk = Block::new(&[(types.i64, loc)]);
        {
            let bi_arg: Value<'c, '_> = h_after_blk.argument(0).unwrap().into();
            let hash_buf_inner2 =
                unsafe { std::mem::transmute::<Value<'c, '_>, Value<'c, '_>>(hash_buf) };

            // entry_off = HASH_OFF_ENTRIES + bi * HASH_ENTRY_SIZE
            let entries_base = h_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, HASH_OFF_ENTRIES).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let entry_size = h_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, HASH_ENTRY_SIZE).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let bucket_off: Value<'c, '_> = h_after_blk
                .append_operation(arith::muli(bi_arg, entry_size, loc))
                .result(0)
                .unwrap()
                .into();
            let entry_off: Value<'c, '_> = h_after_blk
                .append_operation(arith::addi(bucket_off, entries_base, loc))
                .result(0)
                .unwrap()
                .into();
            let entry_ptr = emit_byte_offset_ptr_dynamic(
                context,
                &h_after_blk,
                hash_buf_inner2,
                entry_off,
                types,
                loc,
            );
            let key_tag = emit_load(&h_after_blk, entry_ptr, types.i64, loc);
            let nil_c = h_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, TAG_NIL).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let del_c = h_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, TAG_DELETED).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let not_nil: Value<'c, '_> = h_after_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Ne,
                    key_tag,
                    nil_c,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let not_del: Value<'c, '_> = h_after_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Ne,
                    key_tag,
                    del_c,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let is_live: Value<'c, '_> = h_after_blk
                .append_operation(arith::andi(not_nil, not_del, loc))
                .result(0)
                .unwrap()
                .into();

            // Mark-found: live && !found && entry's (tag, payload)
            // == (prev_tag, prev_payload). For Number keys we use
            // f64 cmpf Oeq to handle the bitcast-i64 representation
            // mismatch (NaN ≠ NaN); for everything else raw i64
            // payload equality. Tag must match either way.
            let prev_tag_inner =
                unsafe { std::mem::transmute::<Value<'c, '_>, Value<'c, '_>>(prev_tag_arg) };
            let prev_payload_inner =
                unsafe { std::mem::transmute::<Value<'c, '_>, Value<'c, '_>>(prev_payload_arg) };
            let tag_eq: Value<'c, '_> = h_after_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Eq,
                    key_tag,
                    prev_tag_inner,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let key_pay_ptr = emit_byte_offset_ptr(
                context,
                &h_after_blk,
                entry_ptr,
                ARRAY_ELEM_OFF_VALUE,
                types,
                loc,
            );
            let key_pay = emit_load(&h_after_blk, key_pay_ptr, types.i64, loc);
            let pay_eq: Value<'c, '_> = h_after_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Eq,
                    key_pay,
                    prev_payload_inner,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let key_eq: Value<'c, '_> = h_after_blk
                .append_operation(arith::andi(tag_eq, pay_eq, loc))
                .result(0)
                .unwrap()
                .into();

            let found_now = emit_load(&h_after_blk, found_slot, types.i1, loc);
            let one_i1_c = h_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i1, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let not_found: Value<'c, '_> = h_after_blk
                .append_operation(arith::xori(found_now, one_i1_c, loc))
                .result(0)
                .unwrap()
                .into();
            let live_not_found: Value<'c, '_> = h_after_blk
                .append_operation(arith::andi(is_live, not_found, loc))
                .result(0)
                .unwrap()
                .into();
            let mark_cond: Value<'c, '_> = h_after_blk
                .append_operation(arith::andi(live_not_found, key_eq, loc))
                .result(0)
                .unwrap()
                .into();
            let mark_then = Region::new();
            let mark_then_blk = Block::new(&[]);
            {
                let true_i1 = mark_then_blk
                    .append_operation(arith::constant(
                        context,
                        IntegerAttribute::new(types.i1, 1).into(),
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                emit_store(&mark_then_blk, true_i1, found_slot, loc);
                mark_then_blk.append_operation(scf::r#yield(&[], loc));
            }
            mark_then.append_block(mark_then_blk);
            let mark_else = Region::new();
            let mark_else_blk = Block::new(&[]);
            mark_else_blk.append_operation(scf::r#yield(&[], loc));
            mark_else.append_block(mark_else_blk);
            h_after_blk.append_operation(scf::r#if(mark_cond, &[], mark_then, mark_else, loc));

            // Record: live && found && !key_eq → write result + done
            let found_now2 = emit_load(&h_after_blk, found_slot, types.i1, loc);
            let live_and_found: Value<'c, '_> = h_after_blk
                .append_operation(arith::andi(is_live, found_now2, loc))
                .result(0)
                .unwrap()
                .into();
            let one_i1_d = h_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i1, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let not_key_eq: Value<'c, '_> = h_after_blk
                .append_operation(arith::xori(key_eq, one_i1_d, loc))
                .result(0)
                .unwrap()
                .into();
            let record_cond: Value<'c, '_> = h_after_blk
                .append_operation(arith::andi(live_and_found, not_key_eq, loc))
                .result(0)
                .unwrap()
                .into();
            let rec_then = Region::new();
            let rec_then_blk = Block::new(&[]);
            {
                emit_store(&rec_then_blk, key_tag, res_k_tag, loc);
                emit_store(&rec_then_blk, key_pay, res_k_pay, loc);
                let v_tag_ptr = emit_byte_offset_ptr(
                    context,
                    &rec_then_blk,
                    entry_ptr,
                    HASH_ENTRY_OFF_VALUE_SLOT,
                    types,
                    loc,
                );
                let v_tag = emit_load(&rec_then_blk, v_tag_ptr, types.i64, loc);
                let v_pay_ptr = emit_byte_offset_ptr(
                    context,
                    &rec_then_blk,
                    entry_ptr,
                    HASH_ENTRY_OFF_VALUE_SLOT + ARRAY_ELEM_OFF_VALUE,
                    types,
                    loc,
                );
                let v_pay = emit_load(&rec_then_blk, v_pay_ptr, types.i64, loc);
                emit_store(&rec_then_blk, v_tag, res_v_tag, loc);
                emit_store(&rec_then_blk, v_pay, res_v_pay, loc);
                let true_i1 = rec_then_blk
                    .append_operation(arith::constant(
                        context,
                        IntegerAttribute::new(types.i1, 1).into(),
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                emit_store(&rec_then_blk, true_i1, done_slot, loc);
                rec_then_blk.append_operation(scf::r#yield(&[], loc));
            }
            rec_then.append_block(rec_then_blk);
            let rec_else = Region::new();
            let rec_else_blk = Block::new(&[]);
            rec_else_blk.append_operation(scf::r#yield(&[], loc));
            rec_else.append_block(rec_else_blk);
            h_after_blk.append_operation(scf::r#if(record_cond, &[], rec_then, rec_else, loc));

            // bi_next = bi + 1
            let one_h = h_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, 1).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let bi_next: Value<'c, '_> = h_after_blk
                .append_operation(arith::addi(bi_arg, one_h, loc))
                .result(0)
                .unwrap()
                .into();
            h_after_blk.append_operation(scf::r#yield(&[bi_next], loc));
        }
        h_after.append_block(h_after_blk);
        let zero_bi = hash_then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        hash_then_blk.append_operation(scf::r#while(
            &[zero_bi],
            &[types.i64],
            h_before,
            h_after,
            loc,
        ));
        hash_then_blk.append_operation(scf::r#yield(&[], loc));
    }
    hash_then.append_block(hash_then_blk);
    let hash_else = Region::new();
    let hash_else_blk = Block::new(&[]);
    hash_else_blk.append_operation(scf::r#yield(&[], loc));
    hash_else.append_block(hash_else_blk);
    block.append_operation(scf::r#if(hash_guard, &[], hash_then, hash_else, loc));

    // === Build return tuple ===
    let r_k_tag = emit_load(&block, res_k_tag, types.i64, loc);
    let r_k_pay = emit_load(&block, res_k_pay, types.i64, loc);
    let r_v_tag = emit_load(&block, res_v_tag, types.i64, loc);
    let r_v_pay = emit_load(&block, res_v_pay, types.i64, loc);
    let next_ret_types = [types.i64, types.i64, types.i64, types.i64];
    emit_llvm_return(
        context,
        &block,
        &next_ret_types,
        &[r_k_tag, r_k_pay, r_v_tag, r_v_pay],
        loc,
    );

    region.append_block(block);
    // Phase 2.5c-full Commit 2a (ADR 0083): the 4×i64 multi-return
    // ABI surfaces as `!llvm.struct<(i64, i64, i64, i64)>` at the
    // LLVM dialect. Caller `extractvalue` mirrors what was
    // previously `op.result(0..4)`.
    let next_ret_ty = llvm_callee_ret_ty(context, &next_ret_types);
    emit_llvm_func(
        context,
        module,
        "__lumelir_next",
        region,
        &[types.ptr, types.i64, types.i64],
        next_ret_ty,
        loc,
    );
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    for stmt in stmts {
        emit_stmt(
            context,
            block,
            stmt,
            slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
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
                    // Phase 2.5c-full Commit 3b body atomic Step 8
                    // (ADR 0083): capturing closures need their cell
                    // ptr stored even when the binding has a known
                    // FuncId — alias copies (`local g = f`) and the
                    // synthetic FunctionDef-LocalInit both go
                    // through this arm and the Local-known-FuncId
                    // codegen reads the cell ptr from the slot.
                    let target_capturing = info
                        .func_id
                        .map(|fid| !functions[fid.0].upvalues.is_empty())
                        .unwrap_or(false);
                    if !is_param && (!known_fid || target_capturing) {
                        let v = emit_expr(
                            context,
                            block,
                            value,
                            slots,
                            locals,
                            functions,
                            types,
                            params_len,
                            in_function_cell_ptr,
                            loc,
                        )?;
                        emit_store(block, v, slots[id.0], loc);
                    }
                }
                ValueKind::TaggedValue => {
                    // Phase 2.6c-tag-locals (ADR 0063) / 2.6c-tag-
                    // locals-fn (ADR 0074): the dst is a 16-byte
                    // tagged slot. Source dispatch:
                    //   - IndexTagged: non-trapping table read +
                    //     tagged store (emit_local_init_tagged).
                    //   - Local(TaggedValue): copy the 16-byte
                    //     slot verbatim (preserves Nil tag).
                    //   - Call(User) returning TaggedValue: emit
                    //     the call and pack 2 i64 results
                    //     (tag, payload) into the slot.
                    //   - else: dispatch on source kind via
                    //     emit_value_slot_store_dispatched.
                    match &value.kind {
                        HirExprKind::Call {
                            callee:
                                Callee::User {
                                    fid: FuncId(fid),
                                    holding_local,
                                },
                            args: call_args,
                        } if matches!(
                            functions[*fid].ret_kinds.first(),
                            Some(ValueKind::TaggedValue)
                        ) =>
                        {
                            emit_call_user_into_tagged_slot(
                                context,
                                block,
                                slots[id.0],
                                *fid,
                                *holding_local,
                                call_args,
                                slots,
                                locals,
                                functions,
                                types,
                                params_len,
                                in_function_cell_ptr,
                                loc,
                            )?;
                        }
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
                                in_function_cell_ptr,
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
                            // Phase 2.6c-tag-locals-fn (ADR 0074):
                            // a TaggedValue slot can be widened
                            // from any concrete kind via `Assign`
                            // (e.g. `_ret_value_N := nil` after the
                            // function-return widening upgrades the
                            // slot from Number to TaggedValue).
                            // Dispatch on the source expression's
                            // kind so Nil / Bool / String / Function
                            // / Table sources all wrap correctly,
                            // not just Number.
                            let value_kind = infer_kind(value, locals, functions);
                            let v = if matches!(value_kind, ValueKind::Nil) {
                                // Nil sources don't materialise an
                                // SSA value; the store helper
                                // ignores the value arg for Nil.
                                // Use a placeholder f64 0.0.
                                let zero_op = arith::constant(
                                    context,
                                    FloatAttribute::new(context, types.f64, 0.0).into(),
                                    loc,
                                );
                                block.append_operation(zero_op).result(0).unwrap().into()
                            } else {
                                emit_expr(
                                    context,
                                    block,
                                    value,
                                    slots,
                                    locals,
                                    functions,
                                    types,
                                    params_len,
                                    in_function_cell_ptr,
                                    loc,
                                )?
                            };
                            emit_value_slot_store_dispatched(
                                context,
                                block,
                                slots[id.0],
                                v,
                                value_kind,
                                types,
                                loc,
                            );
                        }
                    }
                }
                _ => {
                    let v = emit_expr(
                        context,
                        block,
                        value,
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?;
                    // Phase 2.5c-full Commit 3b body atomic Step 6
                    // (ADR 0083): captured non-Function locals'
                    // slots are heap upvalue boxes (pre-allocated
                    // in `emit_function`). Route the store through
                    // the kind-typed box helper so reads from
                    // sibling closures see the same value.
                    if info.is_captured {
                        super::closure::emit_store_upvalue_box_value(
                            context,
                            block,
                            slots[id.0],
                            v,
                            info.kind,
                            types,
                            loc,
                        );
                    } else {
                        emit_store(block, v, slots[id.0], loc);
                    }
                }
            }
        }
        HirStmtKind::Block { stmts } => {
            emit_stmts(
                context,
                block,
                stmts,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
        }
        HirStmtKind::If {
            cond,
            then_body,
            elifs,
            else_body,
        } => {
            emit_if(
                context,
                block,
                cond,
                then_body,
                elifs,
                else_body,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
        }
        HirStmtKind::While {
            cond,
            body,
            break_id,
        } => {
            emit_while(
                context,
                block,
                cond,
                body,
                *break_id,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
        }
        HirStmtKind::Repeat {
            body,
            cond,
            break_id,
        } => {
            emit_repeat(
                context,
                block,
                body,
                cond,
                *break_id,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
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
                context,
                block,
                *var_id,
                start,
                stop,
                step,
                body,
                *break_id,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
        }
        HirStmtKind::ExprStmt(expr) => {
            emit_expr_stmt(
                context,
                block,
                expr,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
        }
        HirStmtKind::MultiAssignFromCall {
            dst_ids,
            callee,
            args,
        } => {
            emit_multi_assign_from_call(
                context,
                block,
                dst_ids,
                callee,
                args,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
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
                context,
                block,
                target,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
            let key_kind = infer_kind(key, locals, functions);
            let value_v = emit_expr(
                context,
                block,
                value,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
            match key_kind {
                ValueKind::Number => {
                    // Phase 2.6c-tag-arr (ADR 0059): hole-write
                    // enabled. Bound check is now `key >= 1`
                    // only; grow handles capacity, gap-fill marks
                    // intermediate slots as Nil-tagged.
                    let key_f = emit_expr(
                        context,
                        block,
                        key,
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?;
                    // Phase 2.6b-hash-key-nan (ADR 0086): NaN
                    // cannot be a table index (Lua spec §3.4.5).
                    // `cmpf Une key_f key_f` is true iff key_f is
                    // NaN. Trap before f2i (whose result on NaN
                    // is implementation-defined) and the bounds
                    // check.
                    let is_nan: Value<'c, 'a> = block
                        .append_operation(arith::cmpf(
                            context,
                            CmpfPredicate::Une,
                            key_f,
                            key_f,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    emit_table_index_nan_trap_if(context, block, is_nan, types, loc);
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
                // Phase 2.6b-hash-keys (ADR 0079): every non-Number
                // hash-eligible kind (String / Bool / Function /
                // Table) routes through the same hash insert
                // path. The key value is materialised into a 16-
                // byte tagged search-key slot for tag-aware probe
                // and equality.
                ValueKind::String | ValueKind::Bool | ValueKind::Function(_) | ValueKind::Table => {
                    let key_value = emit_expr(
                        context,
                        block,
                        key,
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?;
                    let key_slot =
                        emit_build_search_key_slot(context, block, key_kind, key_value, types, loc);
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
                        context, block, hash_buf, cap, key_slot, types, loc,
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
                        // Phase 2.6c-tag-hetero (ADR 0064) +
                        // 2.6c-tag-fn-tbl (ADR 0071): Number /
                        // Bool / String / Function / Table all
                        // share the new-key + count++ path; only
                        // the final tagged store helper differs.
                        // Dispatch via
                        // `emit_value_slot_store_dispatched`.
                        ValueKind::Number
                        | ValueKind::Bool
                        | ValueKind::String
                        | ValueKind::Function(_)
                        | ValueKind::Table => {
                            let new_key_then = Region::new();
                            let new_key_blk = Block::new(&[]);
                            {
                                // Phase 2.6b-hash-keys (ADR 0079):
                                // commit the key into entry+0 via
                                // the kind-dispatched store; the
                                // already-built search slot's
                                // contents (tag + payload) are
                                // re-emitted into the entry's
                                // 16-byte key slot.
                                emit_value_slot_store_dispatched(
                                    context,
                                    &new_key_blk,
                                    entry_ptr,
                                    key_value,
                                    key_kind,
                                    types,
                                    loc,
                                );
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
                            // Phase 2.6c-tag-hash-hard (ADR 0062),
                            // updated by 2.6b-hash-keys (ADR 0079):
                            // hard delete. When the key is present
                            // (was_empty == false), write
                            // `TAG_DELETED` to the key tag word at
                            // entry+0; the payload is left intact
                            // because the probe skips on the tag
                            // alone. The value slot is reset to
                            // Nil. Probe walks past `TAG_DELETED`
                            // entries; rehash drops them
                            // physically. Deleting a missing key is
                            // a no-op (Lua spec). count is
                            // unchanged either way — tombstones
                            // stay counted against load factor
                            // until rehash.
                            let del_then = Region::new();
                            let del_then_blk = Block::new(&[]);
                            {
                                let tombstone_tag = del_then_blk
                                    .append_operation(arith::constant(
                                        context,
                                        IntegerAttribute::new(types.i64, TAG_DELETED).into(),
                                        loc,
                                    ))
                                    .result(0)
                                    .unwrap()
                                    .into();
                                emit_store(&del_then_blk, tombstone_tag, entry_ptr, loc);
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
                            "HIR rejects non-Number/Bool/String/Nil/Function/Table values for hash insert"
                        ),
                    }
                }
                // Phase 2.8e-iter-tk (ADR 0084): TaggedValue key —
                // `for k, v in pairs(t) do t[k] = v + 100 end` and
                // friends. The TaggedValue local's slot is already a
                // 16-byte tagged search-key slot, so we hand it
                // directly to `emit_hash_probe_for_insert` after a
                // tag check. Other key expr shapes (e.g.
                // `IndexTagged`) would need tmp materialization;
                // only `Local(TaggedValue)` is supported in scope.
                ValueKind::TaggedValue => {
                    let search_key_slot = match &key.kind {
                        HirExprKind::Local(LocalId(idx)) => slots[*idx],
                        _ => {
                            return Err(CodegenError::UnsupportedExpr(
                                "TaggedValue-key IndexAssign requires `Local` key (ADR 0084 \
                                 scope)"
                                    .to_owned(),
                            ));
                        }
                    };
                    // Hash path — same structure as the static-key
                    // arm above, but: (a) the search-key slot is the
                    // TaggedValue local's slot rather than a freshly
                    // built 16-byte tmp, and (b) the new-key commit
                    // copies the slot's 16 bytes (tag at +0, payload
                    // at +8) into entry+0 raw rather than calling
                    // `emit_value_slot_store_dispatched` with a
                    // static kind. ADR 0087: nil/NaN tag validity
                    // is enforced at the probe chokepoint
                    // (`emit_hash_probe_loop`); no inline trap here.
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
                        context,
                        block,
                        hash_buf,
                        cap,
                        search_key_slot,
                        types,
                        loc,
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
                    // Empty bucket detection: tag at entry+0 is
                    // TAG_NIL (= 0) iff the bucket is unused.
                    let cur_tag = emit_load(block, entry_ptr, types.i64, loc);
                    let zero_i64_t = block
                        .append_operation(arith::constant(
                            context,
                            IntegerAttribute::new(types.i64, TAG_NIL).into(),
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    let was_empty: Value<'c, 'a> = block
                        .append_operation(arith::cmpi(
                            context,
                            arith::CmpiPredicate::Eq,
                            cur_tag,
                            zero_i64_t,
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
                        ValueKind::Number
                        | ValueKind::Bool
                        | ValueKind::String
                        | ValueKind::Function(_)
                        | ValueKind::Table => {
                            // New-key path: raw 16-byte copy of the
                            // search slot (tag + payload words) into
                            // entry+0; then count++.
                            let new_key_then = Region::new();
                            let new_key_blk = Block::new(&[]);
                            {
                                let key_tag =
                                    emit_load(&new_key_blk, search_key_slot, types.i64, loc);
                                emit_store(&new_key_blk, key_tag, entry_ptr, loc);
                                let key_pay_ptr = emit_byte_offset_ptr(
                                    context,
                                    &new_key_blk,
                                    search_key_slot,
                                    ARRAY_ELEM_OFF_VALUE,
                                    types,
                                    loc,
                                );
                                let entry_pay_ptr = emit_byte_offset_ptr(
                                    context,
                                    &new_key_blk,
                                    entry_ptr,
                                    ARRAY_ELEM_OFF_VALUE,
                                    types,
                                    loc,
                                );
                                let key_pay = emit_load(&new_key_blk, key_pay_ptr, types.i64, loc);
                                emit_store(&new_key_blk, key_pay, entry_pay_ptr, loc);
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
                            // Soft-delete via TAG_DELETED at entry+0
                            // when the key was present. No-op when
                            // bucket was empty (deleting a missing
                            // key is fine per Lua spec).
                            let del_then = Region::new();
                            let del_then_blk = Block::new(&[]);
                            {
                                let tombstone_tag = del_then_blk
                                    .append_operation(arith::constant(
                                        context,
                                        IntegerAttribute::new(types.i64, TAG_DELETED).into(),
                                        loc,
                                    ))
                                    .result(0)
                                    .unwrap()
                                    .into();
                                emit_store(&del_then_blk, tombstone_tag, entry_ptr, loc);
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
                            "HIR rejects non-Number/Bool/String/Nil/Function/Table values for \
                             hash insert"
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
    callee: &Callee,
    args: &[HirExpr],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    // Phase 2.8e-iter-next (ADR 0081) / 2.5x-callee-dispatch (ADR
    // 0082): dispatch on callee kind. The User path is the original
    // multi-return shape; Builtin opens for `Next`; IndirectDispatch
    // is the new ADR 0082 ABI for tagged-callee multi-return.
    match callee {
        Callee::User {
            fid: FuncId(fid),
            holding_local,
        } => emit_multi_assign_from_user_call(
            context,
            block,
            dst_ids,
            *fid,
            *holding_local,
            args,
            slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        ),
        Callee::Builtin(b) => emit_multi_assign_from_builtin(
            context,
            block,
            dst_ids,
            *b,
            args,
            slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        ),
        Callee::Indirect(_) => {
            unreachable!("HIR rejects Indirect in MultiAssignFromCall (ADR 0075)")
        }
        Callee::IndirectDispatch {
            local_id: LocalId(idx),
            sig,
            candidates,
        } => {
            // Phase 2.5x-callee-dispatch (ADR 0082): multi-assign
            // path. Reuse `emit_indirect_dispatch_call` to obtain
            // the dispatch op (whose flat MLIR results match the
            // call site's signature), then unpack each logical
            // return position into its destination slot via the
            // ADR 0076 walker (`flat_result_index` +
            // `emit_pack_tagged_result_at_pos`).
            emit_multi_assign_from_indirect_dispatch(
                context,
                block,
                dst_ids,
                *idx,
                sig,
                candidates,
                args,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )
        }
    }
}

/// Phase 2.5x-callee-dispatch (ADR 0082): emit a per-call-site
/// dispatch chain over `candidates`. Steps:
///
/// 1. Tag check at slot+0 — trap with `s_call_non_function` if
///    the runtime tag isn't `TAG_FUNCTION`.
/// 2. Load fn pointer at slot+8.
/// 3. Evaluate args once in the outer block.
/// 4. Build a nested `scf.if` chain over `candidates`. Each level
///    compares the loaded ptr to `func.constant @user_fn_X`; on
///    match emits a *direct* `func.call @user_fn_X` (no
///    `func.call_indirect` cast — Codex pre-ADR-0082 review's
///    forward-edge integrity recommendation). The innermost
///    else-branch traps with `s_call_unknown_fn_ptr` and yields
///    dummy values to satisfy MLIR's region-yield type check.
///
/// Returns a single `Value` for single-value caller (`emit_expr`).
/// Multi-value caller (`emit_multi_assign_from_indirect_dispatch`)
/// uses `emit_indirect_dispatch_chain` directly to access the full
/// flat result vector.
#[allow(clippy::too_many_arguments)]
fn emit_indirect_dispatch_call<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_idx: usize,
    sig: &IndirectSig,
    candidates: &[FuncId],
    args: &[HirExpr],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    let chain_op_results = emit_indirect_dispatch_chain_in_block(
        context,
        block,
        slot_idx,
        sig,
        candidates,
        args,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;
    Ok(chain_op_results[0])
}

/// Phase 2.5x-callee-dispatch (ADR 0082): multi-assign caller.
/// Materialise the dispatch chain in the outer block, then unpack
/// each logical return position via `flat_result_index` (ADR 0076).
#[allow(clippy::too_many_arguments)]
fn emit_multi_assign_from_indirect_dispatch<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    dst_ids: &[LocalId],
    slot_idx: usize,
    sig: &IndirectSig,
    candidates: &[FuncId],
    args: &[HirExpr],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let chain_results = emit_indirect_dispatch_chain_in_block(
        context,
        block,
        slot_idx,
        sig,
        candidates,
        args,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;
    for (i, dst) in dst_ids.iter().enumerate() {
        let info = &locals[dst.0];
        let dst_slot = slots[dst.0];
        match info.kind {
            ValueKind::TaggedValue => {
                // 2 MLIR results (tag, payload_raw) for this position.
                let base = flat_result_index(&sig.ret_kinds, i);
                let tag = chain_results[base];
                let payload = chain_results[base + 1];
                emit_store(block, tag, dst_slot, loc);
                let pay_ptr = emit_byte_offset_ptr(
                    context,
                    block,
                    dst_slot,
                    ARRAY_ELEM_OFF_VALUE,
                    types,
                    loc,
                );
                emit_store(block, payload, pay_ptr, loc);
            }
            _ => {
                // Phase 2.5c-full Commit 2a (ADR 0083): Function-kind
                // dispatch results are already `!llvm.ptr` — no ucast
                // bridge needed since user fns moved to the LLVM dialect.
                let result_idx = flat_result_index(&sig.ret_kinds, i);
                let v = chain_results[result_idx];
                emit_store(block, v, dst_slot, loc);
            }
        }
    }
    Ok(())
}

/// Phase 2.5x-callee-dispatch (ADR 0082): the dispatch chain
/// builder. Returns the flat MLIR result vector of the outer
/// `scf.if`. Helper used by both the single-value and multi-assign
/// callers above.
#[allow(clippy::too_many_arguments)]
fn emit_indirect_dispatch_chain_in_block<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_idx: usize,
    sig: &IndirectSig,
    candidates: &[FuncId],
    args: &[HirExpr],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<Vec<Value<'c, 'a>>, CodegenError> {
    // ===== Step 1: tag check =====
    let slot_ptr = slots[slot_idx];
    let tag = emit_load(block, slot_ptr, types.i64, loc);
    let tag_function_const = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, TAG_FUNCTION).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let tag_mismatch: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Ne,
            tag,
            tag_function_const,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let trap_then = Region::new();
    let trap_then_blk = Block::new(&[]);
    {
        let msg_ptr = emit_addressof(context, &trap_then_blk, "s_call_non_function", types, loc);
        emit_exit_with_message(context, &trap_then_blk, msg_ptr, types, loc);
        trap_then_blk.append_operation(scf::r#yield(&[], loc));
    }
    trap_then.append_block(trap_then_blk);
    let trap_else = Region::new();
    let trap_else_blk = Block::new(&[]);
    trap_else_blk.append_operation(scf::r#yield(&[], loc));
    trap_else.append_block(trap_else_blk);
    block.append_operation(scf::r#if(tag_mismatch, &[], trap_then, trap_else, loc));

    // ===== Step 2: load cell ptr, then dereference cell.fn_ptr =====
    // Phase 2.5c-full Commit 2b (ADR 0083): TAG_FUNCTION payload now
    // holds the closure cell ptr (since the producer flip);
    // dispatch-chain candidate comparison is still done against the
    // raw fn ptr (`addressof @user_fn_NN`), so we extract `cell.fn_ptr`
    // here. Static `@user_fn_NN_closure` singletons make the load a
    // simple GEP-then-load, no allocation involved.
    let payload_ptr =
        emit_byte_offset_ptr(context, block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
    let cell_ptr = emit_load(block, payload_ptr, types.ptr, loc);
    let loaded_ptr = super::closure::emit_load_closure_fn_ptr(context, block, cell_ptr, types, loc);

    // ===== Step 3: evaluate args once =====
    let mut arg_vals: Vec<Value<'c, 'a>> = Vec::with_capacity(args.len());
    for a in args {
        arg_vals.push(emit_expr(
            context,
            block,
            a,
            slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        )?);
    }

    // ===== Step 4: dispatch chain =====
    let result_types = ret_mlir_types(context, &sig.ret_kinds, types);
    let chain_op_results = emit_dispatch_chain_recursive(
        context,
        block,
        loaded_ptr,
        cell_ptr,
        candidates,
        &arg_vals,
        functions,
        &result_types,
        types,
        loc,
    );
    Ok(chain_op_results)
}

/// Phase 2.5x-callee-dispatch (ADR 0082): recursively build the
/// nested `scf.if` chain. Each level compares `loaded_ptr` to a
/// candidate's `func.constant` address; on match performs a direct
/// `func.call`; on mismatch recurses with the remaining candidates.
/// The base case (no more candidates) emits an unconditional trap
/// with dummy yields to satisfy MLIR's region-yield type check.
#[allow(clippy::too_many_arguments)]
fn emit_dispatch_chain_recursive<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    loaded_ptr: Value<'c, 'a>,
    cell_ptr: Value<'c, 'a>,
    candidates: &[FuncId],
    arg_vals: &[Value<'c, 'a>],
    functions: &[HirFunction],
    result_types: &[Type<'c>],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Vec<Value<'c, 'a>> {
    if candidates.is_empty() {
        // Base: trap + dummy yields. The exit_with_message diverges,
        // so the dummy values are never read at runtime.
        let msg_ptr = emit_addressof(context, block, "s_call_unknown_fn_ptr", types, loc);
        emit_exit_with_message(context, block, msg_ptr, types, loc);
        result_types
            .iter()
            .map(|t| emit_dummy_value(context, block, *t, types, loc))
            .collect()
    } else {
        let first = candidates[0];
        let rest = &candidates[1..];
        let target = &functions[first.0];

        // Phase 2.5c-full Commit 2a (ADR 0083): `@user_fn_X` is now
        // a `llvm.func`; materialise its address with
        // `llvm.mlir.addressof : !llvm.ptr` and compare ptr-equal
        // against `loaded_ptr`.
        let fn_ptr_val = emit_addressof(context, block, &target.mangled_name, types, loc);

        // ptr equality
        let fn_ptr_iv: Value<'c, 'a> = block
            .append_operation(
                OperationBuilder::new("llvm.ptrtoint", loc)
                    .add_operands(&[fn_ptr_val])
                    .add_results(&[types.i64])
                    .build()
                    .expect("llvm.ptrtoint @user_fn_X"),
            )
            .result(0)
            .unwrap()
            .into();
        let loaded_iv: Value<'c, 'a> = block
            .append_operation(
                OperationBuilder::new("llvm.ptrtoint", loc)
                    .add_operands(&[loaded_ptr])
                    .add_results(&[types.i64])
                    .build()
                    .expect("llvm.ptrtoint loaded"),
            )
            .result(0)
            .unwrap()
            .into();
        let eq_cond: Value<'c, 'a> = block
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                loaded_iv,
                fn_ptr_iv,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();

        // Then region: direct `llvm.call @user_fn_X(cell_ptr, args)`
        // and yield unpacked results. Phase 2.5c-full Commit 3b body
        // (ADR 0083): every direct user call now prepends the
        // closure cell ptr. For dispatch chain candidates the cell
        // ptr source is `loaded_ptr` (the actual TaggedValue
        // payload that survived the tag check), passed as
        // `in_function_cell_ptr` to the helper's branch 3 — the
        // helper falls through to it because there's no
        // `holding_local` at the dispatch site.
        let then_region = Region::new();
        let then_blk = Block::new(&[]);
        {
            // Re-borrow loaded_ptr / cell_ptr / arg_vals at the
            // inner block's lifetime. Phase 2.5c-full Commit 3c
            // (ADR 0083): the call needs the *cell* ptr (heap-
            // allocated for capturing fns, singleton for non-
            // capturing) as `in_function_cell_ptr`, not the
            // raw `cell.fn_ptr` we used pre-3c — that crashed
            // when the entry block tried to unpack upvalues
            // off the wrong base.
            let inner_cell_ptr =
                unsafe { std::mem::transmute::<Value<'c, 'a>, Value<'c, '_>>(cell_ptr) };
            let inner_arg_vals =
                unsafe { std::mem::transmute::<&[Value<'c, 'a>], &[Value<'c, '_>]>(arg_vals) };
            let yields = emit_call_user_with_cell(
                context,
                &then_blk,
                target,
                None,
                inner_arg_vals,
                result_types,
                &[],
                types,
                Some(inner_cell_ptr),
                loc,
            );
            then_blk.append_operation(scf::r#yield(&yields, loc));
        }
        then_region.append_block(then_blk);

        // Else region: recurse with `rest` candidates.
        let else_region = Region::new();
        let else_blk = Block::new(&[]);
        {
            let inner_results = emit_dispatch_chain_recursive(
                context,
                &else_blk,
                unsafe { std::mem::transmute::<Value<'c, 'a>, Value<'c, '_>>(loaded_ptr) },
                unsafe { std::mem::transmute::<Value<'c, 'a>, Value<'c, '_>>(cell_ptr) },
                rest,
                unsafe { std::mem::transmute::<&[Value<'c, 'a>], &[Value<'c, '_>]>(arg_vals) },
                functions,
                result_types,
                types,
                loc,
            );
            else_blk.append_operation(scf::r#yield(&inner_results, loc));
        }
        else_region.append_block(else_blk);

        let outer_op = block.append_operation(scf::r#if(
            eq_cond,
            result_types,
            then_region,
            else_region,
            loc,
        ));
        (0..result_types.len())
            .map(|i| outer_op.result(i).unwrap().into())
            .collect()
    }
}

/// Phase 2.5x-callee-dispatch (ADR 0082): produce a dummy SSA
/// value of the given MLIR type, used for unreachable scf.if
/// yield branches (the runtime trap already diverged).
fn emit_dummy_value<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    ty: Type<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    if ty == types.f64 {
        block
            .append_operation(arith::constant(
                context,
                FloatAttribute::new(context, types.f64, 0.0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into()
    } else if ty == types.i1 {
        block
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i1, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into()
    } else if ty == types.i64 {
        block
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into()
    } else if ty == types.ptr {
        emit_null_ptr(context, block, types, loc)
    } else {
        // Function-typed dummy: not exercised today (compatible-set
        // never has Function in ret_kinds), but if it does, we'd
        // need a func.constant. Trap defensively for now.
        unreachable!(
            "ADR 0082 dummy for unsupported result type {:?} (please widen if needed)",
            ty
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_multi_assign_from_user_call<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    dst_ids: &[LocalId],
    fid: usize,
    holding_local: Option<LocalId>,
    args: &[HirExpr],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let target = &functions[fid];
    let mut arg_vals: Vec<Value<'c, 'a>> = Vec::with_capacity(args.len());
    for a in args {
        arg_vals.push(emit_expr(
            context,
            block,
            a,
            slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        )?);
    }
    let ret_types = ret_mlir_types(context, &target.ret_kinds, types);
    // Phase 2.5c-full Commit 3b body atomic Step 4 (ADR 0083):
    // route the cell ptr through the canonical helper. holding_local
    // resolves the binding's slot when set; otherwise fall back to
    // the enclosing fn's entry cell_ptr (recursion shortcut for
    // capturing self-recursion).
    let flat_results = emit_call_user_with_cell(
        context,
        block,
        target,
        holding_local,
        &arg_vals,
        &ret_types,
        slots,
        types,
        in_function_cell_ptr,
        loc,
    );
    for (i, dst) in dst_ids.iter().enumerate() {
        let info = &locals[dst.0];
        let dst_slot = slots[dst.0];
        match info.kind {
            ValueKind::TaggedValue => {
                emit_pack_tagged_result_at_pos(
                    context,
                    block,
                    &flat_results,
                    dst_slot,
                    &target.ret_kinds,
                    i,
                    types,
                    loc,
                );
            }
            _ => {
                let result_idx = flat_result_index(&target.ret_kinds, i);
                let v = flat_results[result_idx];
                emit_store(block, v, dst_slot, loc);
            }
        }
    }
    Ok(())
}

/// Phase 2.8e-iter-next (ADR 0081): multi-assign from a Builtin
/// callee. The only multi-return builtin today is `Next`, whose
/// `(TaggedValue, TaggedValue)` shape lowers to a `func.call` of
/// the module-level `__lumelir_next` helper followed by 4-i64
/// result extraction into the two caller-supplied TaggedValue
/// slots.
#[allow(clippy::too_many_arguments)]
fn emit_multi_assign_from_builtin<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    dst_ids: &[LocalId],
    b: Builtin,
    args: &[HirExpr],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    match b {
        Builtin::Next => emit_call_next_into_locals(
            context,
            block,
            dst_ids,
            args,
            slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        ),
        _ => unreachable!(
            "multi-assign from builtin {:?} not supported (only Next declares multi-return)",
            b
        ),
    }
}

/// Phase 2.8e-iter-next (ADR 0081): call `__lumelir_next(t,
/// prev_tag, prev_payload)` and write the 4-i64 result into the
/// two caller-supplied TaggedValue locals (`dst_ids[0]` = next_k,
/// `dst_ids[1]` = next_v).
///
/// `args[0]` (table) lowers to a ptr; `args[1]` (prev_k) is built
/// in a fresh tagged tmp slot via the dispatched store, so any
/// prev_k expression — Number / String / Bool / Function / Table
/// literal or local — flows through the same shape.
#[allow(clippy::too_many_arguments)]
fn emit_call_next_into_locals<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    dst_ids: &[LocalId],
    args: &[HirExpr],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    debug_assert_eq!(dst_ids.len(), 2);
    debug_assert_eq!(args.len(), 2);
    let table_val = emit_expr(
        context,
        block,
        &args[0],
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;
    // Build prev_k tagged slot. If args[1] is `Local(TaggedValue)`,
    // we can read its slot directly without a tmp; otherwise we
    // materialise into a tmp via the dispatched store.
    let (prev_tag, prev_payload) = emit_extract_prev_k(
        context,
        block,
        &args[1],
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;
    // Phase 2.5c-full Commit 2a (ADR 0083): `__lumelir_next` now
    // returns `!llvm.struct<(i64, i64, i64, i64)>`; unpack the four
    // i64 fields then write them into the two caller-supplied
    // TaggedValue slots (next_k tag/payload, next_v tag/payload).
    let next_ret_types = [types.i64, types.i64, types.i64, types.i64];
    let flat_results = emit_call_user_unpacked(
        context,
        block,
        "__lumelir_next",
        &[table_val, prev_tag, prev_payload],
        &next_ret_types,
        loc,
    );
    let k_tag = flat_results[0];
    let k_pay = flat_results[1];
    let v_tag = flat_results[2];
    let v_pay = flat_results[3];
    let k_slot = slots[dst_ids[0].0];
    let v_slot = slots[dst_ids[1].0];
    emit_store(block, k_tag, k_slot, loc);
    let k_pay_ptr = emit_byte_offset_ptr(context, block, k_slot, ARRAY_ELEM_OFF_VALUE, types, loc);
    emit_store(block, k_pay, k_pay_ptr, loc);
    emit_store(block, v_tag, v_slot, loc);
    let v_pay_ptr = emit_byte_offset_ptr(context, block, v_slot, ARRAY_ELEM_OFF_VALUE, types, loc);
    emit_store(block, v_pay, v_pay_ptr, loc);
    Ok(())
}

/// Phase 2.8e-iter-next (ADR 0081): produce `(prev_tag, prev_payload)`
/// SSA values for the `__lumelir_next` second/third arguments. If
/// the source expression is a TaggedValue local, read its slot's
/// tag and payload directly. Otherwise materialise into a fresh
/// tagged tmp slot (alloca + dispatched store) and read back.
#[allow(clippy::too_many_arguments)]
fn emit_extract_prev_k<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    expr: &HirExpr,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(Value<'c, 'a>, Value<'c, 'a>), CodegenError> {
    let kind = infer_kind(expr, locals, functions);
    if kind == ValueKind::TaggedValue {
        if let HirExprKind::Local(LocalId(idx)) = &expr.kind {
            let slot = slots[*idx];
            let tag = emit_load(block, slot, types.i64, loc);
            let pay_ptr =
                emit_byte_offset_ptr(context, block, slot, ARRAY_ELEM_OFF_VALUE, types, loc);
            let pay = emit_load(block, pay_ptr, types.i64, loc);
            return Ok((tag, pay));
        }
    }
    // Materialise into a fresh tmp tagged slot.
    let tmp = emit_alloca_slot_for_kind(context, block, ValueKind::TaggedValue, types, loc);
    if matches!(kind, ValueKind::Nil) {
        emit_value_slot_store_nil(context, block, tmp, types, loc);
    } else {
        let val = emit_expr(
            context,
            block,
            expr,
            slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        )?;
        emit_value_slot_store_dispatched(context, block, tmp, val, kind, types, loc);
    }
    let tag = emit_load(block, tmp, types.i64, loc);
    let pay_ptr = emit_byte_offset_ptr(context, block, tmp, ARRAY_ELEM_OFF_VALUE, types, loc);
    let pay = emit_load(block, pay_ptr, types.i64, loc);
    Ok((tag, pay))
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let target_ptr = emit_expr(
        context,
        block,
        target,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;
    let key_kind = infer_kind(key, locals, functions);
    match key_kind {
        ValueKind::Number => {
            let key_f = emit_expr(
                context,
                block,
                key,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
            // Phase 2.6b-hash-key-nan (ADR 0086): inline tagged-
            // tmp materialisation (e.g. `print(t[0/0])`) shares
            // the NaN preflight with the standard Index Number-
            // key arm. The OOB-returns-nil contract below applies
            // to in-range misses, not to NaN.
            let is_nan: Value<'c, 'a> = block
                .append_operation(arith::cmpf(context, CmpfPredicate::Une, key_f, key_f, loc))
                .result(0)
                .unwrap()
                .into();
            emit_table_index_nan_trap_if(context, block, is_nan, types, loc);
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
        // Phase 2.6b-hash-keys (ADR 0079) / 2.6b-hash-lookup-miss
        // (ADR 0088): every non-Number hash-eligible kind shares the
        // read path. The key value is materialised into a 16-byte
        // tagged search slot, then the chokepoint helper resolves
        // `null_buf + probe + key_at_null + outcome` and writes the
        // 16-byte result (Nil on missing, copy on found) into
        // `dst_slot`.
        ValueKind::String | ValueKind::Bool | ValueKind::Function(_) | ValueKind::Table => {
            let key_value = emit_expr(
                context,
                block,
                key,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
            let key_slot =
                emit_build_search_key_slot(context, block, key_kind, key_value, types, loc);
            emit_hash_lookup_into_tagged_slot(
                context,
                block,
                target_ptr,
                key_slot,
                dst_slot,
                HashLookupOutcome::NilOnMissing,
                types,
                loc,
            );
        }
        _ => unreachable!("IndexTagged key must be Number or String"),
    }
    Ok(())
}

/// Phase 2.6c-tag-fn-tbl Tidy First (post-ADR 0070): rule-of-
/// three extract for the inline `Index → tmp tagged slot →
/// consumer` pattern shared by `Builtin::Print` (ADR 0065),
/// `Builtin::Type`, `Builtin::ToString` (ADR 0067 / 0070).
/// Allocates a 16-byte `TaggedValue` slot, fills it via the
/// non-trapping `emit_local_init_tagged`, and returns the slot
/// pointer. The caller chains the appropriate
/// `emit_<consumer>_tagged_local` helper.
#[allow(clippy::too_many_arguments)]
fn emit_inline_index_into_tagged_tmp<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    target: &HirExpr,
    key: &HirExpr,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    let tmp = emit_alloca_slot_for_kind(context, block, ValueKind::TaggedValue, types, loc);
    emit_local_init_tagged(
        context,
        block,
        tmp,
        target,
        key,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;
    Ok(tmp)
}

/// Phase 2.6c-tag-locals-fn (ADR 0074): emit a `Callee::User`
/// call whose `ret_kinds[0]` is `TaggedValue` and pack the two
/// MLIR results (i64 tag, i64 payload_raw) into the tagged
/// `dst_slot`. Reads result position 0 only — for multi-
/// position TaggedValue returns into multiple destination
/// slots, see `emit_multi_assign_from_call` which uses
/// [`emit_pack_tagged_result_at_pos`] per position
/// (ADR 0076).
#[allow(clippy::too_many_arguments)]
fn emit_call_user_into_tagged_slot<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    dst_slot: Value<'c, 'a>,
    fid: usize,
    holding_local: Option<LocalId>,
    call_args: &[HirExpr],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let target = &functions[fid];
    let mut arg_vals: Vec<Value<'c, 'a>> = Vec::with_capacity(call_args.len());
    for a in call_args {
        arg_vals.push(emit_expr(
            context,
            block,
            a,
            slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        )?);
    }
    let ret_types = ret_mlir_types(context, &target.ret_kinds, types);
    // Phase 2.5c-full Commit 3b body atomic Step 4 (ADR 0083):
    // route the cell ptr through the canonical helper.
    let flat_results = emit_call_user_with_cell(
        context,
        block,
        target,
        holding_local,
        &arg_vals,
        &ret_types,
        slots,
        types,
        in_function_cell_ptr,
        loc,
    );
    emit_pack_tagged_result_at_pos(
        context,
        block,
        &flat_results,
        dst_slot,
        &target.ret_kinds,
        0,
        types,
        loc,
    );
    Ok(())
}

/// Phase 2.6c-tag-locals-fn (ADR 0074): allocate a tmp 16-byte
/// tagged slot, evaluate a `Callee::User` call with TaggedValue
/// return, store the two results into the tmp, and return the
/// slot pointer. Mirror of [`emit_inline_index_into_tagged_tmp`]
/// (ADR 0071) for the function-call source.
#[allow(clippy::too_many_arguments)]
fn emit_call_user_into_tagged_tmp<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    fid: usize,
    holding_local: Option<LocalId>,
    call_args: &[HirExpr],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    let tmp = emit_alloca_slot_for_kind(context, block, ValueKind::TaggedValue, types, loc);
    emit_call_user_into_tagged_slot(
        context,
        block,
        tmp,
        fid,
        holding_local,
        call_args,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;
    Ok(tmp)
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
        // Phase 2.6b-hash-keys (ADR 0079): zero-init each entry's
        // key tag (offset 0 inside the key slot). TAG_NIL = 0
        // marks the bucket empty; malloc doesn't zero-init so we
        // write i64(0) explicitly at every key-tag position. The
        // payload word and the value slot are left undefined —
        // probe semantics never observe them while the tag says
        // empty.
        let zero_tag = then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, TAG_NIL).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        for i in 0..HASH_INITIAL_CAP {
            let entry_off = HASH_OFF_ENTRIES + i * HASH_ENTRY_SIZE + HASH_ENTRY_OFF_KEY_SLOT;
            let key_tag_slot =
                emit_byte_offset_ptr(context, &then_blk, new_buf, entry_off, types, loc);
            emit_store(&then_blk, zero_tag, key_tag_slot, loc);
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
/// skeleton wrapped by `emit_hash_probe_for_insert`. Walks
/// `hash_buf` from `hash(key_str) & mask` forward, skipping
/// deleted-tombstone buckets (`HASH_DELETED_KEY`), until a stop
/// condition fires.
///
/// Termination (post-ADR-0088 unified semantics):
///   - empty bucket (key == null) terminates the loop (caller
///     treats as "missing-or-insert-here"; the `key_at_null` test
///     after the probe lets caller dispatch via `HashLookupOutcome`
///     for read-side or insert-overwrite semantics for write-side)
///   - matching key terminates the loop and the bucket is returned
///
/// Phase 2.6b-hash-keys (ADR 0079): allocate and fill a 16-byte
/// tagged search-key slot. Caller passes the key's static
/// `ValueKind` and its lowered SSA value; the helper invokes
/// `emit_value_slot_store_dispatched` to populate the slot's
/// `{tag, payload}` shape. The returned `!llvm.ptr` is then
/// passed to `emit_hash_probe_*` for tag-aware probe and
/// equality.
fn emit_build_search_key_slot<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    key_kind: ValueKind,
    key_value: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let slot = emit_alloca_slot_for_kind(context, block, ValueKind::TaggedValue, types, loc);
    emit_value_slot_store_dispatched(context, block, slot, key_value, key_kind, types, loc);
    slot
}

/// Phase 2.6b-hash-keys (ADR 0079): hash a tagged-key slot.
/// Dispatches on the runtime tag at slot+0 — Number uses the
/// f64 bit-pattern, String calls FNV-1a on the payload, Bool
/// extends the i64 zext payload, Function / Table use the
/// payload pointer's bits. Each per-tag arm folds the raw bits
/// through the FNV prime so neighboring values land in
/// different buckets.
fn emit_hash_key_hash_dispatched<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    const FNV_PRIME: i64 = 1099511628211;
    let tag = emit_load(block, slot_ptr, types.i64, loc);
    let payload_ptr =
        emit_byte_offset_ptr(context, block, slot_ptr, ARRAY_ELEM_OFF_VALUE, types, loc);
    let prime_val: Value<'c, 'a> = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, FNV_PRIME).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    // String tag → call FNV-1a directly.
    let tag_string_const = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, TAG_STRING).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let is_string: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            tag,
            tag_string_const,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let str_then = Region::new();
    let str_then_blk = Block::new(&[]);
    {
        let str_ptr = emit_load(&str_then_blk, payload_ptr, types.ptr, loc);
        let h = emit_string_hash(context, &str_then_blk, str_ptr, types, loc);
        str_then_blk.append_operation(scf::r#yield(&[h], loc));
    }
    str_then.append_block(str_then_blk);
    // All other tags: load payload as i64 and multiply by FNV_PRIME.
    // This works for Number (f64 bitcast = same 8 bytes),
    // Bool (zext i1→i64), Function/Table (ptrtoint).
    let str_else = Region::new();
    let str_else_blk = Block::new(&[]);
    {
        let payload_i64 = emit_load(&str_else_blk, payload_ptr, types.i64, loc);
        let h: Value<'c, '_> = str_else_blk
            .append_operation(arith::muli(payload_i64, prime_val, loc))
            .result(0)
            .unwrap()
            .into();
        str_else_blk.append_operation(scf::r#yield(&[h], loc));
    }
    str_else.append_block(str_else_blk);
    let if_op = scf::r#if(is_string, &[types.i64], str_then, str_else, loc);
    block.append_operation(if_op).result(0).unwrap().into()
}

/// Phase 2.6b-hash-keys (ADR 0079): tag-aware key equality on
/// two 16-byte tagged slots. Returns `i1 false` immediately
/// when the tags differ (Lua spec: keys of different kinds
/// are never equal, even if their bit patterns coincide).
/// When the tags match, dispatches on the shared tag — Number
/// uses `cmpf Oeq`, String uses `strcmp`, Bool / Function /
/// Table use raw i64 / ptr equality. Function / Table are
/// raw-pointer identity per Lua spec.
fn emit_hash_key_eq_dispatched<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_a: Value<'c, 'a>,
    slot_b: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let tag_a = emit_load(block, slot_a, types.i64, loc);
    let tag_b = emit_load(block, slot_b, types.i64, loc);
    let tags_eq: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            tag_a,
            tag_b,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let payload_a = emit_byte_offset_ptr(context, block, slot_a, ARRAY_ELEM_OFF_VALUE, types, loc);
    let payload_b = emit_byte_offset_ptr(context, block, slot_b, ARRAY_ELEM_OFF_VALUE, types, loc);
    // tags differ → false; tags equal → dispatch.
    let same_tag_then = Region::new();
    let same_tag_then_blk = Block::new(&[]);
    {
        // String → strcmp; everything else → raw i64 compare.
        let tag_string_const = same_tag_then_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, TAG_STRING).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let is_string: Value<'c, '_> = same_tag_then_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                tag_a,
                tag_string_const,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let str_then = Region::new();
        let str_then_blk = Block::new(&[]);
        {
            let pa = emit_load(&str_then_blk, payload_a, types.ptr, loc);
            let pb = emit_load(&str_then_blk, payload_b, types.ptr, loc);
            let cmp = emit_libc_call_i32(context, &str_then_blk, "strcmp", &[pa, pb], types, loc);
            let zero_i32 = str_then_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i32, 0).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let eq: Value<'c, '_> = str_then_blk
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
            str_then_blk.append_operation(scf::r#yield(&[eq], loc));
        }
        str_then.append_block(str_then_blk);
        // Number tag uses cmpf Oeq for IEEE-754 semantics
        // (NaN != NaN); other tags (Bool / Function / Table)
        // use bit-equality on the i64 payload word.
        let str_else = Region::new();
        let str_else_blk = Block::new(&[]);
        {
            let tag_number_const = str_else_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, TAG_NUMBER).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let is_number: Value<'c, '_> = str_else_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Eq,
                    tag_a,
                    tag_number_const,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let num_then = Region::new();
            let num_then_blk = Block::new(&[]);
            {
                let fa = emit_load(&num_then_blk, payload_a, types.f64, loc);
                let fb = emit_load(&num_then_blk, payload_b, types.f64, loc);
                let cmp_op = arith::cmpf(context, CmpfPredicate::Oeq, fa, fb, loc);
                let eq: Value<'c, '_> = num_then_blk
                    .append_operation(cmp_op)
                    .result(0)
                    .unwrap()
                    .into();
                num_then_blk.append_operation(scf::r#yield(&[eq], loc));
            }
            num_then.append_block(num_then_blk);
            let num_else = Region::new();
            let num_else_blk = Block::new(&[]);
            {
                let ia = emit_load(&num_else_blk, payload_a, types.i64, loc);
                let ib = emit_load(&num_else_blk, payload_b, types.i64, loc);
                let eq: Value<'c, '_> = num_else_blk
                    .append_operation(arith::cmpi(context, arith::CmpiPredicate::Eq, ia, ib, loc))
                    .result(0)
                    .unwrap()
                    .into();
                num_else_blk.append_operation(scf::r#yield(&[eq], loc));
            }
            num_else.append_block(num_else_blk);
            let n_op = scf::r#if(is_number, &[types.i1], num_then, num_else, loc);
            let n_eq: Value<'c, '_> = str_else_blk
                .append_operation(n_op)
                .result(0)
                .unwrap()
                .into();
            str_else_blk.append_operation(scf::r#yield(&[n_eq], loc));
        }
        str_else.append_block(str_else_blk);
        let s_op = scf::r#if(is_string, &[types.i1], str_then, str_else, loc);
        let s_eq: Value<'c, '_> = same_tag_then_blk
            .append_operation(s_op)
            .result(0)
            .unwrap()
            .into();
        same_tag_then_blk.append_operation(scf::r#yield(&[s_eq], loc));
    }
    same_tag_then.append_block(same_tag_then_blk);
    let same_tag_else = Region::new();
    let same_tag_else_blk = Block::new(&[]);
    {
        let i1_false = same_tag_else_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i1, 0).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        same_tag_else_blk.append_operation(scf::r#yield(&[i1_false], loc));
    }
    same_tag_else.append_block(same_tag_else_blk);
    let outer = scf::r#if(tags_eq, &[types.i1], same_tag_then, same_tag_else, loc);
    block.append_operation(outer).result(0).unwrap().into()
}

/// Sentinel buckets are never the terminating answer in either
/// mode; the probe walks past them. Insert with sentinel reuse
/// is intentionally not implemented — sentinels are dropped at
/// the next rehash (ADR 0062).
///
/// Phase 2.6b-hash-keys (ADR 0079): the probe operates on a
/// caller-supplied 16-byte tagged search key slot. Hash and
/// equality dispatch on the slot's tag, supporting Number /
/// String / Bool / Function / Table keys.
fn emit_hash_probe_loop<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    hash_buf: Value<'c, 'a>,
    cap: Value<'c, 'a>,
    search_key_slot: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    // Phase 2.6b-hash-key-validity (ADR 0087): every TaggedValue-
    // key probe routes through this gate. It folds the ADR 0086
    // NaN preflight and ADR 0084 nil trap into one chokepoint, so
    // IndexAssign / Index / iterator-internal callers no longer
    // need per-call repetition. Pass-through for non-Nil/Number
    // tags.
    emit_hash_key_runtime_validity_gate(context, block, search_key_slot, types, loc);
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
    let hash = emit_hash_key_hash_dispatched(context, block, search_key_slot, types, loc);
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
        // Phase 2.6b-hash-keys (ADR 0079): the entry's first 16
        // bytes are now a tagged key slot — `{i64 tag, 8-byte
        // payload}`. Load only the tag here for the empty /
        // tombstone skip test; payload comparison happens inside
        // `emit_hash_key_eq_dispatched` via tag-aware dispatch.
        let key_tag = emit_load(&before_blk, entry_ptr, types.i64, loc);
        let tag_nil_const = before_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, TAG_NIL).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let is_null: Value<'c, '_> = before_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                key_tag,
                tag_nil_const,
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let tag_deleted_const = before_blk
            .append_operation(arith::constant(
                context,
                IntegerAttribute::new(types.i64, TAG_DELETED).into(),
                loc,
            ))
            .result(0)
            .unwrap()
            .into();
        let is_sentinel: Value<'c, '_> = before_blk
            .append_operation(arith::cmpi(
                context,
                arith::CmpiPredicate::Eq,
                key_tag,
                tag_deleted_const,
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
        // Phase 2.6b-hash-keys (ADR 0079): tag-aware key equality
        // gated on `is_skip`. Empty / tombstone buckets yield
        // `false` immediately so the probe walks past them; live
        // buckets dispatch on the search key's tag (Number cmpf,
        // String strcmp, Bool/Function/Table integer or pointer
        // equality).
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
            let eq_inner = emit_hash_key_eq_dispatched(
                context,
                &eq_else_blk,
                entry_ptr,
                search_key_slot,
                types,
                loc,
            );
            eq_else_blk.append_operation(scf::r#yield(&[eq_inner], loc));
        }
        eq_else.append_block(eq_else_blk);
        let eq_op = scf::r#if(is_skip, &[types.i1], eq_then, eq_else, loc);
        let eq: Value<'c, '_> = before_blk.append_operation(eq_op).result(0).unwrap().into();
        // Terminate on (is_null OR eq): empty bucket is a successful
        // terminator (the caller's `key_at_null` check decides
        // missing-key materialization via `HashLookupOutcome`,
        // ADR 0088).
        let terminate: Value<'c, '_> = before_blk
            .append_operation(arith::ori(is_null, eq, loc))
            .result(0)
            .unwrap()
            .into();
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

/// Phase 2.6b-hash (ADR 0058): walk `hash_buf` from a starting
/// bucket until either an empty slot OR a slot whose key equals
/// `key_str` is found, and return that bucket. Sentinel buckets
/// (ADR 0062) are skipped — no reuse. Used by the insert path,
/// the rehash placement path, the IsNilQuery hash arm, and (via
/// `emit_hash_lookup_into_tagged_slot`, ADR 0088) read-side
/// callers.
///
/// ADR 0087: runtime tag-validity (nil/NaN) is enforced inside
/// `emit_hash_probe_loop` via `emit_hash_key_runtime_validity_gate`.
/// Callers must NOT add inline nil/NaN checks before this entry —
/// the chokepoint owns the policy.
///
/// ADR 0088: empty-bucket termination is **not** a trap — the
/// helper / direct caller decides what missing means via
/// `HashLookupOutcome` or by checking `key_at_null` after the probe.
/// The previous `emit_hash_probe_lookup` wrapper that traps on
/// missing was retired with this ADR; lookup-side callers should
/// use `emit_hash_lookup_into_tagged_slot` instead.
fn emit_hash_probe_for_insert<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    hash_buf: Value<'c, 'a>,
    cap: Value<'c, 'a>,
    key_str: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    emit_hash_probe_loop(context, block, hash_buf, cap, key_str, types, loc)
}

/// Phase 2.6b-hash-lookup-miss (ADR 0088): consumer contract for what
/// happens when a hash lookup walks into an empty bucket (= key not
/// present). The chokepoint helper `emit_hash_lookup_into_tagged_slot`
/// owns the policy; call sites pass an outcome value rather than
/// duplicating the missing-branch trap or store inline. Replaces the
/// awkward `trap_on_null: bool` flag previously parameterising
/// `emit_hash_probe_loop` (codex review v3 critical issue).
///
/// `NilOnMissing` materialises a Nil-tagged value into the dst slot —
/// the consumer (caller) chains its own contract downstream
/// (e.g. `emit_value_slot_check_number` for arith Number, the
/// `IsNil` non-trapping path for nil-checks).
/// `TrapMissing` exits with `s_table_missing_key` and is reserved
/// for future strict-key consumers (`next()` style, not in scope here).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // TrapMissing reserved for next() / strict-key consumers
enum HashLookupOutcome {
    NilOnMissing,
    TrapMissing,
}

/// Phase 2.6b-hash-lookup-miss (ADR 0088): chokepoint helper for hash
/// table reads that materialise the lookup result into a 16-byte
/// TaggedValue dst slot. Consolidates the
/// `null_buf check → emit_hash_probe_for_insert → key_at_null check →
/// outcome dispatch` shape that ADR 0084 had duplicated across
/// `emit_local_init_tagged` and Index hash arms.
///
/// Lookup miss policy is owned at this chokepoint; callers must NOT
/// add inline missing-key traps. The probe-loop runtime-validity gate
/// (ADR 0087) fires inside `emit_hash_probe_for_insert` regardless of
/// `HashLookupOutcome`, so nil/NaN keys still trap with their dedicated
/// diagnostics.
///
/// On *found*, the helper raw-copies the entry's 16-byte value slot
/// `{tag, payload}` into `dst_slot`. The dst slot is always a 16-byte
/// TaggedValue layout; the helper is value-kind agnostic.
#[allow(clippy::too_many_arguments)]
fn emit_hash_lookup_into_tagged_slot<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    target_ptr: Value<'c, 'a>,
    search_key_slot: Value<'c, 'a>,
    dst_slot: Value<'c, 'a>,
    outcome: HashLookupOutcome,
    types: &Types<'c>,
    loc: Location<'c>,
) {
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
    emit_lookup_miss_dispatch(context, &then_blk, dst_slot, outcome, types, loc);
    then_blk.append_operation(scf::r#yield(&[], loc));
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    {
        let cap_slot = emit_byte_offset_ptr(context, &else_blk, hash_buf, HASH_OFF_CAP, types, loc);
        let cap = emit_load(&else_blk, cap_slot, types.i64, loc);
        let bucket = emit_hash_probe_for_insert(
            context,
            &else_blk,
            hash_buf,
            cap,
            search_key_slot,
            types,
            loc,
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
        let entry_ptr =
            emit_byte_offset_ptr_dynamic(context, &else_blk, hash_buf, entry_total_off, types, loc);
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
        let inner_then = Region::new();
        let inner_then_blk = Block::new(&[]);
        emit_lookup_miss_dispatch(context, &inner_then_blk, dst_slot, outcome, types, loc);
        inner_then_blk.append_operation(scf::r#yield(&[], loc));
        inner_then.append_block(inner_then_blk);
        let inner_else = Region::new();
        let inner_else_blk = Block::new(&[]);
        {
            // Found: raw 16-byte tagged copy from entry value slot
            // to dst slot (tag at +0, payload at +8).
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

/// Phase 2.6b-hash-lookup-miss (ADR 0088): per-outcome dispatch for
/// the missing-key branches of `emit_hash_lookup_into_tagged_slot`.
/// Same shape fires on both `hash_buf == null` and `key_at_null`
/// branches.
fn emit_lookup_miss_dispatch<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    dst_slot: Value<'c, 'a>,
    outcome: HashLookupOutcome,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    match outcome {
        HashLookupOutcome::NilOnMissing => {
            emit_value_slot_store_nil(context, block, dst_slot, types, loc);
        }
        HashLookupOutcome::TrapMissing => {
            let msg = emit_addressof(context, block, "s_table_missing_key", types, loc);
            emit_exit_with_message(context, block, msg, types, loc);
        }
    }
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
            // Phase 2.6b-hash-keys (ADR 0079): load the key tag at
            // entry+0 (i64) and the key payload (String ptr) at
            // entry+8. Live = tag is neither TAG_NIL (= empty) nor
            // TAG_DELETED (= tombstone, ADR 0062 hard delete).
            let old_key_tag = emit_load(&rehash_after_blk, old_entry_ptr, types.i64, loc);
            let tag_nil_inner = rehash_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, TAG_NIL).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let not_null: Value<'c, '_> = rehash_after_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Ne,
                    old_key_tag,
                    tag_nil_inner,
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let tag_deleted_inner = rehash_after_blk
                .append_operation(arith::constant(
                    context,
                    IntegerAttribute::new(types.i64, TAG_DELETED).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into();
            let not_sentinel: Value<'c, '_> = rehash_after_blk
                .append_operation(arith::cmpi(
                    context,
                    arith::CmpiPredicate::Ne,
                    old_key_tag,
                    tag_deleted_inner,
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
                // Phase 2.6b-hash-keys (ADR 0079): the old entry's
                // first 16 bytes already form a tagged search-key
                // slot; pass `old_entry_ptr` directly so probe's
                // tag-aware hash + equality kicks in for whatever
                // kind sits in that slot.
                let new_bucket = emit_hash_probe_for_insert(
                    context,
                    &migrate_blk,
                    new_buf,
                    new_cap,
                    old_entry_ptr,
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
                // Phase 2.6b-hash-keys (ADR 0079): copy the entire
                // 16-byte tagged key slot from old_entry+0 to
                // new_entry+0 — tag at +0 (i64), payload at +8
                // (raw i64; reinterpreted by the consumer based on
                // tag). This preserves every kind without needing
                // a key-kind-specific store helper.
                let old_key_tag_word = emit_load(&migrate_blk, old_entry_ptr, types.i64, loc);
                emit_store(&migrate_blk, old_key_tag_word, new_entry_ptr, loc);
                let old_key_payload_word_ptr = emit_byte_offset_ptr(
                    context,
                    &migrate_blk,
                    old_entry_ptr,
                    ARRAY_ELEM_OFF_VALUE,
                    types,
                    loc,
                );
                let old_key_payload_word =
                    emit_load(&migrate_blk, old_key_payload_word_ptr, types.i64, loc);
                let new_key_payload_word_ptr = emit_byte_offset_ptr(
                    context,
                    &migrate_blk,
                    new_entry_ptr,
                    ARRAY_ELEM_OFF_VALUE,
                    types,
                    loc,
                );
                emit_store(
                    &migrate_blk,
                    old_key_payload_word,
                    new_key_payload_word_ptr,
                    loc,
                );
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
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
                    context,
                    block,
                    elem,
                    slots,
                    locals,
                    functions,
                    types,
                    params_len,
                    in_function_cell_ptr,
                    loc,
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
                context,
                block,
                target,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
            let key_kind = infer_kind(key, locals, functions);
            match key_kind {
                ValueKind::Number => {
                    // Phase 2.6c-tag-arr (ADR 0059): each slot is
                    // a tagged 16-byte struct. Bounds-check, then
                    // load tag and trap on non-Number, then load
                    // the f64 value at offset 8.
                    let key_f = emit_expr(
                        context,
                        block,
                        key,
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?;
                    // Phase 2.6b-hash-key-nan (ADR 0086): symmetric
                    // to the IndexAssign arm — NaN preflight before
                    // f2i / bounds-check.
                    let is_nan: Value<'c, 'a> = block
                        .append_operation(arith::cmpf(
                            context,
                            CmpfPredicate::Une,
                            key_f,
                            key_f,
                            loc,
                        ))
                        .result(0)
                        .unwrap()
                        .into();
                    emit_table_index_nan_trap_if(context, block, is_nan, types, loc);
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
                // Phase 2.6b-hash-keys (ADR 0079): every non-Number
                // hash-eligible kind shares the lookup path; build
                // a tagged search-key slot for tag-aware probe.
                // Phase 2.6b-hash-keys (ADR 0079) / 2.6b-hash-lookup-
                // miss (ADR 0088): static hash-eligible key kinds
                // route through the chokepoint helper into a tmp
                // TaggedValue slot. Missing key → Nil-tagged tmp →
                // `emit_value_slot_check_number` traps with
                // `s_table_type_mismatch`; consumer-correct contract
                // since the Index value-side is f64-typed (Number).
                ValueKind::String | ValueKind::Bool | ValueKind::Function(_) | ValueKind::Table => {
                    let key_value = emit_expr(
                        context,
                        block,
                        key,
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?;
                    let key_slot =
                        emit_build_search_key_slot(context, block, key_kind, key_value, types, loc);
                    let tmp_slot = emit_alloca_slot_for_kind(
                        context,
                        block,
                        ValueKind::TaggedValue,
                        types,
                        loc,
                    );
                    emit_hash_lookup_into_tagged_slot(
                        context,
                        block,
                        target_ptr,
                        key_slot,
                        tmp_slot,
                        HashLookupOutcome::NilOnMissing,
                        types,
                        loc,
                    );
                    emit_value_slot_check_number(context, block, tmp_slot, types, loc);
                    let value_ptr = emit_byte_offset_ptr(
                        context,
                        block,
                        tmp_slot,
                        ARRAY_ELEM_OFF_VALUE,
                        types,
                        loc,
                    );
                    Ok(emit_load(block, value_ptr, types.f64, loc))
                }
                // Phase 2.8e-iter-tk (ADR 0084) / 2.6b-hash-lookup-
                // miss (ADR 0088): TaggedValue key — `t[k]` where
                // `k` is the iterator binding from
                // `for k, v in pairs(t)`. The local's slot is
                // already a 16-byte tagged search-key slot; flow
                // through the same chokepoint helper as static-key
                // arms. ADR 0087 nil/NaN validity gate fires inside
                // the helper's probe.
                ValueKind::TaggedValue => {
                    let search_key_slot = match &key.kind {
                        HirExprKind::Local(LocalId(idx)) => slots[*idx],
                        _ => {
                            return Err(CodegenError::UnsupportedExpr(
                                "TaggedValue-key Index read requires `Local` key (ADR 0084 \
                                 scope)"
                                    .to_owned(),
                            ));
                        }
                    };
                    let tmp_slot = emit_alloca_slot_for_kind(
                        context,
                        block,
                        ValueKind::TaggedValue,
                        types,
                        loc,
                    );
                    emit_hash_lookup_into_tagged_slot(
                        context,
                        block,
                        target_ptr,
                        search_key_slot,
                        tmp_slot,
                        HashLookupOutcome::NilOnMissing,
                        types,
                        loc,
                    );
                    emit_value_slot_check_number(context, block, tmp_slot, types, loc);
                    let value_ptr = emit_byte_offset_ptr(
                        context,
                        block,
                        tmp_slot,
                        ARRAY_ELEM_OFF_VALUE,
                        types,
                        loc,
                    );
                    Ok(emit_load(block, value_ptr, types.f64, loc))
                }
                _ => unreachable!("HIR rejects non-Number/String key kinds"),
            }
        }
        // Phase 2.6c-tag-hetero-eq (ADR 0066): non-trapping nil
        // probe of any tagged-value source. Operand is one of:
        //   - `Index { target, key }` (ADR 0061 path) — runtime
        //     bounds / probe; OOB / missing / Nil-tag → true.
        //   - `Local(idx)` with kind TaggedValue (ADR 0063 path)
        //     — load tag at slot+0 and compare with TAG_NIL.
        // Other operand shapes never reach here because HIR
        // lowering only ever generates these two.
        HirExprKind::IsNil(operand) => match &operand.kind {
            HirExprKind::Local(LocalId(idx)) => {
                let slot_ptr = slots[*idx];
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
            HirExprKind::Index { target, key } => emit_isnil_index(
                context,
                block,
                target,
                key,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            ),
            _ => unreachable!("IsNil operand must be Index or Local(TaggedValue) per HIR lowering"),
        },
        HirExprKind::Local(LocalId(idx)) => {
            let info = &locals[*idx];
            // Phase 2.5c-full Commit 3b body atomic Step 6 (ADR 0083):
            // captured locals (`is_captured = true`) read through a
            // heap upvalue box; the slot is the box ptr (set by
            // `emit_function` at function entry for params + body
            // locals, or at LocalInit for the alias case). Route
            // through the kind-typed box helper so writes from
            // sibling closures are observable. TaggedValue / Function
            // kinds bypass this path (out of Commit 3 scope).
            if info.is_captured
                && !matches!(info.kind, ValueKind::TaggedValue | ValueKind::Function(_))
            {
                return Ok(super::closure::emit_load_upvalue_box_value(
                    context,
                    block,
                    slots[*idx],
                    info.kind,
                    types,
                    loc,
                ));
            }
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
                ValueKind::Function(_) => {
                    // Three buckets (Phase 2.5b.3, ADR 0019):
                    //   - Function param: slots[idx] is the block arg
                    //     (a closure cell `!llvm.ptr` since Commit 2b).
                    //   - Known FuncId: materialise the static
                    //     `@<fn_sym>_closure` singleton's address.
                    //   - Otherwise: alloca'd `ptr` slot — load it.
                    // Phase 2.5c-full Commit 2b (ADR 0083): TAG_FUNCTION
                    // / Function-kind values are now closure cell ptrs;
                    // the raw fn-code address is reachable via
                    // `emit_load_closure_fn_ptr` at the call site.
                    if *idx < params_len {
                        Ok(slots[*idx])
                    } else if let Some(fid) = info.func_id {
                        let target = &functions[fid.0];
                        // Phase 2.5c-full Commit 3b body atomic Step
                        // 7b (ADR 0083): for capturing closures the
                        // synthetic FunctionDef-local's slot stores
                        // the freshly malloc'd cell ptr (LocalInit
                        // storage rule, Step 8). Loading from the
                        // slot recovers the per-instance cell that
                        // alias copies (`local g = f`) share.
                        // Non-capturing fns still use the static
                        // singleton — there's only one cell for them.
                        if target.upvalues.is_empty() {
                            let closure_sym = format!("{}_closure", target.mangled_name);
                            Ok(emit_addressof(context, block, &closure_sym, types, loc))
                        } else {
                            Ok(emit_load(block, slots[*idx], types.ptr, loc))
                        }
                    } else {
                        Ok(emit_load(block, slots[*idx], types.ptr, loc))
                    }
                }
            }
        }
        HirExprKind::BinOp { op, lhs, rhs } => {
            // `and`/`or` short-circuit — must NOT eagerly evaluate rhs.
            if matches!(op, BinOp::And | BinOp::Or) {
                return emit_short_circuit(
                    context,
                    block,
                    *op,
                    lhs,
                    rhs,
                    slots,
                    locals,
                    functions,
                    types,
                    params_len,
                    in_function_cell_ptr,
                    loc,
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
                    context,
                    block,
                    *op,
                    lhs,
                    rhs,
                    slots,
                    locals,
                    functions,
                    types,
                    params_len,
                    in_function_cell_ptr,
                    loc,
                )? {
                    return Ok(v);
                }
            }
            // Phase 2.7p-tagged-arith-coerce (ADR 0089): TaggedValue
            // arith / bitwise BinOp routes through the arith
            // dispatch chokepoint when at least one operand is
            // `Local(TaggedValue)`. Fall through to the existing
            // `emit_expr` path otherwise (typed Number operands or
            // ADR 0077 static-String coerce wrap).
            if let Some(v) = emit_tagged_arith_runtime_dispatch(
                context,
                block,
                *op,
                lhs,
                rhs,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )? {
                return Ok(v);
            }
            let lhs_val = emit_expr(
                context,
                block,
                lhs,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
            let rhs_val = emit_expr(
                context,
                block,
                rhs,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
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
                    context,
                    block,
                    operand,
                    slots,
                    locals,
                    functions,
                    types,
                    params_len,
                    in_function_cell_ptr,
                    loc,
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
                    context,
                    block,
                    operand,
                    slots,
                    locals,
                    functions,
                    types,
                    params_len,
                    in_function_cell_ptr,
                    loc,
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
            // Phase 2.7p-tagged-arith-coerce (ADR 0089): TaggedValue
            // unary arith routes through the same chokepoint as the
            // BinOp path. When operand is `Local(TaggedValue)` and
            // op is Neg / BitNot, fetch f64 via
            // `emit_load_tagged_operand_as_number`; otherwise fall
            // through to the existing `emit_expr` path.
            let v = if matches!(op, UnaryOp::Neg | UnaryOp::BitNot)
                && let Some(LocalId(idx)) = tagged_local_idx(operand, locals)
            {
                emit_load_tagged_operand_as_number(context, block, slots[idx], types, loc)
            } else {
                emit_expr(
                    context,
                    block,
                    operand,
                    slots,
                    locals,
                    functions,
                    types,
                    params_len,
                    in_function_cell_ptr,
                    loc,
                )?
            };
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
                            let tmp = emit_inline_index_into_tagged_tmp(
                                context,
                                block,
                                target,
                                key,
                                slots,
                                locals,
                                functions,
                                types,
                                params_len,
                                in_function_cell_ptr,
                                loc,
                            )?;
                            emit_print_tagged_local(context, block, tmp, types, loc);
                            continue;
                        }
                        // Phase 2.6c-tag-locals-fn (ADR 0074):
                        // inline `print(f())` where `f` returns
                        // TaggedValue — materialise into a tmp
                        // tagged slot and dispatch.
                        if let HirExprKind::Call {
                            callee:
                                Callee::User {
                                    fid: FuncId(fid),
                                    holding_local,
                                },
                            args: call_args,
                        } = &a.kind
                            && matches!(
                                functions[*fid].ret_kinds.first(),
                                Some(ValueKind::TaggedValue)
                            )
                        {
                            let tmp = emit_call_user_into_tagged_tmp(
                                context,
                                block,
                                *fid,
                                *holding_local,
                                call_args,
                                slots,
                                locals,
                                functions,
                                types,
                                params_len,
                                in_function_cell_ptr,
                                loc,
                            )?;
                            emit_print_tagged_local(context, block, tmp, types, loc);
                            continue;
                        }
                        let v = emit_expr(
                            context,
                            block,
                            a,
                            slots,
                            locals,
                            functions,
                            types,
                            params_len,
                            in_function_cell_ptr,
                            loc,
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
                // Phase 2.6c-tag-consumers (ADR 0067): when the
                // operand is a `Local(TaggedValue)`, dispatch on
                // the runtime tag. Without this, `emit_expr` on
                // the Local extracts as f64 with trap-on-non-
                // Number, which is wrong for Bool/String/Nil
                // payloads.
                if kind == ValueKind::TaggedValue {
                    if let HirExprKind::Local(LocalId(idx)) = &args[0].kind {
                        return Ok(emit_tostring_tagged_local(
                            context,
                            block,
                            slots[*idx],
                            types,
                            loc,
                        ));
                    }
                }
                // Phase 2.6c-tag-consumers-inline (ADR 0070):
                // inline `tostring(t[k])` materialises through a
                // tmp tagged slot (mirror of the `print(t[k])`
                // path in ADR 0065) so Bool / String / Nil
                // payloads dispatch instead of trapping at the
                // f64 extract.
                if let HirExprKind::Index { target, key } = &args[0].kind {
                    let tmp = emit_inline_index_into_tagged_tmp(
                        context,
                        block,
                        target,
                        key,
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?;
                    return Ok(emit_tostring_tagged_local(context, block, tmp, types, loc));
                }
                // Phase 2.6c-tag-locals-fn (ADR 0074): inline
                // `tostring(f())` where `f` returns TaggedValue.
                if let HirExprKind::Call {
                    callee:
                        Callee::User {
                            fid: FuncId(fid),
                            holding_local,
                        },
                    args: call_args,
                } = &args[0].kind
                    && matches!(
                        functions[*fid].ret_kinds.first(),
                        Some(ValueKind::TaggedValue)
                    )
                {
                    let tmp = emit_call_user_into_tagged_tmp(
                        context,
                        block,
                        *fid,
                        *holding_local,
                        call_args,
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?;
                    return Ok(emit_tostring_tagged_local(context, block, tmp, types, loc));
                }
                let arg_val = emit_expr(
                    context,
                    block,
                    &args[0],
                    slots,
                    locals,
                    functions,
                    types,
                    params_len,
                    in_function_cell_ptr,
                    loc,
                )?;
                Ok(emit_tostring(context, block, arg_val, kind, types, loc))
            }
            // Phase 2.7e (ADR 0028): `tonumber(x)`. Number identity
            // path; String dispatched through `sscanf("%lf")` with a
            // NaN sentinel on parse failure.
            Callee::Builtin(Builtin::ToNumber) => {
                let kind = infer_kind(&args[0], locals, functions);
                let arg_val = emit_expr(
                    context,
                    block,
                    &args[0],
                    slots,
                    locals,
                    functions,
                    types,
                    params_len,
                    in_function_cell_ptr,
                    loc,
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
                    context,
                    block,
                    &args[0],
                    slots,
                    locals,
                    functions,
                    types,
                    params_len,
                    in_function_cell_ptr,
                    loc,
                )?;
                let custom_msg = if args.len() == 2 {
                    Some(emit_expr(
                        context,
                        block,
                        &args[1],
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
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
                    context,
                    block,
                    &args[0],
                    slots,
                    locals,
                    functions,
                    types,
                    params_len,
                    in_function_cell_ptr,
                    loc,
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
                // Phase 2.6c-tag-consumers (ADR 0067): when the
                // operand is a `Local(TaggedValue)`, dispatch on
                // the runtime tag instead of the inner kind. The
                // ADR 0063 static-dispatch shortcut is what
                // LIC-2.6c-tag-locals-1 flagged.
                if kind == ValueKind::TaggedValue {
                    if let HirExprKind::Local(LocalId(idx)) = &args[0].kind {
                        return Ok(emit_type_tagged_local(
                            context,
                            block,
                            slots[*idx],
                            types,
                            loc,
                        ));
                    }
                }
                // Phase 2.6c-tag-consumers-inline (ADR 0070):
                // inline `type(t[k])` materialises through a
                // tmp tagged slot so the runtime tag determines
                // the typename, matching `Local(TaggedValue)`
                // semantics (ADR 0067) at the inline source.
                if let HirExprKind::Index { target, key } = &args[0].kind {
                    let tmp = emit_inline_index_into_tagged_tmp(
                        context,
                        block,
                        target,
                        key,
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?;
                    return Ok(emit_type_tagged_local(context, block, tmp, types, loc));
                }
                // Phase 2.6c-tag-locals-fn (ADR 0074): inline
                // `type(f())` where `f` returns TaggedValue.
                if let HirExprKind::Call {
                    callee:
                        Callee::User {
                            fid: FuncId(fid),
                            holding_local,
                        },
                    args: call_args,
                } = &args[0].kind
                    && matches!(
                        functions[*fid].ret_kinds.first(),
                        Some(ValueKind::TaggedValue)
                    )
                {
                    let tmp = emit_call_user_into_tagged_tmp(
                        context,
                        block,
                        *fid,
                        *holding_local,
                        call_args,
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?;
                    return Ok(emit_type_tagged_local(context, block, tmp, types, loc));
                }
                if !matches!(
                    args[0].kind,
                    HirExprKind::FunctionRef(_) | HirExprKind::Local(_)
                ) {
                    let _ = emit_expr(
                        context,
                        block,
                        &args[0],
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?;
                }
                let global = match kind {
                    ValueKind::Number => "s_typename_number",
                    ValueKind::String => "s_typename_string",
                    ValueKind::Bool => "s_typename_boolean",
                    ValueKind::Nil => "s_typename_nil",
                    ValueKind::Function(_) => "s_typename_function",
                    ValueKind::Table => "s_typename_table",
                    // TaggedValue not a Local — falls through to
                    // the inner-kind name. In practice this arm
                    // is unreachable for now (no other expression
                    // shape produces TaggedValue).
                    ValueKind::TaggedValue => "s_typename_number",
                };
                Ok(emit_addressof(context, block, global, types, loc))
            }
            Callee::Builtin(Builtin::Next) => {
                // Phase 2.8e-iter-next (ADR 0081): single-value
                // position `local k = next(t, c)` truncates the
                // multi-return to the first key. Today the only
                // observed call shape is the multi-assign form used
                // by ForPairs HIR-desugar (ADR 0081 commit 3) and
                // direct user multi-assign tests, so the truncated
                // form is HIR-rejected at the source rather than
                // forcing every consumer to materialise a TaggedValue
                // tmp slot. Reachable only if a future HIR site
                // forgets to use MultiAssignFromCall.
                Err(CodegenError::UnsupportedExpr(
                    "next(t, k) in single-value position is not supported (ADR 0081); use \
                     `local k, v = next(t, c)`"
                        .to_owned(),
                ))
            }
            Callee::User {
                fid: FuncId(fid),
                holding_local,
            } => {
                let target = &functions[*fid];
                let mut arg_vals: Vec<Value<'c, 'a>> = Vec::with_capacity(args.len());
                for a in args {
                    arg_vals.push(emit_expr(
                        context,
                        block,
                        a,
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?);
                }
                let ret_types = ret_mlir_types(context, &target.ret_kinds, types);
                // Phase 2.5c-full Commit 3b body atomic Step 4 (ADR
                // 0083): single canonical helper for direct user
                // calls in expression position. Void / multi-result
                // both flow through `emit_call_user_with_cell`; the
                // void path discards results and returns the f64
                // placeholder demanded by the expression-position
                // contract.
                let flat_results = emit_call_user_with_cell(
                    context,
                    block,
                    target,
                    *holding_local,
                    &arg_vals,
                    &ret_types,
                    slots,
                    types,
                    in_function_cell_ptr,
                    loc,
                );
                if target.ret_kinds.is_empty() {
                    // Void call: caller's `infer_kind` returned
                    // Number, so we synthesise a placeholder f64 0.0
                    // (never read).
                    let zero = arith::constant(
                        context,
                        FloatAttribute::new(context, types.f64, 0.0).into(),
                        loc,
                    );
                    Ok(block.append_operation(zero).result(0).unwrap().into())
                } else {
                    // Multi-result calls in expression position
                    // truncate to the first value (Lua semantics).
                    Ok(flat_results[0])
                }
            }
            Callee::Indirect(LocalId(idx)) => {
                // Phase 2.5b.2/b.3: a Function-kind local with no
                // statically known FuncId. Either it is a function
                // parameter (slot is the block argument) or it was
                // bound from a call/expression and lives in an
                // alloca'd `ptr` slot.
                //
                // Phase 2.6c-tag-callee-arity (ADR 0075) removed
                // the previous TaggedValue branch (ADR 0072): HIR
                // now rejects every indirect call through a
                // TaggedValue local, so by construction the local
                // is `Function(arity)` here with statically-known
                // arity.
                //
                // Phase 2.5c-full Commit 2a (ADR 0083): the local
                // already carries a `!llvm.ptr`; emit operand-callee
                // `llvm.call`. The non-variadic pointee signature is
                // inferred from the operand and result MLIR types,
                // so no `var_callee_type` attribute is needed.
                //
                // Phase 2.5c-full Commit 2a-fix (ADR 0083 / ADR 0075
                // amend): the f64 result type is sound because HIR's
                // `check_function_arg_ret_kinds` rejects any
                // Function-kind argument whose source fn declares
                // `ret_kinds != [Number]`. The verifier-level safety
                // net that `!func.func<...>` typing previously
                // provided (and Commit 2a's `!llvm.ptr` erasure
                // removed) is reinstated as a static HIR check.
                let info = &locals[*idx];
                let _ = match info.kind {
                    ValueKind::Function(a) => a,
                    _ => unreachable!(
                        "Callee::Indirect on non-Function local — ADR 0075 \
                         removed the TaggedValue path; HIR rejects upstream"
                    ),
                };
                // Phase 2.5c-full Commit 2b (ADR 0083): the local
                // holds a closure cell ptr (since the producer flip).
                // Extract `cell.fn_ptr` (offset 0) before issuing the
                // operand-callee `llvm.call`.
                let cell_ptr = if *idx < params_len {
                    slots[*idx]
                } else {
                    emit_load(block, slots[*idx], types.ptr, loc)
                };
                let callee_ptr =
                    super::closure::emit_load_closure_fn_ptr(context, block, cell_ptr, types, loc);
                // Phase 2.5c-full Commit 3c (ADR 0083): the cell-ptr
                // first arg ABI applies to indirect callees too. The
                // callee may be a non-capturing user fn (singleton
                // cell) or a capturing one (heap cell); both expect
                // the cell ptr in arg 0 so the entry block can
                // unpack upvalues uniformly.
                let mut arg_vals: Vec<Value<'c, 'a>> = Vec::with_capacity(args.len() + 1);
                arg_vals.push(cell_ptr);
                for a in args {
                    arg_vals.push(emit_expr(
                        context,
                        block,
                        a,
                        slots,
                        locals,
                        functions,
                        types,
                        params_len,
                        in_function_cell_ptr,
                        loc,
                    )?);
                }
                let op_ref = emit_llvm_call_indirect(
                    context,
                    block,
                    callee_ptr,
                    &arg_vals,
                    Some(types.f64),
                    loc,
                );
                Ok(op_ref.result(0).unwrap().into())
            }
            Callee::IndirectDispatch {
                local_id: LocalId(idx),
                sig,
                candidates,
            } => emit_indirect_dispatch_call(
                context,
                block,
                *idx,
                sig,
                candidates,
                args,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            ),
        },
        HirExprKind::FunctionRef(FuncId(id)) => {
            // Phase 2.5b.3 / 2.5c-full Commit 2b (ADR 0083): a
            // function reference materialises as a `!llvm.ptr` to
            // the static `@<fn_sym>_closure` singleton (non-capturing
            // case) or to a freshly malloc'd cell (capturing case,
            // Commit 3b body atomic Step 7a). Capturing closures
            // observe Lua spec §3.4.4 reference-equality: each
            // closure-creation site allocates a fresh cell, so two
            // instances from the same factory differ; alias copies
            // (`local g = f`) share the cell ptr stored in f's slot.
            let target = &functions[*id];
            if target.upvalues.is_empty() {
                let closure_sym = format!("{}_closure", target.mangled_name);
                Ok(emit_addressof(context, block, &closure_sym, types, loc))
            } else {
                // Capturing: malloc a fresh cell, populate fn_ptr,
                // upvalue_count, and each upvalue_box[i] from the
                // current scope's slot for the captured outer
                // local. The outer slot is already a heap upvalue
                // box ptr (Step 6) so we just thread it.
                let cell_ptr = super::closure::emit_allocate_closure_cell(
                    context,
                    block,
                    target.upvalues.len(),
                    types,
                    loc,
                );
                // Store @<fn_sym> at offset 0.
                let fn_addr = emit_addressof(context, block, &target.mangled_name, types, loc);
                super::closure::emit_store_closure_fn_ptr(
                    context, block, cell_ptr, fn_addr, types, loc,
                );
                // Store upvalue count at offset 8.
                let count_const: Value<'c, 'a> = block
                    .append_operation(arith::constant(
                        context,
                        IntegerAttribute::new(types.i64, target.upvalues.len() as i64).into(),
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let count_slot = emit_byte_offset_ptr(
                    context,
                    block,
                    cell_ptr,
                    super::closure::CLOSURE_OFF_UPVALUE_COUNT,
                    types,
                    loc,
                );
                emit_store(block, count_const, count_slot, loc);
                // Populate each upvalue_box[i] from the matching
                // outer slot. The outer slot holds the heap box
                // ptr because the captured-local post-pass marked
                // it `is_captured` and Step 6 routed its storage
                // through `emit_allocate_upvalue_box`.
                for (i, uv) in target.upvalues.iter().enumerate() {
                    let outer_box_ptr = slots[uv.outer_local_id.0];
                    super::closure::emit_store_closure_upvalue_box(
                        context,
                        block,
                        cell_ptr,
                        i,
                        outer_box_ptr,
                        types,
                        loc,
                    );
                }
                Ok(cell_ptr)
            }
        }
        // Phase 2.6c-tag-locals (ADR 0063): IndexTagged is consumed
        // inline by `emit_local_init_tagged` inside `emit_stmt` —
        // it never surfaces in value-context emit.
        HirExprKind::IndexTagged { .. } => {
            unreachable!("IndexTagged is consumed by emit_stmt(LocalInit/Assign)")
        }
        // Phase 2.7p-arith-string-coerce (ADR 0077): String operand
        // of an arithmetic / bitwise BinOp materialises a Number
        // at runtime. The wrapped expression is the original String
        // operand; we evaluate it (yielding a `!llvm.ptr` to the
        // string pool global), then call the trapping coerce
        // helper which runs `sscanf("%lf")` and exits on failure.
        HirExprKind::ArithStringCoerce(operand) => {
            let str_ptr = emit_expr(
                context,
                block,
                operand,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
            Ok(emit_tonumber_for_arith(context, block, str_ptr, types, loc))
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    emit_expr(
        context,
        block,
        expr,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
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

/// Phase 2.7p-arith-string-coerce (ADR 0077): `emit_tonumber`
/// for arithmetic context. Calls the same `sscanf` path as
/// `emit_tonumber(String)` then promotes the NaN-on-failure
/// sentinel into a runtime trap via `s_arith_coerce_failed`.
/// Lua spec §3.4.1: `"abc" + 1` is a runtime error, not silent
/// NaN propagation. The `tonumber` builtin path is unchanged
/// — its NaN sentinel is part of its public contract (ADR
/// 0028) and observable via `x == x` flips.
fn emit_tonumber_for_arith<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    str_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let parsed = emit_tonumber(context, block, str_ptr, ValueKind::String, types, loc);
    // arith.cmpf(uno) is true iff either operand is NaN. Comparing
    // a value to itself with `Une` (unordered or not equal) yields
    // true exactly when the value is NaN.
    let is_nan: Value<'c, 'a> = block
        .append_operation(arith::cmpf(
            context,
            CmpfPredicate::Une,
            parsed,
            parsed,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    let msg_ptr = emit_addressof(context, &then_blk, "s_arith_coerce_failed", types, loc);
    emit_exit_with_message(context, &then_blk, msg_ptr, types, loc);
    then_blk.append_operation(scf::r#yield(&[], loc));
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    else_blk.append_operation(scf::r#yield(&[], loc));
    else_region.append_block(else_blk);
    block.append_operation(scf::r#if(is_nan, &[], then_region, else_region, loc));
    parsed
}

/// Phase 2.7p-tagged-arith-coerce (ADR 0089): chokepoint that
/// materialises an `f64` operand from a 16-byte TaggedValue slot
/// for a Number-consuming arith / bitwise / unary site. The
/// dispatch is **driven by** `tagged.rs::policy_for_tagged_arith_operand`:
/// for each known tag in `DISPATCH_TAGS`, the helper queries the
/// policy and lays down an scf.if branch emitting the policy-
/// specific f64 production. The trailing `else` (no known tag
/// matched) materialises the `TrapNonNumeric` branch.
///
/// Per-policy emission:
/// - `UseNumberPayload` → load f64 at slot+8
/// - `CoerceStringToNumber` → load ptr at slot+8 and delegate to
///   `emit_tonumber_for_arith` (ADR 0077; sscanf-coerce with
///   internal `s_arith_coerce_failed` trap on parse failure)
/// - `TrapNonNumeric` → exit with `s_arith_on_non_numeric`
///   (Lua spec §3.4.3); yields a placeholder f64 to satisfy
///   scf.if region typing (matches the
///   `emit_tagged_eq_runtime_dispatch` pattern)
///
/// The trap policy is owned at this chokepoint; callers must NOT
/// add inline tag checks before this entry. Mirrors ADR 0087 /
/// 0088 pure-policy + effectful-executor split: `tagged.rs` decides;
/// this function emits.
fn emit_load_tagged_operand_as_number<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    // Tags whose policy is non-trapping; checked in order. Any
    // tag not in this list (Bool / Nil / Function / Table /
    // Deleted) falls through to the policy table's
    // `TrapNonNumeric` arm via the trailing else.
    const DISPATCH_TAGS: &[i64] = &[TAG_NUMBER, TAG_STRING];
    let (tag, payload_ptr) = emit_tag_and_payload_ptr(context, block, slot_ptr, types, loc);
    emit_tagged_arith_dispatch_chain(context, block, tag, payload_ptr, DISPATCH_TAGS, types, loc)
}

/// Phase 2.7p-tagged-arith-coerce (ADR 0089): recursive scf.if
/// chain. Each call consumes one `dispatch_tag`; the head tag's
/// `then` branch emits the policy's f64 production, the `else`
/// branch recurses into `rest`. When `rest` is empty, the policy
/// table's `TrapNonNumeric` arm fires (default fall-through).
fn emit_tagged_arith_dispatch_chain<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    tag: Value<'c, 'a>,
    payload_ptr: Value<'c, 'a>,
    dispatch_tags: &[i64],
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let Some((&head_tag, rest)) = dispatch_tags.split_first() else {
        // No more known tags — policy says TrapNonNumeric.
        return emit_arith_operand_plan(
            context,
            block,
            payload_ptr,
            TaggedArithOperandPlan::TrapNonNumeric,
            types,
            loc,
        );
    };
    let head_plan = policy_for_tagged_arith_operand(head_tag);
    let head_const = block
        .append_operation(arith::constant(
            context,
            IntegerAttribute::new(types.i64, head_tag).into(),
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let is_match: Value<'c, 'a> = block
        .append_operation(arith::cmpi(
            context,
            arith::CmpiPredicate::Eq,
            tag,
            head_const,
            loc,
        ))
        .result(0)
        .unwrap()
        .into();
    let then_region = Region::new();
    let then_blk = Block::new(&[]);
    {
        let f = emit_arith_operand_plan(context, &then_blk, payload_ptr, head_plan, types, loc);
        then_blk.append_operation(scf::r#yield(&[f], loc));
    }
    then_region.append_block(then_blk);
    let else_region = Region::new();
    let else_blk = Block::new(&[]);
    {
        let f = emit_tagged_arith_dispatch_chain(
            context,
            &else_blk,
            tag,
            payload_ptr,
            rest,
            types,
            loc,
        );
        else_blk.append_operation(scf::r#yield(&[f], loc));
    }
    else_region.append_block(else_blk);
    let if_op = scf::r#if(is_match, &[types.f64], then_region, else_region, loc);
    block.append_operation(if_op).result(0).unwrap().into()
}

/// Phase 2.7p-tagged-arith-coerce (ADR 0089): per-policy f64
/// emission in a single block. Pure mapping of policy variant to
/// the MLIR sequence that yields an f64 (or terminates with a
/// trap and returns a placeholder f64).
fn emit_arith_operand_plan<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    payload_ptr: Value<'c, 'a>,
    plan: TaggedArithOperandPlan,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    match plan {
        TaggedArithOperandPlan::UseNumberPayload => emit_load(block, payload_ptr, types.f64, loc),
        TaggedArithOperandPlan::CoerceStringToNumber => {
            let str_ptr = emit_load(block, payload_ptr, types.ptr, loc);
            emit_tonumber_for_arith(context, block, str_ptr, types, loc)
        }
        TaggedArithOperandPlan::TrapNonNumeric => {
            let msg_ptr = emit_addressof(context, block, "s_arith_on_non_numeric", types, loc);
            emit_exit_with_message(context, block, msg_ptr, types, loc);
            // scf.if region typing requires a yielded f64 even
            // though `emit_exit_with_message` terminates.
            block
                .append_operation(arith::constant(
                    context,
                    FloatAttribute::new(context, types.f64, 0.0).into(),
                    loc,
                ))
                .result(0)
                .unwrap()
                .into()
        }
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

/// Phase 2.6c-tag-hetero-eq (ADR 0066): non-trapping nil-probe
/// for an `Index { target, key }` source. Body originally lived
/// inline in the `IsNilQuery` arm of `emit_expr` (ADR 0061);
/// extracted here so the unified `IsNil(operand)` arm can call
/// it for the Index branch and inline the simpler Local branch.
/// Returns i1: `true` if the read would observe Nil (OOB,
/// missing key, or Nil-tagged slot), `false` otherwise.
#[allow(clippy::too_many_arguments)]
fn emit_isnil_index<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    target: &HirExpr,
    key: &HirExpr,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    let target_ptr = emit_expr(
        context,
        block,
        target,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
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
                context,
                block,
                key,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
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
            then_blk.append_operation(scf::r#yield(&[i1_true], loc));
            then_region.append_block(then_blk);
            let else_region = Region::new();
            let else_blk = Block::new(&[]);
            {
                let array_buf = emit_table_array_buf(context, &else_blk, target_ptr, types, loc);
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
        // Phase 2.6b-hash-keys (ADR 0079): the IsNil hash-path
        // accepts every non-Number hash-eligible kind. Build a
        // tagged search slot and probe for the bucket; missing
        // keys fall through to the i1_true (= is nil) yield.
        ValueKind::String | ValueKind::Bool | ValueKind::Function(_) | ValueKind::Table => {
            let key_value = emit_expr(
                context,
                block,
                key,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
            )?;
            let key_slot =
                emit_build_search_key_slot(context, block, key_kind, key_value, types, loc);
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
            then_blk.append_operation(scf::r#yield(&[i1_true], loc));
            then_region.append_block(then_blk);
            let else_region = Region::new();
            let else_blk = Block::new(&[]);
            {
                let cap_slot =
                    emit_byte_offset_ptr(context, &else_blk, hash_buf, HASH_OFF_CAP, types, loc);
                let cap = emit_load(&else_blk, cap_slot, types.i64, loc);
                let bucket = emit_hash_probe_for_insert(
                    context, &else_blk, hash_buf, cap, key_slot, types, loc,
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
                let inner_if = scf::r#if(key_at_null, &[types.i1], inner_then, inner_else, loc);
                let inner_result: Value<'c, '_> = else_blk
                    .append_operation(inner_if)
                    .result(0)
                    .unwrap()
                    .into();
                else_blk.append_operation(scf::r#yield(&[inner_result], loc));
            }
            else_region.append_block(else_blk);
            let if_op = scf::r#if(hash_buf_null, &[types.i1], then_region, else_region, loc);
            Ok(block.append_operation(if_op).result(0).unwrap().into())
        }
        _ => unreachable!("IsNil Index key must be Number or String"),
    }
}

/// Phase 2.6c-tag-hetero-fix (ADR 0065) + ADR 0066:
/// `TaggedValue` operand `==` / `~=` runtime tag dispatch. When
/// at least one side is `Local(TaggedValue)`, both-tagged routes
/// to `emit_tagged_eq_local_local` (ADR 0066) and one-tagged-
/// plus-Number/Bool/String typed operand emits a tag check +
/// per-kind compare (ADR 0065). Returns `Some(i1)` when the
/// dispatch fires, `None` to fall through to the existing
/// BinOp path.
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<Option<Value<'c, 'a>>, CodegenError> {
    // ADR 0089 extracted `tagged_local_idx` to module scope so the
    // arith dispatcher can reuse it; we use that shared helper here.
    // Identify which side(s) carry the tagged local.
    let (tagged_id, typed_expr, typed_kind) =
        match (tagged_local_idx(lhs, locals), tagged_local_idx(rhs, locals)) {
            (Some(lhs_id), Some(rhs_id)) => {
                // Phase 2.6c-tag-hetero-eq (ADR 0066): both sides
                // are TaggedValue locals — runtime tag-vs-tag
                // dispatch.
                let eq = emit_tagged_eq_local_local(
                    context,
                    block,
                    slots[lhs_id.0],
                    slots[rhs_id.0],
                    types,
                    loc,
                );
                let final_val = match op {
                    BinOp::Eq => eq,
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
                            .append_operation(arith::xori(eq, i1_one, loc))
                            .result(0)
                            .unwrap()
                            .into()
                    }
                    _ => unreachable!(),
                };
                return Ok(Some(final_val));
            }
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
        context,
        block,
        typed_expr,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
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

/// Phase 2.7p-tagged-arith-coerce (ADR 0089): TaggedValue arith /
/// bitwise BinOp runtime tag dispatch. When op is in the eligible
/// numeric-consumer class **and** at least one side is
/// `Local(TaggedValue)`, route the tagged operand(s) through
/// `emit_load_tagged_operand_as_number` (the chokepoint that
/// dispatches per `policy_for_tagged_arith_operand`) and feed the
/// resulting f64s to `emit_binop`. Returns `Some(f64)` when the
/// dispatch fires, `None` to fall through to the existing BinOp
/// path. Mirrors `emit_tagged_eq_runtime_dispatch` (ADR 0066) in
/// shape and call-site contract.
///
/// Out of scope (intentional):
/// - `Eq / Ne` — already handled by `emit_tagged_eq_runtime_dispatch`.
/// - `Lt / Le / Gt / Ge` — Lua spec §3.4.4: mixed-kind ordering is
///   an error, not a coercion. Existing trap behavior is correct.
/// - `Concat (..)` — already auto-coerces via `tostring`.
///
/// Static String operands (whose HIR kind is `ValueKind::String`)
/// are wrapped by `coerce_arith_operand_if_string` (ADR 0077) into
/// `HirExprKind::ArithStringCoerce`; those reach the dispatcher
/// here as Number-kind expressions and pass through `emit_expr`
/// unchanged. ADR 0089 only adds the runtime path for TaggedValue
/// operands that HIR cannot statically resolve.
#[allow(clippy::too_many_arguments)]
fn emit_tagged_arith_runtime_dispatch<'a, 'c>(
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<Option<Value<'c, 'a>>, CodegenError> {
    if !is_tagged_arith_eligible_op(op) {
        return Ok(None);
    }
    let lhs_tagged = tagged_local_idx(lhs, locals);
    let rhs_tagged = tagged_local_idx(rhs, locals);
    if lhs_tagged.is_none() && rhs_tagged.is_none() {
        return Ok(None);
    }
    let lhs_val = match lhs_tagged {
        Some(LocalId(idx)) => {
            emit_load_tagged_operand_as_number(context, block, slots[idx], types, loc)
        }
        None => emit_expr(
            context,
            block,
            lhs,
            slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        )?,
    };
    let rhs_val = match rhs_tagged {
        Some(LocalId(idx)) => {
            emit_load_tagged_operand_as_number(context, block, slots[idx], types, loc)
        }
        None => emit_expr(
            context,
            block,
            rhs,
            slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        )?,
    };
    Ok(Some(emit_binop(
        context, block, op, lhs_val, rhs_val, types, loc,
    )?))
}

/// Phase 2.7p-tagged-arith-coerce (ADR 0089): the BinOp set whose
/// TaggedValue operands route through the arith dispatch chokepoint.
/// 12 ops total: 7 arith + 5 bitwise. Ordering / Eq-Ne / Concat are
/// excluded (separate handlers; see `emit_tagged_arith_runtime_dispatch`
/// doc-comment for rationale).
fn is_tagged_arith_eligible_op(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Add
            | BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::Mod
            | BinOp::Pow
            | BinOp::FloorDiv
            | BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::Shl
            | BinOp::Shr
    )
}

/// Phase 2.7p-tagged-arith-coerce (ADR 0089): identifier helper.
/// Returns `Some(LocalId)` if `e` is a `Local(TaggedValue)` —
/// the only HIR shape that needs the runtime tag-dispatch route.
fn tagged_local_idx(e: &HirExpr, locals: &[LocalInfo]) -> Option<LocalId> {
    if let HirExprKind::Local(LocalId(idx)) = &e.kind
        && matches!(locals[*idx].kind, ValueKind::TaggedValue)
    {
        return Some(LocalId(*idx));
    }
    None
}

/// Phase 2.6c-tag-consumers (ADR 0067): runtime tag dispatch for
/// `tostring(Local(TaggedValue))`. Reads the slot's tag and
/// yields a `ptr` to the appropriate string representation:
/// Number is materialised via `emit_tostring(... Number)` (a
/// snprintf into a fresh buffer), Bool selects the static
/// `s_true` / `s_false`, String returns the payload pointer
/// directly, and Nil returns `s_nil`.
fn emit_tostring_tagged_local<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    slot_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let (tag, value_ptr) = emit_tag_and_payload_ptr(context, block, slot_ptr, types, loc);
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
    let nil_str = emit_addressof(context, &nil_then_blk, "s_nil", types, loc);
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
        {
            let f = emit_load(&num_then_blk, value_ptr, types.f64, loc);
            let formatted = emit_tostring(context, &num_then_blk, f, ValueKind::Number, types, loc);
            num_then_blk.append_operation(scf::r#yield(&[formatted], loc));
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
                let zero_i64 = make_const_i64(&bool_then_blk, 0);
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
                bool_then_blk.append_operation(scf::r#yield(&[chosen], loc));
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
                        tag,
                        tag_string,
                        loc,
                    ))
                    .result(0)
                    .unwrap()
                    .into();
                let str_then = Region::new();
                let str_then_blk = Block::new(&[]);
                {
                    let payload_ptr = emit_load(&str_then_blk, value_ptr, types.ptr, loc);
                    str_then_blk.append_operation(scf::r#yield(&[payload_ptr], loc));
                }
                str_then.append_block(str_then_blk);
                let str_else = Region::new();
                let str_else_blk = Block::new(&[]);
                {
                    // Phase 2.6c-tag-fn-tbl (ADR 0071): Function
                    // and Table tagged values yield the literal
                    // typename string (mirrors `tostring(<fn>)`
                    // / `tostring(<tbl>)` from ADR 0026 / 0052).
                    // Address-prefixed forms ("function: 0x..")
                    // are out of scope.
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
                        let placeholder = emit_addressof(
                            context,
                            &tbl_else_blk,
                            "s_typename_function",
                            types,
                            loc,
                        );
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<Value<'c, 'a>, CodegenError> {
    let kind = infer_kind(lhs, locals, functions);
    let result_ty = kind_to_mlir_type(kind, types);

    let lhs_val = emit_expr(
        context,
        parent,
        lhs,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
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
        in_function_cell_ptr,
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
        in_function_cell_ptr,
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
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
            context,
            &blk,
            rhs,
            blk_slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    let cond_kind = infer_kind(cond, locals, functions);
    let cond_val = emit_expr(
        context,
        parent,
        cond,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;
    let cond_i1 = emit_truthiness(context, parent, cond_val, cond_kind, types, loc);

    let then_region = build_body_region(
        context,
        then_body,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;
    let else_region = build_else_region(
        context,
        elifs,
        else_body,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;

    let if_op = scf::r#if(cond_i1, &[], then_region, else_region, loc);
    parent.append_operation(if_op);
    Ok(())
}

/// Build the `else` region for an If, recursing on the elseif chain.
#[allow(clippy::too_many_arguments)]
fn build_else_region<'a, 'c>(
    context: &'c Context,
    elifs: &[(HirExpr, Vec<HirStmt>)],
    else_body: &Option<Vec<HirStmt>>,
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
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
            context,
            &blk,
            cond,
            blk_slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        )?;
        let cond_i1 = emit_truthiness(context, &blk, cond_val, cond_kind, types, loc);
        let nested_then = build_body_region(
            context,
            body,
            blk_slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        )?;
        let nested_else = build_else_region(
            context,
            rest,
            else_body,
            blk_slots,
            locals,
            functions,
            types,
            params_len,
            in_function_cell_ptr,
            loc,
        )?;
        blk.append_operation(scf::r#if(cond_i1, &[], nested_then, nested_else, loc));
        blk.append_operation(scf::r#yield(&[], loc));
        region.append_block(blk);
        Ok(region)
    } else {
        // Leaf: either else_body or empty `scf.yield` for absent else.
        match else_body {
            Some(body) => build_body_region(
                context,
                body,
                slots,
                locals,
                functions,
                types,
                params_len,
                in_function_cell_ptr,
                loc,
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
fn build_body_region<'a, 'c>(
    context: &'c Context,
    body: &[HirStmt],
    slots: &[Value<'c, 'a>],
    locals: &[LocalInfo],
    functions: &[HirFunction],
    types: &Types<'c>,
    params_len: usize,
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<Region<'c>, CodegenError> {
    let region = Region::new();
    let blk = Block::new(&[]);
    let blk_slots = transmute_slots(slots);
    emit_stmts(
        context,
        &blk,
        body,
        blk_slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
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
        in_function_cell_ptr,
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
        context,
        body,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
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
        in_function_cell_ptr,
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
        in_function_cell_ptr,
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
    in_function_cell_ptr: Option<Value<'c, 'a>>,
    loc: Location<'c>,
) -> Result<(), CodegenError> {
    // Evaluate start/stop/step once in the parent block, store into
    // dedicated slots (stop/step) and the loop variable's own slot.
    let start_val = emit_expr(
        context,
        parent,
        start,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;
    let stop_val = emit_expr(
        context,
        parent,
        stop,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
    )?;
    let step_val = emit_expr(
        context,
        parent,
        step,
        slots,
        locals,
        functions,
        types,
        params_len,
        in_function_cell_ptr,
        loc,
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
        in_function_cell_ptr,
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

    /// Phase 2.6c-tag-locals-fn (ADR 0074 prep, Tidy First):
    /// shape-level assertion that an MLIR dump contains a given
    /// substring. Use this for ABI-shape checks (function
    /// signatures, call_indirect signatures, return tuples) so
    /// the failure message labels the missing shape clearly.
    fn assert_mlir_has(mlir: &str, expected: &str) {
        assert!(
            mlir.contains(expected),
            "expected MLIR to contain `{expected}`, got:\n{mlir}",
        );
    }

    // ============================================================
    // ADR 0074 (function-return TaggedValue widening) ABI shape
    // tests. Pin the `(i64, i64)` return signature so future
    // refactors (skeleton extraction, arity hardening) can't
    // silently break the multi-result widening contract.
    // ============================================================

    #[test]
    fn emit_function_with_heterogeneous_return_uses_tagged_abi() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function maybe(x) if x > 0 then return 42 end return nil end
print(maybe(1))",
            ),
        )
        .expect("widened function must verify");
        let mlir = module.as_operation().to_string();
        // Phase 2.5c-full Commit 2a (ADR 0083): multi-return is
        // wrapped in `!llvm.struct<(...)>`. Phase 2.5c-full Commit
        // 3b body atomic adds the cell-ptr first arg, so the
        // signature is `(!llvm.ptr, f64) -> !llvm.struct<(i64,
        // i64)>` for a Number-param widened-return user fn.
        assert_mlir_has(&mlir, "(!llvm.ptr, f64) -> !llvm.struct<(i64, i64)>");
    }

    #[test]
    fn emit_pure_number_function_keeps_single_f64_return() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function double(x) return x * 2 end
print(double(21))",
            ),
        )
        .expect("pure-number module must verify");
        let mlir = module.as_operation().to_string();
        // Phase 2.5c-full Commit 3b body atomic adds the cell-ptr
        // first arg, so the signature is `(!llvm.ptr, f64) -> f64`.
        assert_mlir_has(&mlir, "(!llvm.ptr, f64) -> f64");
        assert!(
            !mlir.contains("(i64, i64)"),
            "pure-Number return must not produce TaggedValue ABI, got:\n{mlir}"
        );
    }

    #[test]
    fn emit_function_with_multi_position_tagged_return_uses_4_i64_results() {
        // ADR 0076: two return positions both widening to TaggedValue
        // flatten to 4 i64 results (2 per logical position).
        // Phase 2.5c-full Commit 2a (ADR 0083): the 4-i64 result
        // surfaces as `!llvm.struct<(i64, i64, i64, i64)>` once
        // user fns moved to the LLVM dialect.
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function pick(x) if x > 0 then return 1, nil end return nil, 1 end
local a, b = pick(1)
print(type(a), type(b))",
            ),
        )
        .expect("multi-position widened module must verify");
        let mlir = module.as_operation().to_string();
        // Phase 2.5c-full Commit 3b body atomic adds the cell-ptr
        // first arg.
        assert_mlir_has(
            &mlir,
            "(!llvm.ptr, f64) -> !llvm.struct<(i64, i64, i64, i64)>",
        );
    }

    #[test]
    fn emit_call_to_widened_function_yields_two_results() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function maybe(x) if x > 0 then return 1 end return nil end
local v = maybe(7)
print(v)",
            ),
        )
        .expect("widened-call module must verify");
        let mlir = module.as_operation().to_string();
        // Phase 2.5c-full Commit 3b body atomic: widened ret ABI is
        // `(!llvm.ptr, f64) -> !llvm.struct<(i64, i64)>`; the
        // caller emits ≥2 stores into the destination tagged slot
        // (tag + payload_raw via `extractvalue` + `llvm.store`).
        assert_mlir_has(&mlir, "(!llvm.ptr, f64) -> !llvm.struct<(i64, i64)>");
        let store_count = mlir.matches("llvm.store").count();
        assert!(
            store_count >= 2,
            "expected >= 2 llvm.store after widened call (tag + payload), got {store_count}:\n{mlir}",
        );
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
        // Phase 2.5c-full Commit 2a (ADR 0083): Function-kind
        // params lower to `!llvm.ptr` (LLVM dialect's
        // first-class function-pointer representation). Phase
        // 2.5c-full Commit 3b body atomic shifts every Lua param
        // by +1 since `%arg0` is the closure cell ptr; `apply`'s
        // Function-kind `g` becomes `%arg1` and the f64 `x` is
        // `%arg2`.
        assert_mlir_has(
            &mlir,
            "@user_apply_0(%arg0: !llvm.ptr, %arg1: !llvm.ptr, %arg2: f64)",
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
        // Phase 2.5c-full Commit 2a (ADR 0083): indirect callsite
        // is now `llvm.call` operand-callee form (no `_indirect`
        // suffix in LLVM dialect). The anon symbol confirms the
        // function-pointer source is still wired up — `user_anon_1`
        // because `apply` claims index 0 and the anon function
        // takes index 1 in declaration order.
        assert_mlir_has(&mlir, "llvm.call");
        assert_mlir_has(&mlir, "@user_anon_1");
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
            mlir.contains("llvm.func @user_anon_0"),
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
            mlir.contains("llvm.func @user_f_0"),
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
        // Phase 2.5c-full Commit 2a (ADR 0083): the gd call lowers
        // to `llvm.call @user_gd_*`, and the subsequent `f(5)` —
        // a `Callee::Indirect` through a Function-kind local — uses
        // operand-callee `llvm.call %fn_ptr(...)` (no `_indirect`
        // suffix in the LLVM dialect).
        assert!(
            mlir.contains("llvm.call @user_gd_"),
            "expected `llvm.call @user_gd_*`, got:\n{mlir}"
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

    // ============================================================
    // ADR 0083 Commit 2b — closure cell singleton + producer /
    // consumer flip. These tests pin the IR shape so future
    // refactors can't silently regress the cutover.
    // ============================================================

    #[test]
    fn emit_user_fn_emits_closure_singleton_global() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("local function f() return 1 end\nprint(f())"),
        )
        .expect("closure singleton module must verify");
        let mlir = module.as_operation().to_string();
        // Each user fn `@user_<name>_<idx>` gets a static
        // `@user_<name>_<idx>_closure` cell whose initializer
        // packs `addressof @user_<name>_<idx>` + i64 0
        // (upvalue_count) into `!llvm.struct<(ptr, i64)>`.
        assert_mlir_has(&mlir, "llvm.mlir.global");
        assert_mlir_has(&mlir, "@user_f_0_closure");
    }

    #[test]
    fn emit_function_ref_routes_through_closure_singleton() {
        // `local g = d` is optimised by HIR to skip storage (alias
        // copy of `func_id`), so to actually materialise the
        // Function-kind value we pass it as an argument. The
        // `apply(d, 5)` site triggers the producer flip via the
        // known-FuncId Local path in `HirExprKind::Local`.
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function d(x) return x*2 end\nlocal function apply(g, x) return g(x) end\nprint(apply(d, 5))",
            ),
        )
        .expect("apply-pattern module must verify");
        let mlir = module.as_operation().to_string();
        // Producer side: when a Function-kind value is materialised
        // (FunctionRef / known-FuncId Local), the SSA value is the
        // address of the closure singleton, not the raw fn ptr.
        assert_mlir_has(&mlir, "addressof @user_d_0_closure");
    }

    #[test]
    fn emit_indirect_dispatch_loads_cell_fn_ptr() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function d(x) return x*2 end\nlocal function apply(g, x) return g(x) end\nprint(apply(d, 7))",
            ),
        )
        .expect("indirect dispatch module must verify");
        let mlir = module.as_operation().to_string();
        // Consumer side: the `Callee::Indirect` arm dereferences the
        // cell ptr's first slot (offset 0 = fn_ptr) before issuing
        // the actual `llvm.call`. We look for a `getelementptr` +
        // `llvm.load !llvm.ptr` pattern preceding the indirect call;
        // the i8 byte-offset GEP pattern is what `emit_byte_offset_ptr`
        // produces.
        assert_mlir_has(&mlir, "llvm.getelementptr");
        assert_mlir_has(&mlir, "llvm.load");
        // Producer side must also have flipped to the singleton.
        assert_mlir_has(&mlir, "@user_d_0_closure");
    }

    // ============================================================
    // ADR 0083 Commit 3b body atomic — cell-ptr-first ABI for all
    // user fns. These tests pin the most critical ABI invariants
    // so future refactors (or the cutover commit itself) can't
    // silently regress the cell-ptr threading. Codex review of
    // Commit 3a / 3b prep flagged that the atomic cutover touches
    // 4 direct-user-call paths plus 2 producer arms plus the
    // outer-scope storage rule — verifier-passing wrong-code is
    // plausible without these shape pins.
    // ============================================================

    /// Step 0 / Test #1: every user `llvm.func` accepts a `!llvm.ptr`
    /// closure cell ptr as its first argument. Lua params shift to
    /// args 1..N.
    #[test]
    fn emit_user_fn_signature_starts_with_cell_ptr() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src("local function f(x) return x * 2 end\nprint(f(7))"),
        )
        .expect("module must verify");
        let mlir = module.as_operation().to_string();
        // The function body's first block argument is the closure
        // cell ptr; the original Lua `x` parameter shifts to %arg1.
        assert_mlir_has(&mlir, "llvm.func @user_f_0(%arg0: !llvm.ptr, %arg1: f64)");
    }

    /// Step 0 / Test #2: self-recursion inside a capturing fn
    /// passes the entry `block.argument(0)` (the caller-supplied
    /// cell ptr = the current self instance) as the first arg of
    /// the recursive `llvm.call`.
    #[test]
    fn emit_self_recursion_uses_entry_cell_ptr_as_first_arg() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local x = 1
local function fact(n) if n == 0 then return x end
return fact(n - 1) * n end
print(fact(3))",
            ),
        )
        .expect("module must verify");
        let mlir = module.as_operation().to_string();
        // Inside fact's body, the self-call `fact(n-1)` must thread
        // the entry cell ptr (block.argument(0), printed as %arg0)
        // as the first call operand. The pretty-printer lays this
        // out as `llvm.call @user_fact_0(%arg0, ...)`.
        assert_mlir_has(&mlir, "llvm.call @user_fact_0(%arg0,");
    }

    /// Step 0 / Test #3: a forward / nested capturing call resolves
    /// through the synthetic FunctionDef-local's slot — the cell
    /// ptr is `llvm.load`'d from the slot before the call. This
    /// path covers Test B from Codex review (`outer { local function
    /// inner ... ; inner() }`).
    #[test]
    fn emit_nested_forward_capturing_call_loads_cell_from_slot() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function outer(x)
  local function inner() return x + 100 end
  return inner()
end
print(outer(5))",
            ),
        )
        .expect("module must verify");
        let mlir = module.as_operation().to_string();
        // Inside outer's body, the synthetic `inner` local holds
        // the freshly malloc'd capturing cell. `inner()` must load
        // the cell ptr from the slot and pass it as first arg to
        // `llvm.call @user_inner_1(...)`. We pin the malloc + the
        // direct call symbol; the slot-load gating the call shows
        // up as `llvm.load ... -> !llvm.ptr` immediately above the
        // call.
        assert_mlir_has(&mlir, "llvm.call @malloc");
        assert_mlir_has(&mlir, "llvm.call @user_inner_1");
    }

    /// Step 0 / Test #4: aliasing a non-capturing fn (`local g = f`)
    /// stores the singleton cell ptr into g's slot, and `g()`
    /// loads from g's slot rather than referencing the singleton
    /// directly. This pin guards against regressing the
    /// `HirExprKind::Local` known-FuncId arm — Codex P1 flagged
    /// that the arm must branch on `target.upvalues.is_empty()`.
    /// For non-capturing alias the singleton is loaded from g's
    /// slot at the call site; the slot was populated by the
    /// LocalInit storage rule.
    #[test]
    fn emit_alias_call_routes_through_synthetic_local_slot() {
        let ctx = new_context();
        let module = emit_module(
            &ctx,
            &lower_src(
                "local function f(x) return x * 2 end
local g = f
print(g(5))",
            ),
        )
        .expect("module must verify");
        let mlir = module.as_operation().to_string();
        // f's singleton must be referenced (producer side) and the
        // call site g(5) must use the cell-ptr-first ABI shape
        // `(!llvm.ptr, f64) -> f64`.
        assert_mlir_has(&mlir, "addressof @user_f_0_closure");
        assert_mlir_has(&mlir, "(!llvm.ptr, f64) -> f64");
    }
}
