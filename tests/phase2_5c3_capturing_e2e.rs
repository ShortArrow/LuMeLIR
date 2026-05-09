//! Phase 2.5c-full Commit 3c (ADR 0083): end-to-end runtime
//! tests for capturing closures. These tests pin behaviour
//! that the cell-ptr-first ABI (3b body atomic) makes possible
//! and that 3c surfaces by relaxing the `HirError::ClosureEscapes`
//! reject set.
//!
//! Codex review TDD order:
//!   1. box_sharing — independent verification that 3b body's
//!      heap-box pre-allocation makes sibling closures share
//!      the same upvalue storage. No escape involved; if this
//!      passes, the 3b ABI is structurally sound before we open
//!      escape paths.
//!   2. make_adder — closure as return value (escape relax #1).
//!   3. closure_return — generic closure-returning factory.
//!   4. table_capture — closure stored in table value (escape
//!      relax #2).
//!   5. closure_identity — Lua §3.4.4 cell-ptr identity through
//!      alias and parameter passing.
//!   6. generic_for_capturing — generic-for iter that captures
//!      (drops `f.upvalues.is_empty()` filter in HIR).
//!   7. ir_shape_hardening — pin GEP +16 + store !llvm.ptr in
//!      the malloc'd cell (catches future ABI drift).

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    assert!(result.status.success(), "binary should exit 0");
    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
    let _ = std::fs::remove_file(&output);
    stdout
}

#[test]
fn make_adder_returns_closure_with_captured_upvalue() {
    // Classic make_adder factory. The inner closure captures
    // `n` from `make_adder`'s param scope. Heap-allocated boxes
    // (3b body Step 6) survive `make_adder`'s frame teardown,
    // so `add5(3)` reads through the cell's box ptr to the
    // box that was populated with `5`.
    //
    // Pre-3c this Reds at HIR (`ClosureEscapes` on return);
    // 3c removes that reject because the heap-box ABI makes
    // closure escape sound.
    let src = "local function make_adder(n)
  return function(x) return x + n end
end
local add5 = make_adder(5)
print(add5(3))";
    assert_eq!(run(src, "lumelir_25c3_make_adder").trim(), "8");
}

#[test]
fn closure_return_two_independent_instances_are_isolated() {
    // Two calls to make_counter must produce independent
    // counters. A unique heap cell per call (3b body Step 7's
    // per-call malloc) plus a unique heap upvalue box per
    // call (3b Step 6) make this work — without per-call
    // freshness, the two counters would share a box and
    // print 12 / 12.
    let src = "local function make_counter(start)
  local n = start
  return function() n = n + 1; return n end
end
local c1 = make_counter(10)
local c2 = make_counter(100)
c1()
c1()
c2()
print(c1())
print(c2())";
    let out = run(src, "lumelir_25c3_closure_return");
    assert_eq!(out.trim(), "13\n102");
}

#[test]
fn box_sharing_two_capturing_closures_share_upvalue_storage() {
    // Two `local function`s both capture `x` from the chunk
    // scope. `inc` writes through its captured box; `get` reads
    // through its own captured box. With 3b body's heap-box
    // pre-allocation (Step 6) and FunctionRef populating cells
    // from the outer slot (Step 7), both cells must hold the
    // *same* box ptr — so the writes are visible to the reader.
    //
    // This test only exercises direct-call paths and does not
    // require any 3c escape relax — it's a pure verification
    // that Commit 3b's ABI cutover already wired box sharing
    // correctly.
    let src = "local x = 10
local function inc() x = x + 1 end
local function get() return x end
inc()
inc()
print(get())";
    assert_eq!(run(src, "lumelir_25c3_box_sharing").trim(), "12");
}

#[test]
fn table_capture_closure_stored_in_array_dispatches_correctly() {
    // A capturing closure stored in a table-array slot must
    // round-trip through the tagged-slot dispatch chain
    // (loaded_ptr → cell.fn_ptr compare; cell ptr threaded
    // as in_function_cell_ptr). The captured `m` should reach
    // the inner body via the dispatched call's cell arg.
    let src = "local m = 100
local f = function(x) return x + m end
local t = {f}
local g = t[1]
print(g(5))";
    assert_eq!(run(src, "lumelir_25c3_table_capture").trim(), "105");
}

#[test]
fn closure_identity_aliasing_observes_shared_box() {
    // Lua §3.4.4 closure identity: aliasing a capturing closure
    // (`local g = f`) copies the cell ptr, so writes through the
    // captured `n` via either binding hit the same heap upvalue
    // box. Distinct factory calls produce independent cells +
    // boxes, so cross-pollution is impossible.
    //
    // Function-kind == comparison is a separate codegen surface
    // (not in 3c scope); this test substitutes box-observation
    // for the same identity property.
    let src = "local function make_inc()
  local n = 0
  return function() n = n + 1; return n end
end
local f = make_inc()
local g = f
local h = make_inc()
print(g())
print(g())
print(f())
print(h())";
    let out = run(src, "lumelir_25c3_closure_identity");
    assert_eq!(out.trim(), "1\n2\n3\n1");
}

#[test]
fn generic_for_capturing_iter_inline_anonymous() {
    // ADR 0083 Commit 3c: a capturing anonymous closure used
    // directly as the iter expression (not via factory return)
    // hits the Function-kind branch with a known func_id, so
    // ret_kinds is recoverable. The closure's captured `i` /
    // `n` reach each step via the entry-block cell-ptr unpack.
    let src = "local n = 3
local i = 0
local iter = function(s, c)
  i = i + 1
  if i <= n then return i, i * 10 end
  return nil, nil
end
for k, v in iter, 0, 0 do
  print(k, v)
end";
    let out = run(src, "lumelir_25c3_generic_for_capt");
    assert_eq!(out.trim(), "1\t10\n2\t20\n3\t30");
}

#[test]
fn ir_shape_capturing_cell_uses_offset_16_upvalue_box_array() {
    // Pin the closure cell's IR shape: the heap cell is malloc'd
    // (16 + 8*N bytes), fn_ptr stored at offset 0, upvalue_count
    // at offset 8, upvalue_box[i] at offset 16 + i*8 — see
    // ADR 0083 layout. A drift in any of these offsets would
    // break box sharing or dispatch silently. Codex P3.
    use lumelir::codegen::emit::{emit_module, new_context};
    let ctx = new_context();
    let chunk = lumelir::parser::parse(
        "local x = 1
local function inc() x = x + 1 end
local function get() return x end
inc(); print(get())",
    )
    .unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    let module = emit_module(&ctx, &hir).expect("module verifies");
    let mlir = module.as_operation().to_string();
    // Closure cell is malloc'd via @malloc (heap allocation).
    assert!(
        mlir.contains("@malloc"),
        "capturing closure cell must allocate via @malloc"
    );
    // Upvalue-box array starts at offset 16 — `getelementptr [%cell, 16]`
    // and the load type is `!llvm.ptr` (the box ptr).
    assert!(
        mlir.contains("16 : i64") && mlir.contains("!llvm.ptr"),
        "cell.upvalue_box[0] read must use offset 16 with !llvm.ptr type"
    );
    // Cell-ptr-first ABI: every user fn signature must start
    // with `!llvm.ptr` (the cell ptr arg).
    assert!(
        mlir.contains("(!llvm.ptr") && mlir.contains("@user_inc_"),
        "capturing user fn signature must accept cell ptr as arg 0"
    );
}
