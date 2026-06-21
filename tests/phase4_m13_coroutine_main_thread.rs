//! ADR 0245 — M13-A: Main-thread coroutine introspection.
//! `coroutine.isyieldable()` and `coroutine.running()` are the
//! two `coroutine.*` functions that have spec-correct
//! behavior on the main thread without a coroutine runtime —
//! `false` for isyieldable, `(nil, true)` for running. The
//! `create / resume / yield / wrap / status` runtime needs
//! stack switching (M13-stretch).

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
fn isyieldable_on_main_thread_is_false() {
    assert_eq!(
        run_ok(
            "print(coroutine.isyieldable())",
            "lumelir_m13_isyieldable_main"
        )
        .trim(),
        "false"
    );
}

#[test]
fn running_on_main_thread_yields_nil_and_true() {
    let out = run_ok(
        "local th, main = coroutine.running()
print(th)
print(main)",
        "lumelir_m13_running_main",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "nil");
    assert_eq!(lines[1], "true");
}

#[test]
fn running_single_assign_truncates_to_nil_thread() {
    // Single-result position truncates to position 0 per ADR
    // 0081 Next precedent — that's the `nil` (no thread).
    assert_eq!(
        run_ok(
            "local th = coroutine.running()
print(th)",
            "lumelir_m13_running_truncate"
        )
        .trim(),
        "nil"
    );
}

#[test]
fn isyieldable_composes_with_if_else() {
    let out = run_ok(
        "if coroutine.isyieldable() then
  print(\"in coroutine\")
else
  print(\"main\")
end",
        "lumelir_m13_isyieldable_if",
    );
    assert_eq!(out.trim(), "main");
}

#[test]
fn running_is_main_used_as_predicate() {
    let out = run_ok(
        "local th, is_main = coroutine.running()
if is_main then
  print(\"main thread\")
else
  print(\"coroutine\")
end",
        "lumelir_m13_running_predicate",
    );
    assert_eq!(out.trim(), "main thread");
}
