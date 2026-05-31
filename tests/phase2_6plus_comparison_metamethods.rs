//! Phase 2.6+-comparison-metamethods (ADR 0144): `__eq` / `__lt` /
//! `__le` for Table-Table comparisons, Function-form only.

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

// --- Test 1: __eq Function-form returns true ---

#[test]
fn eq_via_metamethod_returns_true() {
    let src = r#"
local mt = {}
mt.__eq = function(a, b) return true end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
if x == y then
  print("equal")
else
  print("not_equal")
end
"#;
    let out = run_ok(src, "lumelir_eq_meta_true");
    assert_eq!(out, "equal\n");
}

// --- Test 2: Eq ptr-equality short-circuit (same ptr → true even if __eq missing) ---

#[test]
fn eq_aliased_tables_short_circuit_to_true() {
    // No metatable, but aliased Locals share the same ptr.
    let src = r#"
local a = {}
local b = a
if a == b then
  print("aliased_equal")
else
  print("not_equal")
end
"#;
    let out = run_ok(src, "lumelir_eq_aliased");
    assert_eq!(out, "aliased_equal\n");
}

// --- Test 3: Eq distinct tables, no __eq → false ---

#[test]
fn eq_distinct_tables_without_metamethod_returns_false() {
    let src = r#"
local a = {}
local b = {}
if a == b then
  print("equal")
else
  print("not_equal")
end
"#;
    let out = run_ok(src, "lumelir_eq_no_meta");
    assert_eq!(out, "not_equal\n");
}

// --- Test 4: __lt Function-form ---

#[test]
fn lt_via_metamethod_dispatches() {
    let src = r#"
local mt = {}
mt.__lt = function(a, b) return true end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
if x < y then
  print("less")
else
  print("not_less")
end
"#;
    let out = run_ok(src, "lumelir_lt_meta");
    assert_eq!(out, "less\n");
}

// --- Test 5: __le Function-form ---

#[test]
fn le_via_metamethod_dispatches() {
    let src = r#"
local mt = {}
mt.__le = function(a, b) return true end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
if x <= y then
  print("le")
else
  print("not_le")
end
"#;
    let out = run_ok(src, "lumelir_le_meta");
    assert_eq!(out, "le\n");
}

// --- Test 6: Gt swaps to Lt(rhs, lhs) ---

#[test]
fn gt_swaps_to_lt_via_metamethod() {
    // __lt returns true when called as __lt(b, a) — Gt(a, b) swaps
    // to Lt(b, a). The test source asks "a > b" and expects true.
    let src = r#"
local mt = {}
mt.__lt = function(a, b) return true end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
if x > y then
  print("greater")
else
  print("not_greater")
end
"#;
    let out = run_ok(src, "lumelir_gt_swap");
    assert_eq!(out, "greater\n");
}
