//! Integration test: Phase 2.7j — long-bracket strings and
//! level-N block comments (ADR 0038).

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
fn long_bracket_string_basic() {
    assert_eq!(run("print([[hello]])", "lumelir_27j_basic").trim(), "hello");
}

#[test]
fn long_bracket_strips_leading_newline() {
    let src = "print([[\nhello]])";
    assert_eq!(run(src, "lumelir_27j_lead_nl").trim(), "hello");
}

#[test]
fn long_bracket_preserves_internal_newlines() {
    let src = "print([[\nfirst\nsecond]])";
    // Leading \n stripped → "first\nsecond". `print` adds the
    // trailing \n to give "first\nsecond\n".
    let stdout = run(src, "lumelir_27j_internal_nl");
    assert_eq!(stdout, "first\nsecond\n");
}

#[test]
fn level1_long_bracket_allows_double_close_inside() {
    // `]]` inside the level-1 body is preserved verbatim.
    let src = "print([=[has ]] inside]=])";
    assert_eq!(run(src, "lumelir_27j_level1").trim(), "has ]] inside");
}

#[test]
fn level2_long_bracket() {
    // Level-2 brackets accept any single `=` chains inside. Compare
    // the raw stdout (including the trailing `\n` `print` adds) so
    // the body's leading/trailing spaces are preserved.
    let src = "print([==[ a ]=] b ]==])";
    assert_eq!(run(src, "lumelir_27j_level2"), " a ]=] b \n");
}

#[test]
fn long_bracket_no_escape_processing() {
    // `\n` in a long-bracket string is literal backslash + n,
    // not LF — escape processing is for short strings only.
    let src = "print([[a\\nb]])";
    assert_eq!(run(src, "lumelir_27j_no_esc").trim(), "a\\nb");
}

#[test]
fn long_bracket_string_concat_with_short() {
    let src = "print([[long]] .. \"-short\")";
    assert_eq!(run(src, "lumelir_27j_concat").trim(), "long-short");
}

#[test]
fn long_bracket_string_length_via_hash() {
    let src = "print(#[[hello]])";
    assert_eq!(run(src, "lumelir_27j_len").trim(), "5");
}

#[test]
fn level1_block_comment_around_double_close_in_body() {
    let src = "--[=[
   contains ]] inside which would close a level-0 form
]=]
print(\"survived\")";
    assert_eq!(run(src, "lumelir_27j_l1_comment").trim(), "survived");
}

#[test]
fn block_comment_around_print_call_using_long_string() {
    // Mixed: a long comment wrapping a print using a long string.
    let src = "--[==[
print([[never run]])
]==]
print([[done]])";
    assert_eq!(run(src, "lumelir_27j_mixed").trim(), "done");
}

#[test]
fn long_bracket_inside_user_function() {
    let src = "local function greet() return [[hi there]] end
print(greet())";
    assert_eq!(run(src, "lumelir_27j_in_fn").trim(), "hi there");
}

#[test]
fn unterminated_long_bracket_string_is_lex_error() {
    assert!(lumelir::parser::parse("print([[oops").is_err());
}
