//! ADR 0314 — N4-F-2b/2c: `string.gmatch` closure-iterator form.

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

/// ADR 0314 — N4-F-2b contract: the synthesized iterator is a
/// regular HirFunction flowing through the standard closure
/// pipeline — 3 captured upvalues (s, pat, pos), 2 Nil protocol
/// params, and a nil-terminable first return (the second is the
/// statically-nil val slot the 1-name form discards).
#[test]
fn gmatch_synthesized_closure_shape() {
    use lumelir::hir::ValueKind;
    let chunk =
        lumelir::parser::parse(r#"for w in string.gmatch("ab", "%a") do print(w) end"#).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    let iter = hir
        .functions
        .iter()
        .find(|f| f.mangled_name.starts_with("user_anon_"))
        .expect("synthesized iterator function must exist");
    assert_eq!(iter.params.len(), 2);
    assert!(iter.params.iter().all(|p| p.kind == ValueKind::Nil));
    assert_eq!(iter.upvalues.len(), 3, "captures s, pat, pos");
    assert_eq!(iter.ret_kinds, vec![ValueKind::TaggedValue, ValueKind::Nil]);
}

#[test]
fn gmatch_words_for_in() {
    let out = run_ok(
        r#"
for word in string.gmatch("the quick brown fox", "%a+") do
    print(word)
end
"#,
        "lumelir_gmatch_words",
    );
    assert_eq!(out.trim(), "the\nquick\nbrown\nfox");
}

#[test]
fn gmatch_digits() {
    let out = run_ok(
        r#"
for d in string.gmatch("a1b22c333", "%d+") do
    print(d)
end
"#,
        "lumelir_gmatch_digits",
    );
    assert_eq!(out.trim(), "1\n22\n333");
}

#[test]
fn gmatch_no_match_body_never_runs() {
    let out = run_ok(
        r#"
for x in string.gmatch("abc", "%d") do
    print(x)
end
print("done")
"#,
        "lumelir_gmatch_nomatch",
    );
    assert_eq!(out.trim(), "done");
}

#[test]
fn gmatch_literal_pattern() {
    let out = run_ok(
        r#"
for hit in string.gmatch("xoxoxo", "xo") do
    print(hit)
end
"#,
        "lumelir_gmatch_literal",
    );
    assert_eq!(out.trim(), "xo\nxo\nxo");
}

#[test]
fn gmatch_two_loops_fresh_state() {
    let out = run_ok(
        r#"
for a in string.gmatch("p q", "%a") do
    print(a)
end
for b in string.gmatch("r s", "%a") do
    print(b)
end
"#,
        "lumelir_gmatch_twice",
    );
    assert_eq!(out.trim(), "p\nq\nr\ns");
}

#[test]
fn gmatch_empty_match_advances() {
    let out = run_ok(
        r#"
local n = 0
for e in string.gmatch("ab", "%d*") do
    n = n + 1
end
print(n)
"#,
        "lumelir_gmatch_empty_adv",
    );
    assert_eq!(out.trim(), "3");
}
