//! ADR 0258 — N3-D: `<close>` runtime `__close` metamethod
//! dispatch on natural scope exit (Block + function-body
//! fall-off). Per Lua 5.4 §3.3.8: when a to-be-closed variable
//! goes out of scope, its `__close` metamethod is called with
//! `(value, errobj)`; on normal exit errobj is `nil`. Nil /
//! false-valued Locals skip the dispatch entirely. Reverse
//! declaration order is observable.
//!
//! pcall / error-path close, `goto`/`break` exit-edge close,
//! and for/while/repeat body close are deferred to follow-ups.

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

// N3-D-1 lands the wiring (LocalInfo flag + HirStmtKind::CloseLocals
// + lower_scoped_body insertion + codegen pre-flight). The 3 dispatch
// tests below stay Red until N3-D-2 ships real metamethod invocation.

#[test]
fn close_fires_on_block_exit() {
    // `do … end` containing a single <close> Local. The metatable's
    // __close should fire once when the block ends.
    let out = run_ok(
        "local mt = {}
mt.__close = function(v, e) print(\"closed\") end
do
  local x <close> = setmetatable({}, mt)
  print(\"inside\")
end
print(\"after\")",
        "lumelir_n3d_block_close",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "inside");
    assert_eq!(lines[1], "closed");
    assert_eq!(lines[2], "after");
}

#[test]
fn close_fires_on_function_fall_off() {
    let out = run_ok(
        "local mt = {}
mt.__close = function(v, e) print(\"closed\") end
local function f()
  local x <close> = setmetatable({}, mt)
  print(\"inside\")
end
f()
print(\"after\")",
        "lumelir_n3d_fn_close",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "inside");
    assert_eq!(lines[1], "closed");
    assert_eq!(lines[2], "after");
}

#[test]
fn close_runs_in_reverse_declaration_order() {
    let out = run_ok(
        "local mt_a = {}
mt_a.__close = function(v, e) print(\"close-a\") end
local mt_b = {}
mt_b.__close = function(v, e) print(\"close-b\") end
do
  local a <close> = setmetatable({}, mt_a)
  local b <close> = setmetatable({}, mt_b)
end",
        "lumelir_n3d_reverse_order",
    );
    let lines: Vec<&str> = out.lines().collect();
    // b declared second → closes first.
    assert_eq!(lines[0], "close-b");
    assert_eq!(lines[1], "close-a");
}

#[test]
fn nil_valued_close_skips_dispatch() {
    // `<close> = nil` must not trap (would otherwise probe a null
    // metatable and crash). Spec §3.3.8: skipped when value is nil.
    let out = run_ok(
        "do
  local x <close> = nil
end
print(\"ok\")",
        "lumelir_n3d_nil_skips",
    );
    assert_eq!(out.trim(), "ok");
}

#[test]
fn non_table_close_skips_silently() {
    // A number value has no metatable today; per spec a non-function
    // __close is a no-op, and a missing metatable is also a no-op.
    let out = run_ok(
        "do
  local x <close> = 42
end
print(\"ok\")",
        "lumelir_n3d_nontable_skips",
    );
    assert_eq!(out.trim(), "ok");
}
