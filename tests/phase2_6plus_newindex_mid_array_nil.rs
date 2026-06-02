//! Phase 2.6+-newindex-mid-array-nil (ADR 0171):
//! in-range `TAG_NIL` slot triggers Number-key `__newindex`.

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

// --- Test 1: gap-fill nil slot triggers __newindex ---

#[test]
fn newindex_fires_for_gap_fill_nil_slot() {
    // t = {1}; t[3] = 99 (key_high path) grows array with t[2] =
    // TAG_NIL. Then with mt.__newindex = sink, t[2] = "x" should
    // route to sink (in-range slot is nil). Outer t[2] stays nil.
    let src = r#"
local sink = {}
local t = {1}
t[3] = 99
local mt = {}
mt.__newindex = sink
setmetatable(t, mt)
t[2] = 222
print(t[1])
print(t[2])
print(t[3])
print(sink[2])
"#;
    let out = run_ok(src, "lumelir_newidx_midarr_gap");
    assert_eq!(out, "1\nnil\n99\n222\n");
}

// --- Test 2: existing non-nil slot overwrites normally (regression) ---

#[test]
fn newindex_does_not_fire_for_existing_non_nil_overwrite() {
    // t[2] holds value 2 (non-Nil). mt.__newindex = sink. Writing
    // t[2] = 22 must overwrite the outer slot, NOT route to sink.
    let src = r#"
local sink = {}
local t = {1, 2, 3}
local mt = {}
mt.__newindex = sink
setmetatable(t, mt)
t[2] = 22
print(t[2])
print(sink[2])
"#;
    let out = run_ok(src, "lumelir_newidx_midarr_overwrite_pin");
    assert_eq!(out, "22\nnil\n");
}
