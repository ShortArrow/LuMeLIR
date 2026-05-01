//! Integration test: Phase 2.7e — `tonumber` builtin with NaN
//! sentinel on parse failure (ADR 0028).

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
fn tonumber_integer_string() {
    assert_eq!(
        run("print(tonumber(\"42\"))", "lumelir_27e_int").trim(),
        "42"
    );
}

#[test]
fn tonumber_decimal_string() {
    assert_eq!(
        run("print(tonumber(\"1.25\"))", "lumelir_27e_dec").trim(),
        "1.25"
    );
}

#[test]
fn tonumber_scientific_string() {
    assert_eq!(
        run("print(tonumber(\"1e3\"))", "lumelir_27e_e").trim(),
        "1000"
    );
}

#[test]
fn tonumber_negative_string() {
    let src = "local n = tonumber(\"-2.5\")
print(n + 1)";
    assert_eq!(run(src, "lumelir_27e_neg").trim(), "-1.5");
}

#[test]
fn tonumber_failure_yields_nan_sentinel() {
    // NaN sentinel: `x == x` is the canonical failure check.
    let src = "local x = tonumber(\"oops\")
if x == x then print(\"ok\") else print(\"nan\") end";
    assert_eq!(run(src, "lumelir_27e_nan").trim(), "nan");
}

#[test]
fn tonumber_of_number_is_identity() {
    assert_eq!(run("print(tonumber(99))", "lumelir_27e_id").trim(), "99");
}

#[test]
fn tonumber_in_arithmetic_chain() {
    let src = "print(tonumber(\"7\") * tonumber(\"6\"))";
    assert_eq!(run(src, "lumelir_27e_chain").trim(), "42");
}

#[test]
fn tonumber_of_bool_is_static_error() {
    let chunk = lumelir::parser::parse("print(tonumber(true))").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn tonumber_of_nil_is_static_error() {
    let chunk = lumelir::parser::parse("print(tonumber(nil))").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn tonumber_of_function_value_is_static_error() {
    let chunk = lumelir::parser::parse(
        "local f = function() return 1 end
print(tonumber(f))",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}
