//! Phase 2.5c-full (ADR 0083) closure ABI helpers — closure cell
//! and upvalue box layout, allocation, and field access.
//!
//! ## Closure cell layout (16 + 8*N bytes; either heap or static global)
//!
//! ```text
//! offset 0  : !llvm.ptr fn_ptr           ← function code address (mangled @user_fn_NN_code)
//! offset 8  : i64 upvalue_count          ← N (0 for non-capturing fns)
//! offset 16 : !llvm.ptr upvalue_box_0    ← shared box pointer
//! offset 24 : !llvm.ptr upvalue_box_1
//! ...
//! offset 16 + N*8
//! ```
//!
//! Non-capturing functions get a static `llvm.mlir.global` cell
//! emitted once per module (`@user_fn_NN_closure`). Capturing
//! closures `malloc` a fresh cell at the closure-creation site.
//!
//! MLIR feasibility verified 2026-05-07 (see
//! `docs/notes/closure-feasibility.md`): the static cell uses
//! `llvm.mlir.addressof @user_fn_NN` inside an
//! `llvm.mlir.global` initializer region. This requires user
//! functions to be emitted as `llvm.func` (not `func.func`),
//! since `llvm.mlir.addressof` rejects `func.func` symbols.
//! Commit 2a (2026-05-07) landed that migration in `emit.rs`;
//! Commit 2b is the TAG_FUNCTION semantic cutover that
//! materialises the static `@user_fn_NN_closure` cells described
//! below and routes producer / consumer paths through them.
//!
//! ## Upvalue box layout (8 bytes; heap)
//!
//! ```text
//! offset 0  : i64 storage                ← raw 8-byte slot, kind-typed by static analysis
//! ```
//!
//! Number values are bitcast i64 ↔ f64; Bool/Function/Table/String
//! payloads round-trip as raw i64 (matching the tagged-slot
//! convention in `tagged.rs`). TaggedValue upvalues are deferred
//! (Codex pre-ADR-0083 review §5).
//!
//! ## ABI contract (Commit 2b landed)
//!
//! `TAG_FUNCTION` payload (8 bytes) is a closure cell pointer —
//! never a raw fn ptr. The producer / consumer paths flipped
//! atomically in Commit 2b:
//!
//! - Producers (`HirExprKind::FunctionRef`, known-FuncId
//!   `Local`) materialise the cell ptr via
//!   `addressof @user_fn_NN_closure`. The dispatch-chain
//!   candidate side stays at raw `addressof @user_fn_NN` because
//!   consumers normalise to fn_ptr before comparison (avoids a
//!   second addressof per candidate).
//! - Consumers (`emit_indirect_dispatch_chain_in_block`,
//!   `Callee::Indirect`) load `cell.fn_ptr` (offset 0) via
//!   [`emit_load_closure_fn_ptr`] before issuing the actual
//!   `llvm.call`. The `tagged.rs` Function arms still operate
//!   on raw payload ptrs — print/tostring yield "function" /
//!   "table" typename strings (no address printed) and tag-eq
//!   compares ptrs which, for non-capturing singletons, retain
//!   identity equality automatically.
//!
//! Capturing function ABIs will accept upvalue box pointers as
//! trailing parameters (`(lua_params..., uv_box_0,
//! uv_box_1, ...)`); that surface arrives with Commit 3.
//!
//! Closure equality (`f == g`) compares cell pointers — for
//! non-capturing closures the static `@user_fn_NN_closure`
//! singleton makes `f == g` (alias) true. Capturing closures
//! malloc per creation, so two factory products differ (Lua
//! spec §3.4.4).
//!
//! ## Module structure
//!
//! Phase 2.5c-full (ADR 0083) Tidy First commit shipped the layout
//! constants and helper stubs. Commit 2a (2026-05-07) migrated
//! `emit_function` to the LLVM dialect; Commit 2a-fix
//! (`c81f16b`, Codex review follow-up) closed a wrong-code gap
//! exposed by the `!llvm.ptr` Function-value erasure
//! (`Callee::Indirect`'s hardcoded f64 result type lost its
//! MLIR-verifier safety net) by HIR-rejecting non-Number return
//! functions flowing into Function-kind parameters (ADR 0075
//! amend). Commit 2b (2026-05-08) shipped this file's
//! [`emit_load_closure_fn_ptr`] / [`emit_store_closure_fn_ptr`]
//! and the new [`emit_user_fn_closure_global`], and flipped
//! TAG_FUNCTION producer / consumer paths to closure cell ptrs.
//! Commit 3 ships captured-local boxes plus the e2e suite that
//! resolves LIC-2.6c-tag-hetero-closure-escape-1.

