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

/// A command: what it does, the words it takes, and whether the help
/// lists it (what agents run stays out of the list).
pub struct Command {
    pub name: &'static str,
    pub about: &'static str,
    pub args: Option<&'static str>,
    pub hidden: bool,
}

const fn cmd(name: &'static str, about: &'static str, args: Option<&'static str>) -> Command {
    Command { name, about, args, hidden: false }
}

pub const COMMANDS: &[Command] = &[
    cmd("setup", "Install a downloaded Service or Tools package for your account", None),
    cmd("service", "Manage the background service", Some("[start|status|stop]")),
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
    cmd("hosts", "Other machines' XenoNets on this map, over SSH (a prototype, with XENONET_HOSTS=1)", Some("[self|add|remove|test]")),
    cmd("search", "Search every Claude Code and Codex transcript", Some("<words>")),
    cmd("theme", "The console's look and sound: list, switch, or make your own", Some("[id]")),
    cmd("film", "Render a film of today to an MP4, in a hidden window", None),
    cmd("script", "Save what you did in XenoNet Dev as a script, and run scripts", Some("<record|run> [file]")),
    cmd("connect", "Let Claude Code and Codex sessions see and message each other", Some("[claude] [codex]")),
    cmd("disconnect", "Take XenoNet out of Claude Code's and Codex's settings", None),
    cmd("console", "Control installed XenoNet or a development instance", Some("<steps…>")),
    cmd("install", "Put the xenonet command on your PATH", None),
    cmd("uninstall", "Take the xenonet command off your PATH", None),
    cmd("completion", "Tab completion for zsh or bash", Some("<zsh|bash>")),
    Command { name: "mcp", about: "Run the xenonet MCP server (Claude Code and Codex start it)", args: None, hidden: true },
    Command { name: "hook", about: "Pass a Claude Code or Codex hook to XenoNet (their settings run it)", args: Some("<claude|codex>"), hidden: true },
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

/// A command's help, as `xenonet <name> --help` prints it after its usage.
pub fn help(name: &str) -> &'static str {
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
