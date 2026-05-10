//! Phase 2.7p-tagged-arith-coerce (ADR 0089): TaggedValue arith
//! operand coercion chokepoint. Resolves
//! `LIC-2.7p-arith-coerce-tagged-1`.
//!
//! ADR 0077 introduced String → Number coercion for **static
//! String** arith operands via `HirExprKind::ArithStringCoerce` +
//! `emit_tonumber_for_arith`. ADR 0089 extends the same Lua-spec
//! contract (§3.4.3) to **runtime String** operands held in
//! `Local(TaggedValue)` slots, via a runtime tag-dispatch
//! chokepoint that mirrors `emit_tagged_eq_runtime_dispatch`
//! (the existing Eq/Ne pattern at `emit.rs:8756`).
//!
//! Tag → behaviour matrix:
//! - `TAG_NUMBER` → load f64 payload at +8
//! - `TAG_STRING` → load ptr payload, sscanf coerce via
//!   `emit_tonumber_for_arith` (parse failure → trap with
//!   `s_arith_coerce_failed`)
//! - Bool / Nil / Function / Table → trap with new
//!   `s_arith_on_non_numeric`
//!
//! Op classes in scope: Arith (Add/Sub/Mul/Div/Mod/Pow/FloorDiv) +
//! Bitwise (BitAnd/BitOr/BitXor/Shl/Shr) + Unary (Neg/BitNot) =
//! 14 ops total. Ordering (Lt/Le/Gt/Ge) is OUT of scope (Lua §3.4.4
//! mixed-kind ordering is an error, not coercion). Eq/Ne already
//! handled by `emit_tagged_eq_runtime_dispatch`.

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

fn assert_traps_with(out: &std::process::Output, needle: &str) {
    assert!(!out.status.success(), "expected non-zero exit: {out:?}");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains(needle),
        "expected {needle:?} in output, got: {combined}"
    );
}

// --- Step 0a: Behaviour-change Red 4 (success cases) ---

#[test]
fn arith_add_string_tagged_coerces() {
    // `t[1]` carries TAG_STRING with payload "5"; the chokepoint
    // sscanf-coerces to 5.0 and the Add proceeds normally.
    let stdout = run_ok(
        "local t = {\"5\"}
local x = t[1]
print(x + 1)",
        "lumelir_tac_add",
    );
    assert_eq!(stdout.trim(), "6");
}

#[test]
fn arith_mul_string_tagged_coerces() {
    let stdout = run_ok(
        "local t = {\"3\"}
local x = t[1]
print(x * 4)",
        "lumelir_tac_mul",
    );
    assert_eq!(stdout.trim(), "12");
}

#[test]
fn bitwise_and_string_tagged_coerces() {
    // Lua 5.4 §3.4.2: bitwise on string coerces if it parses as
    // exact integer. "0xff" parses via glibc sscanf as 255.0;
    // BitAnd routes f64 → i64 via `emit_f2i` then `arith.andi`.
    let stdout = run_ok(
        "local t = {\"0xff\"}
local x = t[1]
print(x & 3)",
        "lumelir_tac_bitand",
    );
    assert_eq!(stdout.trim(), "3");
}

#[test]
fn unary_neg_string_tagged_coerces() {
    // Unary Neg routes through the same chokepoint via
    // `emit_tagged_unary_arith_runtime_dispatch`.
    let stdout = run_ok(
        "local t = {\"5\"}
local x = t[1]
print(-x)",
        "lumelir_tac_unary_neg",
    );
    assert_eq!(stdout.trim(), "-5");
}

// --- Step 0a: Trap pins for non-numeric tags (4) ---

#[test]
fn arith_on_tagged_bool_traps() {
    // Bool tag in TaggedValue → not coercible. Lua §3.4.3:
    // arithmetic on bool is a runtime error.
    let out = compile_and_run(
        "local t = {true}
local x = t[1]
print(x + 1)",
        "lumelir_tac_bool_traps",
    );
    assert_traps_with(&out, "attempt to perform arithmetic on a non-numeric value");
}

#[test]
fn arith_on_tagged_nil_traps() {
    // Out-of-bounds widened read → Nil-tagged TaggedValue local.
    // Arithmetic must trap with the new s_arith_on_non_numeric.
    // Replaces (with a sharper message) the
    // `phase2_6c_tag_locals::plain_arith_with_nil_traps` pin —
    // that test asserts only non-zero exit, which both pre- and
    // post-ADR-0089 satisfy.
    let out = compile_and_run(
        "local t = {1, 2}
local x = t[5]
print(x + 1)",
        "lumelir_tac_nil_traps",
    );
    assert_traps_with(&out, "attempt to perform arithmetic on a non-numeric value");
}

#[test]
fn arith_on_tagged_function_traps() {
    // Function-kind value stored in a table (ADR 0071 closure-less
    // singleton); arith on a TAG_FUNCTION-tagged local must trap.
    let out = compile_and_run(
        "local function f() return 1 end
local t = {f}
local x = t[1]
print(x + 1)",
        "lumelir_tac_function_traps",
    );
    assert_traps_with(&out, "attempt to perform arithmetic on a non-numeric value");
}

#[test]
fn arith_on_tagged_table_traps() {
    // Table-kind value stored in a table (ADR 0071); arith on a
    // TAG_TABLE-tagged local must trap.
    let out = compile_and_run(
        "local u = {}
local t = {u}
local x = t[1]
print(x + 1)",
        "lumelir_tac_table_traps",
    );
    assert_traps_with(&out, "attempt to perform arithmetic on a non-numeric value");
}

// --- Step 0a: Parse-failure pin (1) ---

#[test]
fn arith_on_tagged_unparseable_string_traps() {
    // String tag with payload "abc" — sscanf parse fails →
    // `emit_tonumber_for_arith` exits with
    // `s_arith_coerce_failed` ("attempt to perform arithmetic on
    // a string value"). Distinct trap surface from non-numeric
    // tags (Bool/Nil/Function/Table).
    let out = compile_and_run(
        "local t = {\"abc\"}
local x = t[1]
print(x + 1)",
        "lumelir_tac_parse_fail",
    );
    assert_traps_with(&out, "attempt to perform arithmetic on a string value");
}

// --- Step 0b: Regression-pins (Day 0 green, must stay green) ---

#[test]
fn static_string_arith_unchanged() {
    // ADR 0077 path: static String operand is wrapped in
    // `HirExprKind::ArithStringCoerce` at HIR; the runtime
    // chokepoint added by ADR 0089 must not interfere.
    let stdout = run_ok("print(\"5\" + 1)", "lumelir_tac_static_string_regress");
    assert_eq!(stdout.trim(), "6");
}

#[test]
fn arith_on_tagged_number_unchanged() {
    // ADR 0063 path: TaggedValue local holding TAG_NUMBER. The
    // chokepoint's UseNumberPayload branch must read f64 at +8
    // and produce the same result as the pre-0089 path.
    let stdout = run_ok(
        "local t = {5}
local x = t[1]
print(x + 1)",
        "lumelir_tac_number_regress",
    );
    assert_eq!(stdout.trim(), "6");
}