use melior::{
    Context,
    dialect::{
        llvm,
        ods::llvm::{AddressOfOperationBuilder, GlobalOperationBuilder},
    },
    ir::{
        Block, BlockLike, Identifier, Location, Module, Region, RegionLike, Value,
        attribute::{FlatSymbolRefAttribute, IntegerAttribute, StringAttribute, TypeAttribute},
        operation::OperationBuilder,
    },
};

use crate::hir::ValueKind;

use super::callabi::emit_pack_struct;
use super::primitive::{Types, emit_byte_offset_ptr, emit_load, emit_store};

// ============================================================
// Layout constants — referenced from `closure.rs` helpers; ADR
// 0083 commits 2/3 wire callers from `emit.rs` and friends. The
// `#[allow(dead_code)]` annotations come off in commit 3.
// ============================================================

/// Phase 2.5c-full (ADR 0083): byte offset of the `fn_ptr` field
/// inside a closure cell. Stays at 0 for natural addressof.
#[allow(dead_code)]
pub(crate) const CLOSURE_OFF_FN_PTR: i64 = 0;

/// Phase 2.5c-full (ADR 0083): byte offset of the `upvalue_count`
/// field inside a closure cell. Static cells set this to 0;
/// capturing closures set this to the upvalue count from the
/// HIR `HirExprKind::ClosureAlloc::upvalue_box_sources.len()`.
#[allow(dead_code)]
pub(crate) const CLOSURE_OFF_UPVALUE_COUNT: i64 = 8;

/// Phase 2.5c-full (ADR 0083): byte offset of the first
/// upvalue-box pointer inside a closure cell. Subsequent boxes
/// follow at `CLOSURE_OFF_BOXES_BASE + i * 8`.
#[allow(dead_code)]
pub(crate) const CLOSURE_OFF_BOXES_BASE: i64 = 16;

/// Phase 2.5c-full (ADR 0083): cell size for a non-capturing
/// function. Capturing closures use `CLOSURE_OFF_BOXES_BASE +
/// upvalue_count * 8`.
#[allow(dead_code)]
pub(crate) const CLOSURE_NON_CAPTURING_SIZE: i64 = 16;

/// Phase 2.5c-full (ADR 0083): byte offset of the value slot
/// inside an upvalue box. Currently the box is exactly one i64.
#[allow(dead_code)]
pub(crate) const UPVALUE_BOX_OFF_VALUE: i64 = 0;

/// Phase 2.5c-full (ADR 0083): upvalue box size in bytes (one
/// i64 storage slot today; widens to a tagged slot if/when ADR
/// 0083's deferred TaggedValue-upvalue path lands).
#[allow(dead_code)]
pub(crate) const UPVALUE_BOX_SIZE: i64 = 8;

// ============================================================
// Closure cell helpers (Commit 2 will fill these in)
// ============================================================

/// Phase 2.5c-full (ADR 0083): allocate a heap closure cell of
/// size `CLOSURE_OFF_BOXES_BASE + upvalue_count * 8` via `malloc`.
/// Caller must populate `fn_ptr`, `upvalue_count`, and each
/// `upvalue_box` slot via the helpers below.
#[allow(dead_code)]
pub(crate) fn emit_allocate_closure_cell<'a, 'c>(
    _context: &'c Context,
    _block: &'a Block<'c>,
    _upvalue_count: usize,
    _types: &Types<'c>,
    _loc: Location<'c>,
) -> Value<'c, 'a> {
    unimplemented!("ADR 0083 commit 2 ships emit_allocate_closure_cell")
}

