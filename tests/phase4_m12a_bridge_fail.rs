//! ADR 0243 — M12-A: Bridge error propagation. `rust.fail(msg)`
//! raises a Lua error from the Rust side via the ADR 0215
//! setjmp/longjmp infrastructure. Composes with `pcall` (ADRs
//! 0216 / 0217) so user code can recover via `local ok, err =
//! pcall(f)`.

use std::process::Command;

fn run_status_stdout(src: &str, output_name: &str) -> (Option<i32>, String) {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let r = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    (
        r.status.code(),
        String::from_utf8_lossy(&r.stdout).into_owned(),
    )
}

#[test]
fn rust_fail_uncaught_exits_1_with_message() {
    let (status, stdout) =
        run_status_stdout("rust.fail(\"boom-from-rust\")", "lumelir_m12a_uncaught");
    assert_eq!(status, Some(1));
    assert!(
        stdout.contains("boom-from-rust"),
        "expected message in output, got: {stdout}"
    );
}

#[test]
fn rust_fail_caught_by_pcall_yields_false_err() {
    let (status, stdout) = run_status_stdout(
        "local function f() rust.fail(\"caught-from-rust\") end
local ok, err = pcall(f)
print(ok)
print(err)",
        "lumelir_m12a_pcall_caught",
    );
    assert_eq!(status, Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "false");
    assert_eq!(lines[1], "caught-from-rust");
}

#[test]
fn rust_fail_composes_with_if_not_ok() {
    let (status, stdout) = run_status_stdout(
        "local function f() rust.fail(\"x\") end
local ok, err = pcall(f)
if not ok then print(\"handled\") end",
        "lumelir_m12a_if_not_ok",
    );
    assert_eq!(status, Some(0));
    assert_eq!(stdout.trim(), "handled");
}

#[test]
fn rust_fail_does_not_corrupt_subsequent_calls() {
    // After a caught fail, subsequent rust.add still works.
    let (status, stdout) = run_status_stdout(
        "local function f() rust.fail(\"once\") end
local ok = pcall(f)
print(rust.add(1, 2))",
        "lumelir_m12a_after_caught",
    );
    assert_eq!(status, Some(0));
    assert_eq!(stdout.trim(), "3");
}
