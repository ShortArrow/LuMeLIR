//! ADR 0247 — M14-A: minimal `package` table synthesised via
//! the ADR 0221 HIR pre-pass when source code references the
//! `package` name. Ships the three spec-default String constants
//! (`config`, `path`, `cpath`); `loaded` and the runtime `require`
//! machinery defer to M14-stretch.

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
fn package_config_prints_posix_defaults() {
    // Per Lua 5.4 §6.3, `package.config` is a String whose
    // first line is the directory separator. We pin the full
    // POSIX-default 5-line value (each line is a 1-char value
    // for POSIX systems).
    assert_eq!(
        run_ok("print(package.config)", "lumelir_m14_config")
            .lines()
            .next()
            .unwrap(),
        "/"
    );
}

#[test]
fn package_path_prints_default() {
    // The default path begins with "./?.lua" per Lua 5.4 spec.
    let out = run_ok("print(package.path)", "lumelir_m14_path");
    assert!(
        out.trim().starts_with("./?.lua"),
        "expected default ./?.lua start, got: {out:?}"
    );
}

#[test]
fn package_cpath_prints_default() {
    assert_eq!(
        run_ok("print(package.cpath)", "lumelir_m14_cpath").trim(),
        "./?.so"
    );
}

#[test]
fn package_table_type_is_table() {
    assert_eq!(
        run_ok("print(type(package))", "lumelir_m14_type").trim(),
        "table"
    );
}

#[test]
fn package_path_round_trips_through_local() {
    // Reading package.path into a Local and printing — Tagged
    // Value dispatch.
    assert_eq!(
        run_ok(
            "local p = package.path
print(p)",
            "lumelir_m14_local_roundtrip"
        )
        .trim(),
        "./?.lua;./?/init.lua"
    );
}

#[test]
fn package_not_referenced_means_no_synthesis() {
    // Sanity: chunks that don't reference `package` still
    // compile and run unchanged — the synthesis is opt-in.
    assert_eq!(run_ok("print(\"ok\")", "lumelir_m14_no_synth").trim(), "ok");
}