/// Phase 2.5c-full (ADR 0083) Commit 2b: load `cell.fn_ptr` (offset
/// 0) from a closure cell pointer. Used by ADR 0082's dispatch chain
/// to extract the raw function-code address before comparing
/// against candidate `addressof @user_fn_NN` ptrs, and by
/// `Callee::Indirect` to derive the actual call target.
#[allow(dead_code)]
pub(crate) fn emit_load_closure_fn_ptr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    cell_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let fn_ptr_slot =
        emit_byte_offset_ptr(context, block, cell_ptr, CLOSURE_OFF_FN_PTR, types, loc);
    emit_load(block, fn_ptr_slot, types.ptr, loc)
}

/// Phase 2.5c-full (ADR 0083) Commit 2b: store `fn_ptr` into a closure
/// cell's `fn_ptr` field (offset 0). Called once per closure
/// allocation; non-capturing static cells emit this through the
/// `llvm.mlir.global` initializer region rather than through this
/// helper, so today only capturing closures (Commit 3) reach it.
#[allow(dead_code)]
pub(crate) fn emit_store_closure_fn_ptr<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    cell_ptr: Value<'c, 'a>,
    fn_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let fn_ptr_slot =
        emit_byte_offset_ptr(context, block, cell_ptr, CLOSURE_OFF_FN_PTR, types, loc);
    emit_store(block, fn_ptr, fn_ptr_slot, loc);
}

/// Phase 2.5c-full (ADR 0083): load the i-th upvalue box pointer
/// from a closure cell. Used by `emit_dispatch_chain_recursive`'s
/// match-then branch to forward upvalue boxes to the direct call.
#[allow(dead_code)]
pub(crate) fn emit_load_closure_upvalue_box<'a, 'c>(
    _context: &'c Context,
    _block: &'a Block<'c>,
    _cell_ptr: Value<'c, 'a>,
    _index: usize,
    _types: &Types<'c>,
    _loc: Location<'c>,
) -> Value<'c, 'a> {
    unimplemented!("ADR 0083 commit 3 ships emit_load_closure_upvalue_box")
}

/// Phase 2.5c-full (ADR 0083): store an upvalue box pointer at
/// the i-th slot of a closure cell. Called for every captured
/// upvalue at closure creation time.
#[allow(dead_code)]
pub(crate) fn emit_store_closure_upvalue_box<'a, 'c>(
    _context: &'c Context,
    _block: &'a Block<'c>,
    _cell_ptr: Value<'c, 'a>,
    _index: usize,
    _box_ptr: Value<'c, 'a>,
    _types: &Types<'c>,
    _loc: Location<'c>,
) {
    unimplemented!("ADR 0083 commit 3 ships emit_store_closure_upvalue_box")
}

// ============================================================
// Upvalue box helpers (Commit 3 will fill these in)
// ============================================================

/// Phase 2.5c-full (ADR 0083): allocate a heap upvalue box via
/// `malloc(UPVALUE_BOX_SIZE)`. Captured-local LocalInit emits
/// this once per scope-entry of the captured local.
#[allow(dead_code)]
pub(crate) fn emit_allocate_upvalue_box<'a, 'c>(
    _context: &'c Context,
    _block: &'a Block<'c>,
    _types: &Types<'c>,
    _loc: Location<'c>,
) -> Value<'c, 'a> {
    unimplemented!("ADR 0083 commit 3 ships emit_allocate_upvalue_box")
}

/// Phase 2.5c-full (ADR 0083): load the value stored in an upvalue
/// box, kind-typed by the captured local's static `ValueKind`.
/// Number is f64 via bitcast i64 → f64; ptr kinds are loaded as
/// `!llvm.ptr` directly; Bool is i1 via i64 → i1 truncation.
#[allow(dead_code)]
pub(crate) fn emit_load_upvalue_box_value<'a, 'c>(
    _context: &'c Context,
    _block: &'a Block<'c>,
    _box_ptr: Value<'c, 'a>,
    _kind: ValueKind,
    _types: &Types<'c>,
    _loc: Location<'c>,
) -> Value<'c, 'a> {
    unimplemented!("ADR 0083 commit 3 ships emit_load_upvalue_box_value")
}

