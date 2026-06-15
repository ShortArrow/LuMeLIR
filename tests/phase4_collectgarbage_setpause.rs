//! ADR 0200 — `collectgarbage("setpause", n)` configurable
//! threshold multiplier. Stores new value, returns previous.

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
fn collectgarbage_setpause_returns_previous_value() {
    // First call: previous is 200 (default). Second call: previous is 150.
    let src = r#"
local prev1 = collectgarbage("setpause", 150)
local prev2 = collectgarbage("setpause", 300)
print(prev1, prev2)
"#;
    assert_eq!(run_ok(src, "lumelir_cg_setpause").trim(), "200\t150");
}
