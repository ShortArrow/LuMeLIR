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
//! ## ABI contract
//!
//! `TAG_FUNCTION` payload (8 bytes) is now a closure cell pointer
//! — never a raw fn ptr. Every function ABI accepts upvalue box
//! pointers as trailing parameters: signature is
//! `(lua_params... , uv_box_0, uv_box_1, ...)`. ADR 0082's
//! `emit_dispatch_chain_recursive` therefore loads
//! `cell.fn_ptr` *before* comparing against candidates'
//! `func.constant @user_fn_X` addresses, then forwards
//! `(args, [load(cell, 16+i*8) for i in 0..upvalue_count])`
//! to the matched direct call.
//!
//! Closure equality (`f == g`) compares cell pointers — same
//! closure aliased through multiple locals is equal; two
//! instances from the same factory are not (Lua spec).
//!
//! ## Module structure
//!
//! Phase 2.5c-full (ADR 0083) Tidy First commit ships only the
//! layout constants and helper stubs. Commit 2a (2026-05-07)
//! migrated `emit_function` to the LLVM dialect without using
//! these helpers; Commit 2b will fill them in for the
//! TAG_FUNCTION cutover, and Commit 3 ships captured-local boxes
//! plus the e2e suite that resolves
//! LIC-2.6c-tag-hetero-closure-escape-1.

use melior::{
    Context,
    ir::{Block, Location, Value},
};

use crate::hir::ValueKind;

use super::primitive::Types;

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

/// Phase 2.5c-full (ADR 0083): load `cell.fn_ptr` from a closure
/// cell pointer. Used by ADR 0082's dispatch chain to compare
/// against candidate `func.constant @user_fn_X` addresses.
#[allow(dead_code)]
pub(crate) fn emit_load_closure_fn_ptr<'a, 'c>(
    _context: &'c Context,
    _block: &'a Block<'c>,
    _cell_ptr: Value<'c, 'a>,
    _types: &Types<'c>,
    _loc: Location<'c>,
) -> Value<'c, 'a> {
    unimplemented!("ADR 0083 commit 2 ships emit_load_closure_fn_ptr")
}

/// Phase 2.5c-full (ADR 0083): store `fn_ptr` into a closure
/// cell's `fn_ptr` field. Called once per closure allocation
/// (static or heap).
#[allow(dead_code)]
pub(crate) fn emit_store_closure_fn_ptr<'a, 'c>(
    _context: &'c Context,
    _block: &'a Block<'c>,
    _cell_ptr: Value<'c, 'a>,
    _fn_ptr: Value<'c, 'a>,
    _types: &Types<'c>,
    _loc: Location<'c>,
) {
    unimplemented!("ADR 0083 commit 2 ships emit_store_closure_fn_ptr")
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
