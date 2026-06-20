//! ADR 0217 — `pcall(f)` multi-return `(ok, err)`. Position 0 is
//! the Bool success flag; position 1 is the error value (String
//! on caught error, Nil on success). Composes with ADR 0021
//! multi-assign.

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
fn pcall_multireturn_success_path_yields_nil_err() {
    assert_eq!(
        run_ok(
            "local function ok_fn() return 1 end
local ok, err = pcall(ok_fn)
print(ok)
print(err)",
            "lumelir_pcall_mr_pass"
        )
        .trim(),
        "true\nnil"
    );
}

#[test]
fn pcall_multireturn_caught_path_yields_error_message() {
    assert_eq!(
        run_ok(
            "local function bad() error(\"oops\") end
local ok, err = pcall(bad)
print(ok)
print(err)",
            "lumelir_pcall_mr_catch"
        )
        .trim(),
        "false\noops"
    );
}

#[test]
fn pcall_multireturn_err_chains_into_print() {
    // After pcall catches, the captured error string flows through
    // a Local and into print — verifies the err slot is a properly
    // boxed string-object (not a corrupted ptr).
    assert_eq!(
        run_ok(
            "local function bad() error(\"hello\") end
local ok, msg = pcall(bad)
if not ok then
  print(msg)
end",
            "lumelir_pcall_mr_chain"
        )
        .trim(),
        "hello"
    );
}
