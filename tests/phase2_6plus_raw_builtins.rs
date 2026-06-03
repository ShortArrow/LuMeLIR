//! Phase 2.6+-raw-set-get-builtins (ADR 0136): `rawset` and
//! `rawget` builtins (hash key only).
//!
//! Red Day 0 entry: written before the `Builtin::RawSet` /
//! `Builtin::RawGet` HIR variants and codegen emit arms exist.
//! Goes Green in C3 when HIR + the `skip_metatable` flag on
//! `emit_hash_indexassign_with_newindex` land.
//!
//! Scope (per ADR 0136):
//!   - `rawset(t, k, v)` and `rawget(t, k)` with hash-eligible
//!     keys (String / Bool / Function / Table).
//!   - Bypass `__newindex` / `__index` metamethod chains.
//!   - Number-key forms HIR-rejected.
//!   - Nil-value `rawset` HIR-rejected (use `t[k] = nil`).

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

// --- Test 1: rawset bypasses __newindex ---

#[test]
fn rawset_bypasses_newindex_metamethod() {
    let src = r#"
local storage = {}
local mt = {}
mt.__newindex = storage
local t = {}
setmetatable(t, mt)
rawset(t, "x", 42)
print(t.x)
if storage.x == nil then
  print("storage_untouched")
else
  print("storage_touched")
end
"#;
    let out = run_ok(src, "lumelir_rawset_bypass_newindex");
    assert_eq!(out, "42\nstorage_untouched\n");
}

// --- Test 2: rawget bypasses __index ---

#[test]
fn rawget_bypasses_index_metamethod() {
    let src = r#"
local fallback = {}
fallback.x = 999
local mt = {}
mt.__index = fallback
local t = {}
setmetatable(t, mt)
print(t.x)
local r = rawget(t, "x")
if r == nil then
  print("raw_nil")
else
  print("raw_present")
end
"#;
    let out = run_ok(src, "lumelir_rawget_bypass_index");
    assert_eq!(out, "999\nraw_nil\n");
}

// --- Test 3: rawset returns its first argument ---

#[test]
fn rawset_returns_table_argument() {
    let src = r#"
local t = {}
local r = rawset(t, "k", "v")
r.other = "w"
print(t.other)
print(t.k)
"#;
    let out = run_ok(src, "lumelir_rawset_returns_table");
    assert_eq!(out, "w\nv\n");
}

// --- Test 4: rawget returns nil for missing key (no trap) ---

#[test]
fn rawget_returns_nil_on_missing_key() {
    let src = r#"
local t = {}
t.present = "yes"
local absent = rawget(t, "absent")
if absent == nil then
  print("nil_ok")
else
  print("unexpected")
end
print(rawget(t, "present"))
"#;
    let out = run_ok(src, "lumelir_rawget_nil_on_miss");
    assert_eq!(out, "nil_ok\nyes\n");
}

// --- Test 5: rawset with no metatable writes directly ---

#[test]
fn rawset_without_metatable_is_direct_write() {
    let src = r#"
local t = {}
rawset(t, "k", 7)
print(t.k)
"#;
    let out = run_ok(src, "lumelir_rawset_no_mt");
    assert_eq!(out, "7\n");
}

// --- Test 6: rawget without metatable reads directly ---

#[test]
fn rawget_without_metatable_is_direct_read() {
    let src = r#"
local t = {}
t.k = 11
print(rawget(t, "k"))
"#;
    let out = run_ok(src, "lumelir_rawget_no_mt");
    assert_eq!(out, "11\n");
}

// --- Test 7: rawset Number-key — ADR 0136 deferral closed by ADR 0172 ---

#[test]
fn rawset_number_key_is_accepted() {
    // ADR 0136 originally HIR-rejected this; ADR 0172 wires it to
    // the raw array-write primitive (`emit_array_index_assign_at`).
    let src = r#"
local t = {}
rawset(t, 1, 100)
print(t[1])
"#;
    let out = compile_and_run(src, "lumelir_rawset_number_key_ok");
    assert!(
        out.status.success(),
        "rawset Number-key now accepted: {out:?}"
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "100\n");
}

// --- Test 8: rawset Nil value is HIR-rejected ---

#[test]
fn rawset_nil_value_is_rejected() {
    let src = r#"
local t = {}
t.k = 1
rawset(t, "k", nil)
"#;
    let result = std::panic::catch_unwind(|| compile_and_run(src, "lumelir_rawset_nil_value"));
    match result {
        Err(_) => {}
        Ok(out) => {
            assert!(
                !out.status.success(),
                "Nil-value rawset must be rejected (HIR or runtime), but binary exited 0: {out:?}"
            );
        }
    }
}
