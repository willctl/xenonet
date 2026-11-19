//! The eidolon command.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    eidolon_cli::run(&args, &mut std::io::stdout(), &mut std::io::stderr())
}
