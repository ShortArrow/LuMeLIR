//! ADR 0242 — M11-C: `os.time`, `os.clock`, `os.getenv`. Time
//! and clock route through libc; getenv handles the NULL-on-
//! miss case by storing Nil in a TaggedValue tmp slot.

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

fn run_ok_env(src: &str, output_name: &str, envs: &[(&str, &str)]) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let mut cmd = Command::new(&output);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let r = cmd.output().expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    assert!(r.status.success(), "binary should exit 0: {r:?}");
    String::from_utf8_lossy(&r.stdout).into_owned()
}

#[test]
fn os_time_returns_positive_number() {
    // The current epoch is > 1.5e9 since 2017 — pin only that
    // it's positive to avoid time-bound brittleness.
    let out = run_ok(
        "local t = os.time()
if t > 0 then print(\"positive\") else print(\"non-positive\") end",
        "lumelir_m11c_time_positive",
    );
    assert_eq!(out.trim(), "positive");
}

#[test]
fn os_clock_returns_non_negative_number() {
    let out = run_ok(
        "local c = os.clock()
if c >= 0 then print(\"ok\") else print(\"bad\") end",
        "lumelir_m11c_clock_nonneg",
    );
    assert_eq!(out.trim(), "ok");
}

#[test]
fn os_getenv_hit_returns_string() {
    let out = run_ok_env(
        "local v = os.getenv(\"LUMELIR_M11C_HIT\")
print(v)",
        "lumelir_m11c_getenv_hit",
        &[("LUMELIR_M11C_HIT", "hello-from-env")],
    );
    assert_eq!(out.trim(), "hello-from-env");
}

#[test]
fn os_getenv_miss_returns_nil() {
    let out = run_ok(
        "local v = os.getenv(\"LUMELIR_M11C_DEFINITELY_NOT_SET_42\")
print(v)",
        "lumelir_m11c_getenv_miss",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn os_getenv_composes_with_if() {
    let out = run_ok_env(
        "local v = os.getenv(\"LUMELIR_M11C_IF\")
if v then print(\"set\") else print(\"unset\") end",
        "lumelir_m11c_getenv_if",
        &[("LUMELIR_M11C_IF", "anything")],
    );
    assert_eq!(out.trim(), "set");
}
