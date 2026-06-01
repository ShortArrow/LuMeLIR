//! Phase 3.gc-v1-step2 (ADR 0158): all 8 remaining allocator sites
//! migrated to `emit_gc_alloc`. Verifies that table / hash / array /
//! closure / scratch allocations bump `collectgarbage("count")`.

use std::process::Command;

fn compile_and_run(src: &str, output_name: &str) -> std::process::Output {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    result
}

fn run_ok(src: &str, output_name: &str) -> String {
    let out = compile_and_run(src, output_name);
    assert!(out.status.success(), "binary should exit 0: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// --- Test 1: table allocation increases the GC count ---

#[test]
fn table_alloc_bumps_gc_count() {
    // Before ADR 0158: tables used bare malloc, untracked. count
    // would not include them.
    // After ADR 0158: every table alloc routes through `emit_gc_alloc`
    // with GC_TYPE_TABLE, contributing to `g_gc_total_bytes`.
    let src = r#"
local before = collectgarbage("count")
local t = {}
local after = collectgarbage("count")
if after > before then
  print("tracked")
else
  print("not_tracked")
end
"#;
    let out = run_ok(src, "lumelir_gc_table_tracked");
    assert_eq!(out, "tracked\n");
}
