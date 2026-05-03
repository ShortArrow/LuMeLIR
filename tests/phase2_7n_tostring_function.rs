//! Integration test: Phase 2.7n — `tostring(f)` for Function values
//! returns the literal `"function"` (ADR 0052). Builds on Phase 2.7c.

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
fn tostring_anonymous_function_returns_literal() {
    assert_eq!(
        run(
            "print(tostring(function(x) return x end))",
            "lumelir_27n_anon",
        )
        .trim(),
        "function"
    );
}

#[test]
fn tostring_local_function_value_returns_literal() {
    let src = "local f = function() end
print(tostring(f))";
    assert_eq!(run(src, "lumelir_27n_local").trim(), "function");
}

#[test]
fn tostring_local_function_def_returns_literal() {
    let src = "local function g() end
print(tostring(g))";
    assert_eq!(run(src, "lumelir_27n_localdef").trim(), "function");
}

#[test]
fn tostring_function_in_concat() {
    // Concat triggers tostring auto-coerce for non-string operands
    // (Phase 2.7c). With Function now in tostring's domain, this
    // mirrors the path for Number/Bool.
    let src = "local f = function() end
print(\"got: \" .. tostring(f))";
    assert_eq!(run(src, "lumelir_27n_concat").trim(), "got: function");
}

#[test]
fn tostring_number_still_works_after_2_7n() {
    // Regression: existing 2.7c numeric path unaffected.
    assert_eq!(run("print(tostring(42))", "lumelir_27n_num").trim(), "42");
}

#[test]
fn type_function_still_works_after_2_7n() {
    // Regression: 2.7f's `type(f)` keeps returning "function".
    let src = "local f = function() end
print(type(f))";
    assert_eq!(run(src, "lumelir_27n_type").trim(), "function");
}
