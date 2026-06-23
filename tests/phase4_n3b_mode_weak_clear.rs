//! ADR 0260 — N3-B: `__mode` weak-table clearing.
//!
//! When a Table's metatable carries a non-Nil `__mode` field, the
//! mark phase skips propagating through its values (and keys); the
//! pre-sweep weak-clear pass then nulls out any hash-bucket entry
//! whose value pointer references a WHITE GC node.
//!
//! Simplification: any non-Nil `__mode` triggers weak-value
//! clearing. Spec-precise "k"/"v"/"kv" string discrimination is a
//! future refinement (would require runtime strchr).

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
fn weak_value_table_clears_unreachable_entry() {
    // Standard weak-value idiom: cache table with __mode = "v".
    // The handle's only reference is the cache itself (weak), so
    // mark phase leaves it WHITE; weak-clear sets cache.handle to
    // nil before sweep frees the handle.
    let out = run_ok(
        "local cache = setmetatable({}, {__mode = \"v\"})
local function populate()
  cache.handle = {\"transient\"}
end
populate()
collectgarbage()
if cache.handle == nil then print(\"cleared\") else print(\"kept\") end",
        "lumelir_n3b_weak_clear",
    );
    assert_eq!(out.trim(), "cleared");
}

#[test]
fn strong_table_keeps_entry_through_gc() {
    // Same shape as above but WITHOUT `__mode`. The entry stays.
    let out = run_ok(
        "local cache = setmetatable({}, {})
local function populate()
  cache.handle = {\"persistent\"}
end
populate()
collectgarbage()
if cache.handle == nil then print(\"lost\") else print(\"kept\") end",
        "lumelir_n3b_strong_kept",
    );
    assert_eq!(out.trim(), "kept");
}

#[test]
fn weak_table_with_rooted_value_does_not_clear() {
    // The value is also held in a strong chunk slot, so it stays
    // BLACK at mark — weak-clear must NOT erase it.
    let out = run_ok(
        "local kept = {\"alive\"}
local cache = setmetatable({}, {__mode = \"v\"})
cache.handle = kept
collectgarbage()
if cache.handle == nil then print(\"lost\") else print(\"kept\") end",
        "lumelir_n3b_rooted_safe",
    );
    assert_eq!(out.trim(), "kept");
}
