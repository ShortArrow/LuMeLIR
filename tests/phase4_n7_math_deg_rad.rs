//! ADR 0268 — N7-7: math.deg / math.rad inline pi-conversion.

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
fn math_deg_pi_yields_180() {
    let out = run_ok("print(math.deg(math.pi))", "n7_deg_pi")
        .trim()
        .to_owned();
    assert_eq!(out, "180");
}

#[test]
fn math_rad_180_yields_pi() {
    // print precision: just verify > 3.14 and < 3.15 by branching.
    let out = run_ok(
        "local r = math.rad(180)
if r > 3.14 and r < 3.15 then print(\"ok\") else print(\"bad\") end",
        "n7_rad_180",
    );
    assert_eq!(out.trim(), "ok");
}

#[test]
fn math_deg_zero_is_zero() {
    assert_eq!(run_ok("print(math.deg(0))", "n7_deg_zero").trim(), "0");
}
