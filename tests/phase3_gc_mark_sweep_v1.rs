//! Phase 3.gc-v1-step3 (ADR 0185): v1 safety-mode mark + sweep
//! regression pins. `collectgarbage()` now runs the
//! `emit_gc_mark` + `emit_gc_sweep` pair instead of a 0.0 stub;
//! v1 safety mode pre-marks every g_gc_head entry BLACK so
//! sweep frees nothing and observable behaviour is unchanged.
//!
//! These tests pin that contract: delta=0, count-unchanged
//! across collection, sequential calls don't corrupt the heap.

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

// --- Test 1: `collectgarbage()` returns 0 in v1 safety mode ---

#[test]
fn collectgarbage_returns_zero_in_safety_mode() {
    // v1 safety mode pre-marks every node BLACK; sweep frees
    // nothing; delta is exactly 0.
    let src = r#"
local s = tostring(1)
local freed = collectgarbage()
if freed == 0 then
  print("zero")
else
  print("nonzero")
end
"#;
    let out = run_ok(src, "lumelir_gc_v1_returns_zero");
    assert_eq!(out, "zero\n");
}

// --- Test 2: `collectgarbage("count")` unchanged across collection ---

#[test]
fn collectgarbage_count_unchanged_across_safety_mode_collection() {
    // Pre-collection count == post-collection count because
    // safety mode frees nothing. This is the v1 contract pin.
    let src = r#"
local s = tostring(1)
local before = collectgarbage("count")
collectgarbage()
local after = collectgarbage("count")
if before == after then
  print("unchanged")
else
  print("changed")
end
"#;
    let out = run_ok(src, "lumelir_gc_v1_count_unchanged");
    assert_eq!(out, "unchanged\n");
}

// --- Test 3: sequential `collectgarbage()` calls do not corrupt heap ---

#[test]
fn collectgarbage_sequential_calls_keep_heap_consistent() {
    // Multiple back-to-back collections must not break the
    // g_gc_head linked list or g_gc_total_bytes accounting.
    // After three runs, a fresh allocation must still register
    // in `count` (non-zero increment).
    let src = r#"
local a = tostring(1)
collectgarbage()
collectgarbage()
collectgarbage()
local before = collectgarbage("count")
local b = tostring(2)
local after = collectgarbage("count")
if after > before then
  print("accumulating")
else
  print("stalled")
end
"#;
    let out = run_ok(src, "lumelir_gc_v1_sequential");
    assert_eq!(out, "accumulating\n");
}
