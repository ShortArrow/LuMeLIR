//! Phase 2.6+-newindex-function-form-number-key (ADR 0169):
//! `t[i] = v` with `mt.__newindex = function(t, k, v) ... end`.

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

// --- Test 1: __newindex Function form fires for Number-key OOB write ---

#[test]
fn newindex_fn_number_key_dispatches() {
    // `t[5] = 7` with i > #t and mt.__newindex = function(t,k,v):
    // capture k+v into a side global `observed`. Outer t left at
    // length 3, t[5] reads nil. Side effect proves the call fired.
    let src = r#"
local observed = {}
local function recorder(t, k, v)
  observed[1] = k
  observed[2] = v
end
recorder({}, 1.0, 1.0)
local mt = {}
mt.__newindex = recorder
local t = {1, 2, 3}
setmetatable(t, mt)
t[5] = 77
print(#t)
print(t[5])
print(observed[1])
print(observed[2])
"#;
    let out = run_ok(src, "lumelir_newidx_fn_numkey_basic");
    assert_eq!(out, "3\nnil\n5\n77\n");
}

// --- Test 2: ADR 0168 Table form still works when both arms could match ---

#[test]
fn newindex_table_form_unchanged_when_fn_candidate_exists() {
    // A `(Table, Number, Number) → ()` candidate exists in the
    // module, but mt.__newindex points to a Table. ADR 0168 Table
    // arm must win.
    let src = r#"
local function noop(t, k, v) end
noop({}, 1.0, 1.0)
local sink = {}
local mt = {}
mt.__newindex = sink
local t = {1, 2, 3}
setmetatable(t, mt)
t[5] = 99
print(sink[5])
print(t[5])
"#;
    let out = run_ok(src, "lumelir_newidx_fn_numkey_table_pin");
    assert_eq!(out, "99\nnil\n");
}