/// Phase 2.5c-full (ADR 0083): store a value into an upvalue
/// box, kind-typed mirror of [`emit_load_upvalue_box_value`].
#[allow(dead_code)]
pub(crate) fn emit_store_upvalue_box_value<'a, 'c>(
    _context: &'c Context,
    _block: &'a Block<'c>,
    _box_ptr: Value<'c, 'a>,
    _value: Value<'c, 'a>,
    _kind: ValueKind,
    _types: &Types<'c>,
    _loc: Location<'c>,
) {
    unimplemented!("ADR 0083 commit 3 ships emit_store_upvalue_box_value")
}

// ============================================================
// Static singleton emit (Commit 2b)
// ============================================================

/// Phase 2.5c-full (ADR 0083) Commit 2b: emit a per-user-fn static
/// singleton `@<fn_sym>_closure` of type `!llvm.struct<(ptr, i64)>`,
/// initialised to `{ addressof @<fn_sym>, i64 0 }` (upvalue_count
/// = 0, the non-capturing case). The cell pointer becomes the
/// observable identity of the function value: producers
/// (`HirExprKind::FunctionRef`, known-FuncId Local) materialise it
/// via `addressof @<fn_sym>_closure`, while consumers
/// (ADR 0082 dispatch chain, `Callee::Indirect`, the
/// `tagged.rs` Function arms) load `cell.fn_ptr` (offset 0) before
/// issuing the actual call or comparison.
///
/// Singleton property: one global per `HirFunction`. Aliasing a
/// function value (`local g = f`) just copies the cell pointer, so
/// `f == g` is true via raw pointer compare — matching Lua spec
/// §3.4.4 for non-capturing functions. Capturing closures (Commit
/// 3) malloc a fresh cell per creation, so factory products differ
/// as required.
pub(crate) fn emit_user_fn_closure_global<'c>(
    context: &'c Context,
    module: &Module<'c>,
    fn_sym: &str,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let cell_struct = llvm::r#type::r#struct(context, &[types.ptr, types.i64], false);
    let init_region = Region::new();
    let init_blk = Block::new(&[]);
    let fn_addr: Value<'c, '_> = init_blk
        .append_operation(
            AddressOfOperationBuilder::new(context, loc)
                .res(types.ptr)
                .global_name(FlatSymbolRefAttribute::new(context, fn_sym))
                .build()
                .into(),
        )
        .result(0)
        .unwrap()
        .into();
    let zero_count: Value<'c, '_> = init_blk
        .append_operation(
            OperationBuilder::new("llvm.mlir.constant", loc)
                .add_attributes(&[(
                    Identifier::new(context, "value"),
                    IntegerAttribute::new(types.i64, 0).into(),
                )])
                .add_results(&[types.i64])
                .build()
                .expect("llvm.mlir.constant"),
        )
        .result(0)
        .unwrap()
        .into();
    let packed = emit_pack_struct(context, &init_blk, cell_struct, &[fn_addr, zero_count], loc);
    init_blk.append_operation(
        OperationBuilder::new("llvm.return", loc)
            .add_operands(&[packed])
            .build()
            .expect("llvm.return in closure singleton initializer"),
    );
    init_region.append_block(init_blk);

    let closure_sym = format!("{fn_sym}_closure");
    let global_op = GlobalOperationBuilder::new(context, loc)
        .initializer(init_region)
        .global_type(TypeAttribute::new(cell_struct))
        .sym_name(StringAttribute::new(context, &closure_sym))
        .linkage(llvm::attributes::linkage(
            context,
            llvm::attributes::Linkage::Internal,
        ))
        .constant(melior::ir::attribute::Attribute::unit(context))
        .build();
    module.body().append_operation(global_op.into());
}
