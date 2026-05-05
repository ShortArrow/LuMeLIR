//! Integration test: Phase 2.6c-tag-defensive-trap (ADR 0069)
//! — backstop the assumption that Function / Table values
//! never reach the runtime tagged-slot dispatch path. The new
//! `emit_tagged_unknown_tag_trap` is currently unreachable in
//! generated programs because HIR rejects Function / Table
//! values in tables (LIC-2.6c-tag-hetero-fn-tbl-1). These
//! tests fix that boundary so the trap stays guard-rail and
//! flips to "regression suite" the moment a future sub-phase
//! starts lowering Function / Table values.

#[test]
fn function_value_in_array_rejected_at_hir() {
    // ADR 0064 admits Number / Bool / String values into arrays;
    // Function values still reject. This is the gate that keeps
    // TAG_FUNCTION (=4) out of runtime tagged slots.
    let chunk = lumelir::parser::parse("local function f() return 1 end\nlocal t = {f}").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn function_value_in_hash_rejected_at_hir() {
    let chunk =
        lumelir::parser::parse("local function f() return 1 end\nlocal t = {}\nt.k = f").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn table_value_in_hash_rejected_at_hir() {
    // Same gate, Table-valued. Symmetry with the Function case
    // — both reserved tags (4 / 5) are HIR-blocked.
    let chunk = lumelir::parser::parse("local t = {}\nlocal u = {}\nt.k = u").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}
