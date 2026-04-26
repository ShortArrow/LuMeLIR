/// Errors arising from the MLIR codegen pipeline.
#[derive(Debug, thiserror::Error)]
pub enum CodegenError {
    #[error("unsupported expression: {0}")]
    UnsupportedExpr(String),

    #[error("MLIR module verification failed")]
    VerificationFailed,

    #[error("MLIR lowering failed: {0}")]
    LoweringFailed(String),

    #[error("linker failed: {0}")]
    LinkFailed(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
