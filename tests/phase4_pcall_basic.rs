//! ADR 0216 — `pcall(f)` single-return Bool. Minimum-viable scope:
//! - arg is a Local of `ValueKind::Function(0)`
//! - returns `true` when `f` returns normally
//! - returns `false` when `f` calls `error(msg)` (caught via the
//!   ADR 0215 longjmp infrastructure)

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
    assert!(r.status.success(), "pcall binary should exit 0: {r:?}");
    String::from_utf8_lossy(&r.stdout).into_owned()
}

#[test]
fn pcall_of_passing_function_returns_true() {
    assert_eq!(
        run_ok(
            "local function ok_fn() return 1 end
local ok = pcall(ok_fn)
print(ok)",
            "lumelir_pcall_pass"
        )
        .trim(),
        "true"
    );
}

#[test]
fn pcall_of_erroring_function_returns_false() {
    assert_eq!(
        run_ok(
            "local function bad() error(\"oops\") end
local ok = pcall(bad)
print(ok)",
            "lumelir_pcall_catch"
        )
        .trim(),
        "false"
    );
}

#[test]
fn pcall_caught_error_does_not_propagate_to_main() {
    // After pcall catches the error, execution continues. The
    // subsequent print proves main did not exit 1 (the ADR 0215
    // uncaught path).
    assert_eq!(
        run_ok(
            "local function bad() error(\"boom\") end
local ok = pcall(bad)
print(\"after\")",
            "lumelir_pcall_continue"
        )
        .trim(),
        "after"
    );
}

#[test]
fn pcall_function_with_internal_local_continues() {
    // The fn body uses a Local then returns. Proves Pcall's
    // setjmp dance does not corrupt local bindings on the normal
    // (then) path.
    assert_eq!(
        run_ok(
            "local function pure() local x = 1 + 2 return x end
local ok = pcall(pure)
print(ok)",
            "lumelir_pcall_internal_local"
        )
        .trim(),
        "true"
    );
}
// Future: nested pcall via captured-upvalue Function arg
// (`local function outer() pcall(inner) end`) requires HIR
// captures the upvalue Function differently from direct Local
// reference; deferred to a future ADR alongside multi-return.
