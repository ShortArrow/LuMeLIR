//! Phase 3.gc-v1-step1 (ADR 0157): allocator wrapper + string-obj
//! migration + `collectgarbage("count")` builtin.

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

// --- Test 1: `collectgarbage("count")` returns a positive Number ---

#[test]
fn collectgarbage_count_returns_positive_after_alloc() {
    // tostring(1) allocates a boxed String object via
    // emit_string_obj_alloc → emit_gc_alloc, which bumps
    // g_gc_total_bytes. So `count` must be > 0 here.
    let src = r#"
local s = tostring(1)
local kb = collectgarbage("count")
if kb > 0 then
  print("positive")
else
  print("zero")
end
"#;
    let out = run_ok(src, "lumelir_gc_count_positive");
    assert_eq!(out, "positive\n");
}

// --- Test 2: `collectgarbage()` no-op stub returns 0 ---

#[test]
fn collectgarbage_no_arg_returns_zero_stub() {
    // No-op stub until ADRs 0159 + 0161 land mark/sweep.
    let src = r#"print(collectgarbage())"#;
    let out = run_ok(src, "lumelir_gc_noop");
    assert_eq!(out, "0\n");
}

// --- Test 3: Unsupported option HIR-rejects ---

#[test]
fn collectgarbage_unsupported_option_rejects() {
    let src = r#"collectgarbage("stop")"#;
    let result = std::panic::catch_unwind(|| compile_and_run(src, "lumelir_gc_unsupported_opt"));
    match result {
        Err(_) => {}
        Ok(out) => {
            assert!(
                !out.status.success(),
                "unsupported collectgarbage option must reject, but binary exited 0: {out:?}"
            );
        }
    }
}
