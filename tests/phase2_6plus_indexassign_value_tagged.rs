//! Phase 2.6+ indexassign-value-tagged (ADR 0187): IndexAssign
//! accepts TaggedValue values on non-Number keys. HIR
//! materialises non-Local TaggedValue sources into a synth Local
//! (reuses the ADR 0179 helper); codegen's static-key match adds
//! a `ValueKind::TaggedValue` arm symmetric to ADR 0138-M's
//! TaggedValue-key arm.

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

// --- Test 1: Local TaggedValue value source on String key ---

#[test]
fn indexassign_local_tagged_value_string_key() {
    let src = r#"
local function pick(b)
  if b then return 1 end
  return nil
end
local t = {}
local v = pick(true)
t.x = v
print(t.x)
"#;
    let out = run_ok(src, "lumelir_idx_tagged_local");
    assert_eq!(out, "1\n");
}

// --- Test 2: Non-Local (Call-return) TaggedValue value source on String key ---

#[test]
fn indexassign_call_tagged_value_string_key() {
    let src = r#"
local function pick(b)
  if b then return 1 end
  return nil
end
local t = {}
t.x = pick(true)
print(t.x)
"#;
    let out = run_ok(src, "lumelir_idx_tagged_call");
    assert_eq!(out, "1\n");
}

// --- Test 3: TaggedValue source whose runtime tag is Nil (hash-delete via TaggedValue) ---

#[test]
fn indexassign_tagged_value_nil_runtime_tag_deletes() {
    // First set t.x to a real value, then overwrite via a
    // TaggedValue source whose runtime tag turns out to be Nil.
    // Lua 5.4: `t.x = nil` is a hash-delete; the codegen slot
    // copy must propagate the Nil tag through
    // `emit_hash_indexassign_with_newindex` so the read after
    // returns nil.
    let src = r#"
local function pick(b)
  if b then return 7 end
  return nil
end
local t = {}
t.x = 99
local v = pick(false)
t.x = v
if t.x == nil then print("nil") else print("non-nil") end
"#;
    let out = run_ok(src, "lumelir_idx_tagged_runtime_nil");
    assert_eq!(out, "nil\n");
}
