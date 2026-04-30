//! Integration test: Phase 2.5d — multi-value `return` and multi-
//! binding `local` (ADR 0021).

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
fn multi_return_two_numbers_bound_to_two_locals() {
    let src = "local function pair() return 10, 20 end
local a, b = pair()
print(a)
print(b)";
    assert_eq!(run(src, "lumelir_25d_pair").trim(), "10\n20");
}

#[test]
fn multi_return_three_values() {
    let src = "local function triple() return 1, 2, 3 end
local a, b, c = triple()
print(a + b + c)";
    assert_eq!(run(src, "lumelir_25d_triple").trim(), "6");
}

#[test]
fn multi_return_with_arithmetic() {
    let src = "local function divmod(n, d) return n / d, n - d end
local q, r = divmod(7, 2)
print(q)
print(r)";
    assert_eq!(run(src, "lumelir_25d_divmod").trim(), "3.5\n5");
}

#[test]
fn parallel_binding_from_literals() {
    let src = "local a, b, c = 1, 2, 3
print(a)
print(b)
print(c)";
    assert_eq!(run(src, "lumelir_25d_par").trim(), "1\n2\n3");
}

#[test]
fn arity_mismatch_in_multi_binding_is_static_error() {
    // 3 LHS, 2-result call.
    let chunk = lumelir::parser::parse(
        "local function pair() return 1, 2 end
local a, b, c = pair()",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn inconsistent_return_arity_in_function_is_static_error() {
    let chunk = lumelir::parser::parse(
        "local function bad(x)
  if x > 0 then return 1, 2 end
  return 3
end",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn single_return_unchanged_after_2_5d() {
    let src = "local function sq(x) return x * x end
print(sq(7))";
    assert_eq!(run(src, "lumelir_25d_regress").trim(), "49");
}

#[test]
fn multi_call_truncated_to_first_result_in_expression_position() {
    // `local x = pair()` takes only the first result (Lua truncation).
    let src = "local function pair() return 10, 20 end
local x = pair()
print(x)";
    assert_eq!(run(src, "lumelir_25d_trunc").trim(), "10");
}
