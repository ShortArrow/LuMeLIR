//! Phase 2.6+-metatables-newindex-write (ADR 0135): Metatables
//! `__newindex` write-path, Table form only, hash key only.
//!
//! Red Day 0 entry: written before the
//! `emit_check_metatable_newindex_with_depth` chokepoint and the
//! `emit_hash_indexassign_with_newindex` helper extract exist.
//! Goes Green in C3 when the helper + IndexAssign wiring land.
//!
//! Scope (per ADR 0135 + ADR 0133 deferral table):
//!   - `t[k] = v` with `k` a hash-eligible key missing in `t`'s
//!     hash → recurse into `mt["__newindex"]`'s Table.
//!   - Existing-key overwrites bypass `__newindex` (Lua §2.4).
//!   - Number-key array writes bypass per Non-goals (existing
//!     ADR 0057 grow-extend semantics).
//!   - Function-form `__newindex` rejects (HIR or runtime trap).
//!   - Max chain depth = 8 hops (shared static unroll with 0134).
//!
//! Parser carry-over (mirrors ADR 0134 tests): the table-
//! constructor parser only accepts array form `{e1, ...}`. Hash
//! entries land via post-construction `t.k = v` assignments.

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

// --- Test 1: setmetatable + __newindex Table → writes go to fallback ---

#[test]
fn newindex_table_redirects_write_to_fallback_table() {
    let src = r#"
local storage = {}
local mt = {}
mt.__newindex = storage
local t = {}
setmetatable(t, mt)
t.x = 42
print(storage.x)
"#;
    let out = run_ok(src, "lumelir_newindex_redirect");
    assert_eq!(out, "42\n");
}

// --- Test 2: __newindex chain (3 hops) ---

#[test]
fn newindex_chain_three_hops() {
    let src = r#"
local sink = {}
local mt_c = {}
mt_c.__newindex = sink
local c = {}
setmetatable(c, mt_c)
local mt_b = {}
mt_b.__newindex = c
local b = {}
setmetatable(b, mt_b)
local mt_a = {}
mt_a.__newindex = b
local t = {}
setmetatable(t, mt_a)
t.k = 777
print(sink.k)
"#;
    let out = run_ok(src, "lumelir_newindex_three_hops");
    assert_eq!(out, "777\n");
}

// --- Test 3: __newindex cycle traps at depth cap ---

#[test]
fn newindex_chain_cycle_traps_at_depth_cap() {
    // Real cycle: mt is its own metatable AND mt.__newindex = mt.
    // Writing t.k = v in this configuration walks mt → mt → mt → …
    // and never terminates without an explicit depth trap.
    let src = r#"
local mt = {}
mt.__newindex = mt
setmetatable(mt, mt)
local t = {}
setmetatable(t, mt)
t.missing = 1
"#;
    let out = compile_and_run(src, "lumelir_newindex_cycle_trap");
    assert!(
        !out.status.success(),
        "self-referential __newindex chain must trap, but binary exited 0: {out:?}"
    );
}

// --- Test 4: no __newindex → raw write (zero-regress baseline) ---

#[test]
fn missing_newindex_metafield_falls_through_to_raw_write() {
    let src = r#"
local mt = {}
local t = {}
setmetatable(t, mt)
t.x = 99
print(t.x)
"#;
    let out = run_ok(src, "lumelir_newindex_no_metafield_raw");
    assert_eq!(out, "99\n");
}

// --- Test 5: existing key — __newindex does NOT fire (Lua §2.4) ---

#[test]
fn existing_key_overwrite_bypasses_newindex() {
    let src = r#"
local storage = {}
local mt = {}
mt.__newindex = storage
local t = {}
t.k = "original"
setmetatable(t, mt)
t.k = "overwritten"
print(t.k)
if storage.k == nil then
  print("storage_untouched")
else
  print("storage_touched")
end
"#;
    let out = run_ok(src, "lumelir_newindex_existing_key_bypass");
    assert_eq!(out, "overwritten\nstorage_untouched\n");
}

// --- Test 6: __newindex = Function form is rejected ---

#[test]
fn newindex_function_form_is_rejected() {
    // Function-form `__newindex` is explicitly out of scope per
    // ADR 0135. Storing a Function value into `mt.__newindex` and
    // attempting a missing-key write should NOT silently succeed.
    let src = r#"
local function f(t, k, v) end
local mt = {}
mt.__newindex = f
local t = {}
setmetatable(t, mt)
t.missing = 1
"#;
    let result = std::panic::catch_unwind(|| compile_and_run(src, "lumelir_newindex_fn_reject"));
    match result {
        Err(_) => {
            // HIR / codegen panic — accepted as rejection.
        }
        Ok(out) => {
            assert!(
                !out.status.success(),
                "Function-form __newindex must be rejected (HIR or runtime), but binary exited 0: {out:?}"
            );
        }
    }
}

