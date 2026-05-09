//! Phase 2.8e-iter-generic (ADR 0085): full Lua 5.4 §3.3.5 generic-for
//! protocol — `for k, v in ITER, STATE, CTL do BODY end`.
//!
//! Phase 1 scope (per ADR 0085):
//! - iter must resolve to `Builtin::Next`, a top-level user function,
//!   or a Function-kind local with FuncId; closure-as-iter is rejected
//!   until ADR 0083 lands the env-threading ABI.
//! - iter must return `(TaggedValue, TaggedValue)` so a `nil` first
//!   result can terminate the loop. User functions reach this shape
//!   when they have at least one `return nil, nil` or `return` (with
//!   no values widened by ADR 0074).
//! - 2 names in the binding (`for k, v in ...`); 1-name and 3+-name
//!   forms remain HIR-/parser-rejected.

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output).output().unwrap();
    let _ = std::fs::remove_file(&output);
    assert!(result.status.success(), "binary should exit 0: {result:?}");
    String::from_utf8_lossy(&result.stdout).into_owned()
}

fn run_lines_sorted(src: &str, output_name: &str) -> Vec<String> {
    let stdout = run(src, output_name);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = trimmed.split('\n').map(str::to_owned).collect();
    lines.sort();
    lines
}

#[test]
fn generic_for_with_next_builtin() {
    // `for k, v in next, t, nil` is the canonical desugar of
    // `for k, v in pairs(t)`. ADR 0085 makes it user-writable.
    let src = "local t = {}
t.a = 1
t.b = 2
t.c = 3
for k, v in next, t, nil do
  print(k, v)
end";
    assert_eq!(
        run_lines_sorted(src, "lumelir_gen_next"),
        vec!["a\t1", "b\t2", "c\t3"]
    );
}

#[test]
fn generic_for_with_user_fn_iter() {
    // User function as iter (non-capturing). The iter returns
    // (Number, Number) on live entries and (nil, nil) on
    // exhaustion — ADR 0074 widens its ret_kinds to (TaggedValue,
    // TaggedValue) so generic-for can receive the nil terminator.
    let src = "local function range_iter(n, i)
  i = i + 1
  if i <= n then return i, i * 10 end
  return nil, nil
end
for idx, val in range_iter, 3, 0 do
  print(idx, val)
end";
    assert_eq!(run(src, "lumelir_gen_user").trim(), "1\t10\n2\t20\n3\t30");
}

#[test]
fn generic_for_with_function_alias_iter() {
    // Same as above but iter is reached via a function alias —
    // exercises the `Local(Function)` → `Callee::User(known FuncId)`
    // path of the iter resolution dispatch.
    let src = "local function range_iter(n, i)
  i = i + 1
  if i <= n then return i, i * 10 end
  return nil, nil
end
local f = range_iter
for idx, val in f, 2, 0 do
  print(idx, val)
end";
    assert_eq!(run(src, "lumelir_gen_alias").trim(), "1\t10\n2\t20");
}

#[test]
fn generic_for_break_exits_loop() {
    // ADR 0015 break wrap propagates through the synthetic body —
    // a mid-iteration `break` skips the rest of the body and the
    // next `while` re-check terminates.
    let src = "local function range_iter(n, i)
  i = i + 1
  if i <= n then return i, i * 10 end
  return nil, nil
end
for idx, val in range_iter, 5, 0 do
  if idx == 3 then break end
  print(idx, val)
end
print(\"done\")";
    assert_eq!(run(src, "lumelir_gen_break").trim(), "1\t10\n2\t20\ndone");
}

#[test]
fn generic_for_nested_two_levels() {
    let src = "local function range_iter(n, i)
  i = i + 1
  if i <= n then return i, i * 10 end
  return nil, nil
end
for o_idx, o_val in range_iter, 2, 0 do
  for i_idx, i_val in range_iter, 2, 0 do
    print(o_idx, i_idx, o_val + i_val)
  end
end";
    assert_eq!(
        run(src, "lumelir_gen_nested").trim(),
        "1\t1\t20\n1\t2\t30\n2\t1\t30\n2\t2\t40"
    );
}

#[test]
fn generic_for_iter_returns_nil_terminates_immediately() {
    // iter returns nil first call → loop body never runs.
    // Empty table over `next` exercises the termination path through
    // the standard TaggedValue ABI without needing a Nil-only user
    // function (whose static `(Nil, Nil)` ret_kinds creates a
    // codegen mismatch in the multi-assign value receivers).
    let src = "local t = {}
for k, v in next, t, nil do
  print(\"never\")
end
print(\"after\")";
    assert_eq!(run(src, "lumelir_gen_empty").trim(), "after");
}

#[test]
fn generic_for_capturing_iter_via_inline_anonymous_now_runs() {
    // ADR 0083 Commit 3c: a capturing iter assigned to a
    // Function-kind local with a known func_id (anonymous
    // FunctionExpr literal) flows through the generic-for
    // dispatch with the cell-ptr-first ABI. The captured `n`
    // and `i` are reachable on each step.
    //
    // The factory-return form (`local iter = make_factory(3)`)
    // remains rejected for an unrelated reason: ret_kinds for
    // a Function-kind local without func_id defaults to single
    // Number, so the iter type-check fails before reaching the
    // closure layer. That gap is out of scope for 3c.
    let chunk = lumelir::parser::parse(
        "local n = 3
local i = 0
local iter = function(s, c)
  i = i + 1
  if i <= n then return i, i * 10 end
  return nil, nil
end
for k, v in iter, 0, 0 do
  print(k, v)
end",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_ok(),
        "inline anonymous capturing iter must lower post-3c"
    );
}

#[test]
fn generic_for_iter_returning_pure_number_pair_rejected() {
    // ADR 0085 Phase 1 requires iter's first return to be
    // TaggedValue so a `nil` can terminate. A user function whose
    // ret_kinds is statically (Number, Number) — no fall-through
    // empty return — would loop forever; HIR rejects it upfront.
    let chunk = lumelir::parser::parse(
        "local function bad_iter(s, c)
  return c + 1, c * 10
end
for k, v in bad_iter, 0, 0 do
  print(k, v)
end",
    )
    .unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("expected TypeMismatch on Number-only iter");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("TaggedValue"),
        "expected error to mention TaggedValue requirement, got: {msg}"
    );
}
