//! core/doctor/index.js's check of the firewall in front of the network's
//! gateway (docs/guide/network.md): on a LAN or tailnet address, other
//! machines connect to it straight, and the firewalls of macOS and Windows
//! let a program accept them by its path. On a Mac it asks socketfilterfw
//! whether the firewall is on, blocks everything, or lets the service's
//! program in; `--getappblocked` says "permitted" of any path, even one
//! that isn't there, so the list of programs decides. On Windows,
//! PowerShell says which of Windows Defender Firewall's profiles are on,
//! which this computer's networks use, and the inbound rules for the
//! service's program (`windows_verdict`); XenoNet only reads them. It reads
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

/// Windows PowerShell, which reads Windows Defender Firewall through its
/// NetSecurity cmdlets, whose answers don't change with the system's
/// language.
pub const POWERSHELL: &str = "powershell.exe";

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The script that reads, and only reads, each profile's state, the
/// profiles of the networks this computer is on, and the inbound rules for
/// each of `paths`, as JSON.
pub fn windows_script(paths: &[String]) -> String {
    let quoted: Vec<String> = paths.iter().map(|p| ps_quote(p)).collect();
    [
        "$ErrorActionPreference = 'Stop'".to_string(),
        format!("$paths = @({})", quoted.join(", ")),
        "$profiles = @(Get-NetFirewallProfile | ForEach-Object { [pscustomobject]@{ name = [string]$_.Name; enabled = [string]$_.Enabled; inbound = [string]$_.DefaultInboundAction; rules = [string]$_.AllowInboundRules } })".into(),
        "$networks = @(Get-NetConnectionProfile -ErrorAction SilentlyContinue | ForEach-Object { [string]$_.NetworkCategory })".into(),
        "$rules = @(Get-NetFirewallApplicationFilter | ForEach-Object { $p = if ($_.Program) { [Environment]::ExpandEnvironmentVariables($_.Program) } else { '' }; foreach ($want in $paths) { if ($p -ieq $want) { $f = $_; Get-NetFirewallRule -AssociatedNetFirewallApplicationFilter $f | Where-Object { [string]$_.Direction -eq 'Inbound' } | ForEach-Object { [pscustomobject]@{ program = $want; name = [string]$_.DisplayName; enabled = [string]$_.Enabled; action = [string]$_.Action; profile = [string]$_.Profile } } } } })".into(),
        "[pscustomobject]@{ profiles = $profiles; networks = $networks; rules = $rules } | ConvertTo-Json -Depth 4 -Compress".into(),
    ]
    .join("\n")
}

/// The netsh line that lets a program in, run as administrator.
pub fn netsh_allow(exe: &str, profiles: &[&str]) -> String {
    let profiles: Vec<String> = profiles.iter().map(|p| p.to_lowercase()).collect();
    format!("netsh advfirewall firewall add rule name=\"XenoNet Service\" dir=in action=allow program=\"{exe}\" enable=yes profile={}", profiles.join(","))
}

/// "Private network", "Domain and Private networks".
fn networks_of(names: &[&str]) -> String {
    format!("{} network{}", names.join(" and "), if names.len() > 1 { "s" } else { "" })
}

