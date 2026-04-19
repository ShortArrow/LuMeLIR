use std::process::ExitCode;

fn main() -> ExitCode {
    match lumelir::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lumelir: {err:#}");
            ExitCode::FAILURE
        }
    }
}
