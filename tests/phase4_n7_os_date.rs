//! ADR 0271 — N7-10: os.date() no-arg form via ctime(&time(NULL)).

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
fn os_date_returns_string_type() {
    let out = run_ok("print(type(os.date()))", "n7_date_type");
    assert_eq!(out.trim(), "string");
}

#[test]
fn os_date_returns_nonempty() {
    let out = run_ok(
        "local d = os.date()
if string.len(d) > 0 then print(\"nonempty\") else print(\"empty\") end",
        "n7_date_nonempty",
    );
    assert_eq!(out.trim(), "nonempty");
}

#[test]
fn os_date_no_trailing_newline() {
    // ctime appends '\n' which we strip. Verify by checking the
    // print output contains exactly one trailing newline (from print).
    let out = run_ok(
        "io.write(os.date())
print(\"|END\")",
        "n7_date_trim",
    );
    // io.write doesn't append newline; trimmed ctime + literal "|END\n"
    // — so the output ends with "|END\n", not "\n|END\n".
    assert!(out.ends_with("|END\n"), "got: {out:?}");
    assert!(
        !out.contains("\n|END"),
        "trailing newline not stripped: {out:?}"
    );
}
