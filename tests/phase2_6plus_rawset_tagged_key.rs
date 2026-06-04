//! Phase 2.6+-rawset-tagged-key (ADR 0175):
//! `rawset(t, k, v)` with `Local(TaggedValue)` key, non-TaggedValue value.

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

// --- Test 1: pairs-body rawset with String-tag key, literal Number value ---

#[test]
fn rawset_tagged_local_key_string_tag_literal_value() {
    // For each iterated key from src, rawset dst[k] = 99. Both
    // src.x and src.y should land in dst as 99. Bypasses any
    // metatable on dst.
    let src = r#"
local sink = {}
local mt = {}
mt.__newindex = sink
local dst = {}
setmetatable(dst, mt)
local src_tbl = {}
src_tbl.x = 0
src_tbl.y = 0
for k, v in pairs(src_tbl) do
  rawset(dst, k, 99)
end
print(dst.x)
print(dst.y)
print(sink.x)
"#;
    let out = run_ok(src, "lumelir_rawset_tagged_str_lit");
    assert_eq!(out, "99\n99\nnil\n");
}

// --- Test 2: rawset with Number-tag from runtime tagged local ---

#[test]
fn rawset_tagged_local_key_number_tag_literal_value() {
    // box's iterated value is Number at runtime; rawset(t, k, 77)
    // must dispatch via the Number-key sub-arm and land in t's
    // array part.
    let src = r#"
local t = {1, 2, 3}
local box = {}
box.n = 2
for kk, v in pairs(box) do
  rawset(t, v, 77)
end
print(t[2])
"#;
    let out = run_ok(src, "lumelir_rawset_tagged_num_lit");
    assert_eq!(out, "77\n");
}

// --- Test 3: rawset returns t for tagged-key form ---

#[test]
fn rawset_tagged_local_key_returns_table() {
    let src = r#"
local dst = {}
local box = {}
box.k = 1
for kk, v in pairs(box) do
  local r = rawset(dst, kk, "hello")
  print(r.k)
end
"#;
    let out = run_ok(src, "lumelir_rawset_tagged_returns_t");
    assert_eq!(out, "hello\n");
}
