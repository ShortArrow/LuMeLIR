//! ADR 0283 — N4-F-1: `string.find(s, pat, init)` 3-arg form.
//! Foundation for gmatch manual-loop iteration. Closure-form
//! `string.gmatch` deferred to N4-F-2.

use std::process::Command;

fn run_ok(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let r = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    assert!(r.status.success(), "binary should exit 0: {r:?}");
    String::from_utf8_lossy(&r.stdout).into_owned()
}

#[test]
fn find_with_init_skips_earlier_match() {
    // "abcabc": pos of first 'b' is 2; with init=3 it's 5.
    let out = run_ok("print(string.find(\"abcabc\", \"b\", 3))", "n4f_init_skip");
    assert_eq!(out.trim(), "5");
}

#[test]
fn find_init_no_match_returns_nil() {
    let out = run_ok(
        "local r = string.find(\"abcdef\", \"%d\", 1)
if r == nil then print(\"nil\") else print(r) end",
        "n4f_init_nil",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn find_init_1_matches_first() {
    let out = run_ok("print(string.find(\"42xx42\", \"%d+\", 1))", "n4f_init_1");
    assert_eq!(out.trim(), "1");
}

#[test]
fn find_init_past_first_finds_second() {
    let out = run_ok(
        "print(string.find(\"42xx42\", \"%d+\", 3))",
        "n4f_init_past",
    );
    assert_eq!(out.trim(), "5");
}

#[test]
fn find_init_with_multi_return() {
    let out = run_ok(
        "local a, b = string.find(\"hello world\", \"world\", 1)
print(a)
print(b)",
        "n4f_init_multi",
    );
    let lines: Vec<&str> = out.trim().split('\n').collect();
    assert_eq!(lines, vec!["7", "11"]);
}

#[test]
fn find_init_enables_manual_gmatch_loop() {
    // Pattern `%d+` repeatedly applied via init = end + 1.
    let out = run_ok(
        "local s = \"1 22 333\"
local pos = 1
while true do
    local a, b = string.find(s, \"%d+\", pos)
    if a == nil then break end
    print(string.sub(s, a, b))
    pos = b + 1
end",
        "n4f_manual_gmatch",
    );
    let lines: Vec<&str> = out.trim().split('\n').collect();
    assert_eq!(lines, vec!["1", "22", "333"]);
}
