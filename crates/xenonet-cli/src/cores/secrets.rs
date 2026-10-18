//! core/secret-store.js for the command: your computer's own credential
//! store (xenonet-actors' secret store says what its program is given, a
//! secret only ever on standard input, and reads its answer), its program
//! run here as the twin's runFile ran it: never an error, -1 for a program
//! that couldn't start or didn't finish in 20 seconds.

use super::native::{on_path, run};
use crate::sys;
use xenonet_actors::secret_store::{Command, Ran, Run, SecretStore};

/// createSecretStore() for this computer: secret-tool on PATH, on Linux.
pub fn store() -> SecretStore {
    let platform = sys::platform();
    let tool = if platform == "darwin" || platform == "win32" { None } else { on_path("secret-tool") };
    SecretStore::new(platform, tool)
}

pub struct Runner;

impl Run for Runner {
    fn run(&mut self, c: Command) -> Result<Ran, String> {
        // An empty input, as child.stdin.end("") gives, when there's none.
        let r = run(&c.file, &c.args, 20000.0, Some(c.input.as_deref().unwrap_or("")));
        Ok(Ran { code: r.code.map_or(-1.0, f64::from), stdout: r.stdout })
    }
}
