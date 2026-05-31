//! Phase 2.6+-tostring-metamethod (ADR 0142): `tostring(t)` for a
//! Table operand consults `mt.__tostring` and dispatches via the
//! existing ADR 0082 IndirectDispatch closure-cell chain.

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

// --- Test 1: __tostring Function-form dispatches ---

#[test]
fn tostring_consults_metatable_tostring() {
    let src = r#"
local mt = {}
mt.__tostring = function(t) return "Vec(1,2)" end
local t = {}
setmetatable(t, mt)
print(tostring(t))
"#;
    let out = run_ok(src, "lumelir_tostring_meta_basic");
    assert_eq!(out, "Vec(1,2)\n");
}

// --- Test 2: no metatable → "table" literal ---

#[test]
fn tostring_no_metatable_returns_table_literal() {
    let src = r#"
local t = {}
print(tostring(t))
"#;
    let out = run_ok(src, "lumelir_tostring_no_mt");
    assert_eq!(out, "table\n");
}

// --- Test 3: metatable without __tostring → "table" literal ---

#[test]
fn tostring_metatable_without_field_returns_table_literal() {
    let src = r#"
local mt = {}
mt.k = 1
local t = {}
setmetatable(t, mt)
print(tostring(t))
"#;
    let out = run_ok(src, "lumelir_tostring_mt_no_field");
    assert_eq!(out, "table\n");
}

// --- Test 4: __tostring set to non-Function → "table" literal ---

#[test]
fn tostring_non_function_metafield_returns_table_literal() {
    // Lua spec actually permits non-Function __tostring (returns the
    // string directly). Our scope is Function-form only; non-Function
    // falls back to "table".
    let src = r#"
local mt = {}
mt.__tostring = "ignored"
local t = {}
setmetatable(t, mt)
print(tostring(t))
"#;
    let out = run_ok(src, "lumelir_tostring_nonfn_metafield");
    assert_eq!(out, "table\n");
}

// --- Test 5: multiple candidates dispatch correctly ---

#[test]
fn tostring_multi_candidate_dispatch_picks_correct() {
    // Two user fns with sig (Table) → String. The metatable points
    // at the second one; runtime dispatch via closure ptr comparison
    // picks the right candidate.
    let src = r#"
local mt = {}
mt.__tostring = function(t) return "B" end
local t = {}
setmetatable(t, mt)
print(tostring(t))
"#;
    // Force a second (Table) → String candidate to exist in the
    // module by writing another anon FunctionExpr stored in a table
    // slot — even though we don't call it directly, its FuncId
    // enters the candidate set.
    let src_with_other_candidate = format!(
        "{}\nlocal other_mt = {{}}\nother_mt.fn = function(x) return \"A\" end\n{}",
        "", src,
    );
    let out = run_ok(&src_with_other_candidate, "lumelir_tostring_multi_cand");
    assert_eq!(out, "B\n");
}
