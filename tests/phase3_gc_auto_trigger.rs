//! Phase 3.gc-v1-step4 (ADR 0186): auto-trigger threshold +
//! llvm.func factoring regression pins. The threshold check runs
//! inside `emit_gc_alloc`; when crossing 1 MiB it calls
//! `@gc_mark` + `@gc_sweep` and doubles the threshold (post-sweep
//! total * 2, capped at 1 GiB). In v1 safety mode the actual
//! collection is still observably no-op.
//!
//! These tests pin the contract: the trigger does not crash,
//! `count` continues to accumulate (Phase 2 leak still in
//! effect), and explicit `collectgarbage()` still returns 0.

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

// --- Test 1: Threshold-crossing loop does not crash ---

#[test]
fn auto_trigger_does_not_crash_on_threshold_cross() {
    // Allocate ~1.5 MiB worth of strings to cross the 1 MiB
    // trigger threshold. The auto-trigger fires mid-loop; in v1
    // safety mode it pre-marks BLACK, sweeps no nodes, and
    // doubles the threshold. Loop must complete normally.
    let src = r#"
local i = 1
while i <= 800 do
  local s = string.rep("x", 2000)
  i = i + 1
end
print("ok")
"#;
    let out = run_ok(src, "lumelir_gc_auto_trigger_no_crash");
    assert_eq!(out, "ok\n");
}

// --- Test 2: count is consistent across auto-trigger ---

#[test]
fn auto_trigger_keeps_count_monotonic_in_v1() {
    // Phase 2 leak semantics are preserved in v1 safety mode:
    // count must monotonically grow even though the trigger
    // fires mid-loop.
    let src = r#"
local before = collectgarbage("count")
local i = 1
while i <= 800 do
  local s = string.rep("y", 2000)
  i = i + 1
end
local after = collectgarbage("count")
if after > before then
  print("grew")
else
  print("stalled")
end
"#;
    let out = run_ok(src, "lumelir_gc_auto_trigger_count_monotonic");
    assert_eq!(out, "grew\n");
}

// --- Test 3: explicit collectgarbage() post-loop frees transients ---

#[test]
fn explicit_collectgarbage_frees_transients_post_auto_trigger() {
    // Per ADR 0218 (chunk-safe real freeing): the auto-trigger
    // fires during the loop, and the explicit post-loop call
    // observes a positive delta because the per-iteration
    // `string.rep` allocations not anchored to a surviving slot
    // are WHITE → freed. Previous expectation (delta == 0) was
    // the ADR 0185 v1 safety-mode contract, now superseded.
    let src = r#"
local i = 1
while i <= 800 do
  local s = string.rep("z", 2000)
  i = i + 1
end
local freed = collectgarbage()
if freed > 0 then
  print("freed")
else
  print("zero")
end
"#;
    let out = run_ok(src, "lumelir_gc_auto_trigger_explicit_freed");
    assert_eq!(out, "freed\n");
}
