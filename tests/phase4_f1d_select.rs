//! ADR 0299 — F1-D: `select` desugar over the vararg pack.

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
fn select_hash_counts_extras() {
    let out = run_ok(
        "local function f(...) print(select(\"#\", ...)) end
f(10, 20, 30)",
        "f1d_count",
    );
    assert_eq!(out.trim(), "3");
}

#[test]
fn select_hash_zero_for_no_extras() {
    let out = run_ok(
        "local function f(...) print(select(\"#\", ...)) end
f()",
        "f1d_count_zero",
    );
    assert_eq!(out.trim(), "0");
}

#[test]
fn select_n_returns_nth_extra() {
    let out = run_ok(
        "local function f(...) print(select(2, ...)) end
f(10, 20, 30)",
        "f1d_nth",
    );
    assert_eq!(out.trim(), "20");
}

#[test]
fn select_n_past_end_is_nil() {
    let out = run_ok(
        "local function f(...)
    local v = select(5, ...)
    print(type(v))
end
f(1, 2)",
        "f1d_past_end",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn select_with_declared_params_first() {
    // Declared params don't count as extras.
    let out = run_ok(
        "local function f(a, b, ...) print(select(\"#\", ...)) end
f(1, 2, 3, 4, 5)",
        "f1d_declared_first",
    );
    assert_eq!(out.trim(), "3");
}