fn unique<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for n in names {
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// What Windows Defender Firewall does to the service's programs (`paths`:
/// the one it runs as, and the one a restart runs, if another) on the
/// networks this computer is on, from what the script said (`seen`, None
/// when it couldn't be read): off, letting them in, blocking every incoming
/// connection, blocking them, or not having let them in yet, with the netsh
/// line that lets them in. A public network is no place to accept
/// connections, so that fix makes it private first. `shown` and `after`
/// are the two paths as the doctor shows them.
pub fn windows_verdict(seen: Option<&Value>, paths: &[String], shown: &str, after: Option<&str>) -> Value {
    let fix = |profiles: &[&str]| {
        let want = unique(profiles.iter().map(|n| if *n == "Public" { "Private" } else { *n }));
        let lines: Vec<String> = paths.iter().map(|p| netsh_allow(p, &want)).collect();
        let lines = lines.join(", then ");
        if profiles.contains(&"Public") {
            format!("This network is Public: make it Private in Settings › Network & internet, in its properties. Then in a terminal run as administrator: {lines}")
        } else {
            format!("In a terminal run as administrator: {lines}")
        }
    };
    let list = |v: Option<&Value>| -> Vec<Value> {
        match v {
            Some(Value::Array(a)) => a.clone(),
            None | Some(Value::Null) => Vec::new(),
            Some(other) => vec![other.clone()],
        }
    };
    let seen = seen.filter(|s| s.is_object());
    let profiles = list(seen.and_then(|s| s.get("profiles")));
    let Some(seen) = seen.filter(|_| !profiles.is_empty()) else {
        return info(
            "XenoNet couldn't read Windows Defender Firewall's settings",
            Some(&format!("If other machines can't reach this gateway, let {shown} in. {}", fix(&["Private"]))),
        );
    };
    let field = |v: &Value, k: &str| v.get(k).map_or_else(|| "undefined".to_string(), js_string);
    let profile = |name: &str, k: &str| profiles.iter().find(|p| p.get("name").and_then(Value::as_str) == Some(name)).and_then(|p| p.get(k)).map(js_string);
    // The profiles of the networks this computer is on (all three when it
    // can't tell), and those the firewall is on for.
    let networks = list(seen.get("networks"));
    let known = unique(networks.iter().filter_map(|n| match n.as_str() {
        Some("Public") => Some("Public"),
        Some("Private") => Some("Private"),
        Some("DomainAuthenticated") => Some("Domain"),
        _ => None,
    }));
    let used = if known.is_empty() { vec!["Domain", "Private", "Public"] } else { known };
    let on: Vec<&str> = used.iter().copied().filter(|n| profile(n, "enabled").as_deref() == Some("True")).collect();
    if on.is_empty() {
        return ok(format!("Windows Defender Firewall is off for {}", networks_of(&used)));
    }
    let all: Vec<&str> = on.iter().copied().filter(|n| profile(n, "rules").as_deref() == Some("False")).collect();
    if !all.is_empty() {
        return warn(
            format!("Windows Defender Firewall blocks every incoming connection on {}, so no other machine can reach this gateway", networks_of(&all)),
            Some("Turn off Blocks all incoming connections: Windows Security › Firewall & network protection, then that network's profile."),
        );
    }
    let covers = |rule: &Value, name: &str| {
        let profile = field(rule, "profile");
        rule.get("enabled").and_then(Value::as_str) == Some("True") && (profile == "Any" || profile.split(',').any(|p| p.trim() == name))
    };
    let rules = list(seen.get("rules"));
    let rules_of = |path: &str| -> Vec<&Value> { rules.iter().filter(|r| field(r, "program").to_lowercase() == path.to_lowercase()).collect() };
    let action = |r: &Value| r.get("action").and_then(Value::as_str).map(str::to_string);
    let blocking: Vec<&Value> = paths.iter().flat_map(|p| rules_of(p)).filter(|r| action(r).as_deref() == Some("Block") && on.iter().any(|n| covers(r, n))).collect();
    let blocked: Vec<&str> = on.iter().copied().filter(|n| blocking.iter().any(|r| covers(r, n))).collect();
    if !blocked.is_empty() {
        let names: Vec<String> = blocking.iter().map(|r| format!("\"{}\"", field(r, "name"))).collect();
        let names = unique(names.iter().map(String::as_str)).join(", ");
        return warn(
            format!("Windows Defender Firewall blocks {shown} on {}, so other machines can't reach this gateway", networks_of(&blocked)),
            Some(&format!("Turn off the rule that blocks it ({names}) in Windows Defender Firewall with Advanced Security › Inbound Rules. {}", fix(&blocked))),
        );
    }
    let open =
        |path: &str, name: &str| profile(name, "inbound").as_deref() == Some("Allow") || rules_of(path).iter().any(|r| action(r).as_deref() == Some("Allow") && covers(r, name));
    let missing: Vec<&str> = on.iter().copied().filter(|n| paths.iter().any(|p| !open(p, n))).collect();
    if missing.is_empty() {
        let also = after.map_or(String::new(), |a| format!(", and {a}, which the service runs from after a restart"));
        return ok(format!("Windows Defender Firewall lets {shown} in on {}{also}", networks_of(&on)));
    }
    warn(format!("Windows Defender Firewall hasn't let {shown} in on {}, so other machines can't reach this gateway", networks_of(&missing)), Some(&fix(&missing)))
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
        let mut paths = vec![exe.clone()];
        paths.extend(next.clone());
        let r = c.run(POWERSHELL, &["-NoProfile", "-NonInteractive", "-Command", &windows_script(&paths)], 30000.0).await?;
        let read = r.get("code").and_then(Value::as_f64) == Some(0.0) && !truthy(r.get("timedOut"));
        let seen = if read { xenonet_base::json::parse(&r.get("stdout").filter(|v| !v.is_null()).map(js_string).unwrap_or_default()).ok() } else { None };
        let after = next.as_deref().map(|n| c.tilde(n));
        return Ok(windows_verdict(seen.as_ref(), &paths, &c.tilde(&exe), after.as_deref()));
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
    fn windows_defender_firewall_lets_the_service_in_or_the_netsh_line_does() {
        let exe = "C:\\Users\\ada\\AppData\\Local\\XenoNet\\runtime\\components\\service\\active\\bin\\xenonet.exe".to_string();
        let paths = std::slice::from_ref(&exe);
        let profiles = json!([
            { "name": "Domain", "enabled": "True", "inbound": "NotConfigured", "rules": "NotConfigured" },
            { "name": "Private", "enabled": "True", "inbound": "NotConfigured", "rules": "NotConfigured" },
            { "name": "Public", "enabled": "True", "inbound": "NotConfigured", "rules": "NotConfigured" },
        ]);
        let rule = |action: &str, profile: &str| json!({ "program": exe, "name": "xenonet", "enabled": "True", "action": action, "profile": profile });
        let verdict = |rules: Value, networks: Value| windows_verdict(Some(&json!({ "profiles": profiles, "networks": networks, "rules": rules })), paths, "~\\xenonet.exe", None);
        let line = format!("netsh advfirewall firewall add rule name=\"XenoNet Service\" dir=in action=allow program=\"{exe}\" enable=yes profile=private");
        assert_eq!(verdict(json!([rule("Allow", "Private, Public")]), json!(["Private"]))["status"], "ok");
        // A single rule comes as an object, not a list.
        assert_eq!(verdict(rule("Allow", "Any"), json!("Private"))["status"], "ok");
        let missing = verdict(json!([rule("Allow", "Domain")]), json!(["Private"]));
        assert_eq!(missing["status"], "warn");
        assert_eq!(missing["fix"], json!(format!("In a terminal run as administrator: {line}")));
        let blocked = verdict(json!([rule("Allow", "Private"), rule("Block", "Private")]), json!(["Private"]));
        assert!(blocked["detail"].as_str().unwrap().starts_with("Windows Defender Firewall blocks ~\\xenonet.exe on Private network"), "{blocked}");
        let public = verdict(json!([]), json!(["Public"]));
        assert!(public["fix"].as_str().unwrap().starts_with("This network is Public: make it Private") && public["fix"].as_str().unwrap().ends_with(&line), "{public}");
        assert_eq!(windows_verdict(None, paths, "~\\xenonet.exe", None)["status"], "info");
        assert!(windows_script(&["C:\\it's\\xenonet.exe".into()]).contains("$paths = @('C:\\it''s\\xenonet.exe')"));
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
