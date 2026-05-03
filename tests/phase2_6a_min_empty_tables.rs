//! Integration test: Phase 2.6a-min — empty table `{}` and `#t`
//! (ADR 0053). Header layout: `[length: i64]` only, 8 bytes,
//! malloc'd. Tables are reference values represented as `!llvm.ptr`.

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    assert!(result.status.success(), "binary should exit 0");
    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
    let _ = std::fs::remove_file(&output);
    stdout
}

#[test]
fn empty_table_compiles_and_runs() {
    let src = "local t = {}
print(\"ok\")";
    assert_eq!(run(src, "lumelir_26a_compiles").trim(), "ok");
}

#[test]
fn empty_table_length_is_zero() {
    let src = "local t = {}
print(#t)";
    assert_eq!(run(src, "lumelir_26a_len").trim(), "0");
}

#[test]
fn multiple_empty_tables_are_independent_pointers() {
    // Each `{}` allocates a fresh table — both have length 0
    // and the binary doesn't crash from aliasing.
    let src = "local a = {}
local b = {}
print(#a)
print(#b)";
    assert_eq!(run(src, "lumelir_26a_two"), "0\n0\n");
}

#[test]
fn empty_table_literal_arg_param_kind_inferred() {
    // Param-kind back-inference (2.5e) sees `{}` as the first call
    // site arg and refines `t` to Table. A bare ident arg
    // (`take(x)`) doesn't infer to Table yet — the Ident path falls
    // to Number — that's a known limitation tracked for a future
    // phase.
    let src = "local function take(t) return #t end
print(take({}))";
    assert_eq!(run(src, "lumelir_26a_arg").trim(), "0");
}

#[test]
fn empty_table_alias_shares_storage() {
    // Reference semantics: `local b = a` aliases the same table.
    // (The length is 0 for both — non-trivial reference behaviour
    // appears once write indexing arrives.)
    let src = "local a = {}
local b = a
print(#a)
print(#b)";
    assert_eq!(run(src, "lumelir_26a_alias"), "0\n0\n");
}

#[test]
fn string_length_still_works_after_2_6a() {
    // Regression: 2.7a's `#"hello"` string-length path is
    // unaffected by the new Table-aware `#` arm.
    assert_eq!(run("print(#\"hello\")", "lumelir_26a_strlen").trim(), "5");
}