// --- Test 7: __newindex = non-Table other kind traps ---

#[test]
fn newindex_with_number_metafield_traps() {
    // mt.__newindex = 7 (Number) is just as invalid as Function.
    // ADR 0135 only accepts Table-form `__newindex`.
    let src = r#"
local mt = {}
mt.__newindex = 7
local t = {}
setmetatable(t, mt)
t.missing = 1
"#;
    let out = compile_and_run(src, "lumelir_newindex_number_meta_trap");
    assert!(
        !out.status.success(),
        "non-Table __newindex must trap, but binary exited 0: {out:?}"
    );
}

// --- Test 8: Number-key array write bypasses metatable ---

#[test]
fn number_key_array_write_does_not_consult_newindex() {
    // Per ADR 0135 Non-goals, array (Number-key) writes preserve
    // ADR 0057 grow-extend semantics. With a metatable __newindex
    // set, `t[5] = v` should STILL store directly into `t`'s array
    // part, NOT into the metatable's storage.
    let src = r#"
local storage = {}
local mt = {}
mt.__newindex = storage
local t = {1, 2, 3}
setmetatable(t, mt)
t[5] = 555
print(t[5])
"#;
    let out = run_ok(src, "lumelir_newindex_number_key_bypass");
    assert_eq!(out, "555\n");
}

// --- Test 9: TaggedValue-key __newindex (pairs body idiom) ---

#[test]
fn taggedvalue_key_newindex_redirects_through_metatable() {
    // `for k, v in pairs(src) do dst[k] = v end` style — `k` is a
    // TaggedValue local. With dst's metatable.__newindex set, the
    // write should redirect.
    let src = r#"
local sink = {}
local mt = {}
mt.__newindex = sink
local dst = {}
setmetatable(dst, mt)
local src_tbl = {}
src_tbl.alpha = 1
for k, v in pairs(src_tbl) do
  dst[k] = v
end
print(sink.alpha)
"#;
    let out = run_ok(src, "lumelir_newindex_tagged_key");
    assert_eq!(out, "1\n");
}

// --- Test 10: setmetatable + __newindex + getmetatable round-trip ---

#[test]
fn setmetatable_then_newindex_then_getmetatable_round_trip() {
    let src = r#"
local sink = {}
local mt = {}
mt.__newindex = sink
local t = {}
setmetatable(t, mt)
t.field = "hello"
local mt_loaded = getmetatable(t)
print(type(mt_loaded))
print(sink.field)
"#;
    let out = run_ok(src, "lumelir_newindex_round_trip");
    assert_eq!(out, "table\nhello\n");
}

// --- Test 11: independent tables, independent __newindex ---

#[test]
fn two_tables_independent_newindex_metatables_resolve_separately() {
    let src = r#"
local sink_a = {}
local sink_b = {}
local mt_a = {}
mt_a.__newindex = sink_a
local mt_b = {}
mt_b.__newindex = sink_b
local ta = {}
local tb = {}
setmetatable(ta, mt_a)
setmetatable(tb, mt_b)
ta.value = "A"
tb.value = "B"
print(sink_a.value)
print(sink_b.value)
"#;
    let out = run_ok(src, "lumelir_newindex_two_independent");
    assert_eq!(out, "A\nB\n");
}

// --- Test 12: __newindex actually writes to mt's table, not t ---

#[test]
fn newindex_target_is_metatable_storage_not_original_table() {
    // After `t.x = 42` via __newindex, `t.x` itself must still be
    // `nil` (t has no __index — the read path doesn't chain). The
    // value lives in mt.__newindex (the redirect target). This is
    // the read/write asymmetry that drives the existence of
    // ADR 0134 + ADR 0135.
    let src = r#"
local storage = {}
local mt = {}
mt.__newindex = storage
local t = {}
setmetatable(t, mt)
t.x = 42
if t.x == nil then
  print("t_does_not_have_x")
else
  print("t_has_x")
end
print(storage.x)
"#;
    let out = run_ok(src, "lumelir_newindex_target_storage");
    assert_eq!(out, "t_does_not_have_x\n42\n");
}
