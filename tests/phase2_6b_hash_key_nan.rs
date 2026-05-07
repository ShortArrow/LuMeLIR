//! Phase 2.6b-hash-key-nan (ADR 0086): runtime trap "table index is
//! NaN" for static Number-key array path and TaggedValue-key hash
//! probe entry. Resolves LIC-2.6b-hash-key-nan-runtime-1.
//!
//! Lua spec §3.4.5: NaN cannot be used as a table index. Before
//! this phase, `t[0/0]` failed `cmpf Oeq` deep inside the hash
//! probe and surfaced as the generic `s_table_missing_key` (read
//! side) or as undefined behaviour in `f2i(NaN)` followed by an
//! `s_table_oob` (write side). ADR 0086 detects NaN at the entry
//! of every key-bearing path and traps with a dedicated message.
//!
//! Insertion sites covered:
//! - Static Number-key IndexAssign (array path): `t[0/0] = 1`
//! - Static Number-key Index read (array path): `print(t[0/0])`
//! - TaggedValue-key hash probe entry: NaN flowed via pairs/local
//!   into a TaggedValue local used as a key.
//!
//! Regression backstops ensure normal Number / TaggedValue keys
//! and the ADR 0084 nil trap continue working unchanged.

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

fn assert_traps_with_nan_msg(out: &std::process::Output) {
    assert!(!out.status.success(), "expected non-zero exit: {out:?}");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("table index is NaN"),
        "expected 'table index is NaN' in output, got: {combined}"
    );
}

#[test]
fn static_number_key_nan_write_traps() {
    // `t[0/0] = 1` — static Number key arm of IndexAssign. ADR
    // 0086 inserts a `cmpf Une` NaN preflight before f2i / bounds
    // check.
    let out = compile_and_run(
        "local t = {}
t[0/0] = 1",
        "lumelir_nan_static_write",
    );
    assert_traps_with_nan_msg(&out);
}

#[test]
fn static_number_key_nan_read_traps() {
    // `print(t[0/0])` — static Number key arm of Index. Same
    // preflight on the read path.
    let out = compile_and_run(
        "local t = {}
t[1] = 10
print(t[0/0])",
        "lumelir_nan_static_read",
    );
    assert_traps_with_nan_msg(&out);
}

#[test]
fn taggedvalue_key_nan_via_widened_function_traps() {
    // ADR 0074 widens `nan_or_str`'s ret_kinds to TaggedValue (mixed
    // Number / String returns), so `k` is TaggedValue with runtime
    // TAG_NUMBER + payload NaN. Indexing through `t[k]` goes via
    // the TaggedValue-key IndexAssign arm and the hash probe entry
    // preflight added by ADR 0086.
    let out = compile_and_run(
        "local function nan_or_str(x)
  if x == 0 then return 0/0 end
  return \"key\"
end
local k = nan_or_str(0)
local t = {}
t[k] = 1",
        "lumelir_nan_tagged_write",
    );
    assert_traps_with_nan_msg(&out);
}

#[test]
fn regression_normal_number_keys_unchanged() {
    // Non-NaN Number keys must continue to work — the preflight
    // is a single-cycle `cmpf Une self-self` that is false for
    // every finite / infinite double.
    let stdout = run_ok(
        "local t = {}
t[1] = 10
t[2] = 20
t[3] = 30
print(t[1], t[2], t[3])",
        "lumelir_nan_regression_num",
    );
    assert_eq!(stdout.trim(), "10\t20\t30");
}

#[test]
fn regression_taggedvalue_key_string_unchanged() {
    // TaggedValue keys whose dynamic tag is not Number must skip
    // the NaN preflight (only the TAG_NUMBER branch reads payload
    // as f64). pairs over a string-keyed table exercises the
    // path.
    let stdout = run_ok(
        "local t = {}
t.a = 1
t.b = 2
t.c = 3
for k, v in pairs(t) do
  t[k] = v + 100
end
print(t.a, t.b, t.c)",
        "lumelir_nan_regression_tag_str",
    );
    assert_eq!(stdout.trim(), "101\t102\t103");
}

#[test]
fn regression_nil_trap_still_fires() {
    // ADR 0084 `s_table_index_nil` trap is on the same code path —
    // adding the NaN preflight must not displace it. ADR 0074
    // widens `maybe_str`'s ret_kinds to TaggedValue (mixed Nil/
    // String returns), so `k` is TaggedValue with runtime TAG_NIL.
    let out = compile_and_run(
        "local function maybe_str(x)
  if x == 0 then return nil end
  return \"key\"
end
local k = maybe_str(0)
local t = {}
t[k] = 1",
        "lumelir_nan_regression_nil",
    );
    assert!(!out.status.success(), "nil-key write must still trap");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("table index is nil"),
        "expected 'table index is nil' to still fire, got: {combined}"
    );
}
