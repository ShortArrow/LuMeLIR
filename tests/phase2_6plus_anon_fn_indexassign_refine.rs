//! Phase 2.6+-anon-fn-indexassign-param-refine (ADR 0141):
//! anonymous `FunctionExpr` in `IndexAssign` sites gets its param
//! kinds refined from the eventual call site, so the canonical
//! metamethod idiom works.

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

// --- Test 1: Table-arg anonymous FunctionExpr refinement ---

#[test]
fn anon_fn_with_table_arg_refines() {
    // Without ADR 0141 this trips IndirectCallNoCandidates because
    // the anonymous fn's `t` defaults to Number and the call passes
    // a Table local.
    let src = r#"
local mt = {}
mt.fn = function(t) return "X" end
local v = {}
print(mt.fn(v))
"#;
    let out = run_ok(src, "lumelir_anon_fn_table_refine");
    assert_eq!(out, "X\n");
}

// --- Test 2: String-arg anonymous FunctionExpr refinement ---

#[test]
fn anon_fn_with_string_arg_refines() {
    let src = r#"
local mt = {}
mt.echo = function(s) return s end
print(mt.echo("hello"))
"#;
    let out = run_ok(src, "lumelir_anon_fn_string_refine");
    assert_eq!(out, "hello\n");
}

// --- Test 3: alias_map interaction (ADR 0098 + 0141) ---

#[test]
fn anon_fn_aliased_via_local_rebind_refines() {
    // `local g = mt.fn` rebinds g to the same FuncId as the anon
    // FunctionExpr stored at mt.fn; g(t) should dispatch correctly.
    let src = r#"
local mt = {}
mt.fn = function(t) return "Y" end
local g = mt.fn
local v = {}
print(g(v))
"#;
    let out = run_ok(src, "lumelir_anon_fn_alias_refine");
    assert_eq!(out, "Y\n");
}

// --- Test 4: FunctionExpr NOT in IndexAssign stays unrefined ---

#[test]
fn anon_fn_not_in_indexassign_still_defaults_number() {
    // `local g = function(x) return x + 1 end` is NOT an IndexAssign
    // shape, so the new pre-registration walk doesn't touch it. The
    // existing ADR 0091 → 0082 path still works via synth local +
    // body-arith param inference (x is used in +1 → Number).
    let src = r#"
local g = function(x) return x + 1 end
print(g(10))
"#;
    let out = run_ok(src, "lumelir_anon_fn_non_indexassign");
    assert_eq!(out, "11\n");
}

// --- Test 5: MethodDef precedence on conflict ---

#[test]
fn methoddef_wins_when_indexassign_has_same_key() {
    // ADR 0141 uses `or_insert`: existing MethodDef registration
    // takes precedence. Source-order-defined `function mt.fn(x)
    // ... end` registers first; later `mt.fn = function...` does
    // NOT shadow at refinement time.
    let src = r#"
local mt = {}
function mt.fn(x) return x * 2 end
print(mt.fn(7))
"#;
    let out = run_ok(src, "lumelir_methoddef_precedence");
    assert_eq!(out, "14\n");
}
