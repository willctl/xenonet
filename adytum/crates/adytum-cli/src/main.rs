//! The adytum command: cells on this machine, from the shell.

use std::io::Write;
use std::sync::Arc;

use adytum_cli::{run, Context};
use adytum_lima::Lima;

fn main() {
    let tokens: Vec<String> = std::env::args().skip(1).collect();
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let code = match start(&tokens, &mut stdout, &mut stderr) {
        Ok(code) => code,
        Err(message) => {
            let _ = writeln!(stderr, "adytum: {message}");
            1
        }
    };
    let _ = stdout.flush();
    std::process::exit(code);
}

fn start(tokens: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<i32, String> {
    let home = adytum_lima::adytum_home().ok_or("Could not find a home folder for Adytum. Set ADYTUM_HOME to a short absolute path.")?;
    let ctx = Context { driver: Arc::new(Lima::system(adytum_lima::lima_home_in(&home))), home };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| format!("Could not start the async runtime: {e}. Try again."))?;
    Ok(runtime.block_on(run(tokens, &ctx, out, err)))
}
