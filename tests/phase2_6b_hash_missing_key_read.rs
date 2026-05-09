//! Phase 2.6b-hash-lookup-miss (ADR 0088): Index hash read miss is
//! reified as a Nil-tagged TaggedValue slot; the consumer chains its
//! own contract downstream. Resolves LIC-2.6b-hash-missing-key-read-1.
//!
//! ADR 0084 introduced TaggedValue-key Index read with an inline
//! `emit_hash_probe_lookup` trap on missing — Lua §3.4.5 violation.
//! Same shape was duplicated in the static-key Index arm (ADR 0079).
//! ADR 0088 dissolves the conflation: the chokepoint helper
//! `emit_hash_lookup_into_tagged_slot` materialises a tmp 16-byte
//! TaggedValue slot (Nil on missing); the Index arm then runs
//! `emit_value_slot_check_number` which traps with
//! `s_table_type_mismatch` ("table value type mismatch") if the
//! consumer expected a Number and got Nil.
//!
//! User-visible diagnostic shift in arith/cmp contexts (narrow):
//!   `t[missing] + 1`  Day 0: "table missing key"
//!                     Day 6: "table value type mismatch"
//! Both still trap. LocalInit / print contexts already returned nil
//! via `widen_index_for_local_init` and remain unchanged
//! (regression-pin below).
//!
//! Step 0a: 2 Red (behaviour-change pins, assert NEW diagnostic)
//! Step 0b: 2 regression-pins (Day 0 green, must stay green)

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

fn assert_traps_with_type_mismatch(out: &std::process::Output) {
    assert!(!out.status.success(), "expected non-zero exit: {out:?}");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("table value type mismatch"),
        "expected 'table value type mismatch' in output, got: {combined}"
    );
}

// --- Step 0a: behaviour-change pins (Red Day 0, Green after Step 4) ---

#[test]
fn static_string_key_missing_in_arith_traps_with_type_mismatch() {
    // `t["x"] + 1` — Index static String arm reaches helper via
    // tmp tagged slot. Empty table → hash_buf is null → helper writes
    // Nil into tmp → emit_value_slot_check_number traps with
    // s_table_type_mismatch. Day 0 produces s_table_missing_key.
    let out = compile_and_run(
        "local t = {}
print(t[\"x\"] + 1)",
        "lumelir_lookup_miss_static_arith",
    );
    assert_traps_with_type_mismatch(&out);
}

#[test]
fn taggedvalue_key_missing_in_arith_traps_with_type_mismatch() {
    // k = TaggedValue from `pairs(source)`, then `target[k] + 1`
    // where `target` is a separate empty table. Routes through Index
    // TaggedValue arm — same restructure path. Day 0 produces
    // s_table_missing_key; Day 6 produces s_table_type_mismatch.
    let out = compile_and_run(
        "local source = {}
source.a = 1
local target = {}
for k, v in pairs(source) do
  print(target[k] + 1)
end",
        "lumelir_lookup_miss_tagged_arith",
    );
    assert_traps_with_type_mismatch(&out);
}

// --- Step 0b: regression-pins (Day 0 green, must stay green) ---

#[test]
fn static_string_key_missing_in_localinit_returns_nil() {
    // `local v = t["x"]` triggers `widen_index_for_local_init`
    // → IndexTagged → `emit_local_init_tagged` String arm. The
    // String arm uses `emit_hash_probe_for_insert + null_buf check
    // → store Nil + key_at_null → store Nil`. Already correct.
    //
    // Special interest: this test exercises the **hash_buf == null**
    // branch (empty table, no hash buffer ever allocated). The
    // Step 3 helper migration must preserve this branch.
    let stdout = run_ok(
        "local t = {}
local v = t[\"x\"]
print(type(v))",
        "lumelir_lookup_miss_localinit",
    );
    assert_eq!(stdout.trim(), "nil");
}

#[test]
fn valid_number_lookup_via_arith_unchanged() {
    // Happy-path regression: a valid Number value at a String key
    // must continue to flow through the Index static-key arm
    // arithmetic correctly. Step 4 restructures this arm to use
    // tmp slot + check_number; the check_number must NOT fire on
    // a TAG_NUMBER value.
    let stdout = run_ok(
        "local t = {}
t[\"x\"] = 5
print(t[\"x\"] + 1)",
        "lumelir_lookup_valid_number",
    );
    assert_eq!(stdout.trim(), "6");
}
