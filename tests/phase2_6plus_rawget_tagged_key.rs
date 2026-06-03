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
for k, v in pairs(t) do
  print(rawget(t, k))
end
"#;
    let out = run_ok(src, "lumelir_rawget_tagged_str");
    // Hash iteration emits both 10 and 20 (order non-deterministic).
    let lines: Vec<&str> = out.trim().split('\n').collect();
    assert_eq!(lines.len(), 2, "got: {out:?}");
    let mut sorted = lines.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["10", "20"], "got: {out:?}");
}

// --- Test 2: rawget with Number-tag from runtime tagged local ---

#[test]
fn rawget_tagged_local_key_number_tag() {
    // box holds an iterator whose values are Number; pairs-body
    // binds them as TaggedValue/Number. rawget(t, k) must dispatch
    // into the Number arm and return t[k].
    let src = r#"
local t = {10, 20, 30}
local box = {}
box.n = 2
for kk, v in pairs(box) do
  print(rawget(t, v))
end
"#;
    let out = run_ok(src, "lumelir_rawget_tagged_num");
    assert_eq!(out, "20\n");
}

// --- Test 3: rawget bypasses __index for TaggedValue key ---

#[test]
fn rawget_tagged_local_key_bypasses_index() {
    // `t.z` (static-String Index) consults __index and returns
    // "from_mt". `rawget(t, k_tagged)` where k_tagged carries
    // String "z" at runtime must NOT consult __index — returns
    // nil. (`t[v]` with TaggedValue key has its own deferral; we
    // sidestep it here.)
    let src = r#"
local fallback = {}
fallback.z = "from_mt"
local mt = {}
mt.__index = fallback
local t = {}
setmetatable(t, mt)
print(t.z)
local box = {}
box.a = "z"
for kk, v in pairs(box) do
  local raw = rawget(t, v)
  if raw == nil then
    print("raw_nil")
  else
    print("raw_present")
  end
end
"#;
    let out = run_ok(src, "lumelir_rawget_tagged_bypass");
    assert_eq!(out, "from_mt\nraw_nil\n");
}
