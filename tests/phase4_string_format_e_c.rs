//! ADR 0204 — `string.format` `%e` (scientific) + `%c` (char).

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
fn string_format_scientific() {
    assert_eq!(
        run_ok(r#"print(string.format("%e", 1.5))"#, "lumelir_sf_e").trim(),
        "1.500000e+00"
    );
}

#[test]
fn string_format_char() {
    assert_eq!(
        run_ok(r#"print(string.format("%c", 65))"#, "lumelir_sf_c").trim(),
        "A"
    );
}
