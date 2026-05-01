//! Integration test: Phase 2.8c — block comments `--[[ ... ]]`
//! (ADR 0034).

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
fn block_comment_around_statement_skipped() {
    let src = "--[[ disabled
    print(\"never\")
]]
print(\"alive\")";
    assert_eq!(run(src, "lumelir_28c_around").trim(), "alive");
}

#[test]
fn block_comment_inline_in_expression() {
    let src = "print(1 --[[ between operands ]] + 2)";
    assert_eq!(run(src, "lumelir_28c_expr").trim(), "3");
}

#[test]
fn block_comment_spanning_multiple_lines_inside_function() {
    let src = "local function f(x)
  --[[ doc:
       returns x doubled
       (multiline rationale)
  ]]
  return x * 2
end
print(f(5))";
    assert_eq!(run(src, "lumelir_28c_fn").trim(), "10");
}

#[test]
fn block_comment_only_file_compiles() {
    let src = "--[[
empty program with header comment
]]";
    assert_eq!(run(src, "lumelir_28c_only").trim(), "");
}

#[test]
fn line_and_block_comments_can_coexist() {
    let src = "-- single
--[[ block
spanning lines
]]
-- another single
print(\"both ok\")";
    assert_eq!(run(src, "lumelir_28c_mix").trim(), "both ok");
}

#[test]
fn unterminated_block_comment_is_lex_error() {
    assert!(lumelir::parser::parse("--[[ no close").is_err());
}
