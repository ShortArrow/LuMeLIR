//! ADR 0219 — chunk-safe predicate relaxed to allow non-
//! capturing user functions. The GC mark phase falls back to v1
//! safety mode while `g_gc_fn_depth > 0` so allocations live in
//! user fn frames are not freed mid-execution; chunk-level
//! collectgarbage() after the fn returns still observes real
//! freeing of transient strings.

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
fn chunk_with_user_fn_still_frees_transients_at_chunk_level() {
    // The user fn never calls collectgarbage; the chunk-level
    // collect after the fn returns observes a positive delta.
    let out = run_ok(
        "local function doubled(x) return x * 2 end
local n = doubled(5)
local s = tostring(n)
local freed = collectgarbage()
print(s)
if freed > 0 then print(\"freed\") else print(\"nope\") end",
        "lumelir_gc_with_fn_chunk_freed",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "freed");
}

#[test]
fn collect_inside_user_fn_does_real_freeing_via_frame_walk() {
    // ADR 0257 — N2-D supersedes ADR 0219. The depth-guard
    // fallback to v1 safety mode is dropped when the chunk is
    // frame-safe; the frame-root walk keeps the fn's live `s`
    // BLACK while transients dropped during the call get swept.
    let out = run_ok(
        "local function gc_inside()
  local s = tostring(7)
  s = tostring(8)
  return collectgarbage()
end
local freed = gc_inside()
if freed > 0 then print(\"freed\") else print(\"zero\") end",
        "lumelir_gc_inside_fn_real",
    );
    assert_eq!(out.trim(), "freed");
}

#[test]
fn rooted_string_via_user_fn_return_survives() {
    // The fn returns a fresh string; chunk slot `s` is rooted in
    // the root table; collectgarbage at chunk level preserves it.
    let out = run_ok(
        "local function make() return tostring(42) end
local s = make()
collectgarbage()
print(s)",
        "lumelir_gc_fn_return_rooted",
    );
    assert_eq!(out.trim(), "42");
}
