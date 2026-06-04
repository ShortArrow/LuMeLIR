//! Phase 2.6+-index-tagged-key (ADR 0177):
//! `t[k]` for Local(TaggedValue) key — tagged-consumer path.

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

// --- Test 1: pairs-body t[k] with String-tag key ---

#[test]
fn index_tagged_local_key_string_tag() {
    // dst[k] read for k carrying String tag at runtime.
    let src = r#"
local src_tbl = {}
src_tbl.x = 10
src_tbl.y = 20
local dst = {}
dst.x = 100
dst.y = 200
for k, v in pairs(src_tbl) do
  local got = dst[k]
  print(got)
end
"#;
    let out = run_ok(src, "lumelir_index_tagged_str");
    // Hash iteration emits both 100 and 200 (order non-deterministic).
    let lines: Vec<&str> = out.trim().split('\n').collect();
    assert_eq!(lines.len(), 2, "got: {out:?}");
    let mut sorted = lines.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["100", "200"], "got: {out:?}");
}

// --- Test 2: t[k] with Number-tag (runtime tagged Number key) ---

#[test]
fn index_tagged_local_key_number_tag() {
    let src = r#"
local t = {10, 20, 30}
local box = {}
box.n = 2
for kk, v in pairs(box) do
  local got = t[v]
  print(got)
end
"#;
    let out = run_ok(src, "lumelir_index_tagged_num");
    assert_eq!(out, "20\n");
}

// --- Test 3: t[k] consults __index when key missing in t ---

#[test]
fn index_tagged_local_key_consults_index() {
    let src = r#"
local fallback = {}
fallback.z = "from_mt"
local mt = {}
mt.__index = fallback
local t = {}
setmetatable(t, mt)
local box = {}
box.a = "z"
for kk, v in pairs(box) do
  local got = t[v]
  print(got)
end
"#;
    let out = run_ok(src, "lumelir_index_tagged_chain");
    assert_eq!(out, "from_mt\n");
}
