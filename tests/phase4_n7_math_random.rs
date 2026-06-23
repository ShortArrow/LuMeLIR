//! ADR 0263 — N7-2: `math.random([m[, n]])` via libc `rand()`.

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
fn math_random_no_arg_in_unit_interval() {
    // `math.random() < 1 and math.random() >= 0` should be true.
    let out = run_ok(
        "local r = math.random()
if r >= 0 and r < 1 then print(\"ok\") else print(\"bad\") end",
        "n7_random_unit",
    );
    assert_eq!(out.trim(), "ok");
}

#[test]
fn math_random_one_arg_in_range() {
    // `math.random(10)` is in [1, 10].
    let out = run_ok(
        "local r = math.random(10)
if r >= 1 and r <= 10 then print(\"ok\") else print(\"bad\") end",
        "n7_random_n",
    );
    assert_eq!(out.trim(), "ok");
}

#[test]
fn math_random_two_arg_in_range() {
    // `math.random(5, 8)` is in [5, 8].
    let out = run_ok(
        "local r = math.random(5, 8)
if r >= 5 and r <= 8 then print(\"ok\") else print(\"bad\") end",
        "n7_random_mn",
    );
    assert_eq!(out.trim(), "ok");
}
