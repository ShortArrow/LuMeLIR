//! ADR 0218 — chunk-safe real GC freeing. For chunks with no
//! Tables / user functions / TaggedValue locals (chunk-safe
//! gate), the GC mark phase walks `g_chunk_root_table` instead
//! of the v1 safety-mode mark-all-BLACK loop. Transient
//! intermediate allocations (concat scratch buffers, intermediate
//! string objects) become WHITE and are freed by sweep.

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
fn rooted_string_survives_collection() {
    // The chunk-mark walks `g_chunk_root_table`; `s` is the only
    // root and points at the final concat result. Sweep must NOT
    // free that one.
    assert_eq!(
        run_ok(
            "local s = \"hello\"
collectgarbage()
print(s)",
            "lumelir_gc_rooted_survives"
        )
        .trim(),
        "hello"
    );
}

#[test]
fn concat_chain_intermediate_objects_are_freed() {
    // The final value of `s` is `"abcd"`; intermediate `"ab"`,
    // `"abc"` are unreferenced after each reassignment. After
    // collectgarbage `freed > 0`, but `s` still prints correctly.
    let out = run_ok(
        "local s = \"a\"
s = s .. \"b\"
s = s .. \"c\"
s = s .. \"d\"
local freed = collectgarbage()
print(s)
if freed > 0 then print(\"freed\") else print(\"nope\") end",
        "lumelir_gc_concat_chain",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "abcd");
    assert_eq!(lines[1], "freed");
}

#[test]
fn second_collect_after_concat_chain_is_smaller_delta() {
    // After the first collection's sweep, subsequent
    // collectgarbage() should report a small delta (or 0) because
    // there is nothing transient left to free. Pin: 1st > 2nd
    // (loosely) — both must succeed without crash.
    let out = run_ok(
        "local s = \"a\"
s = s .. \"b\"
s = s .. \"c\"
local first = collectgarbage()
local second = collectgarbage()
if first >= second then print(\"monotone\") else print(\"bad\") end",
        "lumelir_gc_second_collect",
    );
    assert_eq!(out.trim(), "monotone");
}
