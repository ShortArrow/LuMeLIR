//! ADR 0291 — N7-20: math.randomseed(seed) via libc srand.

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
fn randomseed_call_is_stable() {
    // Just verify the builtin compiles and runs without error.
    let out = run_ok("math.randomseed(42) print(\"ok\")", "n7_20_call");
    assert_eq!(out.trim(), "ok");
}

#[test]
fn randomseed_makes_random_deterministic() {
    // With a fixed seed, math.random() should be reproducible.
    let out = run_ok(
        "math.randomseed(1)
local a = math.random(1, 100)
math.randomseed(1)
local b = math.random(1, 100)
if a == b then print(\"same\") else print(\"different\") end",
        "n7_20_deterministic",
    );
    assert_eq!(out.trim(), "same");
}

#[test]
fn randomseed_different_seeds_differ() {
    // Different seeds → typically different sequences (at least one
    // draw in 100 will differ).
    let out = run_ok(
        "math.randomseed(1)
local a = math.random(1, 1000000)
math.randomseed(2)
local b = math.random(1, 1000000)
if a ~= b then print(\"different\") else print(\"same\") end",
        "n7_20_differ",
    );
    assert_eq!(out.trim(), "different");
}
