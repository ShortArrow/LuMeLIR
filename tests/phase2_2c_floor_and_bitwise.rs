//! Integration test: Phase 2.2c — floor division `//` and the five
//! bitwise operators (ADR 0022).

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    assert!(result.status.success(), "binary should exit 0");
    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
    let _ = std::fs::remove_file(&output);
    stdout
}

#[test]
fn floor_div_positive_truncates_down() {
    assert_eq!(run("print(7 // 2)", "lumelir_22c_fdiv_pos").trim(), "3");
}

#[test]
fn floor_div_negative_floors_toward_minus_infinity() {
    // Lua: -7 // 2 == -4 (floor), not -3 (trunc).
    assert_eq!(run("print(-7 // 2)", "lumelir_22c_fdiv_neg").trim(), "-4");
}

#[test]
fn bitwise_and_masks_lower_nibble() {
    assert_eq!(run("print(15 & 5)", "lumelir_22c_band").trim(), "5");
}

#[test]
fn bitwise_or_sets_two_bits() {
    assert_eq!(run("print(1 | 2)", "lumelir_22c_bor").trim(), "3");
}

#[test]
fn bitwise_xor_via_tilde() {
    assert_eq!(run("print(5 ~ 3)", "lumelir_22c_bxor").trim(), "6");
}

#[test]
fn shift_left_doubles() {
    assert_eq!(run("print(1 << 3)", "lumelir_22c_shl").trim(), "8");
}

#[test]
fn shift_right_arithmetic_halves() {
    assert_eq!(run("print(16 >> 2)", "lumelir_22c_shr").trim(), "4");
}

#[test]
fn unary_bitnot_complements_zero() {
    assert_eq!(run("print(~0)", "lumelir_22c_bnot_zero").trim(), "-1");
}

#[test]
fn unary_bitnot_complements_one() {
    assert_eq!(run("print(~1)", "lumelir_22c_bnot_one").trim(), "-2");
}

#[test]
fn precedence_bitwise_below_shift() {
    // `1 | 2 << 3` parses as `1 | (2 << 3)` == `1 | 16` == 17.
    assert_eq!(run("print(1 | 2 << 3)", "lumelir_22c_prec1").trim(), "17");
}

#[test]
fn precedence_bitand_above_bitor() {
    // `1 | 2 & 0` parses as `1 | (2 & 0)` == `1 | 0` == 1.
    assert_eq!(run("print(1 | 2 & 0)", "lumelir_22c_prec2").trim(), "1");
}

#[test]
fn floor_div_in_local() {
    let src = "local q = 100 // 7\nprint(q)";
    assert_eq!(run(src, "lumelir_22c_fdiv_local").trim(), "14");
}

#[test]
fn bitwise_in_user_function() {
    let src = "local function pack(hi, lo) return hi << 4 | lo end
print(pack(3, 5))";
    assert_eq!(run(src, "lumelir_22c_pack").trim(), "53");
}

// --- Phase 2.7w-emit-f2i-gate-sweep (ADR 0114) ---
//
// Lua 5.4 §3.4.2: "All bitwise operations convert their operands
// to integers ... If a float operand does not have an integer
// representation, a runtime error is raised."
//
// Pre-0114 the codegen called `arith.fptosi` raw on the bitwise
// f64 operands (`emit_f2i` at `src/codegen/emit.rs:8861-8876`),
// which is UB for NaN/Inf and silently truncates fractions. ADR
// 0114 inserts `emit_check_integer_arg` at all 10 unprotected
// sites (string.* / table.* / bitwise) — these pins verify the
// bitwise side traps as the spec mandates.
//
// Test pattern: non-zero exit assertion (message wording loosely
// coupled — `compile_and_run` direct, not `run`).

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

#[test]
fn bitwise_band_non_integer_lhs_traps() {
    // 1.5 has no integer representation — Lua §3.4.2 mandates trap.
    let src = "print(1.5 & 3)";
    let out = compile_and_run(src, "lumelir_22c_band_nonint_lhs_trap");
    assert!(
        !out.status.success(),
        "1.5 & 3 must trap, but exited 0: {out:?}"
    );
}

#[test]
fn bitwise_band_non_integer_rhs_traps() {
    let src = "print(3 & 1.5)";
    let out = compile_and_run(src, "lumelir_22c_band_nonint_rhs_trap");
    assert!(
        !out.status.success(),
        "3 & 1.5 must trap, but exited 0: {out:?}"
    );
}

#[test]
fn bitwise_band_nan_traps() {
    // 0/0 = NaN. cmpf-Oeq(NaN-NaN, 0) = false (NaN != 0) → finite
    // check fails → s_bitwise_non_integer trap.
    let src = "local x = 0/0\nprint(x & 3)";
    let out = compile_and_run(src, "lumelir_22c_band_nan_trap");
    assert!(
        !out.status.success(),
        "NaN & 3 must trap, but exited 0: {out:?}"
    );
}

#[test]
fn bitwise_bor_inf_traps() {
    // 1/0 = +Inf. Inf - Inf = NaN ≠ 0 → finite check fails → trap.
    let src = "local x = 1/0\nprint(x | 3)";
    let out = compile_and_run(src, "lumelir_22c_bor_inf_trap");
    assert!(
        !out.status.success(),
        "Inf | 3 must trap, but exited 0: {out:?}"
    );
}

#[test]
fn bitwise_bxor_non_integer_traps() {
    let src = "print(1.5 ~ 3)";
    let out = compile_and_run(src, "lumelir_22c_bxor_nonint_trap");
    assert!(
        !out.status.success(),
        "1.5 ~ 3 must trap, but exited 0: {out:?}"
    );
}

#[test]
fn bitwise_shl_non_integer_rhs_traps() {
    // Shift amount non-integer → trap.
    let src = "print(1 << 1.5)";
    let out = compile_and_run(src, "lumelir_22c_shl_nonint_trap");
    assert!(
        !out.status.success(),
        "1 << 1.5 must trap, but exited 0: {out:?}"
    );
}

#[test]
fn bitwise_bnot_non_integer_traps() {
    let src = "print(~1.5)";
    let out = compile_and_run(src, "lumelir_22c_bnot_nonint_trap");
    assert!(
        !out.status.success(),
        "~1.5 must trap, but exited 0: {out:?}"
    );
}

#[test]
fn bitwise_bnot_nan_traps() {
    let src = "local x = 0/0\nprint(~x)";
    let out = compile_and_run(src, "lumelir_22c_bnot_nan_trap");
    assert!(
        !out.status.success(),
        "~NaN must trap, but exited 0: {out:?}"
    );
}
