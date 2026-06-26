//! ADR 0276 — N7-15: os.setlocale() no-arg form (query LC_ALL).

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
fn os_setlocale_returns_non_empty() {
    let out = run_ok(
        "local s = os.setlocale()
print(#s > 0)",
        "n7_setlocale_nonempty",
    );
    assert_eq!(out.trim(), "true");
}

#[test]
fn os_setlocale_default_is_c() {
    // Without explicit setup, the C runtime starts in the "C" locale.
    let out = run_ok(
        "local s = os.setlocale()
if s == \"C\" then print(\"c\") else print(s) end",
        "n7_setlocale_c",
    );
    assert_eq!(out.trim(), "c");
}
