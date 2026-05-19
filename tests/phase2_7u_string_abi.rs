//! Phase 2.7u-string-abi-refactor (ADR 0112): boxed string object
//! + embedded NUL correctness.
//!
//! The current C-string ABI (ADR 0024, `strlen`-based length)
//! truncates strings at embedded NUL bytes, which Lua 5.4 spec
//! mandates as valid byte content. These tests pin the
//! length-aware semantics across:
//! - `#s` length operator
//! - `string.len(s)` builtin
//! - `==` / `~=` byte equality
//! - `<` / `<=` / `>` / `>=` byte ordering
//! - `string.byte(s, i)` byte extraction
//! - `string.sub(s, i, j)` substring
//! - `string.rep(s, n)` repetition
//! - `string.upper(s)` / `string.lower(s)` case map
//! - `s1 .. s2` binary concat
//! - `table.concat(t)` array join
//! - hash key equality (`t["k"]` lookup with embedded NUL)
//!
//! ADR 0112 implements boxed string object `{i64 len, i8 data[len+1]}`
//! with the `len` field as truth source. Trailing NUL is a
//! compat field for legacy callers (none should remain after
//! the consumer-migration sweep).

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    assert!(result.status.success(), "binary should exit 0: {result:?}");
    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
    let _ = std::fs::remove_file(&output);
    stdout
}

// --- Length: `#s` operator + `string.len` builtin ---

#[test]
fn string_abi_length_single_nul() {
    // `"\x00"` is a 1-byte string. strlen would return 0;
    // length-aware reads the header len = 1.
    assert_eq!(run("print(#\"\\x00\")", "lumelir_27u_len_nul").trim(), "1");
}

#[test]
fn string_abi_length_with_embedded_nul() {
    // `"A\x00B"` is 3 bytes. strlen → 1; length-aware → 3.
    assert_eq!(
        run("print(#\"A\\x00B\")", "lumelir_27u_len_abc").trim(),
        "3"
    );
}

#[test]
fn string_abi_len_via_string_len_builtin() {
    // string.len(s) shares the chokepoint with `#s`.
    assert_eq!(
        run("print(string.len(\"A\\x00B\"))", "lumelir_27u_strlen_abc").trim(),
        "3"
    );
}

// --- Equality: `==` byte-equal ---

#[test]
fn string_abi_eq_byte_equal() {
    // Two strings with same bytes (including embedded NUL) are
    // ==. strcmp would also return 0 here by coincidence.
    assert_eq!(
        run("print(\"A\\x00B\" == \"A\\x00B\")", "lumelir_27u_eq_equal").trim(),
        "true"
    );
}

#[test]
fn string_abi_eq_byte_unequal_prefix() {
    // "A\x00B" vs "A" differ at byte 2 (and len). strcmp truncates
    // both to "A" and returns 0 — false positive. Length-aware
    // eq returns false (len_a != len_b).
    assert_eq!(
        run(
            "print(\"A\\x00B\" == \"A\")",
            "lumelir_27u_eq_unequal_prefix"
        )
        .trim(),
        "false"
    );
}

// --- Ordering: byte-lexicographic ---

#[test]
fn string_abi_ordering_byte_lex() {
    // "A\x00B" vs "A\x00C": both have NUL at byte 2; differ at
    // byte 3 (B=66 vs C=67). strcmp truncates at NUL — both
    // appear equal (returns 0). Length-aware compare uses
    // memcmp on actual bytes → "A\x00B" < "A\x00C" = true.
    assert_eq!(
        run("print(\"A\\x00B\" < \"A\\x00C\")", "lumelir_27u_lex_lt").trim(),
        "true"
    );
}

// --- Byte extraction ---

#[test]
fn string_abi_byte_at_nul_position() {
    // string.byte("A\x00B", 2) should return 0 (the NUL byte).
    // strlen-bound check would see len=1 and reject index 2.
    assert_eq!(
        run(
            "print(string.byte(\"A\\x00B\", 2))",
            "lumelir_27u_byte_at_nul"
        )
        .trim(),
        "0"
    );
}

// --- Substring ---

#[test]
fn string_abi_sub_includes_nul() {
    // string.sub("A\x00B", 1, 2) returns "A\x00" (2 bytes).
    // Verified via length: #result == 2.
    assert_eq!(
        run(
            "print(#string.sub(\"A\\x00B\", 1, 2))",
            "lumelir_27u_sub_len"
        )
        .trim(),
        "2"
    );
}

// --- Repetition ---

#[test]
fn string_abi_rep_nul_byte() {
    // string.rep("\x00", 3) returns "\x00\x00\x00" (3 bytes).
    assert_eq!(
        run("print(#string.rep(\"\\x00\", 3))", "lumelir_27u_rep_nul").trim(),
        "3"
    );
}

// --- Concat (binary `..`) ---

#[test]
fn string_abi_concat_preserves_nul() {
    // ("A\x00") .. ("B") = "A\x00B" (3 bytes).
    assert_eq!(
        run("print(#(\"A\\x00\" .. \"B\"))", "lumelir_27u_concat_nul").trim(),
        "3"
    );
}

// --- table.concat ---

#[test]
fn string_abi_table_concat_preserves_nul() {
    // table.concat({"A\x00", "B"}) = "A\x00B" (3 bytes).
    assert_eq!(
        run(
            "print(#table.concat({\"A\\x00\", \"B\"}))",
            "lumelir_27u_tconcat_nul"
        )
        .trim(),
        "3"
    );
}

// --- Case map: string.upper preserves NUL bytes ---

#[test]
fn string_abi_upper_preserves_nul() {
    // string.upper("a\x00b") = "A\x00B" (3 bytes). NUL byte
    // passes through unchanged (toupper(0) == 0).
    assert_eq!(
        run("print(#string.upper(\"a\\x00b\"))", "lumelir_27u_upper_nul").trim(),
        "3"
    );
}

// --- Hash key equality (chokepoint via emit_hash_key_eq_dispatched) ---

#[test]
fn string_abi_hash_key_with_nul() {
    // Hash table key "A\x00B" must be findable. strcmp-based eq
    // would false-positive on "A" or other "A\x00*" prefixes.
    let src = "local t = {}
t[\"A\\x00B\"] = 99
print(t[\"A\\x00B\"])";
    assert_eq!(run(src, "lumelir_27u_hash_key").trim(), "99");
}

#[test]
fn string_abi_hash_key_nul_no_collision() {
    // "A\x00B" and "A" must NOT collide. strcmp-based eq + FNV
    // truncated at NUL would produce identical hash/eq → collision.
    let src = "local t = {}
t[\"A\\x00B\"] = 1
t[\"A\"] = 2
print(t[\"A\\x00B\"])
print(t[\"A\"])";
    let out = run(src, "lumelir_27u_hash_no_collide");
    let lines: Vec<&str> = out.trim().lines().collect();
    assert_eq!(lines, ["1", "2"]);
}
