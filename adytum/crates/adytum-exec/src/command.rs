use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A program and its argument list. Never a shell string: every argument
/// reaches the child as one argument, exactly as given.
#[derive(Clone, PartialEq, Eq)]
pub struct Command {
    program: OsString,
    args: Vec<OsString>,
    env: BTreeMap<OsString, OsString>,
    cwd: Option<PathBuf>,
    deadline: Option<Duration>,
    stdin: Option<Vec<u8>>,
}

impl Command {
    #[must_use]
    pub fn new(program: impl AsRef<OsStr>) -> Command {
        Command { program: program.as_ref().to_owned(), args: Vec::new(), env: BTreeMap::new(), cwd: None, deadline: None, stdin: None }
    }

    #[must_use]
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Command {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args.extend(args.into_iter().map(|a| a.as_ref().to_owned()));
        self
    }

    /// Sets one variable for the child. The child's environment starts
    /// empty, so this and `pass_env` are the only ways a variable gets in.
    #[must_use]
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Command {
        self.env.insert(key.as_ref().to_owned(), value.as_ref().to_owned());
        self
    }

    /// Copies the named variables from this process, skipping any that
    /// aren't set.
    #[must_use]
    pub fn pass_env(mut self, names: &[&str]) -> Command {
        for name in names {
            if let Some(value) = std::env::var_os(name) {
                self.env.insert(OsString::from(name), value);
            }
        }
        self
    }

    #[must_use]
    pub fn cwd(mut self, dir: impl AsRef<Path>) -> Command {
        self.cwd = Some(dir.as_ref().to_owned());
        self
    }

    /// The longest the command may run before its process group is stopped.
    #[must_use]
    pub fn deadline(mut self, deadline: Duration) -> Command {
        self.deadline = Some(deadline);
        self
    }

    /// Bytes written to the child's stdin, which is then closed.
    #[must_use]
    pub fn stdin_bytes(mut self, bytes: Vec<u8>) -> Command {
        self.stdin = Some(bytes);
        self
    }

    pub fn program(&self) -> &OsStr {
        &self.program
    }

    pub fn arg_list(&self) -> &[OsString] {
        &self.args
    }

    pub fn env_vars(&self) -> &BTreeMap<OsString, OsString> {
        &self.env
    }

    pub fn working_dir(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    pub fn deadline_limit(&self) -> Option<Duration> {
        self.deadline
    }

    pub fn stdin(&self) -> Option<&[u8]> {
        self.stdin.as_deref()
    }

    /// The program followed by its arguments, as a list of strings (lossy
    /// for non-UTF-8). Used in messages and recorded transcripts.
    pub fn argv(&self) -> Vec<String> {
        std::iter::once(&self.program).chain(self.args.iter()).map(|a| a.to_string_lossy().into_owned()).collect()
    }
}

/// Environment values may be secrets, so Debug names the variables only.
impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Command")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .field("cwd", &self.cwd)
            .field("deadline", &self.deadline)
            .field("stdin_bytes", &self.stdin.as_ref().map(Vec::len))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_names_variables_but_never_values() {
        let cmd = Command::new("tool").env("TOKEN", "hunter2-secret").arg("x");
        let shown = format!("{cmd:?}");
        assert!(shown.contains("TOKEN"));
        assert!(!shown.contains("hunter2"));
    }

    #[test]
    fn pass_env_skips_unset_names() {
        let cmd = Command::new("tool").pass_env(&["ADYTUM_EXEC_SURELY_UNSET_VARIABLE"]);
        assert!(cmd.env_vars().is_empty());
    }
}
