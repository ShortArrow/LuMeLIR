//! Link LLVM IR into a native binary.
//!
//! Pipeline: `llc` (LLVM IR → object) → `cc` (object → native binary).

use std::io::Write;
use std::path::Path;
use std::process::Command;

use super::error::CodegenError;

/// Compile LLVM IR text to a native executable at `output`.
pub fn to_native(llvm_ir: &str, output: &Path) -> Result<(), CodegenError> {
    let obj_path = output.with_extension("o");

    compile_to_object(llvm_ir, &obj_path)?;
    link_object(&obj_path, output)?;

    // Clean up intermediate object file.
    let _ = std::fs::remove_file(&obj_path);
    Ok(())
}

fn compile_to_object(llvm_ir: &str, obj_path: &Path) -> Result<(), CodegenError> {
    let mut child = Command::new("llc")
        .args(["-filetype=obj", "-relocation-model=pic", "-o"])
        .arg(obj_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| CodegenError::LinkFailed(format!("failed to spawn llc: {e}")))?;

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(llvm_ir.as_bytes())?;

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CodegenError::LinkFailed(format!("llc failed: {stderr}")));
    }
    Ok(())
}

fn link_object(obj_path: &Path, output: &Path) -> Result<(), CodegenError> {
    let mut cmd = Command::new("cc");
    cmd.arg(obj_path).args(["-o"]).arg(output);
    // ADR 0191 — bridge object provided by `build.rs` at compile
    // time via `cargo:rustc-env=LUMELIR_BRIDGE_OBJ`. Resolved by
    // `option_env!` so the lookup happens once at compile time
    // rather than per-link at runtime.
    if let Some(bridge_obj) = option_env!("LUMELIR_BRIDGE_OBJ") {
        cmd.arg(bridge_obj);
    }
    // ADR 0315 — N5-A: coroutine core object (ucontext).
    if let Some(coro_obj) = option_env!("LUMELIR_CORO_OBJ") {
        cmd.arg(coro_obj);
    }
    cmd.args(["-lc", "-lm"]);
    let status = cmd
        .status()
        .map_err(|e| CodegenError::LinkFailed(format!("failed to spawn cc: {e}")))?;

    if !status.success() {
        return Err(CodegenError::LinkFailed("cc linking failed".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::emit::{emit_module, new_context};
    use crate::codegen::lower::to_llvm_ir;
    use crate::{hir, parser};

    #[test]
    fn to_native_produces_executable() {
        let ctx = new_context();
        let chunk = parser::parse("print(1 + 2)").unwrap();
        let hir = hir::lower(&chunk).unwrap();
        let module = emit_module(&ctx, &hir).unwrap();
        let llvm_ir = to_llvm_ir(&module).unwrap();

        let tmp = std::env::temp_dir().join("lumelir_test_link");
        to_native(&llvm_ir, &tmp).unwrap();

        assert!(tmp.exists(), "output binary should exist");

        // Run it and check stdout.
        let output = std::process::Command::new(&tmp)
            .output()
            .expect("failed to run compiled binary");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.trim(), "3", "expected '3', got: '{}'", stdout.trim());

        let _ = std::fs::remove_file(&tmp);
    }
}
