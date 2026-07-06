//! ADR 0302 — F2-R3: math.type accepts any value, returns nil
//! for non-numbers (Lua 5.4 §6.7).

use std::process::Command;

fn run_ok(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let r = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    assert!(r.status.success(), "binary should exit 0: {r:?}");
    String::from_utf8_lossy(&r.stdout).into_owned()
}

#[test]
fn string_arg_yields_nil() {
    let out = run_ok("print(math.type(\"x\"))", "f2r3_str");
    assert_eq!(out.trim(), "nil");
}

#[test]
fn bool_arg_yields_nil() {
    let out = run_ok("print(math.type(true))", "f2r3_bool");
    assert_eq!(out.trim(), "nil");
}

#[test]
fn table_arg_yields_nil() {
    let out = run_ok(
        "local t = {}
print(math.type(t))",
        "f2r3_table",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn numbers_still_classify() {
    let out = run_ok(
        "print(math.type(1))
print(math.type(1.5))",
        "f2r3_numbers",
    );
    let lines: Vec<&str> = out.trim().split('\n').collect();
    assert_eq!(lines, vec!["integer", "float"]);
}
