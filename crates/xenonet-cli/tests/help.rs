// The binary's help, as a person reads it: the apps it names are the
// registry's (Grok Build among them, beside Claude Code and Codex), doctor
// --help lists every group of checks --only takes, and nothing speaks of
// a Node the binary doesn't have. The wording itself is the help files';
// these are the facts it has to keep.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use std::process::Command;

fn help(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_xenonet")).args(args).arg("--no-color").env("NO_COLOR", "1").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{args:?}");
    String::from_utf8(out.stdout).unwrap()
}

// The command's line in xenonet --help: its usage, then what it does.
fn listed(top: &str, name: &str) -> String {
    top.lines().find(|l| l.starts_with(&format!("  {name} ")) || l.trim() == name).unwrap().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_list_of_commands_names_every_app_connect_goes_through() {
    let top = help(&["--help"]);
    assert_eq!(listed(&top, "connect"), "connect [claude] [codex] [grok] Let Claude Code, Codex and Grok Build sessions see and message each other");
    assert_eq!(listed(&top, "disconnect"), "disconnect Take XenoNet out of Claude Code's, Codex's and Grok Build's settings");
    assert_eq!(listed(&top, "search"), "search <words> Search every Claude Code, Codex and Grok Build transcript");
    assert!(!top.contains("\n  mcp ") && !top.contains("\n  hook "), "what agents run stays out of the list");
}

#[test]
fn each_commands_help_says_what_it_does_now() {
    let connect = help(&["connect", "--help"]);
    assert!(connect.starts_with("xenonet connect [claude] [codex] [grok]\n\nAdds XenoNet to Claude Code's, Codex's and Grok Build's own settings"), "{connect}");
    let disconnect = help(&["disconnect", "--help"]);
    assert!(disconnect.contains("out of Claude Code's, Codex's and\nGrok Build's settings"), "{disconnect}");
    let doctor = help(&["doctor", "--help"]);
    assert!(doctor.contains("Claude Code, Codex and Grok Build installed,\nsigned in and connected"), "{doctor}");
    let only: String = doctor.lines().skip_while(|l| !l.contains("--only")).take(3).collect::<Vec<_>>().join(" ");
    let only = only.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(only.contains("xenonet, contracts, claude, codex, grok, system, agents, actors, service, or one check's id"), "{only}");
    assert!(help(&["hook", "--help"]).starts_with("xenonet hook <claude|codex|grok>\n\nPasses a Claude Code, Codex or Grok Build hook event"));
    let install = help(&["install", "--help"]);
    assert!(install.contains("Installs a small script that runs this xenonet binary."), "{install}");
    for name in ["setup", "service", "update", "status", "doctor", "drift", "install", "uninstall", "connect", "disconnect", "mcp", "hook"] {
        assert!(!help(&[name, "--help"]).contains("Node"), "{name} --help speaks of no Node");
    }
}
