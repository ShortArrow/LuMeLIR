//! Integration test: Phase 2.3b — `if`/`elseif`/`else` + `while` + truthiness.

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
fn if_true_executes_then_body() {
    assert_eq!(
        run("if true then print(1) end", "lumelir_23b_t").trim(),
        "1"
    );
}

#[test]
fn if_false_skips_then_body() {
    assert_eq!(
        run("if false then print(1) end", "lumelir_23b_f").trim(),
        ""
    );
}

#[test]
fn if_else_picks_correct_arm() {
    assert_eq!(
        run(
            "if 1 < 2 then print(true) else print(false) end",
            "lumelir_23b_else"
        )
        .trim(),
        "true"
    );
}

#[test]
fn nil_is_falsy() {
    assert_eq!(
        run("if nil then print(1) else print(2) end", "lumelir_23b_nil").trim(),
        "2"
    );
}

#[test]
fn zero_is_truthy_in_lua() {
    // Lua: `0` is truthy, unlike C/JS. Number-truthiness is a static
    // `arith.constant true`, so we always take the `then` arm.
    assert_eq!(
        run("if 0 then print(true) end", "lumelir_23b_zero").trim(),
        "true"
    );
}

#[test]
fn elseif_chain_picks_middle_arm() {
    let src = "local x = 5\nif x == 1 then print(1) elseif x == 5 then print(5) else print(0) end";
    assert_eq!(run(src, "lumelir_23b_elif").trim(), "5");
}

#[test]
fn while_counts_to_three() {
    let src = "local i = 0\nwhile i < 3 do i = i + 1 end\nprint(i)";
    assert_eq!(run(src, "lumelir_23b_while").trim(), "3");
}

#[test]
fn nested_while_with_inner_if() {
    let src = "local i = 0\nwhile i < 5 do i = i + 1\nif i == 3 then print(i) end end";
    assert_eq!(run(src, "lumelir_23b_nested").trim(), "3");
}
