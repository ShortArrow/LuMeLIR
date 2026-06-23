//! ADR 0270 — N7-9: math.atan accepts optional 2nd arg (Lua 5.4
//! replacement for deprecated math.atan2).

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
fn math_atan_single_arg_still_works() {
    // math.atan(1) ≈ π/4 ≈ 0.785; verify by range.
    let out = run_ok(
        "local r = math.atan(1)
if r > 0.78 and r < 0.79 then print(\"ok\") else print(\"bad\") end",
        "n7_atan_1arg",
    );
    assert_eq!(out.trim(), "ok");
}

#[test]
fn math_atan_two_arg_is_atan2() {
    // math.atan(1, 0) = π/2 ≈ 1.5708.
    let out = run_ok(
        "local r = math.atan(1, 0)
if r > 1.57 and r < 1.58 then print(\"ok\") else print(\"bad\") end",
        "n7_atan_2arg",
    );
    assert_eq!(out.trim(), "ok");
}

#[test]
fn math_atan_quadrant_via_2arg() {
    // math.atan(-1, -1) = -3π/4 ≈ -2.356.
    let out = run_ok(
        "local r = math.atan(-1, -1)
if r < -2.35 and r > -2.36 then print(\"ok\") else print(\"bad\") end",
        "n7_atan_quad",
    );
    assert_eq!(out.trim(), "ok");
}
