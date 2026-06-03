//! Phase 2.6+-rawget-tagged-key (ADR 0174):
//! `rawget(t, k)` with `Local(TaggedValue)` key.

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

// --- Test 1: pairs-body rawget with String-tag key ---

#[test]
fn rawget_tagged_local_key_string_tag() {
    // For string-keyed table, pairs binds k as TaggedValue/String at
    // runtime. rawget(t, k) must dispatch via the hash arm.
    let src = r#"
local t = {}
t.x = 10
t.y = 20
local got
for k, v in pairs(t) do
  got = rawget(t, k)
end
print(got)
"#;
    let out = run_ok(src, "lumelir_rawget_tagged_str");
    // Hash iteration order is non-deterministic; either 10 or 20
    // must surface, never nil.
    let v = out.trim();
    assert!(v == "10" || v == "20", "got: {v:?}");
}

// --- Test 2: rawget with Number-tag from runtime tagged local ---

#[test]
fn rawget_tagged_local_key_number_tag() {
    // Tagged local carrying a Number value at runtime; rawget(t, k)
    // dispatches to the Number arm and returns t[k].
    let src = r#"
local t = {10, 20, 30}
local box = {n = 2}
local k
for _, kv in pairs(box) do k = kv end
print(rawget(t, k))
"#;
    let out = run_ok(src, "lumelir_rawget_tagged_num");
    assert_eq!(out, "20\n");
}

// --- Test 3: rawget bypasses __index for TaggedValue key ---

#[test]
fn rawget_tagged_local_key_bypasses_index() {
    let src = r#"
local fallback = {}
fallback.z = "from_mt"
local mt = {}
mt.__index = fallback
local t = {}
setmetatable(t, mt)
local box = {a = "z"}
local k
for _, kv in pairs(box) do k = kv end
print(t[k])
local raw = rawget(t, k)
if raw == nil then
  print("raw_nil")
else
  print("raw_present")
end
"#;
    let out = run_ok(src, "lumelir_rawget_tagged_bypass");
    assert_eq!(out, "from_mt\nraw_nil\n");
}
