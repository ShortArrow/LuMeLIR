//! Integration test: Phase 2.6c-tag-defensive-trap (ADR 0069)
//! — backstop the assumption that values whose runtime tag is
//! not yet supported never reach the runtime tagged-slot
//! dispatch path. After ADR 0071 this means **closures with
//! upvalues** (LIC-2.6c-tag-hetero-closure-escape-1) — closure-
//! less Function values and plain Table values now ship
//! through the dispatch chain just fine.
//!
//! The trap (`emit_tagged_unknown_tag_trap`) remains as the
//! fail-fast guard rail for any future runtime tag value the
//! consumers don't yet handle.

#[test]
fn closure_with_upvalue_in_array_now_lowers_post_3c() {
    // Phase 2.5c-full Commit 3c (ADR 0083 supersedes 0044/0071):
    // closure-with-upvalues stores into the tagged slot via the
    // cell-ptr-first ABI. The runtime tag check + dispatch chain
    // (offset 0 / 8 → cell.fn_ptr) still gate non-Function
    // payloads via the s_call_non_function trap.
    let chunk =
        lumelir::parser::parse("local x = 1\nlocal f = function() return x end\nlocal t = {f}")
            .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_ok());
}

#[test]
fn closure_with_upvalue_in_hash_now_lowers_post_3c() {
    let chunk = lumelir::parser::parse(
        "local x = 1\nlocal f = function() return x end\nlocal t = {}\nt.k = f",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_ok());
}

#[test]
fn closure_less_function_in_array_now_accepted_post_2_6c_tag_fn_tbl() {
    // Foil to the pair above: ADR 0071 explicitly permits
    // closure-less Function values. The lowering must succeed
    // (the value reaches `emit_value_slot_store_function`).
    let chunk = lumelir::parser::parse("local function f() return 1 end\nlocal t = {f}").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_ok());
}
