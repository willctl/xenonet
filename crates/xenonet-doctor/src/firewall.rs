//! core/doctor/index.js's check of the firewall in front of the network's
//! gateway (docs/guide/network.md): on a LAN or tailnet address, other
//! machines connect to it straight, and the firewalls of macOS and Windows
//! let a program accept them by its path. On a Mac it asks socketfilterfw
//! whether the firewall is on, blocks everything, or lets the service's
//! program in; `--getappblocked` says "permitted" of any path, even one
//! that isn't there, so the list of programs decides. It reads
//! `ctx.network()` (this machine's part in its network), the service's own
//! program (`self.exe`) and the one a restart runs (`ctx.restartExe()`),
//! which only the service's machine answers, and `ctx.realpath()`.

use crate::checks::{info, ok, skip, unreadable, warn, Ctx};
use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;
use xenonet_app::jsv::truthy;
use xenonet_app::task::Failure;
use xenonet_base::text::js_string;

/// The checks, in order: id, group and title.
pub const CHECKS: [(&str, &str, &str); 1] = [("network.firewall", "Network", "Other machines can reach this gateway")];

/// macOS's application firewall's command.
pub const SOCKETFILTERFW: &str = "/usr/libexec/ApplicationFirewall/socketfilterfw";

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("a fixed pattern"))
}

/// What `--getglobalstate` says: its State, 0 off, 1 on, 2 on and blocking
/// every incoming connection. None when it doesn't say.
pub fn global_state(text: &str) -> Option<f64> {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"State\s*=\s*([0-9]+)").captures(text).and_then(|c| c[1].parse().ok())
}

/// Whether `--getblockall` says every incoming connection is blocked.
pub fn blocks_all(text: &str) -> bool {
    text.to_ascii_lowercase().contains("enabled")
}

/// The programs `--listapps` lists, each with whether it may accept
/// incoming connections: after `Total number of apps = N`, each is
/// `N : <path>`, and `(Allow incoming connections)` or `(Block incoming
/// connections)` after it. None when it isn't that.
pub fn listed(text: &str) -> Option<Vec<(String, bool)>> {
    static TOTAL: OnceLock<Regex> = OnceLock::new();
    static ENTRY: OnceLock<Regex> = OnceLock::new();
    static SAYS: OnceLock<Regex> = OnceLock::new();
    if !re(&TOTAL, r"Total number of apps\s*=\s*[0-9]+").is_match(text) {
        return None;
    }
    let says = re(&SAYS, r"\(\s*(Allow|Block) incoming connections\s*\)");
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    for line in text.lines() {
        let rest = re(&ENTRY, r"^\s*[0-9]+\s*:\s(.*)$").captures(line).map(|c| c[1].to_string());
        let (line, entry) = match rest {
            Some(rest) => (rest, true),
            None => (line.to_string(), false),
        };
        let verdict = says.captures(&line).map(|c| (c.get(0).map_or(0, |m| m.start()), &c[1] == "Allow"));
        if entry {
            name = Some(line[..verdict.map_or(line.len(), |(at, _)| at)].trim_end().to_string());
        }
        if let (Some(n), Some((_, allow))) = (&name, verdict) {
            out.push((n.clone(), allow));
            name = None;
        }
    }
    Some(out)
}

