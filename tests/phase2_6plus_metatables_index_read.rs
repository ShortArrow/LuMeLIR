//! Phase 2.6+-metatables-index-read (ADR 0134): Metatables
//! `__index` read-path, Table form only.
//!
//! Red Day 0 entry: tests are written before the SetMetatable /
//! GetMetatable builtins and the `emit_check_metatable_index`
//! chokepoint exist. They go Green incrementally across commits
//! C5 (basic builtins) and C6 (__index dispatch).
//!
//! Scope (per ADR 0134 + ADR 0133 deferral table):
//!   - `__index` read-path, Table form only.
//!   - Chain max depth = 2000 (Lua reference); cycle trap.
//!   - Function-form `__index`, `__newindex`, arith / cmp /
//!     `__tostring` / `__call` metamethods OUT OF SCOPE.
//!
//! Parser carry-over: this project's table-constructor parser
//! only accepts the array form `{e1, e2, ...}`. Hash entries
//! must be added via post-construction `t.k = v` assignment
//! (the ADR 0058 path). Tests below follow that pattern.

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

// --- Test 1: setmetatable + getmetatable round-trip ---

#[test]
fn setmetatable_and_getmetatable_round_trip() {
    let src = r#"
local t = {}
local mt = {}
setmetatable(t, mt)
print(type(getmetatable(t)))
"#;
    let out = run_ok(src, "lumelir_mt_round_trip");
    assert_eq!(out, "table\n");
}

// --- Test 2: __index Table fallback (1 hop) ---

#[test]
fn index_fallback_to_metatable_table_one_hop() {
    let src = r#"
local fallback = {}
fallback.x = 42
local mt = {}
mt.__index = fallback
local t = {}
setmetatable(t, mt)
print(t.x)
"#;
    let out = run_ok(src, "lumelir_mt_index_one_hop");
    assert_eq!(out, "42\n");
}

// --- Test 3: __index Table chain (3 hops) ---

#[test]
fn index_chain_three_hops() {
    let src = r#"
local c = {}
c.z = 100
local b = {}
local mt_b = {}
mt_b.__index = c
setmetatable(b, mt_b)
local a = {}
local mt_a = {}
mt_a.__index = b
setmetatable(a, mt_a)
local t = {}
local mt_t = {}
mt_t.__index = a
setmetatable(t, mt_t)
print(t.z)
"#;
    let out = run_ok(src, "lumelir_mt_index_three_hops");
    assert_eq!(out, "100\n");
}

// --- Test 4: __index cycle (self-reference) traps at depth cap ---

#[test]
fn index_chain_cycle_traps_at_depth_cap() {
    // setmetatable(t, mt) where mt.__index = mt creates an
    // infinite chain — the depth-2000 cap must trap, not hang.
    let src = r#"
local mt = {}
mt.__index = mt
local t = {}
setmetatable(t, mt)
print(t.missing)
"#;
    let out = compile_and_run(src, "lumelir_mt_index_cycle_trap");
    assert!(
        !out.status.success(),
        "self-referential __index chain must trap, but binary exited 0: {out:?}"
    );
}

// --- Test 5: no __index field → nil-on-miss ---

#[test]
fn metatable_without_index_returns_nil_on_miss() {
    let src = r#"
local t = {}
local mt = {}
setmetatable(t, mt)
if t.missing == nil then
  print("nil")
else
  print("not_nil")
end
"#;
    let out = run_ok(src, "lumelir_mt_no_index_nil");
    assert_eq!(out, "nil\n");
}

// --- Test 6: __index = Function form is rejected ---

#[test]
fn index_function_form_is_rejected() {
    // Function-form `__index` is explicitly out of scope per ADR
    // 0134. Storing a Function value into `mt.__index` should
    // either be HIR-rejected or runtime-trapped.
    let src = r#"
local function f(t, k) return 99 end
local mt = {}
mt.__index = f
local t = {}
setmetatable(t, mt)
print(t.missing)
"#;
    // The exact rejection surface (HIR error vs runtime trap) is
    // a follow-up decision; the test only asserts the program
    // does NOT silently succeed with f's return value.
    let result = std::panic::catch_unwind(|| compile_and_run(src, "lumelir_mt_index_fn_reject"));
    match result {
        Err(_) => {
            // HIR / codegen panic — accepted as rejection.
        }
        Ok(out) => {
            assert!(
                !out.status.success(),
                "Function-form __index must be rejected (HIR or runtime), but binary exited 0: {out:?}"
            );
        }
    }
}

// --- Test 7: array path OOB → __index fallback ---

#[test]
fn array_oob_falls_through_to_metatable_index() {
    let src = r#"
local fallback = {}
fallback[5] = 555
local t = {1, 2, 3}
local mt = {}
mt.__index = fallback
setmetatable(t, mt)
print(t[5])
"#;
    let out = run_ok(src, "lumelir_mt_array_oob_fallback");
    assert_eq!(out, "555\n");
}

// --- Test 8: hash miss → __index fallback ---

#[test]
fn hash_miss_falls_through_to_metatable_index() {
    let src = r#"
local fallback = {}
fallback.present = "found"
local t = {}
t.present_in_t = "no_metatable_needed"
local mt = {}
mt.__index = fallback
setmetatable(t, mt)
print(t.present)
"#;
    let out = run_ok(src, "lumelir_mt_hash_miss_fallback");
    assert_eq!(out, "found\n");
}

// --- Test 9: TaggedValue Index consumer via __index ---

#[test]
fn tagged_value_index_consumer_via_index() {
    // print(t[k]) where t[k] resolves through __index should
    // route through the existing print TaggedValue dispatch.
    let src = r#"
local fallback = {}
fallback.kind = "table"
local t = {}
local mt = {}
mt.__index = fallback
setmetatable(t, mt)
local x = t.kind
print(type(x))
print(x)
"#;
    let out = run_ok(src, "lumelir_mt_index_tagged_consumer");
    assert_eq!(out, "string\ntable\n");
}

// --- Test 10: getmetatable() returns Nil when no metatable ---

#[test]
fn getmetatable_returns_nil_when_no_metatable() {
    let src = r#"
local t = {}
print(type(getmetatable(t)))
"#;
    let out = run_ok(src, "lumelir_mt_get_nil");
    assert_eq!(out, "nil\n");
}

// --- Test 11: setmetatable returns t (Lua §6.1) ---

#[test]
fn setmetatable_returns_the_target_table() {
    // Per Lua 5.4 §6.1, setmetatable returns its first argument.
    let src = r#"
local t = {}
t.seed = 7
local mt = {}
local same = setmetatable(t, mt)
print(same.seed)
"#;
    let out = run_ok(src, "lumelir_mt_setmetatable_returns_t");
    assert_eq!(out, "7\n");
}

// --- Test 12: independent tables retain separate metatables ---

#[test]
fn two_tables_independent_metatables_resolve_separately() {
    let src = r#"
local fallback_a = {}
fallback_a.answer = "A"
local fallback_b = {}
fallback_b.answer = "B"
local mt_a = {}
mt_a.__index = fallback_a
local mt_b = {}
mt_b.__index = fallback_b
local ta = {}
local tb = {}
setmetatable(ta, mt_a)
setmetatable(tb, mt_b)
print(ta.answer)
print(tb.answer)
"#;
    let out = run_ok(src, "lumelir_mt_two_independent");
    assert_eq!(out, "A\nB\n");
}
