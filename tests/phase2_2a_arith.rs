//! Integration test: Phase 2.2a — full arithmetic operator set.

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
fn precedence_mul_over_add() {
    assert_eq!(run("print(2 * 3 + 4)", "lumelir_22a_prec1").trim(), "10");
}

#[test]
fn subtraction_works() {
    assert_eq!(run("print(10 - 7)", "lumelir_22a_sub").trim(), "3");
}

#[test]
fn division_is_float() {
    // 7 / 2 == 3.5 (Lua's `/` is always float).
    assert_eq!(run("print(7 / 2)", "lumelir_22a_div").trim(), "3.5");
}

#[test]
fn pow_right_associative() {
    // 2 ^ 3 ^ 2 == 2 ^ 9 == 512
    assert_eq!(run("print(2 ^ 3 ^ 2)", "lumelir_22a_pow").trim(), "512");
}

#[test]
fn unary_minus_works() {
    assert_eq!(run("print(-5 + 3)", "lumelir_22a_neg").trim(), "-2");
}

#[test]
fn lua_modulo_follows_divisor_sign() {
    // Lua 5 % -3 == -1 (floor modulo, sign of divisor).
    assert_eq!(run("print(5 % -3)", "lumelir_22a_mod").trim(), "-1");
}
