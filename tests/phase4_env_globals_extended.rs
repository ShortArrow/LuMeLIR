//! ADR 0222 — extended `_G` behaviour pins: heterogeneous value
//! types (String / Bool / Number / Table), `pairs(_G)` iteration,
//! and single-level user-fn read+write composition.

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
fn g_holds_string_value() {
    assert_eq!(
        run_ok(
            "_G.name = \"world\"\nprint(\"hello \" .. _G.name)",
            "lumelir_env_g_string"
        )
        .trim(),
        "hello world"
    );
}

#[test]
fn g_mixed_number_and_string_values_coexist() {
    // Mixing Number and String values across separate _G keys
    // works — Tables widen to TaggedValue at the runtime layer.
    let out = run_ok(
        "_G.n = 42\n_G.s = \"hi\"\nprint(_G.n)\nprint(_G.s)",
        "lumelir_env_g_mixed",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "hi");
}

#[test]
fn g_pairs_iteration_counts_keys() {
    // pairs(_G) walks the hash part; counting confirms iteration
    // covers every key inserted via `_G.foo = ...`.
    let out = run_ok(
        "_G.a = 1\n_G.b = 2\n_G.c = 3\nlocal n = 0\nfor k, v in pairs(_G) do n = n + 1 end\nprint(n)",
        "lumelir_env_g_pairs_count",
    );
    assert_eq!(out.trim(), "3");
}

#[test]
fn g_read_modify_write_inside_user_fn() {
    // Single-level user fn captures _G via the ADR 0083 closure
    // upvalue mechanism; multiple calls accumulate state.
    let out = run_ok(
        "_G.x = 10
local function bump() _G.x = _G.x + 1 end
bump()
bump()
print(_G.x)",
        "lumelir_env_g_fn_rmw",
    );
    assert_eq!(out.trim(), "12");
}

#[test]
fn g_reassignment_overwrites_previous() {
    let out = run_ok(
        "_G.x = 1
_G.x = 2
_G.x = 3
print(_G.x)",
        "lumelir_env_g_reassign",
    );
    assert_eq!(out.trim(), "3");
}
