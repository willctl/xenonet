//! The agent's process: its command as the registry gives it, started with
//! no window, in a process tree that stops together, with an environment
//! that holds no credential of XenoNet's and no token; and its standard
//! input and output as the lines ACP's connection runs over.

use adytum_exec::{Command, Spawned, StderrTail};
use agent_client_protocol::Lines;
use futures::{Sink, Stream};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// How much of the agent's standard error is kept, for the error when it
/// fails to start or exits: the last bytes of it.
const STDERR_KEPT: usize = 8 * 1024;

/// Words in a variable's name that mark it as holding a secret. The
/// agent signs in with what it keeps itself; XenoNet hands it no key.
const SECRET_WORDS: &[&str] = &["TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL", "API_KEY", "APIKEY", "PRIVATE_KEY", "ACCESS_KEY", "AUTH_KEY", "SESSION_KEY", "COOKIE"];

/// Whether a variable of this name is kept from the agent: every one of
/// XenoNet's own (its credential, its actor tokens, its port and folders),
/// and every name that says it holds a secret.
pub fn holds_secret(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    up.starts_with("XENONET_") || SECRET_WORDS.iter().any(|w| up.contains(w))
}

/// The environment the agent runs with: `base` (the service's, as a rule)
/// without what `holds_secret` names, and without the variables that mark
/// a process as another app's session (`marks`, the start of their names,
/// as the registry gives them).
pub fn agent_env(base: impl IntoIterator<Item = (String, String)>, marks: &[&str]) -> Vec<(String, String)> {
    base.into_iter().filter(|(k, _)| !holds_secret(k) && !marks.iter().any(|m| k.starts_with(m))).collect()
}

/// XenoNet's own MCP server, the one server a session it opens is told of:
/// its name in the session, and the command that runs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mcp {
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
}

/// An agent's command: the program, its arguments, the folder it starts in
/// and its environment, which only `agent_env` makes.
#[derive(Clone, Debug)]
pub struct Launch {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
}

impl Launch {
    /// The agent `program` with `args`, started in `cwd`, with `base`
    /// through `agent_env` (and `marks`) as its whole environment.
    pub fn new(program: impl Into<PathBuf>, args: Vec<String>, cwd: impl Into<PathBuf>, base: impl IntoIterator<Item = (String, String)>, marks: &[&str]) -> Launch {
        Launch { program: program.into(), args, cwd: cwd.into(), env: agent_env(base, marks) }
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// The environment it runs with, all of it.
    pub fn env(&self) -> &[(String, String)] {
        &self.env
    }

    /// Starts it through adytum-exec: no window on Windows, its own process
    /// group (so a signal to the service's terminal doesn't reach it, and
    /// stopping it stops what it started), and `env` as its whole
    /// environment. Its standard error is kept, the last of it, for errors.
    pub(crate) fn spawn(&self) -> std::io::Result<Running> {
        let cmd = Command::new(&self.program).args(&self.args).cwd(&self.cwd);
        let cmd = self.env.iter().fold(cmd, |c, (k, v)| c.env(k, v));
        let mut process = Spawned::start(&cmd).map_err(|e| std::io::Error::other(e.to_string()))?;
        let (Some(stdin), Some(stdout)) = (process.stdin.take(), process.stdout.take()) else {
            process.kill_group();
            return Err(std::io::Error::other("the agent's standard input and output weren't there"));
        };
        let stderr = process.stderr_tail(STDERR_KEPT).unwrap_or_default();
        let stdin = tokio::process::ChildStdin::from_std(stdin)?;
        let stdout = tokio::process::ChildStdout::from_std(stdout)?;
        Ok(Running { process, stdin, stdout, stderr })
    }
}

/// A started agent.
pub(crate) struct Running {
    pub process: Spawned,
    pub stdin: tokio::process::ChildStdin,
    pub stdout: tokio::process::ChildStdout,
    pub stderr: StderrTail,
}

/// The agent's standard input and output as the newline-delimited JSON
/// ACP's stdio transport is: one message a line each way.
pub(crate) fn lines(
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
) -> Lines<impl Sink<String, Error = std::io::Error> + Send + 'static, impl Stream<Item = std::io::Result<String>> + Send + 'static> {
    let out = Box::pin(futures::sink::unfold(stdin, async |mut w, line: String| {
        w.write_all(line.as_bytes()).await?;
        w.write_all(b"\n").await?;
        w.flush().await?;
        Ok::<_, std::io::Error>(w)
    }));
    let input = Box::pin(futures::stream::unfold(BufReader::new(stdout).lines(), async |mut l| match l.next_line().await {
        Ok(Some(line)) => Some((Ok(line), l)),
        Ok(None) => None,
        Err(e) => Some((Err(e), l)),
    }));
    Lines::new(out, input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_gets_no_credential_and_no_token() {
        let base = [
            ("PATH", "/bin"),
            ("HOME", "/h"),
            ("XENONET_PORT", "4319"),
            ("XENONET_ACTOR_TOKEN", "xenonet_actor_00"),
            ("GITHUB_TOKEN", "ghp_x"),
            ("OpenAI_Api_Key", "sk"),
            ("AWS_SECRET_ACCESS_KEY", "k"),
            ("CLAUDECODE", "1"),
            ("GROK_HOME", "/h/.grok"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        let env = agent_env(base, &["CLAUDECODE"]);
        let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["PATH", "HOME", "GROK_HOME"]);
        let l = Launch::new("agent", vec![], "/w", [("XENONET_UPSTREAM".into(), "x".into()), ("LANG".into(), "C".into())], &[]);
        assert_eq!(l.env(), [("LANG".to_string(), "C".to_string())]);
    }
}
