//! Phase 2.6+ non-local-tagged residuals (ADR 0188): closes
//! the 4 residual non-Local TaggedValue source dispatcher sites
//! surfaced by a systematic audit. Each routes through
//! `materialize_tagged_source_if_needed` (ADR 0179 chokepoint)
//! at HIR; codegen Local-only checks become defensive.

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

// --- G1: rawset value-position non-Local TaggedValue ---

#[test]
fn rawset_value_non_local_tagged() {
    let src = r#"
local function pick(b)
  if b then return 7 end
  return nil
end
local t = {}
rawset(t, "k", pick(true))
print(rawget(t, "k"))
"#;
    let out = run_ok(src, "lumelir_residual_rawset_v");
    assert_eq!(out, "7\n");
}

// --- G2: Index read with non-Local TaggedValue key ---

#[test]
fn index_read_non_local_tagged_key() {
    let src = r#"
local function pick(src)
  for k, _ in pairs(src) do return k end
end
local other = {}
rawset(other, "name", "z")
local t = {}
rawset(t, "name", 42)
print(t[pick(other)])
"#;
    let out = run_ok(src, "lumelir_residual_idx_read_k");
    assert_eq!(out, "42\n");
}

// --- G3: IndexTagged read with non-Local TaggedValue key (nested target) ---

#[test]
fn indextagged_read_non_local_tagged_key() {
    // Note: rawset on a nested Index target (`rawset(a.inner, ...)`)
    // is rejected at HIR because Index reads default to Number (see
    // `src/hir/mod.rs:180`). Populate via an intermediate Local to
    // keep the focus on the IndexTagged READ key gap.
    let src = r#"
local function pick(src)
  for k, _ in pairs(src) do return k end
end
local other = {}
rawset(other, "name", "y")
local inner = {}
rawset(inner, "name", "deep")
local a = {}
a.inner = inner
print(a.inner[pick(other)])
"#;
    let out = run_ok(src, "lumelir_residual_idxtag_read_k");
    assert_eq!(out, "deep\n");
}

// --- G4: IndexAssign with non-Local TaggedValue key ---

#[test]
fn indexassign_non_local_tagged_key() {
    let src = r#"
local function pick(src)
  for k, _ in pairs(src) do return k end
end
local other = {}
rawset(other, "foo", "x")
local t = {}
t[pick(other)] = 99
print(rawget(t, "foo"))
"#;
    let out = run_ok(src, "lumelir_residual_idxa_k");
    assert_eq!(out, "99\n");
}
