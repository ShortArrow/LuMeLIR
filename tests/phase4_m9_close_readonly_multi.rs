//! ADR 0237 — M9-B: `<close>` is implicitly `<const>` per
//! Lua 5.4 §3.3.8 (variable cannot be assigned after
//! declaration). Multi-binding `local NAME <ATTR>, ...` now
//! also enforces readonly per name. The `__close` metamethod
//! dispatch is still M9-C scope; this ADR only covers the
//! readonly contract.

use std::process::Command;

fn run_ok(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let r = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    assert!(r.status.success(), "binary should exit 0: {r:?}");
    String::from_utf8_lossy(&r.stdout).into_owned()
}

#[test]
fn close_assign_is_rejected() {
    let src = "local x <close> = 1\nx = 2";
    let chunk = lumelir::parser::parse(src).unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("Assign to <close> must error");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("ReadOnlyAssign"),
        "expected ReadOnlyAssign, got: {msg}"
    );
}

#[test]
fn multi_binding_const_attr_enforced_per_name() {
    let src = "local a <const>, b = 1, 2\na = 99";
    let chunk = lumelir::parser::parse(src).unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("Assign to const a must error");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("ReadOnlyAssign"),
        "expected ReadOnlyAssign, got: {msg}"
    );
}

#[test]
fn multi_binding_mutable_name_can_be_assigned() {
    // `b` has no attribute; remains mutable while `a` is const.
    assert_eq!(
        run_ok(
            "local a <const>, b = 1, 2
b = 99
print(a)
print(b)",
            "lumelir_m9b_multi_mixed"
        )
        .trim(),
        "1\n99"
    );
}

#[test]
fn multi_binding_unknown_attr_rejected_at_hir() {
    let src = "local a <foo>, b = 1, 2";
    let chunk = lumelir::parser::parse(src).unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("unknown attr must error");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("UnknownAttribute"),
        "expected UnknownAttribute, got: {msg}"
    );
}

#[test]
fn const_and_close_both_read_round_trip() {
    assert_eq!(
        run_ok(
            "local a <const> = 7
local b <close> = 11
print(a + b)",
            "lumelir_m9b_both_attrs"
        )
        .trim(),
        "18"
    );
}
