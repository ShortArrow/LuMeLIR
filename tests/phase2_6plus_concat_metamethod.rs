//! Phase 2.6+-concat-metamethod (ADR 0143): `Table .. Table`
//! consults `lhs.metatable.__concat` and dispatches via the ADR
//! 0142 `emit_dispatch_chain_from_slot_ptr` helper.

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

// --- Test 1: __concat Function-form dispatches ---

#[test]
fn concat_table_table_via_metamethod() {
    let src = r#"
local mt = {}
mt.__concat = function(a, b) return "joined" end
local t = setmetatable({}, mt)
print(t .. t)
"#;
    let out = run_ok(src, "lumelir_concat_meta_basic");
    assert_eq!(out, "joined\n");
}

// --- Test 2: No metatable → trap ---

#[test]
fn concat_table_table_without_metatable_traps() {
    let src = r#"
local a = {}
local b = {}
print(a .. b)
"#;
    let out = compile_and_run(src, "lumelir_concat_no_mt");
    assert!(
        !out.status.success(),
        "concat Table .. Table without metatable must trap: {out:?}"
    );
}

// --- Test 3: Metatable without __concat → trap ---

#[test]
fn concat_table_table_without_concat_field_traps() {
    let src = r#"
local mt = {}
mt.k = 1
local t = setmetatable({}, mt)
print(t .. t)
"#;
    let out = compile_and_run(src, "lumelir_concat_no_field");
    assert!(
        !out.status.success(),
        "concat with metatable missing __concat must trap: {out:?}"
    );
}

// --- Test 4: __concat result is concatenable with String ---

#[test]
fn concat_metamethod_result_chains_with_string() {
    // print(("prefix" .. (t .. t))) should produce "prefix" + meta
    // result. The metamethod returns a String, then a normal
    // String..String concat finishes the job.
    let src = r#"
local mt = {}
mt.__concat = function(a, b) return "X" end
local t = setmetatable({}, mt)
print("[" .. (t .. t) .. "]")
"#;
    let out = run_ok(src, "lumelir_concat_chain");
    assert_eq!(out, "[X]\n");
}
