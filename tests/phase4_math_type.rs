//! ADR 0210 — `math.type(x)` static-shape subtype distinction.
//! Returns "integer" for HirExprKind::Integer literals, "float"
//! for HirExprKind::Number literals, nil otherwise (subtype not
//! statically derivable through Locals / Calls until ADR 0211+
//! adds tagged-slot subtype tracking).

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
fn math_type_integer_literal_returns_integer() {
    assert_eq!(
        run_ok("print(math.type(42))", "lumelir_mt_int").trim(),
        "integer"
    );
}

#[test]
fn math_type_float_literal_returns_float() {
    assert_eq!(
        run_ok("print(math.type(42.5))", "lumelir_mt_float").trim(),
        "float"
    );
}

#[test]
fn math_type_local_returns_nil_phase_b() {
    // Phase B: subtype lost at Local declaration (silent demotion
    // to ValueKind::Number). ADR 0211+ tracks subtype through
    // tagged slots so this returns "integer".
    assert_eq!(
        run_ok("local x = 42; print(math.type(x))", "lumelir_mt_local").trim(),
        "nil"
    );
}
