//! ADR 0256 — N2-C: capturing closures as GC roots. The
//! `chunk_safe_for_real_gc` predicate now allows chunks with
//! capturing user functions. Function-kind chunk slots are
//! registered into the chunk root table; a new propagation pass
//! walks BLACK closure cells (`GC_TYPE_CLOSURE_CELL`) and marks
//! their upvalue boxes BLACK via the membership-checked helper.

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
fn counter_closure_survives_collection_between_calls() {
    // Classic Number-upvalue counter pattern. The closure cell
    // + upvalue box need to survive collectgarbage so the
    // counter resumes at the right value.
    let out = run_ok(
        "local function make_counter()
  local count = 0
  return function()
    count = count + 1
    return count
  end
end
local c = make_counter()
print(c())
print(c())
collectgarbage()
print(c())
print(c())",
        "lumelir_n2c_counter",
    );
    assert_eq!(out.trim(), "1\n2\n3\n4");
}

#[test]
fn returned_closure_can_be_called_multiple_times() {
    let out = run_ok(
        "local function make_adder(x)
  return function(y) return x + y end
end
local add3 = make_adder(3)
collectgarbage()
print(add3(10))
print(add3(20))",
        "lumelir_n2c_adder",
    );
    assert_eq!(out.trim(), "13\n23");
}

#[test]
fn capturing_closure_with_only_param_upvalue_survives() {
    let out = run_ok(
        "local function wrap(n)
  return function() return n * n end
end
local sq = wrap(7)
collectgarbage()
print(sq())",
        "lumelir_n2c_square",
    );
    assert_eq!(out.trim(), "49");
}

#[test]
fn multiple_independent_closures_each_keep_their_state() {
    let out = run_ok(
        "local function counter()
  local i = 0
  return function() i = i + 1 return i end
end
local a = counter()
local b = counter()
print(a())
print(b())
print(a())
collectgarbage()
print(b())
print(a())",
        "lumelir_n2c_two_counters",
    );
    assert_eq!(out.trim(), "1\n1\n2\n2\n3");
}

#[test]
fn collectgarbage_returns_positive_when_unrelated_transient_freed() {
    // The capturing closure stays live; an unrelated transient
    // gets freed. The byte delta must be positive.
    let out = run_ok(
        "local function make()
  local n = 0
  return function() n = n + 1 return n end
end
local c = make()
local transient = tostring(7)
local freed = collectgarbage()
print(c())
if freed > 0 then print(\"freed\") else print(\"zero\") end",
        "lumelir_n2c_transient",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "freed");
}
