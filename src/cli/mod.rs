mod compile;
mod run;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

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
        /// Output binary path.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Target backend (cpu | gpu | fpga).
        #[arg(long, default_value = "cpu")]
        target: String,
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
        } => compile::invoke(&input, output.as_deref(), &target),
        Commands::Run { input } => run::invoke(&input),
    }
}
