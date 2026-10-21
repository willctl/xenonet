//! The xenonet command's commands (cli/lib/commands.js), for its help and
//! its shell completion, and each one's code: `commands/<name>.rs` ports
//! `cli/commands/<name>.js`. Each has its help, and `run(args, flags)`.

use crate::args::Flags;
use crate::out::Done;

pub mod actor;
pub mod agents;
pub mod assistant;
pub mod completion;
pub mod connect;
pub mod console;
pub mod decide;
pub mod diagnose;
pub mod disconnect;
pub mod doctor;
pub mod drift;
pub mod film;
pub mod gateway;
pub mod hook;
pub mod hosts;
pub mod install;
pub mod live;
pub mod logs;
pub mod mcp;
pub mod messages;
pub mod net;
pub mod refusals;
pub mod relay;
pub mod root;
pub mod script;
pub mod search;
pub mod service;
pub mod session;
pub mod setup;
pub mod spend;
pub mod status;
pub mod theme;
pub mod uninstall;
pub mod update;
pub mod who;
pub mod words;

/// A command: what it does, the words it takes, and whether the help
/// lists it (what agents run stays out of the list). What it does and the
/// words it takes can name apps with words.rs's tokens, which about() and
/// args() fill in from the app registry.
pub struct Command {
    pub name: &'static str,
    about: &'static str,
    args: Option<&'static str>,
    pub hidden: bool,
}

impl Command {
    /// What it does, in a line.
    pub fn about(&self) -> String {
        words::line(self.about)
    }

    /// The words it takes, for its usage.
    pub fn args(&self) -> Option<String> {
        self.args.map(words::line)
    }
}

const fn cmd(name: &'static str, about: &'static str, args: Option<&'static str>) -> Command {
    Command { name, about, args, hidden: false }
}

pub const COMMANDS: &[Command] = &[
    cmd("setup", "Install a downloaded Service or Tools package for your account", None),
    cmd("service", "Manage the background service, and start it at login", Some("[start|status|stop|install|uninstall]")),
    cmd("update", "Check or install service and tools updates", Some("[service|tools|all]")),
    cmd("status", "Whether XenoNet is running, what it reads and what needs you", None),
    cmd("doctor", "Check sign-ins, folders and connections, with a fix for each problem", None),
    cmd("drift", "What's changed in the apps' data XenoNet reads since you last looked", None),
    cmd("logs", "What went wrong in XenoNet lately, with a fix for each when there's one", None),
    cmd("decide", "The decision model XenoNet can ask instead of Codex: its key, kept in your credential store", Some("[status|key|forget]")),
    cmd("refusals", "Provider refusals XenoNet kept: list, show, export for a dataset, scan", Some("[list|show|export|capture|context|scan]")),
    cmd("diagnose", "A summary of XenoNet's state that's safe to share, or a session that looks at it with you", Some("[--ask]")),
    cmd("agents", "The sessions at work, and what each is doing", None),
    cmd("actor", "Actors: list them, ask to register one, or report as one", Some("[list|register|…]")),
    cmd("assistant", "Remote actors' connections: list them, revoke one, or turn them all off", Some("[list|audit|revoke|disable]")),
    cmd("gateway", "The actor gateway, for actors outside this machine: its tunnel, address and apps", Some("[tunnels|on|off|check|repair|address|token|revoke]")),
    cmd("who", "Which sessions and actors have a hand in a file", Some("<path>")),
    cmd("session", "A session's models, tokens, estimated cost, turns and tools", Some("[words|id]")),
    cmd("live", "Messages that reach sessions as they're sent: pause, consent, actors' scopes, audit", Some("[pause|resume|allow|grant|revoke|audit]")),
    cmd("spend", "About what today's work would cost at API prices, by app and by actor", None),
    cmd("messages", "What sessions and actors said to each other today", None),
    cmd("root", "The projects folder XenoNet reads, or read another", Some("[folder]")),
    cmd(
        "net",
        "One network across your machines: create or join one, add a machine, see the hosts",
        Some("[create|join|status|hosts|add-machine|remove|leave|set|approve|gateway]"),
    ),
    cmd("hosts", "Other machines' XenoNets on this map, over SSH (a prototype, with XENONET_HOSTS=1)", Some("[self|add|remove|test]")),
    cmd("search", "Search every {apps lines} transcript", Some("<words>")),
    cmd("theme", "The console's look and sound: list, switch, or make your own", Some("[id]")),
    cmd("film", "Render a film of today to an MP4, in a hidden window", None),
    cmd("script", "Save what you did in XenoNet Dev as a script, and run scripts", Some("<record|run> [file]")),
    cmd("connect", "Let {apps connect} sessions see and message each other", Some("{ids connect}")),
    cmd("disconnect", "Take XenoNet out of {apps connect's} settings", None),
    cmd("console", "Control installed XenoNet or a development instance", Some("<steps…>")),
    cmd("install", "Put the xenonet command on your PATH", None),
    cmd("uninstall", "Take the xenonet command off your PATH", None),
    cmd("completion", "Tab completion for zsh or bash", Some("<zsh|bash>")),
    Command { name: "mcp", about: "Run the xenonet MCP server ({apps connect} start it)", args: None, hidden: true },
    Command { name: "hook", about: "Pass a {apps connect or} hook to XenoNet (their settings run it)", args: Some("<{ids connect|}>"), hidden: true },
    Command { name: "relay", about: "Stream this XenoNet's map to a console on another machine (consoles run it over SSH)", args: None, hidden: true },
];

pub const ALIASES: &[(&str, &str)] = &[("st", "status"), ("peers", "agents"), ("dev", "console"), ("actors", "actor")];

