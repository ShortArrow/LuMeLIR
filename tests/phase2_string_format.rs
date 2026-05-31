//! Phase 2.7q-stdlib-string-format (ADR 0152): `string.format`
//! minimum-scope subset (`%d` / `%f` / `%s` / `%%`).

use std::process::Command;

fn compile_and_run(src: &str, output_name: &str) -> std::process::Output {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    result
}

fn run_ok(src: &str, output_name: &str) -> String {
    let out = compile_and_run(src, output_name);
    assert!(out.status.success(), "binary should exit 0: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// --- Test 1: %d integer ---

#[test]
fn format_percent_d() {
    let src = r#"print(string.format("count=%d", 42))"#;
    let out = run_ok(src, "lumelir_fmt_d");
    assert_eq!(out, "count=42\n");
}

// --- Test 2: %s string ---

#[test]
fn format_percent_s() {
    let src = r#"print(string.format("hello %s", "world"))"#;
    let out = run_ok(src, "lumelir_fmt_s");
    assert_eq!(out, "hello world\n");
}

// --- Test 3: %f float ---

#[test]
fn format_percent_f() {
    let src = r#"print(string.format("pi=%f", 3.14))"#;
    let out = run_ok(src, "lumelir_fmt_f");
    // %f default precision is 6
    assert!(
        out.starts_with("pi=3.14"),
        "expected pi=3.14..., got {out:?}"
    );
}

// --- Test 4: %% literal ---

#[test]
fn format_percent_percent() {
    let src = r#"print(string.format("100%% pure"))"#;
    let out = run_ok(src, "lumelir_fmt_pct_pct");
    assert_eq!(out, "100% pure\n");
}

// --- Test 5: mixed %s + %d ---

#[test]
fn format_mixed_string_and_number() {
    let src = r#"print(string.format("%s = %d", "answer", 42))"#;
    let out = run_ok(src, "lumelir_fmt_mixed");
    assert_eq!(out, "answer = 42\n");
}

// --- Test 6: too few args → HIR rejects ---

#[test]
fn format_too_few_args_rejects() {
    let src = r#"print(string.format("%d %d", 1))"#;
    let result = std::panic::catch_unwind(|| compile_and_run(src, "lumelir_fmt_arity"));
    match result {
        Err(_) => {}
        Ok(out) => {
            assert!(
                !out.status.success(),
                "too-few-args string.format must reject, but binary exited 0: {out:?}"
            );
        }
    }
}
