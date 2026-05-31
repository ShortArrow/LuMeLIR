//! Phase 2.6+-raw-equal-len-builtins (ADR 0137): `rawequal` and
//! `rawlen` builtins (Table operand only).
//!
//! Red Day 0 entry: written before the `Builtin::RawEqual` /
//! `Builtin::RawLen` HIR variants and codegen emit arms exist.
//! Goes Green in C3 when HIR + codegen land.
//!
//! Scope (per ADR 0137):
//!   - `rawequal(t1, t2)` — ptr-equality on two Tables (Lua §3.4.4).
//!   - `rawlen(t)` — i64 length at header offset 0, returned as f64.
//!   - Non-Table operands HIR-rejected; broader operand surface
//!     lands with the `__eq` / `__len` metamethod ADRs.

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

// --- Test 1: rawequal on same Table returns true ---

#[test]
fn rawequal_same_table_is_true() {
    let src = r#"
local t = {}
local r = rawequal(t, t)
if r then
  print("same")
else
  print("different")
end
"#;
    let out = run_ok(src, "lumelir_rawequal_same");
    assert_eq!(out, "same\n");
}

// --- Test 2: rawequal on distinct Tables returns false (Lua §3.4.4) ---

#[test]
fn rawequal_distinct_tables_is_false() {
    let src = r#"
local a = {}
local b = {}
local r = rawequal(a, b)
if r then
  print("same")
else
  print("different")
end
"#;
    let out = run_ok(src, "lumelir_rawequal_distinct");
    assert_eq!(out, "different\n");
}

// --- Test 3: rawequal on alias copies (same ptr) returns true ---

#[test]
fn rawequal_aliased_tables_is_true() {
    let src = r#"
local a = {}
local b = a
local r = rawequal(a, b)
if r then
  print("same")
else
  print("different")
end
"#;
    let out = run_ok(src, "lumelir_rawequal_alias");
    assert_eq!(out, "same\n");
}

// --- Test 4: rawlen on empty table is 0 ---

#[test]
fn rawlen_empty_table_is_zero() {
    let src = r#"
local t = {}
print(rawlen(t))
"#;
    let out = run_ok(src, "lumelir_rawlen_empty");
    assert_eq!(out, "0\n");
}

// --- Test 5: rawlen on array table returns array length ---

#[test]
fn rawlen_array_returns_length() {
    let src = r#"
local t = {10, 20, 30}
print(rawlen(t))
"#;
    let out = run_ok(src, "lumelir_rawlen_array");
    assert_eq!(out, "3\n");
}

// --- Test 6: rawequal non-Table arg is rejected ---

#[test]
fn rawequal_non_table_is_rejected() {
    let src = r#"
local r = rawequal(1, 2)
"#;
    let result = std::panic::catch_unwind(|| compile_and_run(src, "lumelir_rawequal_nontable"));
    match result {
        Err(_) => {}
        Ok(out) => {
            assert!(
                !out.status.success(),
                "Non-Table rawequal must be rejected, but binary exited 0: {out:?}"
            );
        }
    }
}
