use std::process::ExitCode;

use devmap::error::DevMapError;

fn main() -> ExitCode {
    match devmap::run(std::env::args_os()) {
        Ok(output) => {
            print!("{}", output.stdout);
            ExitCode::SUCCESS
        }
        Err(DevMapError::Cli(error)) => error.exit(),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
