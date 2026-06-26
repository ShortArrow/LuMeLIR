//! ADR 0273 — N7-12: math.log accepts optional base arg.

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
fn math_log_single_arg_natural() {
    // math.log(e) ≈ 1.0; use exp(1) for e.
    let out = run_ok(
        "local r = math.log(math.exp(1))
if r > 0.999 and r < 1.001 then print(\"ok\") else print(\"bad\") end",
        "n7_log_1arg",
    );
    assert_eq!(out.trim(), "ok");
}

#[test]
fn math_log_base_10() {
    // math.log(1000, 10) = 3.
    let out = run_ok(
        "local r = math.log(1000, 10)
if r > 2.999 and r < 3.001 then print(\"ok\") else print(\"bad\") end",
        "n7_log_base10",
    );
    assert_eq!(out.trim(), "ok");
}

#[test]
fn math_log_base_2() {
    // math.log(8, 2) = 3.
    let out = run_ok(
        "local r = math.log(8, 2)
if r > 2.999 and r < 3.001 then print(\"ok\") else print(\"bad\") end",
        "n7_log_base2",
    );
    assert_eq!(out.trim(), "ok");
}
