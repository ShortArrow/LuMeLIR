//! Integration test: Phase 2.7d — lexicographic comparison
//! (`<` `<=` `>` `>=`) for String operands (ADR 0027).

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
fn lt_returns_true_when_lhs_precedes_rhs() {
    assert_eq!(
        run("print(\"apple\" < \"banana\")", "lumelir_27d_lt_t").trim(),
        "true"
    );
}

#[test]
fn lt_returns_false_when_lhs_follows_rhs() {
    assert_eq!(
        run("print(\"banana\" < \"apple\")", "lumelir_27d_lt_f").trim(),
        "false"
    );
}

#[test]
fn le_returns_true_for_equal_strings() {
    assert_eq!(
        run("print(\"abc\" <= \"abc\")", "lumelir_27d_le_eq").trim(),
        "true"
    );
}

#[test]
fn ge_returns_true_for_equal_strings() {
    assert_eq!(
        run("print(\"abc\" >= \"abc\")", "lumelir_27d_ge_eq").trim(),
        "true"
    );
}

#[test]
fn gt_returns_true_when_lhs_follows_rhs() {
    assert_eq!(
        run("print(\"z\" > \"a\")", "lumelir_27d_gt_t").trim(),
        "true"
    );
}

#[test]
fn lexicographic_compare_in_if_condition() {
    let src = "local s = \"mid\"
if s < \"zzz\" then print(\"less\") else print(\"ge\") end";
    assert_eq!(run(src, "lumelir_27d_if").trim(), "less");
}

#[test]
fn shorter_prefix_is_lexicographically_less() {
    // "ab" < "abc" — strcmp returns < 0 since "ab" is a prefix.
    assert_eq!(
        run("print(\"ab\" < \"abc\")", "lumelir_27d_prefix").trim(),
        "true"
    );
}

#[test]
fn cross_kind_string_vs_number_compare_is_static_error() {
    // Phase 2.7d still rejects mixed-kind comparisons — only
    // String-String and Number-Number are admissible.
    let chunk = lumelir::parser::parse("print(\"a\" < 1)").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn number_ordering_unchanged_after_2_7d() {
    // Regression: existing Number-Number ordering still works.
    assert_eq!(run("print(1 < 2)", "lumelir_27d_regress").trim(), "true");
}
