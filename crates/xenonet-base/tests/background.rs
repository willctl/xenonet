//! Every program XenoNet starts in the background shows no window on
//! Windows (`xenonet_base::process::Background`): one started without it
//! flashes a console window each time, and the service starts git every
//! few seconds. This reads the crates' sources and names any place that
//! starts a program without it.

use std::path::{Path, PathBuf};

/// How many lines after a command is made its window may be hidden.
const NEAR: usize = 40;

/// What hides a command's window.
const HIDES: [&str; 4] = ["no_window", "NO_WINDOW", "0x0800_0000", "hide_window"];

/// Commands that never run on Windows, by file and what they start.
const UNIX_ONLY: [(&str, &str); 4] = [
    ("xenonet-mcp/src/process.rs", "\"ps\""),
    ("xenonet-wire-map/src/git.rs", "xcode-select"),
    ("xenonet-wire-map/src/live.rs", "\"security\""),
    ("xenonet-cli/src/commands/runs.rs", "\"stty\""),
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn every_program_xenonet_starts_shows_no_window_on_windows() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    for c in std::fs::read_dir(crates).unwrap().flatten() {
        let src = c.path().join("src");
        if src.is_dir() {
            sources(&src, &mut files);
        }
    }
    let mut bare = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(crates).unwrap().to_string_lossy().replace('\\', "/");
        let text = std::fs::read_to_string(f).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            // A file's tests come last; what they start isn't the service's.
            if line.trim_start().starts_with("mod tests") {
                break;
            }
            // Command::new or Proc::new on its own, not a type whose name ends so.
            let made = ["Command::new(", "Proc::new("].iter().any(|m| line.match_indices(m).any(|(at, _)| !line[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_')))
                && !line.trim_start().starts_with("//");
            if !made || UNIX_ONLY.iter().any(|(file, what)| rel == *file && line.contains(what)) {
                continue;
            }
            let near = &lines[i..lines.len().min(i + NEAR)];
            if !near.iter().any(|l| HIDES.iter().any(|h| l.contains(h))) {
                bare.push(format!("{rel}:{}", i + 1));
            }
        }
    }
    assert!(bare.is_empty(), "these start a program that would flash a window on Windows; call xenonet_base::process::Background::no_window on it:\n{}", bare.join("\n"));
}
