//! Phase 2.8e-iter-ipairs (ADR 0078): `for i, v in ipairs(t) do
//! body end` as a parser-level Lua iteration sugar. Plan C
//! scope — only the `ipairs(table_expr)` shape is recognised;
//! `pairs(t)` and arbitrary callable iterators (generic for
//! protocol) remain unsupported until ADR 0075's indirect-call
//! reject is reopened with a signature side table.
//!
//! HIR desugars `ForIpairs` to existing primitives
//! (LocalInit + While + IsNil + Break + BinOp), so codegen
//! sees no new shape.
//!
//! Out of scope (separate LIC entries):
//! - `pairs(t)` hash iteration — LIC-2.8e-iter-pairs-1.
//! - Generic for protocol with arbitrary callable iter —
//!   LIC-2.8e-iter-generic-1.

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
// Basic iteration
// ============================================================

#[test]
fn ipairs_basic_three_elements() {
    let src = "local t = {10, 20, 30}
for i, v in ipairs(t) do print(i, v) end";
    assert_eq!(
        run(src, "lumelir_ipairs_basic").trim(),
        "1\t10\n2\t20\n3\t30"
    );
}

#[test]
fn ipairs_empty_table_no_iterations() {
    let src = "local t = {}
for i, v in ipairs(t) do print(i) end
print(\"after\")";
    assert_eq!(run(src, "lumelir_ipairs_empty").trim(), "after");
}

#[test]
fn ipairs_single_element() {
    let src = "for i, v in ipairs({42}) do print(i, v) end";
    assert_eq!(run(src, "lumelir_ipairs_single").trim(), "1\t42");
}

// ============================================================
// Stops at first nil (Lua spec)
// ============================================================

#[test]
fn ipairs_stops_at_hole() {
    let src = "local t = {1, 2}
t[5] = 99
for i, v in ipairs(t) do print(i, v) end";
    assert_eq!(run(src, "lumelir_ipairs_hole").trim(), "1\t1\n2\t2");
}

// ============================================================
// Body access patterns
// ============================================================

#[test]
fn ipairs_modifies_outer_state() {
    let src = "local sum = 0
for i, v in ipairs({1, 2, 3, 4}) do sum = sum + v end
print(sum)";
    assert_eq!(run(src, "lumelir_ipairs_sum").trim(), "10");
}

#[test]
fn ipairs_uses_idx_in_body() {
    let src = "for i, v in ipairs({\"a\", \"b\", \"c\"}) do
  print(i .. \":\" .. v)
end";
    assert_eq!(run(src, "lumelir_ipairs_concat").trim(), "1:a\n2:b\n3:c");
}

// ============================================================
// Break support
// ============================================================

#[test]
fn ipairs_break_exits_loop() {
    let src = "for i, v in ipairs({1, 2, 3}) do
  if i == 2 then break end
  print(v)
end
print(\"done\")";
    assert_eq!(run(src, "lumelir_ipairs_break").trim(), "1\ndone");
}

// ============================================================
// Nested iteration
// ============================================================

#[test]
fn ipairs_nested_inner_outer() {
    // Outer iterates {10, 20} (vals), inner iterates {3, 4} (vals).
    // Print (outer_val, inner_val) so we observe both loops.
    let src = "for _i, ov in ipairs({10, 20}) do
  for _j, iv in ipairs({3, 4}) do
    print(ov, iv)
  end
end";
    assert_eq!(
        run(src, "lumelir_ipairs_nested").trim(),
        "10\t3\n10\t4\n20\t3\n20\t4"
    );
}

// ============================================================
// Negative — restricted shapes (parser-level reject)
// ============================================================

#[test]
fn pairs_call_rejected_at_parser() {
    // Plan C scope: only `ipairs(t)` is recognised. `pairs(t)`
    // requires hash-iteration protocol (LIC-2.8e-iter-pairs-1).
    let result = lumelir::parser::parse(
        "local t = {}
for k, v in pairs(t) do print(k) end",
    );
    assert!(
        result.is_err(),
        "pairs(t) in for-in must be parser-rejected (LIC-2.8e-iter-pairs-1)"
    );
}

#[test]
fn arbitrary_iter_call_rejected_at_parser() {
    // Plan C scope: only `ipairs(t)` is recognised. Generic for
    // protocol with arbitrary callable iter requires ADR 0075's
    // indirect-call reject to be reopened (LIC-2.8e-iter-generic-1).
    let result = lumelir::parser::parse(
        "local function my_iter() return nil end
for x, y in my_iter() do print(x) end",
    );
    assert!(
        result.is_err(),
        "arbitrary iter callable in for-in must be parser-rejected (LIC-2.8e-iter-generic-1)"
    );
}
