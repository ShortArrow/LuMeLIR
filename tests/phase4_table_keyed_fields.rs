//! ADR 0199 — Lua 5.4 §3.4.9 table constructor keyed forms:
//!   - `{[k]=v}`    bracket-key
//!   - `{name=v}`   named-key (sugar for `["name"]=v`)
//!   - mixed positional + keyed
//!   - trailing-comma + keyed

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
fn table_bracket_key_number() {
    let src = "local t = {[1]=99}; print(t[1])";
    assert_eq!(run_ok(src, "lumelir_tbl_bk").trim(), "99");
}

#[test]
fn table_named_key_string_sugar() {
    let src = r#"local t = {name="x"}; print(t.name)"#;
    assert_eq!(run_ok(src, "lumelir_tbl_nk").trim(), "x");
}

#[test]
fn table_mixed_positional_and_keyed() {
    let src = r#"local t = {10, name="y", [3]=true}; print(t[1], t.name, t[3])"#;
    assert_eq!(run_ok(src, "lumelir_tbl_mixed").trim(), "10\ty\ttrue");
}

#[test]
fn table_trailing_comma_with_keyed() {
    let src = "local t = {a=1,}; print(t.a)";
    assert_eq!(run_ok(src, "lumelir_tbl_trailing").trim(), "1");
}
