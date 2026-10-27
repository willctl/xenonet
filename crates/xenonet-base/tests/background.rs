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
const UNIX_ONLY: [(&str, &str); 3] = [("xenonet-mcp/src/process.rs", "\"ps\""), ("xenonet-wire-map/src/git.rs", "xcode-select"), ("xenonet-wire-map/src/live.rs", "\"security\"")];

/// Programs started for you to use in your own terminal, by file and what
/// they start. They inherit the console, and CREATE_NO_WINDOW would give
/// each a console of its own with no window, so you couldn't see or type
/// into it: `xenonet diagnose --ask` runs Claude Code or Codex there, as
/// itself or through cmd.exe for an npm install's .cmd. Every other
/// program XenoNet starts still has to hide its window.
const INTERACTIVE: [(&str, &str); 2] = [("xenonet-cli/src/commands/diagnose.rs", "Command::new(\"cmd.exe\")"), ("xenonet-cli/src/commands/diagnose.rs", "Command::new(&bin)")];

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
            if !made || UNIX_ONLY.iter().chain(&INTERACTIVE).any(|(file, what)| rel == *file && line.contains(what)) {
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

#[test]
fn the_programs_started_for_your_terminal_keep_its_console() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for (file, what) in INTERACTIVE {
        let text = std::fs::read_to_string(crates.join(file)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let at = lines.iter().position(|l| l.contains(what)).unwrap_or_else(|| panic!("{file} no longer starts {what}; take it out of INTERACTIVE"));
        // The command's own setup: up to the end of its block.
        let own: Vec<&str> = lines[at..].iter().take_while(|l| !l.trim_start().starts_with('}')).copied().collect();
        assert!(!own.iter().any(|l| HIDES.iter().any(|h| l.contains(h))), "{file}: {what} inherits the console, so it mustn't hide its window:\n{}", own.join("\n"));
    }
}
