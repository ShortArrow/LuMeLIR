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

// --- Test 3: TaggedValue source on bracketed String key + numeric output ---

#[test]
fn indexassign_tagged_value_bracket_string_key() {
    // Bracketed-key form `t["k"] = v` for the static-String-key
    // arm — same codegen path as `t.k = v` but a different
    // parser shape; pins that the HIR materialisation fires for
    // both syntactic forms.
    let src = r#"
local function pick(b)
  if b then return 42 end
  return nil
end
local t = {}
t["k"] = pick(true)
print(t["k"])
"#;
    let out = run_ok(src, "lumelir_idx_tagged_bracket");
    assert_eq!(out, "42\n");
}

// --- Test 4: Nested IndexTagged target with TaggedValue value (audit pin) ---

#[test]
fn indexassign_nested_indextagged_target_tagged_value() {
    // Nested target `a.inner.x = ...` is widened by ADR 0095 at
    // HIR (Index → IndexTagged on the target). Codegen normalises
    // both shapes through `emit_resolve_table_target_ptr` and
    // shares the post-target match — so the ADR 0187 fix should
    // cover IndexTagged targets too. Probe (commit-less, removed
    // after) confirmed this; this test pins the audit finding so
    // a future refactor cannot silently regress the shared path.
    let src = r#"
local function pick(b)
  if b then return 7 end
  return nil
end
local a = {}
a.inner = {}
a.inner.x = pick(true)
print(a.inner.x)
"#;
    let out = run_ok(src, "lumelir_idx_tagged_nested");
    assert_eq!(out, "7\n");
}

// --- Test 5: Double-nested IndexTagged target with TaggedValue value ---

#[test]
fn indexassign_double_nested_indextagged_target_tagged_value() {
    // Same as above but two levels of nesting. Same code path,
    // different parser shape — pins that IndexTagged dispatch
    // is depth-agnostic for the value-side TaggedValue case.
    let src = r#"
local function pick(b)
  if b then return 5 end
  return nil
end
local a = {}
a.b = {}
a.b.c = {}
a.b.c.x = pick(true)
print(a.b.c.x)
"#;
    let out = run_ok(src, "lumelir_idx_tagged_double_nested");
    assert_eq!(out, "5\n");
}
