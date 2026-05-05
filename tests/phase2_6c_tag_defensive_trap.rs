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
fn closure_with_upvalue_in_array_rejected_at_hir() {
    // ADR 0071's `function_ref_id` + upvalues check rejects
    // closures-with-upvalues at HIR-time so the underlying
    // function pointer never enters a tagged slot. This is
    // the gate that keeps the closure-escape risk out of the
    // runtime tag dispatch path.
    let chunk =
        lumelir::parser::parse("local x = 1\nlocal f = function() return x end\nlocal t = {f}")
            .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn closure_with_upvalue_in_hash_rejected_at_hir() {
    let chunk = lumelir::parser::parse(
        "local x = 1\nlocal f = function() return x end\nlocal t = {}\nt.k = f",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn closure_less_function_in_array_now_accepted_post_2_6c_tag_fn_tbl() {
    // Foil to the pair above: ADR 0071 explicitly permits
    // closure-less Function values. The lowering must succeed
    // (the value reaches `emit_value_slot_store_function`).
    let chunk = lumelir::parser::parse("local function f() return 1 end\nlocal t = {f}").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_ok());
}
