//! ADR 0221 — `_G` and `_ENV` as chunk-level Tables. The HIR
//! lower step prepends a synthetic `local _G = {}` (and
//! `local _ENV = _G`) when the source references either name.
//! `_G[k] = v` writes and `_G[k]` reads ride the existing
//! Table machinery.

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
fn g_write_then_read_round_trips() {
    assert_eq!(
        run_ok("_G.foo = 42\nprint(_G.foo)", "lumelir_env_g_round_trip").trim(),
        "42"
    );
}

#[test]
fn g_string_key_assignment() {
    assert_eq!(
        run_ok(
            "_G[\"answer\"] = 42\nprint(_G.answer)",
            "lumelir_env_g_string_key"
        )
        .trim(),
        "42"
    );
}

#[test]
fn env_alias_reads_same_table_as_g() {
    // `_ENV = _G` synthesised binding — writes via _G must be
    // visible via _ENV and vice versa.
    let out = run_ok(
        "_G.a = 1\n_ENV.b = 2\nprint(_ENV.a)\nprint(_G.b)",
        "lumelir_env_alias",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
}

#[test]
fn g_holds_multiple_keys() {
    let out = run_ok(
        "_G.x = 10\n_G.y = 20\n_G.z = 30\nprint(_G.x)\nprint(_G.y)\nprint(_G.z)",
        "lumelir_env_g_multi",
    );
    assert_eq!(out.trim(), "10\n20\n30");
}
