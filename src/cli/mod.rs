mod compile;
pub(crate) mod diag;
mod run;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::pipeline::EmitStage;

#[derive(Parser)]
#[command(
    name = "lumelir",
    version,
    about = "Lua -> MLIR -> CPU/GPU/FPGA compiler toolchain",
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile a Lua source file into a native binary.
    Compile {
        /// Input Lua source file.
        input: PathBuf,
        /// Output path. Without `--emit`, the native binary path
        /// (defaults to the input file with the extension stripped).
        /// With `--emit`, the dump destination (defaults to stdout).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Target backend (cpu | gpu | fpga).
        #[arg(long, default_value = "cpu")]
        target: String,
        /// Stop the pipeline at the named stage and emit its text
        /// representation. ADR 0090.
        #[arg(long, value_enum)]
        emit: Option<EmitStage>,
    },
    /// Compile and immediately execute a Lua source file.
    Run {
        /// Input Lua source file.
        input: PathBuf,
    },
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Compile {
            input,
            output,
            target,
            emit,
        } => compile::invoke(&input, output.as_deref(), &target, emit),
        Commands::Run { input } => run::invoke(&input),
    }
}
