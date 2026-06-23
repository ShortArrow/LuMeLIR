//! ADR 0265 — N7-4: `os.remove(filename)` via libc `remove()`.

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
fn os_remove_existing_file_returns_true() {
    // Create a tmp file, then ask the binary to remove it.
    let tmp = std::env::temp_dir().join("n7_remove_target.txt");
    std::fs::write(&tmp, "scratch").unwrap();
    let path_str = tmp.to_str().unwrap();
    let src = format!(
        "local ok = os.remove(\"{}\")\nif ok then print(\"removed\") else print(\"kept\") end",
        path_str
    );
    let out = run_ok(&src, "n7_remove_ok");
    assert_eq!(out.trim(), "removed");
    assert!(!tmp.exists(), "file should be gone");
}

#[test]
fn os_remove_missing_file_returns_false() {
    let src = "local ok = os.remove(\"/tmp/lumelir_n7_definitely_missing_file_xyz.txt\")
if ok then print(\"removed\") else print(\"kept\") end";
    let out = run_ok(src, "n7_remove_missing");
    assert_eq!(out.trim(), "kept");
}
