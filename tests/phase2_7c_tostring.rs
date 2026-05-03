//! Integration test: Phase 2.7c — `tostring` builtin and concat
//! auto-coerce (ADR 0026).

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
fn tostring_integer_drops_trailing_zero() {
    // Lua-spec %.14g: `42` not `42.000000`.
    assert_eq!(run("print(tostring(42))", "lumelir_27c_int").trim(), "42");
}

#[test]
fn tostring_decimal_keeps_fraction() {
    assert_eq!(
        run("print(tostring(1.25))", "lumelir_27c_dec").trim(),
        "1.25"
    );
}

#[test]
fn tostring_true_returns_true_word() {
    assert_eq!(
        run("print(tostring(true))", "lumelir_27c_true").trim(),
        "true"
    );
}

#[test]
fn tostring_false_returns_false_word() {
    assert_eq!(
        run("print(tostring(false))", "lumelir_27c_false").trim(),
        "false"
    );
}

#[test]
fn tostring_nil_returns_nil_word() {
    assert_eq!(run("print(tostring(nil))", "lumelir_27c_nil").trim(), "nil");
}

#[test]
fn tostring_string_is_identity() {
    assert_eq!(
        run("print(tostring(\"hi\"))", "lumelir_27c_str").trim(),
        "hi"
    );
}

#[test]
fn concat_auto_coerce_number() {
    assert_eq!(
        run("print(\"count: \" .. 5)", "lumelir_27c_concat_num").trim(),
        "count: 5"
    );
}

#[test]
fn concat_auto_coerce_bool() {
    assert_eq!(
        run("print(\"flag: \" .. true)", "lumelir_27c_concat_bool").trim(),
        "flag: true"
    );
}

#[test]
fn concat_auto_coerce_nil() {
    assert_eq!(
        run("print(\"value: \" .. nil)", "lumelir_27c_concat_nil").trim(),
        "value: nil"
    );
}

#[test]
fn concat_number_then_string() {
    // The lhs of `..` can also auto-coerce.
    assert_eq!(
        run("print(7 .. \" days\")", "lumelir_27c_lhs_num").trim(),
        "7 days"
    );
}

#[test]
fn tostring_in_user_function_returning_string() {
    let src = "local function show(n) return \"n=\" .. tostring(n) end
print(show(42))";
    assert_eq!(run(src, "lumelir_27c_user").trim(), "n=42");
}

#[test]
fn tostring_of_function_value_now_succeeds_after_2_7n() {
    // Phase 2.7c rejected Function-kind tostring; Phase 2.7n
    // (ADR 0052) lifts that — `tostring(f)` returns "function".
    // See `tests/phase2_7n_tostring_function.rs` for full coverage.
    let chunk = lumelir::parser::parse(
        "local f = function() return 1 end
print(tostring(f))",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_ok());
}
