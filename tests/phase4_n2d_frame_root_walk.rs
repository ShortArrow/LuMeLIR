//! ADR 0257 — N2-D: per-frame stack walk for non-chunk-level
//! locals. Inside a user fn frame, String / Table / Function-kind
//! locals were previously invisible to the GC mark phase — the
//! `g_gc_fn_depth > 0` guard fell back to v1 safety mode (mark
//! everything BLACK). N2-D registers each user fn's String / Table /
//! Function slots into a linked frame stack (g_frame_root_head) at
//! entry and unlinks at exit. The mark phase walks chunk roots +
//! the frame chain, then runs the existing Table + closure-cell
//! propagation passes. The depth-guard fallback is dropped when
//! all user fns have only frame-safe local kinds.

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
fn fn_local_table_survives_midfn_gc() {
    // Table allocated inside user fn; collectgarbage runs mid-fn.
    // Without N2-D the slot is invisible to the chunk root walk;
    // v1 fallback keeps it alive but forfeits freeing. With N2-D
    // the frame walk marks it BLACK directly.
    let out = run_ok(
        "local function build()
  local t = {\"alive\"}
  collectgarbage()
  print(t[1])
end
build()",
        "lumelir_n2d_fn_local_table",
    );
    assert_eq!(out.trim(), "alive");
}

#[test]
fn fn_local_string_survives_midfn_gc() {
    let out = run_ok(
        "local function build()
  local s = tostring(42)
  collectgarbage()
  print(s)
end
build()",
        "lumelir_n2d_fn_local_string",
    );
    assert_eq!(out.trim(), "42");
}

#[test]
fn two_fn_local_tables_both_survive() {
    let out = run_ok(
        "local function build()
  local a = {\"first\"}
  local b = {\"second\"}
  collectgarbage()
  print(a[1])
  print(b[1])
end
build()",
        "lumelir_n2d_two_tables",
    );
    assert_eq!(out.trim(), "first\nsecond");
}

#[test]
fn nested_call_keeps_outer_frame_alive() {
    // outer's local Table must survive while inner runs gc.
    let out = run_ok(
        "local function inner()
  collectgarbage()
end
local function outer()
  local t = {\"keep\"}
  inner()
  print(t[1])
end
outer()",
        "lumelir_n2d_nested",
    );
    assert_eq!(out.trim(), "keep");
}

#[test]
fn midfn_collectgarbage_frees_inside_fn_transients() {
    // The N2-D differentiator: mid-fn `collectgarbage()` must
    // really free transients allocated and dropped inside the
    // user fn frame. Under the v1 safety-mode fallback (depth>0
    // → mark-all-BLACK) every g_gc_head node is BLACK so the
    // sweep frees nothing — `freed` would be 0. With N2-D the
    // real-mode mark phase walks chunk roots + frame chain and
    // leaves the dropped String WHITE, so sweep frees it.
    let out = run_ok(
        "local function churn()
  local s = tostring(7)
  s = tostring(8)
  local freed = collectgarbage()
  print(s)
  if freed > 0 then print(\"freed\") else print(\"zero\") end
end
churn()",
        "lumelir_n2d_midfn_real_free",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "8");
    assert_eq!(lines[1], "freed");
}

#[test]
fn transient_inside_frame_can_be_freed_after_return() {
    // A Table allocated and dropped inside the fn is freed by a
    // subsequent collectgarbage() after the fn returns. The chunk-
    // level kept reference stays alive.
    let out = run_ok(
        "local function spawn()
  local _ = {\"transient\"}
  return 1
end
spawn()
local kept = {\"alive\"}
local freed = collectgarbage()
print(kept[1])
if freed > 0 then print(\"freed\") else print(\"zero\") end",
        "lumelir_n2d_transient",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "alive");
    assert_eq!(lines[1], "freed");
}
