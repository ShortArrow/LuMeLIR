//! Phase 3 GC `collectgarbage()` end-to-end pins. ADR 0185 v1
//! safety-mode (mark-all-BLACK, sweep frees nothing) was the
//! transient contract; ADR 0218 supersedes it for chunk-safe
//! programs (no Tables / user functions / TaggedValue locals)
//! by walking a chunk-level String-slot root table — transient
//! intermediate allocations are freed.
//!
//! These tests pin the new real-freeing contract for chunk-safe
//! programs. Sequential calls still must not corrupt the heap.

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

// --- Test 1: `collectgarbage()` returns positive delta after
// transient allocations exist (ADR 0218 real freeing path). ---

#[test]
fn collectgarbage_returns_positive_delta_when_transients_exist() {
    // `tostring(1)` allocates an intermediate scratch buffer
    // alongside the final string object; the buffer is not
    // rooted by any chunk slot, so the mark phase leaves it
    // WHITE and sweep frees it. `freed > 0` proves the path is
    // live (it was pinned at 0 under ADR 0185 v1 safety mode).
    let src = r#"
local s = tostring(1)
local freed = collectgarbage()
if freed > 0 then
  print("freed")
else
  print("zero")
end
"#;
    let out = run_ok(src, "lumelir_gc_freed");
    assert_eq!(out, "freed\n");
}

// --- Test 2: `collectgarbage("count")` decreases across a
// collection that has transients (ADR 0218). ---

#[test]
fn collectgarbage_count_decreases_after_collection_with_transients() {
    let src = r#"
local s = tostring(1)
local before = collectgarbage("count")
collectgarbage()
local after = collectgarbage("count")
if after < before then
  print("decreased")
else
  print("unchanged")
end
"#;
    let out = run_ok(src, "lumelir_gc_count_decreased");
    assert_eq!(out, "decreased\n");
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
