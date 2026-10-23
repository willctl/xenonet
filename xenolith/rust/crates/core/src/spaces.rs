//! Spaces: one wiki each, for a project, an identity, an actor or a machine. The configured wiki
//! folder is this machine's own space (machine:local), so a wiki from before spaces is that space
//! as it is. Every other space is a wiki of the same layout in `spaces/<kind>/<name>/` under it.
//!
//! Space ids read like XenoNet's selector terms: `project:xenonet`, `identity:adal`,
//! `actor:hermes`, `machine:local`. Names are lowercase, so an id is one folder on every OS.
//! Reads of the machine space never reach into `spaces/` (wiki::read_section refuses the path),
//! so a grant on one space never shows another.

use crate::wiki::{self, Result, wiki_err};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const KINDS: [&str; 4] = ["project", "identity", "actor", "machine"];
/// The folder under the machine space that holds every other space.
pub const SPACES_DIR: &str = "spaces";
/// This machine's own space: the configured wiki folder.
pub const LOCAL: &str = "local";

static NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9._-]{0,63}$").unwrap());

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpaceId {
    pub kind: String,
    pub name: String,
}

impl SpaceId {
    /// "project:xenonet" (any case) as a space id; an error says what a space id looks like.
    pub fn parse(text: &str) -> Result<SpaceId> {
        let t = text.trim().to_lowercase();
        let Some((kind, name)) = t.split_once(':') else {
            return wiki_err(format!("\"{}\" is not a space: use project:<name>, identity:<name>, actor:<slug> or machine:local.", text.trim()));
        };
        if !KINDS.contains(&kind) {
            return wiki_err(format!("\"{kind}\" is not a kind of space: use project, identity, actor or machine."));
        }
        if !NAME_RE.is_match(name) || name.contains("..") {
            return wiki_err(format!("\"{name}\" is not a space name: lowercase letters, digits, '.', '_' and '-', starting with a letter or digit, up to 64."));
        }
        Ok(SpaceId { kind: kind.into(), name: name.into() })
    }

    pub fn local() -> SpaceId {
        SpaceId { kind: "machine".into(), name: LOCAL.into() }
    }

    pub fn is_local(&self) -> bool {
        self.kind == "machine" && self.name == LOCAL
    }
}

impl std::fmt::Display for SpaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind, self.name)
    }
}

/// Where a space's wiki lives: the root itself for machine:local.
pub fn space_dir(root: &Path, id: &SpaceId) -> PathBuf {
    if id.is_local() { root.to_path_buf() } else { root.join(SPACES_DIR).join(&id.kind).join(&id.name) }
}