/// The command an alias stands for, or the name as it is.
pub fn alias(name: &str) -> &str {
    ALIASES.iter().find(|(a, _)| *a == name).map_or(name, |(_, to)| to)
}

pub fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// A command's help, as `xenonet <name> --help` prints it after its usage,
/// with the apps and the doctor's groups it names filled in (words.rs).
pub fn help(name: &str) -> String {
    words::help(text(name))
}

// A command's help as it's written.
fn text(name: &str) -> &'static str {
    match name {
        "actor" => actor::HELP,
        "agents" => agents::HELP,
        "assistant" => assistant::HELP,
        "completion" => completion::HELP,
        "connect" => connect::HELP,
        "console" => console::HELP,
        "decide" => decide::HELP,
        "diagnose" => diagnose::HELP,
        "disconnect" => disconnect::HELP,
        "doctor" => doctor::HELP,
        "drift" => drift::HELP,
        "film" => film::HELP,
        "gateway" => gateway::HELP,
        "hook" => hook::HELP,
        "hosts" => hosts::HELP,
        "install" => install::HELP,
        "live" => live::HELP,
        "logs" => logs::HELP,
        "mcp" => mcp::HELP,
        "messages" => messages::HELP,
        "net" => net::HELP,
        "refusals" => refusals::HELP,
        "relay" => relay::HELP,
        "root" => root::HELP,
        "script" => script::HELP,
        "search" => search::HELP,
        "service" => service::HELP,
        "session" => session::HELP,
        "setup" => setup::HELP,
        "spend" => spend::HELP,
        "status" => status::HELP,
        "theme" => theme::HELP,
        "uninstall" => uninstall::HELP,
        "update" => update::HELP,
        "who" => who::HELP,
        _ => find(name).map_or("", |c| c.about),
    }
}

/// Runs a command, `name` being one of COMMANDS.
pub fn run(name: &str, args: &[String], flags: &Flags) -> Done {
    match name {
        "actor" => actor::run(args, flags),
        "agents" => agents::run(args, flags),
        "assistant" => assistant::run(args, flags),
        "completion" => completion::run(args, flags),
        "connect" => connect::run(args, flags),
        "console" => console::run(args, flags),
        "decide" => decide::run(args, flags),
        "diagnose" => diagnose::run(args, flags),
        "disconnect" => disconnect::run(args, flags),
        "doctor" => doctor::run(args, flags),
        "drift" => drift::run(args, flags),
        "film" => film::run(args, flags),
        "gateway" => gateway::run(args, flags),
        "hook" => hook::run(args, flags),
        "hosts" => hosts::run(args, flags),
        "install" => install::run(args, flags),
        "live" => live::run(args, flags),
        "logs" => logs::run(args, flags),
        "mcp" => mcp::run(args, flags),
        "messages" => messages::run(args, flags),
        "net" => net::run(args, flags),
        "refusals" => refusals::run(args, flags),
        "relay" => relay::run(args, flags),
        "root" => root::run(args, flags),
        "script" => script::run(args, flags),
        "search" => search::run(args, flags),
        "service" => service::run(args, flags),
        "session" => session::run(args, flags),
        "setup" => setup::run(args, flags),
        "spend" => spend::run(args, flags),
        "status" => status::run(args, flags),
        "theme" => theme::run(args, flags),
        "uninstall" => uninstall::run(args, flags),
        "update" => update::run(args, flags),
        "who" => who::run(args, flags),
        _ => Err(crate::out::Fail::error(format!("no command \"{name}\". xenonet --help lists them."))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every help as `xenonet <name> --help` prints it, usage first.
    fn every_help() -> Vec<(String, String)> {
        COMMANDS.iter().map(|c| (c.name.to_string(), format!("xenonet {} {}\n\n{}\n{}", c.name, c.args().unwrap_or_default(), c.about(), help(c.name)))).collect()
    }

    #[test]
    fn every_help_names_the_registrys_apps_and_leaves_no_token() {
        for (name, text) in every_help() {
            for token in ["{apps ", "{ids ", "{doctor "] {
                assert!(!text.contains(token), "{name}: {token}");
            }
            assert!(!text.contains('\u{a0}'), "{name}");
            assert!(!text.contains("Node"), "{name}: no Node in the binary's help");
        }
        let of = |name: &str| every_help().into_iter().find(|(n, _)| n == name).map(|(_, t)| t).unwrap_or_default();
        for app in xenonet_base::apps::with("connect") {
            for name in ["connect", "disconnect", "mcp", "hook", "doctor"] {
                assert!(of(name).contains(&app.name), "{name} --help names {}, whole on a line", app.name);
            }
            assert!(of("connect").contains(&format!("[{}]", app.id)), "connect takes {}", app.id);
        }
        for app in xenonet_base::apps::with("lines") {
            assert!(of("search").contains(&app.name), "search reads {}'s transcripts", app.name);
        }
        assert!(of("connect").starts_with("xenonet connect [claude] [codex] [grok]\n"));
        assert!(of("hook").starts_with("xenonet hook <claude|codex|grok>\n"));
        assert!(of("disconnect").contains("Take XenoNet out of Claude Code's, Codex's and Grok Build's settings"));
    }

    #[test]
    fn doctor_help_lists_every_group_of_checks() {
        let text = help("doctor");
        let only: String = text.lines().skip_while(|l| !l.trim_start().starts_with("--only")).take_while(|l| !l.trim_start().starts_with("--json")).collect::<Vec<_>>().join(" ");
        for group in words::groups() {
            assert!(only.contains(&format!(" {group},")) || only.contains(&format!(" {group} ")), "--only lists {group}: {only}");
        }
        assert!(only.contains("service") && only.contains("grok"));
    }
}
