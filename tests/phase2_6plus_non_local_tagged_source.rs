//! Phase 2.6+ non-local-tagged-source (ADR 0179):
//! HIR materialises non-Local TaggedValue (Call-return) source
//! into a synth local at RawGet/RawSet builtin-arg positions.

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

// pick_key / pick_val are upvalue-capturing closures returning a
// TaggedValue (Local from the pairs-body) — used to produce a
// Call expression of TaggedValue kind at the rawget/rawset arg
// position. Parameters default to Number kind in HIR so the
// helper cannot take the source table as an arg; we capture
// the source as an upvalue instead.

// --- Test 1: rawget(target, pick_key()) — Call-return TaggedValue as key ---

#[test]
fn rawget_with_call_return_tagged_key() {
    let src = r#"
local other = {}
other.foo = "anything"

local function pick_key()
  for k, v in pairs(other) do
    return k
  end
end

local target = {}
target.foo = 42

local got = rawget(target, pick_key())
print(got)
"#;
    let out = run_ok(src, "lumelir_rawget_call_return_tagged");
    assert_eq!(out, "42\n");
}

// --- Test 2: rawset(target, pick_key(), v) — Call-return TaggedValue as key ---

#[test]
fn rawset_with_call_return_tagged_key() {
    let src = r#"
local other = {}
other.foo = "anything"

local function pick_key()
  for k, v in pairs(other) do
    return k
  end
end

local target = {}
rawset(target, pick_key(), 99)
print(target.foo)
"#;
    let out = run_ok(src, "lumelir_rawset_call_return_tagged_key");
    assert_eq!(out, "99\n");
}

// --- Test 3: rawset(target, k_local, pick_val()) — Call-return TaggedValue as value ---

#[test]
fn rawset_with_call_return_tagged_value() {
    let src = r#"
local other = {}
other.foo = 77

local function pick_val()
  for k, v in pairs(other) do
    return v
  end
end

local src_iter = {}
src_iter.theKey = "key_payload"

local target = {}
for k, v in pairs(src_iter) do
  rawset(target, k, pick_val())
end
print(target.theKey)
"#;
    let out = run_ok(src, "lumelir_rawset_call_return_tagged_value");
    assert_eq!(out, "77\n");
}
