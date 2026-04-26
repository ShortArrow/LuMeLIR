//! Lower an MLIR module to LLVM IR text.
//!
//! Uses `mlir-opt` passes to convert arith/func to LLVM dialect,
//! then `mlir-translate --mlir-to-llvmir` to emit LLVM IR.

use std::io::Write;
use std::process::Command;

use melior::ir::Module;

use super::error::CodegenError;

/// Lower an MLIR module to LLVM IR (as a String).
///
/// Pipeline: `mlir-opt` (arith→llvm, func→llvm, reconcile-casts) → `mlir-translate`.
pub fn to_llvm_ir(module: &Module<'_>) -> Result<String, CodegenError> {
    let mlir_text = module.as_operation().to_string();

    let lowered = run_mlir_opt(&mlir_text)?;
    let llvm_ir = run_mlir_translate(&lowered)?;
    Ok(llvm_ir)
}

fn run_mlir_opt(mlir: &str) -> Result<String, CodegenError> {
    let mut child = Command::new("mlir-opt")
        .args([
            "--convert-arith-to-llvm",
            "--convert-func-to-llvm",
            "--reconcile-unrealized-casts",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| CodegenError::LoweringFailed(format!("failed to spawn mlir-opt: {e}")))?;

    child.stdin.as_mut().unwrap().write_all(mlir.as_bytes())?;

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CodegenError::LoweringFailed(format!(
            "mlir-opt failed: {stderr}"
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_mlir_translate(mlir: &str) -> Result<String, CodegenError> {
    let mut child = Command::new("mlir-translate")
        .arg("--mlir-to-llvmir")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            CodegenError::LoweringFailed(format!("failed to spawn mlir-translate: {e}"))
        })?;

    child.stdin.as_mut().unwrap().write_all(mlir.as_bytes())?;

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CodegenError::LoweringFailed(format!(
            "mlir-translate failed: {stderr}"
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::emit::{emit_module, new_context};
    use crate::{hir, parser};

    #[test]
    fn to_llvm_ir_produces_valid_llvm() {
        let ctx = new_context();
        let chunk = parser::parse("print(1 + 2)").unwrap();
        let hir = hir::lower(&chunk).unwrap();
        let module = emit_module(&ctx, &hir).unwrap();
        let llvm_ir = to_llvm_ir(&module).unwrap();
        assert!(
            llvm_ir.contains("define"),
            "expected LLVM IR 'define', got:\n{llvm_ir}"
        );
        assert!(
            llvm_ir.contains("@main"),
            "expected @main in LLVM IR, got:\n{llvm_ir}"
        );
    }
}
