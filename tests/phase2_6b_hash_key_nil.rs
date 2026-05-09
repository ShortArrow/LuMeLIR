//! Phase 2.6b-hash-key-validity (ADR 0087): runtime tag-validity
//! gate at the hash probe chokepoint. The previous ADR 0084 surface
//! had an inline `s_table_index_nil` trap on the IndexAssign /
//! Index TaggedValue arms; ADR 0087 retires those duplicates and
//! enforces the policy once inside `emit_hash_probe_loop` so every
//! caller (IndexAssign / Index / iterator-internal) inherits the
//! gate without per-call repetition.
//!
//! These two e2e pin both probe entries (`emit_hash_probe_for_insert`
//! and `emit_hash_probe_lookup`) — they were green before ADR 0087
//! via the inline traps and must remain green after the migration.
//! A future regression where a new caller skips the chokepoint or
//! the chokepoint regresses on nil would surface here.

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

fn assert_traps_with_nil_msg(out: &std::process::Output) {
    assert!(!out.status.success(), "expected non-zero exit: {out:?}");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("table index is nil"),
        "expected 'table index is nil' in output, got: {combined}"
    );
}

#[test]
fn chokepoint_traps_nil_on_taggedvalue_indexassign() {
    // `k` is widened to TaggedValue (mixed nil/string returns,
    // ADR 0074); at runtime `k` carries TAG_NIL. The IndexAssign
    // route used to fire its own inline trap before ADR 0087;
    // post-0087 it relies on the probe chokepoint via
    // `emit_hash_probe_for_insert`.
    let out = compile_and_run(
        "local function maybe_str(x)
  if x == 0 then return nil end
  return \"key\"
end
local k = maybe_str(0)
local t = {}
t[k] = 1",
        "lumelir_validity_gate_write",
    );
    assert_traps_with_nil_msg(&out);
}

#[test]
fn chokepoint_traps_nil_on_taggedvalue_index_read() {
    // Same TaggedValue-nil shape but on the read side. Routes
    // through `emit_hash_probe_lookup`, the probe loop's other
    // entry. The chokepoint gate fires before the lookup-mode
    // missing-key trap.
    let out = compile_and_run(
        "local function maybe_str(x)
  if x == 0 then return nil end
  return \"key\"
end
local k = maybe_str(0)
local t = {}
t[\"key\"] = 1
local sum = 0
sum = sum + t[k]
print(sum)",
        "lumelir_validity_gate_read",
    );
    assert_traps_with_nil_msg(&out);
}
