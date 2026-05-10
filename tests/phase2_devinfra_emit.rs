//! Phase 2.devinfra-emit (ADR 0090): pipeline-stage emission via
//! `lumelir compile --emit <stage>`. Stops the pipeline at the named
//! stage and writes the artifact's text representation to stdout
//! default (or `-o PATH` to file).
//!
//! Stages:
//! - `hir`  — HIR after `hir::lower`, pure render via Debug
//! - `mlir` — MLIR module after `emit_module`, pure render via
//!   `module.as_operation().to_string()`
//! - `llvm` — LLVM IR after `mlir-opt + mlir-translate`, **effectful**
//!   (subprocess); `--emit llvm` is the only stage that invokes
//!   external tools.
//!
//! Each behaviour test asserts both **inclusion** of layer-specific
//! tokens AND **exclusion** of cross-layer tokens, per codex review
//! v2 critical #3.

use std::process::Command;

const SAMPLE_LUA: &str = "local x = 1
print(x + 2)";

fn fixture_path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, SAMPLE_LUA).expect("write fixture");
    path
}

fn run_lumelir(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lumelir"))
        .args(args)
        .output()
        .expect("failed to run lumelir CLI")
}

fn assert_stdout_includes(out: &std::process::Output, needles: &[&str]) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "lumelir exited non-zero: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for needle in needles {
        assert!(
            stdout.contains(needle),
            "stdout missing {needle:?}; got:\n{stdout}"
        );
    }
}

fn assert_stdout_excludes(out: &std::process::Output, forbidden: &[&str]) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    for tok in forbidden {
        assert!(
            !stdout.contains(tok),
            "stdout unexpectedly contains {tok:?}; got:\n{stdout}"
        );
    }
}

#[test]
fn emit_hir_includes_hir_tokens_and_excludes_lower_layer() {
    // HIR Debug output has stable struct/variant names like `HirChunk`
    // (top-level) and `LocalInit` (a HirStmt variant). The MLIR keyword
    // `module {` and the LLVM keyword `define ` must NOT leak through.
    let fixture = fixture_path("lumelir_emit_hir_fix.lua");
    let out = run_lumelir(&["compile", "--emit", "hir", fixture.to_str().unwrap()]);
    assert_stdout_includes(&out, &["HirChunk", "LocalInit"]);
    assert_stdout_excludes(&out, &["module {", "define "]);
    let _ = std::fs::remove_file(&fixture);
}

#[test]
fn emit_mlir_includes_mlir_tokens_and_excludes_other_layers() {
    // MLIR text always opens with `module {`. ADR 0083 Commit 2a
    // migrated user functions to `llvm.func` (via
    // LLVMFuncOperationBuilder) so the pre-`mlir-opt` MLIR text
    // already shows the LLVM dialect prefix. HIR-specific
    // `HirChunk` and LLVM-IR-specific `define i` must be absent.
    let fixture = fixture_path("lumelir_emit_mlir_fix.lua");
    let out = run_lumelir(&["compile", "--emit", "mlir", fixture.to_str().unwrap()]);
    assert_stdout_includes(&out, &["module {", "llvm.func"]);
    assert_stdout_excludes(&out, &["HirChunk", "define i"]);
    let _ = std::fs::remove_file(&fixture);
}

#[test]
fn emit_llvm_includes_llvm_tokens_and_excludes_hir_mlir_text() {
    // LLVM IR text (post `mlir-translate`) contains `define` for
    // function definitions and `@main` for the program entry.
    // HIR-specific `HirChunk` and the MLIR dialect-prefixed
    // `llvm.func` (which becomes `define` after translate) must
    // not appear.
    let fixture = fixture_path("lumelir_emit_llvm_fix.lua");
    let out = run_lumelir(&["compile", "--emit", "llvm", fixture.to_str().unwrap()]);
    assert_stdout_includes(&out, &["define", "@main"]);
    assert_stdout_excludes(&out, &["HirChunk", "llvm.func"]);
    let _ = std::fs::remove_file(&fixture);
}

#[test]
fn emit_to_file_via_dash_o() {
    // `-o PATH` redirects the dump to a file; stdout stays empty.
    // Ensures the I/O adapter (`write_dump`) handles both stdout
    // (default) and file-write modes.
    let fixture = fixture_path("lumelir_emit_file_fix.lua");
    let dump_path = std::env::temp_dir().join("lumelir_emit_file.mlir");
    let _ = std::fs::remove_file(&dump_path);
    let out = run_lumelir(&[
        "compile",
        "--emit",
        "mlir",
        "-o",
        dump_path.to_str().unwrap(),
        fixture.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "lumelir exited non-zero: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "stdout should be empty when -o is set; got:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let contents = std::fs::read_to_string(&dump_path).expect("file should exist");
    assert!(
        contents.contains("module {"),
        "expected MLIR `module {{` in dump file; got:\n{contents}"
    );
    let _ = std::fs::remove_file(&fixture);
    let _ = std::fs::remove_file(&dump_path);
}

#[test]
fn regression_full_compile_unchanged() {
    // No `--emit` → existing behaviour: compile to native binary,
    // run it, expect expected stdout. ADR 0090 must not regress
    // the default compile path.
    let fixture = fixture_path("lumelir_regress_fix.lua");
    let bin_path = std::env::temp_dir().join("lumelir_regress_bin");
    let _ = std::fs::remove_file(&bin_path);
    let compile_out = run_lumelir(&[
        "compile",
        fixture.to_str().unwrap(),
        "-o",
        bin_path.to_str().unwrap(),
    ]);
    assert!(
        compile_out.status.success(),
        "compile should succeed: stderr=\n{}",
        String::from_utf8_lossy(&compile_out.stderr)
    );
    let exec_out = Command::new(&bin_path)
        .output()
        .expect("failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&exec_out.stdout);
    assert_eq!(stdout.trim(), "3");
    assert!(exec_out.status.success(), "binary should exit 0");
    let _ = std::fs::remove_file(&fixture);
    let _ = std::fs::remove_file(&bin_path);
}
