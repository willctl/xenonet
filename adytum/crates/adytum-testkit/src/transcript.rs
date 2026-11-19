use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A golden file: the calls a tool saw, in order.
///
/// ```json
/// { "tool": "limactl", "version": "2.1.2", "calls": [
///   { "argv": ["limactl", "list", "--json"], "stdin": null,
///     "stdout": "...", "stderr": "", "status": 0 } ] }
/// ```
///
/// `argv` and `stdout` may hold `{home}`, `{tmp}` (and any variable given to
/// `FakeRunner::with_var`); in `argv` an argument that is exactly `{any}`
/// matches one argument of anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub tool: String,
    pub version: String,
    pub calls: Vec<Call>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub argv: Vec<String>,
    #[serde(default)]
    pub stdin: Option<String>,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default)]
    pub status: i32,
}

impl Transcript {
    pub fn from_json(text: &str) -> Result<Transcript, TranscriptError> {
        serde_json::from_str(text).map_err(TranscriptError::Parse)
    }

    pub fn from_file(path: &Path) -> Result<Transcript, TranscriptError> {
        let text = std::fs::read_to_string(path).map_err(|source| TranscriptError::Read { path: path.to_path_buf(), source })?;
        Transcript::from_json(&text)
    }

    pub fn to_json(&self) -> Result<String, TranscriptError> {
        serde_json::to_string_pretty(self).map_err(TranscriptError::Parse)
    }

    pub fn write_to(&self, path: &Path) -> Result<(), TranscriptError> {
        let mut text = self.to_json()?;
        text.push('\n');
        std::fs::write(path, text).map_err(|source| TranscriptError::Write { path: path.to_path_buf(), source })
    }
}

/// Why a golden file could not be read or written.
#[derive(Debug)]
pub enum TranscriptError {
    Read { path: PathBuf, source: std::io::Error },
    Write { path: PathBuf, source: std::io::Error },
    Parse(serde_json::Error),
}

impl fmt::Display for TranscriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranscriptError::Read { path, source } => write!(f, "Could not read the golden file {}: {source}. Check the path.", path.display()),
            TranscriptError::Write { path, source } => write!(f, "Could not write the golden file {}: {source}. Check the folder exists.", path.display()),
            TranscriptError::Parse(e) => write!(f, "The golden file is not a valid transcript: {e}. Check it against the format in the Transcript docs."),
        }
    }
}

impl std::error::Error for TranscriptError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TranscriptError::Read { source, .. } | TranscriptError::Write { source, .. } => Some(source),
            TranscriptError::Parse(e) => Some(e),
        }
    }
}

/// Replaces each `{name}` with its value.
pub(crate) fn substitute(text: &str, vars: &[(String, String)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// The reverse, for recording: longest values first, so a folder inside the
/// home folder isn't half-replaced.
pub(crate) fn abstract_paths(text: &str, vars: &[(String, String)]) -> String {
    let mut ordered: Vec<&(String, String)> = vars.iter().filter(|(_, v)| !v.is_empty()).collect();
    ordered.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    let mut out = text.to_owned();
    for (name, value) in ordered {
        out = out.replace(value.as_str(), &format!("{{{name}}}"));
    }
    out
}

pub(crate) fn default_vars() -> Vec<(String, String)> {
    let mut vars = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        vars.push(("home".to_owned(), home.to_string_lossy().trim_end_matches('/').to_owned()));
    }
    vars.push(("tmp".to_owned(), std::env::temp_dir().to_string_lossy().trim_end_matches('/').to_owned()));
    vars
}

pub(crate) fn set_var(vars: &mut Vec<(String, String)>, name: &str, value: &str) {
    vars.retain(|(n, _)| n != name);
    vars.push((name.to_owned(), value.trim_end_matches('/').to_owned()));
}
