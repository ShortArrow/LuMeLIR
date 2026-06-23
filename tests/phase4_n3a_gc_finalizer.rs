//! ADR 0259 — N3-A: `__gc` finalizer runtime dispatch.
//!
//! When an unreachable Table is about to be freed at sweep time,
//! its metatable is probed for `__gc`. If present as a Function,
//! the finalizer fires before the free. Per Lua 5.4 §2.5.3.
//!
//! Today's strategy parallels ADR 0258 (N3-D `__close`): user fn
//! default param ABI is `(Number) → ()`; the canonical metamethod
//! body `function(t) ... end` matches by construction. Args are
//! placeholder `0.0`; metamethods that ignore `t` (the common
//! release / log / cleanup pattern) work. Full `(Table) → ()`
//! arg pass-through is deferred to the same future ADR that
//! resolves the dynamic-typing question for ADR 0258.

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
fn finalizer_fires_on_fn_frame_table() {
    // Table created in a user fn's frame becomes unreachable when
    // the fn returns (N2-D frame walk pops the node). Subsequent
    // collectgarbage sweeps it WHITE; __gc fires before free.
    let out = run_ok(
        "local mt = {}
mt.__gc = function(t) print(\"finalized\") end
local function gen()
  local t = setmetatable({}, mt)
end
gen()
collectgarbage()
print(\"after\")",
        "lumelir_n3a_fn_frame_gc",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "finalized");
    assert_eq!(lines[1], "after");
}

#[test]
fn finalizer_does_not_fire_for_rooted_table() {
    // Chunk-level Table stays rooted, never reaches WHITE, never
    // gets __gc'd by collectgarbage. Lua-spec-correct behavior.
    let out = run_ok(
        "local mt = {}
mt.__gc = function(t) print(\"finalized\") end
local kept = setmetatable({}, mt)
collectgarbage()
print(\"after\")",
        "lumelir_n3a_rooted_no_gc",
    );
    assert_eq!(out.trim(), "after");
}

#[test]
fn finalizer_absent_does_not_trap() {
    // A Table whose metatable has no __gc field can still be
    // collected without firing anything.
    let out = run_ok(
        "local mt = {}
local function gen()
  local t = setmetatable({}, mt)
end
gen()
collectgarbage()
print(\"ok\")",
        "lumelir_n3a_no_gc_field",
    );
    assert_eq!(out.trim(), "ok");
}
