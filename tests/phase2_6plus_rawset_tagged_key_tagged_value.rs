//! Phase 2.6+-rawset-tagged-key-tagged-value (ADR 0176):
//! `rawset(t, k, v)` with both key + value Local(TaggedValue).

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

// --- Test 1: canonical pairs-body shape with both key + value tagged ---

#[test]
fn rawset_tagged_key_tagged_value_pairs_body_string_keys() {
    // Copy src into dst via rawset(dst, k, v) inside pairs body.
    // mt.__newindex must NOT fire (rawset bypasses).
    let src = r#"
local sink = {}
local mt = {}
mt.__newindex = sink
local dst = {}
setmetatable(dst, mt)
local src_tbl = {}
src_tbl.x = 10
src_tbl.y = 20
for k, v in pairs(src_tbl) do
  rawset(dst, k, v)
end
print(dst.x)
print(dst.y)
print(sink.x)
"#;
    let out = run_ok(src, "lumelir_rawset_tk_tv_pairs_str");
    assert_eq!(out, "10\n20\nnil\n");
}

// --- Test 2: pairs-body shape with Number-tag key + tagged value ---

#[test]
fn rawset_tagged_key_tagged_value_pairs_body_number_keys() {
    // src is array {1,2,3}; pairs iterates (1,1), (2,2), (3,3).
    // rawset(dst, k, v) with k carrying Number tag at runtime
    // must dispatch via the Number-key sub-arm AND the value
    // (also tagged) must be slot-copied into dst's array.
    let src = r#"
local dst = {}
local src_tbl = {1, 2, 3}
for k, v in pairs(src_tbl) do
  rawset(dst, k, v)
end
print(dst[1])
print(dst[2])
print(dst[3])
print(#dst)
"#;
    let out = run_ok(src, "lumelir_rawset_tk_tv_pairs_num");
    assert_eq!(out, "1\n2\n3\n3\n");
}
