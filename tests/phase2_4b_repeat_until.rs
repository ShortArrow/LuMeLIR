//! Integration test: Phase 2.4b — `repeat <body> until <cond>`
//! loop (ADR 0035).

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
fn repeat_runs_body_until_cond_true() {
    let src = "local i = 0
repeat
  i = i + 1
  print(i)
until i == 3";
    assert_eq!(run(src, "lumelir_24b_basic").trim(), "1\n2\n3");
}

#[test]
fn repeat_runs_body_at_least_once_even_if_cond_already_true() {
    // Distinct from while: the body fires before the cond is checked.
    let src = "local i = 100
repeat
  print(i)
  i = i + 1
until i > 50";
    assert_eq!(run(src, "lumelir_24b_once").trim(), "100");
}

#[test]
fn repeat_cond_sees_body_local() {
    // Lua 5.4 §3.3.4: the until-cond may reference body-declared
    // locals.
    let src = "repeat
  local n = 7
  print(n)
until n == 7";
    assert_eq!(run(src, "lumelir_24b_cond_local").trim(), "7");
}

#[test]
fn repeat_with_break_exits_immediately() {
    let src = "local i = 0
repeat
  i = i + 1
  if i == 2 then break end
  print(i)
until i == 5
print(\"after\")";
    assert_eq!(run(src, "lumelir_24b_break").trim(), "1\nafter");
}

#[test]
fn repeat_inside_user_function() {
    let src = "local function count(n)
  local i = 0
  repeat
    i = i + 1
  until i == n
  return i
end
print(count(5))";
    assert_eq!(run(src, "lumelir_24b_in_fn").trim(), "5");
}

#[test]
fn nested_repeat_loops_break_targets_innermost() {
    let src = "local total = 0
local i = 0
repeat
  i = i + 1
  local j = 0
  repeat
    j = j + 1
    if j == 2 then break end
    total = total + 1
  until j == 99
until i == 3
print(total)";
    // Outer runs 3 times; inner each time prints/sums when j==1
    // then breaks at j==2 → adds 1 per outer iter → total = 3.
    assert_eq!(run(src, "lumelir_24b_nested").trim(), "3");
}

#[test]
fn repeat_cond_uses_external_local() {
    let src = "local stop = 4
local i = 0
repeat
  i = i + 1
until i == stop
print(i)";
    assert_eq!(run(src, "lumelir_24b_external").trim(), "4");
}
