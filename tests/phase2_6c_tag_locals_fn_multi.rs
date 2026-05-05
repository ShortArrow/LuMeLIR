//! Phase 2.6c-tag-locals-fn-multi (ADR 0076): multi-position
//! TaggedValue widening — `return 1, nil` and `return nil, 1`
//! interleaving widens both return positions to TaggedValue
//! independently. Closes LIC-2.6c-tag-locals-fn-multi-1.
//!
//! HIR widening is per-position-independent (ADR 0074); the
//! ABI shape (multiple `(i64, i64)` pairs flattened into MLIR
//! results) was already supported by `ret_mlir_types`'s
//! `flat_map`. The new work in ADR 0076 is the caller-side
//! result-index walker that maps logical position N to its
//! flat MLIR result range, packing 2 results into a tagged
//! slot when the position is TaggedValue.
//!
//! Direct call (`Callee::User`) only — indirect TaggedValue
//! call remains HIR-rejected (ADR 0075).

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

fn run(src: &str, output_name: &str) -> String {
    let out = compile_and_run(src, output_name);
    assert!(out.status.success(), "binary should exit 0, got {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// ============================================================
// Cross-position interleave (both positions widen)
// ============================================================

#[test]
fn multi_return_number_or_nil_both_positions_widen() {
    let src = "local function pick(x)
  if x > 0 then return 1, nil end
  return nil, 1
end
local a, b = pick(-1)
print(type(a), type(b))
local c, d = pick(1)
print(type(c), type(d))";
    assert_eq!(
        run(src, "lumelir_multi_num_nil").trim(),
        "nil\tnumber\nnumber\tnil"
    );
}

#[test]
fn multi_return_bool_or_nil_interleave() {
    let src = "local function pick(x)
  if x > 0 then return true, nil end
  return nil, false
end
local a, b = pick(1)
print(type(a), type(b))
local c, d = pick(-1)
print(type(c), type(d))";
    assert_eq!(
        run(src, "lumelir_multi_bool_nil").trim(),
        "boolean\tnil\nnil\tboolean"
    );
}

#[test]
fn multi_return_string_or_nil_interleave() {
    let src = "local function pick(x)
  if x > 0 then return \"a\", nil end
  return nil, \"b\"
end
local a, b = pick(1)
print(type(a), type(b))
local c, d = pick(-1)
print(type(c), type(d))";
    assert_eq!(
        run(src, "lumelir_multi_str_nil").trim(),
        "string\tnil\nnil\tstring"
    );
}

#[test]
fn multi_return_number_string_interleave() {
    let src = "local function pick(x)
  if x > 0 then return 42, \"x\" end
  return nil, \"y\"
end
local a, b = pick(1)
print(tostring(a), b)
local c, d = pick(-1)
print(tostring(c), d)";
    assert_eq!(run(src, "lumelir_multi_num_str").trim(), "42\tx\nnil\ty");
}

// ============================================================
// Single-position widening (the other position stays concrete)
// ============================================================

#[test]
fn multi_return_first_position_widens_only() {
    let src = "local function pick(x)
  if x > 0 then return 42, \"hi\" end
  return nil, \"bye\"
end
local a, b = pick(1)
print(tostring(a), b)
local c, d = pick(-1)
print(tostring(c), d)";
    assert_eq!(
        run(src, "lumelir_multi_first_widen").trim(),
        "42\thi\nnil\tbye"
    );
}

#[test]
fn multi_return_second_position_widens_only() {
    let src = "local function pick(x)
  if x > 0 then return \"hi\", 1 end
  return \"bye\", nil
end
local a, b = pick(1)
print(a, tostring(b))
local c, d = pick(-1)
print(c, tostring(d))";
    assert_eq!(
        run(src, "lumelir_multi_second_widen").trim(),
        "hi\t1\nbye\tnil"
    );
}

// ============================================================
// 3-position multi-return
// ============================================================

#[test]
fn multi_return_three_positions_two_widen() {
    let src = "local function triple(x)
  if x > 0 then return 1, 2, 3 end
  return 1, nil, nil
end
local a, b, c = triple(1)
print(tostring(a), tostring(b), tostring(c))
local d, e, f = triple(-1)
print(tostring(d), tostring(e), tostring(f))";
    assert_eq!(
        run(src, "lumelir_multi_3pos").trim(),
        "1\t2\t3\n1\tnil\tnil"
    );
}

// ============================================================
// Consumers (eq nil)
// ============================================================

#[test]
fn multi_position_widened_eq_nil_check() {
    let src = "local function pick(x)
  if x > 0 then return 1, nil end
  return nil, 1
end
local a, b = pick(-1)
if a == nil then print(\"a-nil\") else print(\"a-val\") end
if b == nil then print(\"b-nil\") else print(\"b-val\") end";
    assert_eq!(run(src, "lumelir_multi_eq_nil").trim(), "a-nil\nb-val");
}

#[test]
fn multi_position_widened_consumed_by_tostring_inline() {
    let src = "local function pick(x)
  if x > 0 then return 1, nil end
  return nil, 1
end
local a, b = pick(-1)
print(tostring(a))
print(tostring(b))";
    assert_eq!(run(src, "lumelir_multi_tostring").trim(), "nil\n1");
}

// ============================================================
// Regression — pure-Number multi-return ABI unchanged
// ============================================================

#[test]
fn regression_pure_number_multi_return_unchanged() {
    let src = "local function pair() return 10, 20 end
local a, b = pair()
print(a, b)";
    assert_eq!(run(src, "lumelir_multi_reg_num").trim(), "10\t20");
}

#[test]
fn regression_pure_string_multi_return_unchanged() {
    let src = "local function pair() return \"x\", \"y\" end
local a, b = pair()
print(a, b)";
    assert_eq!(run(src, "lumelir_multi_reg_str").trim(), "x\ty");
}
