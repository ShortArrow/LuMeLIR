//! ADR 0236 — M9-A: Lua 5.4 §3.3.7 `<const>` / `<close>` local
//! attributes. Parser accepts `local NAME <const>` and
//! `local NAME <close>`; HIR sets `LocalInfo::is_const` for the
//! `const` case and inserts the Local into `readonly_locals` so
//! any subsequent Assign is rejected as `ReadOnlyAssign`. The
//! `<close>` attribute parses but its `__close` metamethod
//! dispatch is deferred (M9-C scope).

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
fn const_local_round_trips_through_read() {
    assert_eq!(
        run_ok(
            "local x <const> = 42
print(x)",
            "lumelir_m9a_const_read"
        )
        .trim(),
        "42"
    );
}

#[test]
fn const_local_assign_is_rejected_at_hir() {
    let src = "local x <const> = 1\nx = 2";
    let chunk = lumelir::parser::parse(src).unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("Assign to const must error");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("ReadOnlyAssign"),
        "expected ReadOnlyAssign, got: {msg}"
    );
}

#[test]
fn close_attribute_parses_but_does_not_constrain_writes() {
    // <close> compiles; the metamethod dispatch is M9-C. Until
    // then a Local marked <close> behaves like any other Local
    // for read/write purposes (no readonly_locals entry).
    assert_eq!(
        run_ok(
            "local r <close> = 7
print(r)",
            "lumelir_m9a_close_read"
        )
        .trim(),
        "7"
    );
}

#[test]
fn unknown_attribute_is_rejected_at_hir() {
    let src = "local x <foo> = 1";
    let chunk = lumelir::parser::parse(src).unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("unknown attr must error");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("UnknownAttribute"),
        "expected UnknownAttribute, got: {msg}"
    );
}

#[test]
fn const_local_inside_user_fn_body() {
    // Verify <const> attribute also works in nested scopes.
    assert_eq!(
        run_ok(
            "local function f()
  local pi <const> = 3
  return pi
end
print(f())",
            "lumelir_m9a_const_in_fn"
        )
        .trim(),
        "3"
    );
}
