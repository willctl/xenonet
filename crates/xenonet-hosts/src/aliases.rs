//! core/hosts/aliases.js: a home folder's SSH aliases (Host gh-work with
//! HostName github.com), read again when ~/.ssh/config changes. Only read,
//! never written. The reading of the file's words is identities.js's.

use xenonet_app::nodepath::NATIVE;

/// When to read ~/.ssh/config again: the stamp (its mtime) it had.
pub struct AliasReader {
    pub file: Option<String>,
    stamp: Option<Option<f64>>,
}

/// What to do now: nothing (what was read stands), forget the aliases
/// (there's no file), or read the file again.
#[derive(Debug, PartialEq)]
pub enum Look {
    Same,
    None,
    Read(String),
}

impl AliasReader {
    pub fn new(home: Option<&str>) -> Self {
        AliasReader { file: home.filter(|h| !h.is_empty()).map(|h| NATIVE.join(&[h, ".ssh", "config"])), stamp: None }
    }

    pub fn look(&mut self) -> Look {
        let Some(file) = &self.file else { return Look::None };
        let now = std::fs::metadata(file).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok());
        if self.stamp == Some(now) {
            return Look::Same;
        }
        self.stamp = Some(now);
        match now {
            None => Look::None,
            Some(_) => Look::Read(file.clone()),
        }
    }
}
