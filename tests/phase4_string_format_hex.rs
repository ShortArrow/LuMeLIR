//! ADR 0202 — `string.format` `%x` / `%X` hex specs.

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
fn string_format_lowercase_hex() {
    assert_eq!(
        run_ok(r#"print(string.format("%x", 255))"#, "lumelir_sf_x").trim(),
        "ff"
    );
}

#[test]
fn string_format_uppercase_hex() {
    assert_eq!(
        run_ok(r#"print(string.format("%X", 255))"#, "lumelir_sf_X").trim(),
        "FF"
    );
}
