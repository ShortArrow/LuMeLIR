//! ADR 0266/0267 — N7-5/N7-6: os.rename + os.tmpname via libc.

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
fn os_rename_existing_returns_true() {
    let src_path = std::env::temp_dir().join("n7_rename_src.txt");
    let dst_path = std::env::temp_dir().join("n7_rename_dst.txt");
    let _ = std::fs::remove_file(&dst_path);
    std::fs::write(&src_path, "x").unwrap();
    let src_s = src_path.to_str().unwrap();
    let dst_s = dst_path.to_str().unwrap();
    let lua = format!(
        "local ok = os.rename(\"{}\", \"{}\")
if ok then print(\"ok\") else print(\"bad\") end",
        src_s, dst_s
    );
    let out = run_ok(&lua, "n7_rename_ok");
    assert_eq!(out.trim(), "ok");
    assert!(!src_path.exists() && dst_path.exists());
    let _ = std::fs::remove_file(&dst_path);
}

#[test]
fn os_rename_missing_source_returns_false() {
    let src =
        "local ok = os.rename(\"/tmp/lumelir_n7_definitely_missing_xyz.txt\", \"/tmp/n7_dest.txt\")
if ok then print(\"ok\") else print(\"bad\") end";
    let out = run_ok(src, "n7_rename_bad");
    assert_eq!(out.trim(), "bad");
}

#[test]
fn os_tmpname_returns_nonempty_string() {
    let out = run_ok(
        "local p = os.tmpname()
print(type(p))
if string.len(p) > 0 then print(\"nonempty\") else print(\"empty\") end",
        "n7_tmpname",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "string");
    assert_eq!(lines[1], "nonempty");
}