/// A path for Terminal: under the home folder as "$HOME/…", which keeps
/// the account's name off the page; anywhere else in single quotes.
pub fn shell_path(p: &str, home: &str) -> String {
    match p.strip_prefix(home).filter(|rest| !home.is_empty() && rest.starts_with('/')) {
        Some(rest) => format!("\"$HOME{}\"", rest.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`")),
        None => format!("'{}'", p.replace('\'', "'\\''")),
    }
}

/// How to let `paths` in on a Mac: the prompt macOS shows once, unless
/// one was blocked, which it doesn't ask about again; the commands; or
/// System Settings.
pub fn allow_fix(paths: &[String], blocked: bool, home: &str) -> String {
    let commands: Vec<String> = paths
        .iter()
        .map(|p| {
            let q = shell_path(p, home);
            format!("sudo {SOCKETFILTERFW} --add {q} then sudo {SOCKETFILTERFW} --unblockapp {q}")
        })
        .collect();
    let first = if blocked { "In Terminal: " } else { "macOS asks once whether xenonet may accept incoming connections: choose Allow. Or in Terminal: " };
    format!("{first}{}. Or System Settings › Network › Firewall › Options.", commands.join(", then "))
}

// A program's standing in the list: Some(true) let in, Some(false)
// blocked, None not listed. The firewall keeps the path it was given, so
// one listed under another spelling of the same file counts too.
async fn listing(c: &Ctx<'_>, apps: &[(String, bool)], path: &str) -> Result<Option<bool>, Failure> {
    if let Some((_, allow)) = apps.iter().find(|(p, _)| p == path) {
        return Ok(Some(*allow));
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    for (p, allow) in apps.iter().filter(|(p, _)| p.rsplit('/').next() == Some(name)) {
        let real = c.call("realpath", json!([p])).await.map_err(|_| Failure::Rethrow)?;
        if real.filter(|v| !v.is_null()).map(|v| js_string(&v)).as_deref() == Some(path) {
            return Ok(Some(*allow));
        }
    }
    Ok(None)
}

// What a program said, when it said it: its output, or None.
async fn said(c: &Ctx<'_>, args: &[&str]) -> Result<Option<String>, Failure> {
    let r = c.run(SOCKETFILTERFW, args, 5000.0).await?;
    if r.get("code").and_then(Value::as_f64) != Some(0.0) || truthy(r.get("timedOut")) {
        return Ok(None);
    }
    Ok(Some(r.get("stdout").filter(|v| !v.is_null()).map(js_string).unwrap_or_default()))
}

/// The check's verdict.
pub(crate) async fn run(c: &Ctx<'_>, id: &str) -> Result<Value, Failure> {
    if id != "network.firewall" {
        return Err(Failure::msg(format!("no such check: {id}")));
    }
    let net = match c.call("network", json!([])).await.map_err(|_| Failure::Rethrow)? {
        None => return Ok(skip("needs XenoNet running")),
        Some(v) if !truthy(Some(&v)) => return Ok(skip("this machine isn't in a network")),
        Some(v) => v,
    };
    if !truthy(net.get("gateway")) {
        return Ok(skip("this machine isn't the network's gateway"));
    }
    match net.get("kind").and_then(Value::as_str) {
        Some("lan") => {}
        Some("loopback") => return Ok(skip("the gateway's address is on this computer alone, where no firewall stands in the way")),
        Some(_) => return Ok(skip("other machines reach the gateway through its tunnel, which connects on this computer, where no firewall stands in the way")),
        None => return Ok(skip("the gateway has no address yet")),
    }
    let platform = c.f.platform();
    if platform != "darwin" && platform != "win32" {
        return Ok(skip("for macOS and Windows, whose firewalls let programs in by their path"));
    }
    let exe = c.f.data.get("self").and_then(|s| s.get("exe")).filter(|v| truthy(Some(v))).map(js_string);
    let Some(exe) = exe else { return Ok(info("XenoNet couldn't tell which program its service runs as", None)) };
    let next = c.call("restartExe", json!([])).await.map_err(|_| Failure::Rethrow)?.filter(|v| truthy(Some(v))).map(|v| js_string(&v)).filter(|n| *n != exe);
    if platform == "win32" {
        return Ok(info(
            format!("XenoNet doesn't read Windows Defender Firewall's rules, which let a program in by its path; the service runs as {}", c.tilde(&exe)),
            Some("If other machines can't reach this gateway, let that program in: Windows Security › Firewall & network protection › Allow an app through firewall › Allow another app. Updates keep its path."),
        ));
    }
    let home = c.f.data.get("home").filter(|v| !v.is_null()).map(js_string).unwrap_or_default();
    let Some(state) = said(c, &["--getglobalstate"]).await?.as_deref().and_then(global_state) else {
        return Ok(info("XenoNet couldn't read whether the firewall is on", Some(&unreadable("socketfilterfw"))));
    };
    if state == 0.0 {
        return Ok(ok("the firewall is off"));
    }
    if state == 2.0 || said(c, &["--getblockall"]).await?.as_deref().is_some_and(blocks_all) {
        return Ok(warn(
            "the firewall blocks every incoming connection, so no other machine can reach this gateway",
            Some("Turn off Block all incoming connections: System Settings › Network › Firewall › Options."),
        ));
    }
    let Some(apps) = said(c, &["--listapps"]).await?.as_deref().and_then(listed) else {
        return Ok(info("the firewall is on, and XenoNet couldn't read which programs it lets in", Some(&allow_fix(&[exe], false, &home))));
    };
    let now = listing(c, &apps, &exe).await?;
    let later = match &next {
        Some(n) => listing(c, &apps, n).await?,
        None => now,
    };
    let (shown, after) = (c.tilde(&exe), next.as_deref().map(|n| c.tilde(n)));
    if now == Some(true) && later == Some(true) {
        let also = after.map_or(String::new(), |a| format!(", and {a}, which the service runs from after a restart"));
        return Ok(ok(format!("the firewall is on and lets {shown} in{also}")));
    }
    let detail = match (now, &after) {
        (Some(true), Some(a)) => format!("the firewall lets {shown} in, but after a restart the service runs from {a}, which it hasn't let in"),
        (Some(false), _) => format!("the firewall blocks {shown}, so other machines can't reach this gateway"),
        _ => format!("the firewall is on and hasn't let {shown} in, so other machines can't reach this gateway"),
    };
    let detail = match (now, &after) {
        (Some(true), _) | (_, None) => detail,
        (_, Some(a)) => format!("{detail}; after a restart it runs from {a}"),
    };
    let mut paths = Vec::new();
    if now != Some(true) {
        paths.push(exe.clone());
    }
    if let Some(n) = next.filter(|_| later != Some(true)) {
        paths.push(n);
    }
    Ok(warn(detail, Some(&allow_fix(&paths, now == Some(false) || later == Some(false), &home))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{self, Facts};
    use xenonet_app::task::{Outcome, Task};

    const EXE: &str = "/Users/ada/Library/Application Support/XenoNet/runtime/components/service/active/bin/xenonet";

    // The check's verdict on a Mac, with `network` as ctx.network()
    // answers and `apps` as --listapps lists them.
    fn verdict(network: Value, apps: &str) -> Value {
        let facts = Facts { data: json!({ "platform": "darwin", "home": "/Users/ada", "self": { "port": 47840, "exe": EXE } }) };
        let mut task = Task::new(|io| async move { checks::run(io, "network.firewall".into(), facts).await });
        let mut reply = None;
        loop {
            match task.step(reply.take()) {
                Outcome::Ask(q) => {
                    let args = &q["args"];
                    let value = match q["fn"].as_str().unwrap() {
                        "network" => network.clone(),
                        "restartExe" | "realpath" => Value::Null,
                        "run" => match args[1][0].as_str().unwrap() {
                            "--getglobalstate" => json!({ "code": 0, "stdout": "Firewall is enabled. (State = 1)\n" }),
                            "--getblockall" => json!({ "code": 0, "stdout": "Firewall has block all state set to disabled.\n" }),
                            _ => json!({ "code": 0, "stdout": apps }),
                        },
                        other => panic!("asked for {other}"),
                    };
                    reply = Some(Ok(json!({ "value": value })));
                }
                Outcome::Done(v) => return v,
                Outcome::Failed(f) => panic!("{}", f.message()),
            }
        }
    }

    #[test]
    fn the_gateways_program_let_in_or_how_to() {
        let gateway = json!({ "gateway": true, "address": "100.64.0.7:48850", "kind": "lan" });
        let allowed = verdict(gateway.clone(), &format!("Total number of apps = 1 \n1 : {EXE} \n             (Allow incoming connections)\n"));
        assert_eq!(allowed["status"], json!("ok"), "{allowed}");
        let unlisted = verdict(gateway, "Total number of apps = 0 \n");
        assert_eq!(unlisted["status"], json!("warn"));
        let q = "\"$HOME/Library/Application Support/XenoNet/runtime/components/service/active/bin/xenonet\"";
        assert!(unlisted["fix"].as_str().unwrap().contains(&format!("sudo {SOCKETFILTERFW} --add {q} then sudo {SOCKETFILTERFW} --unblockapp {q}.")), "{unlisted}");
        assert_eq!(verdict(json!({ "gateway": false, "kind": "lan" }), "")["status"], json!("skip"));
    }

    #[test]
    fn what_socketfilterfw_says() {
        assert_eq!(global_state("Firewall is enabled. (State = 1)\n"), Some(1.0));
        assert_eq!(global_state("Firewall is disabled. (State = 0)\n"), Some(0.0));
        assert_eq!(global_state("something else"), None);
        assert!(blocks_all("Firewall has block all state set to enabled.\n") && blocks_all("Block all ENABLED!"));
        assert!(!blocks_all("Firewall has block all state set to disabled.\n"));
        let list = "Total number of apps = 3 \n1 : /Applications/Spotify.app \n             (Allow incoming connections)\n2 : /Users/ada/My Apps/xenonet \n             (Block incoming connections)\n3 : com.docker.docker \n             (Allow incoming connections)\n";
        assert_eq!(listed(list), Some(vec![("/Applications/Spotify.app".into(), true), ("/Users/ada/My Apps/xenonet".into(), false), ("com.docker.docker".into(), true)]));
        assert_eq!(listed("1 : /x (Allow incoming connections)\nTotal number of apps = 1\n"), Some(vec![("/x".into(), true)]), "on one line");
        assert_eq!(listed("Usage: socketfilterfw …"), None);
        assert_eq!(shell_path("/Users/ada/Library/Application Support/x", "/Users/ada"), "\"$HOME/Library/Application Support/x\"");
        assert_eq!(shell_path("/Users/adam/x", "/Users/ada"), "'/Users/adam/x'");
        assert_eq!(shell_path("/opt/it's", "/Users/ada"), "'/opt/it'\\''s'");
    }
}
