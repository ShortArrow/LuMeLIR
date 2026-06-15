//! ADR 0198 — `next(t)` arity 1: second arg defaults to nil per
//! Lua 5.4 §6.1. Multi-assign call form (codegen path per ADR
//! 0081); single-value position remains rejected.

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
fn next_arity_1_multi_assign_yields_first_key_value() {
    let src = r#"
local t = {}
rawset(t, "a", 1)
local k, v = next(t)
print(k, v)
"#;
    assert_eq!(run_ok(src, "lumelir_next_arity_1").trim(), "a\t1");
}