/// A space folder that is really there: a directory, not a link to one elsewhere.
fn real_dir(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

pub fn exists(root: &Path, id: &SpaceId) -> bool {
    if id.is_local() {
        return root.is_dir();
    }
    real_dir(&root.join(SPACES_DIR)) && real_dir(&root.join(SPACES_DIR).join(&id.kind)) && real_dir(&space_dir(root, id))
}

/// Every space under `root`: machine:local first, then the others by kind and name. Folders that
/// are not valid ids, and links, are left out.
pub fn list(root: &Path) -> Vec<SpaceId> {
    let mut out = vec![SpaceId::local()];
    let base = root.join(SPACES_DIR);
    if !real_dir(&base) {
        return out;
    }
    let mut rest = vec![];
    for kind in KINDS {
        let kdir = base.join(kind);
        if !real_dir(&kdir) {
            continue;
        }
        for e in fs::read_dir(&kdir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Ok(id) = SpaceId::parse(&format!("{kind}:{name}")) {
                if id.name == name && !id.is_local() && real_dir(&e.path()) {
                    rest.push(id);
                }
            }
        }
    }
    rest.sort();
    out.extend(rest);
    out
}

/// Creates a space's folder and wiki skeleton (as wiki::ensure_wiki does for the root), and returns
/// its folder. Creating one that exists changes nothing.
pub fn create(root: &Path, id: &SpaceId) -> Result<PathBuf> {
    // Every space lives under this machine's, which comes first.
    wiki::ensure_wiki(root)?;
    if id.is_local() {
        return Ok(root.to_path_buf());
    }
    for p in [root.join(SPACES_DIR), root.join(SPACES_DIR).join(&id.kind)] {
        if fs::symlink_metadata(&p).is_ok_and(|m| !m.is_dir()) {
            return wiki_err(format!("Refused: {} is not a folder.", wiki::disp(&p)));
        }
    }
    let dir = wiki::confined_path(root, &space_dir(root, id))?;
    if fs::symlink_metadata(&dir).is_ok_and(|m| !m.is_dir()) {
        return wiki_err(format!("Refused: {} is not a folder.", wiki::disp(&dir)));
    }
    fs::create_dir_all(&dir)?;
    wiki::ensure_skeleton(&dir)?;
    Ok(dir)
}

/// The space a path in the root's `spaces/` folder belongs to, for refusing such paths in the root.
pub fn is_spaces_path(rel: &str) -> bool {
    rel.replace('\\', "/").trim_start_matches("./").split('/').next().is_some_and(|first| first.eq_ignore_ascii_case(SPACES_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::random_hex;

    #[test]
    fn ids_parse_lowercase_and_refuse_what_is_not_a_folder_name() {
        assert_eq!(SpaceId::parse("Project:XenoNet").unwrap().to_string(), "project:xenonet");
        assert_eq!(SpaceId::parse(" actor:hermes ").unwrap(), SpaceId { kind: "actor".into(), name: "hermes".into() });
        assert!(SpaceId::parse("machine:local").unwrap().is_local());
        for bad in ["xenonet", "app:claude", "project:", "project:../x", "project:a/b", "project:-x", "project:a b", "project:.hidden", "project:a..b"] {
            assert!(SpaceId::parse(bad).is_err(), "{bad}");
        }
        assert!(SpaceId::parse(&format!("project:{}", "a".repeat(65))).is_err());
    }

    #[test]
    fn spaces_live_under_the_root_and_list_in_order() {
        let root = std::env::temp_dir().join(format!("xenolith-spaces-{}", random_hex(8)));
        fs::create_dir_all(&root).unwrap();
        assert_eq!(list(&root), vec![SpaceId::local()]);
        let atlas = SpaceId::parse("project:atlas").unwrap();
        let dir = create(&root, &atlas).unwrap();
        assert!(dir.ends_with("spaces/project/atlas"));
        assert!(dir.join("pages").is_dir() && dir.join("PROTOCOL.md").is_file());
        create(&root, &SpaceId::parse("actor:hermes").unwrap()).unwrap();
        fs::create_dir_all(root.join("spaces/project/Not-An-Id")).unwrap();
        fs::create_dir_all(root.join("spaces/other/x")).unwrap();
        let names: Vec<String> = list(&root).iter().map(ToString::to_string).collect();
        assert_eq!(names, ["machine:local", "actor:hermes", "project:atlas"]);
        assert!(exists(&root, &atlas));
        assert!(!exists(&root, &SpaceId::parse("project:nope").unwrap()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_space_folder_is_not_a_space() {
        let base = std::env::temp_dir().join(format!("xenolith-spaces-link-{}", random_hex(8)));
        let (root, outside) = (base.join("wiki"), base.join("outside"));
        fs::create_dir_all(root.join("spaces/project")).unwrap();
        fs::create_dir_all(outside.join("pages")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("spaces/project/elsewhere")).unwrap();
        let id = SpaceId::parse("project:elsewhere").unwrap();
        assert!(!exists(&root, &id));
        assert_eq!(list(&root), vec![SpaceId::local()]);
        assert!(create(&root, &id).is_err(), "creating through the link is refused");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn spaces_paths_are_recognised_however_they_are_written() {
        for p in ["spaces/project/x/pages/a.md", "./spaces", "Spaces/x", "spaces\\project\\x"] {
            assert!(is_spaces_path(p), "{p}");
        }
        for p in ["pages/spaces.md", "log/2026", "spacesuit.md"] {
            assert!(!is_spaces_path(p), "{p}");
        }
    }
}
