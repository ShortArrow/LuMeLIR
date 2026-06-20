//! ADR 0223 — `local _ENV = sandbox` rebind. The HIR pre-pass
//! synthesises `local _ENV = _G` at chunk start; a user-declared
//! `local _ENV = ...` later in the same scope shadows it via
//! standard Lua lexical scoping rules. `_G` retains the original
//! chunk-level binding throughout.

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
fn local_env_rebind_shadows_synthesised_alias() {
    let out = run_ok(
        "local _ENV = {}
_ENV.x = 42
print(_ENV.x)",
        "lumelir_env_sandbox_rebind",
    );
    assert_eq!(out.trim(), "42");
}

#[test]
fn local_env_rebind_isolates_from_g() {
    // After rebind, _ENV writes go to the sandbox; _G writes stay
    // in the original chunk-level _G table. Each side is read-back
    // independently from the other.
    let out = run_ok(
        "_G.outer = 1
local _ENV = {}
_ENV.inner = 2
_G.via_g = 3
print(_G.outer)
print(_G.via_g)
print(_ENV.inner)",
        "lumelir_env_sandbox_isolated",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "3");
    assert_eq!(lines[2], "2");
}

#[test]
fn local_env_sandbox_carries_multiple_keys() {
    let out = run_ok(
        "local _ENV = {}
_ENV.a = 10
_ENV.b = 20
_ENV.c = 30
print(_ENV.a)
print(_ENV.b)
print(_ENV.c)",
        "lumelir_env_sandbox_multikey",
    );
    assert_eq!(out.trim(), "10\n20\n30");
}
