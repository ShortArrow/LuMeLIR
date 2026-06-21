//! ADR 0254 — N2-A: Table mark phase — array part walk.
//! After the chunk-roots scan marks reachable Tables BLACK, a
//! second pass walks each BLACK Table's array_buf and marks
//! referenced String objects BLACK. The chunk-safe predicate
//! is widened to allow ValueKind::Table slots, so programs
//! with Table-typed Locals now participate in real freeing
//! instead of staying in v1 safety mode.

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
fn table_in_chunk_slot_survives_collection() {
    // The Table itself is rooted via its chunk slot. After
    // collectgarbage the Table + its array_buf + its String
    // payload are all still readable.
    let out = run_ok(
        "local t = {\"hello\"}
collectgarbage()
print(t[1])",
        "lumelir_n2a_table_survives",
    );
    assert_eq!(out.trim(), "hello");
}

#[test]
fn multi_element_table_survives_collection() {
    let out = run_ok(
        "local t = {\"alpha\", \"beta\", \"gamma\"}
collectgarbage()
print(t[1])
print(t[2])
print(t[3])",
        "lumelir_n2a_multi",
    );
    assert_eq!(out.trim(), "alpha\nbeta\ngamma");
}

#[test]
fn unreferenced_string_outside_table_is_freed() {
    // The Table refs "kept". A transient `tostring(1)` not
    // anchored to any chunk slot is WHITE → freed. collectgarbage
    // returns a positive delta proving the freeing happened.
    let out = run_ok(
        "local t = {\"kept\"}
local transient = tostring(1)
local freed = collectgarbage()
print(t[1])
if freed > 0 then print(\"freed\") else print(\"zero\") end",
        "lumelir_n2a_transient",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "kept");
    assert_eq!(lines[1], "freed");
}

#[test]
fn table_index_write_then_read_after_gc() {
    let out = run_ok(
        "local t = {}
t[1] = \"written\"
collectgarbage()
print(t[1])",
        "lumelir_n2a_index_assign",
    );
    assert_eq!(out.trim(), "written");
}

#[test]
fn table_reassign_old_string_still_safe_after_gc() {
    // Reassigning t[1] replaces the slot; the prior value's
    // reference goes away but Lua doesn't crash + new value
    // round-trips.
    let out = run_ok(
        "local t = {\"first\"}
t[1] = \"second\"
collectgarbage()
print(t[1])",
        "lumelir_n2a_reassign",
    );
    assert_eq!(out.trim(), "second");
}

#[test]
fn two_tables_each_keep_their_strings() {
    let out = run_ok(
        "local a = {\"A1\"}
local b = {\"B1\"}
collectgarbage()
print(a[1])
print(b[1])",
        "lumelir_n2a_two_tables",
    );
    assert_eq!(out.trim(), "A1\nB1");
}
