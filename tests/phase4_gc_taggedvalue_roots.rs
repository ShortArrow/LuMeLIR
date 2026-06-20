//! ADR 0220 — chunk-safe predicate relaxed to include TaggedValue
//! chunk locals. The GC mark phase walks a parallel
//! `g_chunk_tv_table`; for each TV slot it reads the tag at
//! offset 0 and follows the payload ptr at offset 8 when the tag
//! is TAG_STRING, comparing against `g_gc_head` entries.
//!
//! Also covers the pcall + setjmp-depth-restore composition
//! required to prevent caught longjmp from leaking g_gc_fn_depth.

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
fn tagged_value_err_string_rooted_across_collection() {
    // `bad` raises a GC-allocated string ("oops_" .. "x"). pcall
    // catches and stores it into the TaggedValue `err` slot. The
    // chunk root scan walks `g_chunk_tv_table`, recognises tag ==
    // TAG_STRING, and matches the payload ptr — so the obj
    // survives. Without ADR 0220 the obj would be freed and
    // print(err) would observe corrupted memory.
    assert_eq!(
        run_ok(
            "local function bad() error(\"oops_\" .. \"x\") end
local ok, err = pcall(bad)
collectgarbage()
print(err)",
            "lumelir_gc_tv_err_rooted"
        )
        .trim(),
        "oops_x"
    );
}

#[test]
fn caught_pcall_does_not_leak_fn_depth() {
    // After pcall catches an error, g_gc_fn_depth must be back
    // to its pre-pcall value. If it leaked, the subsequent
    // chunk-level collectgarbage would fall back to v1 safety
    // mode and `freed` would be 0; with the ADR 0220 restore,
    // the chunk-level call observes real freeing.
    let out = run_ok(
        "local function bad() local s = \"a\" .. \"b\"; error(\"x\") end
local ok, err = pcall(bad)
local freed = collectgarbage()
if freed > 0 then print(\"freed\") else print(\"leaked\") end",
        "lumelir_gc_caught_no_leak",
    );
    assert_eq!(out.trim(), "freed");
}

#[test]
fn pcall_success_path_preserves_real_freeing() {
    // Success path: f returns normally; depth restore brings
    // g_gc_fn_depth back to 0; chunk-level collect after still
    // observes real freeing of transient intermediates.
    let out = run_ok(
        "local function good() return 1 + 2 end
local ok, err = pcall(good)
local s = \"a\"
s = s .. \"b\"
s = s .. \"c\"
s = s .. \"d\"
local freed = collectgarbage()
print(s)
if freed > 0 then print(\"freed\") else print(\"zero\") end",
        "lumelir_gc_pcall_success_freeing",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "abcd");
    assert_eq!(lines[1], "freed");
}
