//! The process checks (core/engine/verify.js over
//! crates/xenonet-engine/src/verify.rs): the process at the other end of a
//! request's TCP connection, found by its ports as verify.js finds it, and
//! the ones above it, checked against what the request says it is. The
//! lookups block on commands, so each check runs on a blocking thread.

use crate::js::{json, no_method, now_ms, number, text};
use crate::procs::{self, System};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_engine::peer::{Calls, Result};
use xenonet_engine::sql::Arg;
use xenonet_engine::store::Shared;
use xenonet_engine::verify::Verifier;
use xenonet_service::cores::Verify;
use xenonet_service::http::Peer;

/// What a check reads: procs, the clock, and the process on the other end
/// of this request's socket.
pub struct VerifyCalls<'a> {
    pub sys: &'a System,
    pub peer: Option<Peer>,
}

impl Calls for VerifyCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        match (path, method) {
            ("procs", m) => procs::call(self.sys, m, &args),
            ("", "now") => Ok(Arg::Num(now_ms())),
            ("", "peerOf") => Ok(match self.peer {
                Some(p) => crate::js::arg(&procs::peer_of(self.sys, p.local.port(), p.remote.port())),
                None => Arg::Null,
            }),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Ok(match path.strip_prefix("procs") {
            Some(rest) => procs::get(self.sys, rest.trim_start_matches('.')),
            None => Arg::Undef,
        })
    }
    fn kind(&mut self, path: &str) -> Result<String> {
        Ok(match path.strip_prefix("procs") {
            Some(rest) => procs::kind(rest.trim_start_matches('.')).into(),
            None if matches!(path, "now" | "peerOf") => "function".into(),
            None => "undefined".into(),
        })
    }
    fn now(&mut self) -> f64 {
        now_ms()
    }
}

/// The Applications folders the tray app may run from: the machine's, and
/// the home folder's, as written and under the home folder's real path,
/// since lsof names an executable by its real path. A demo's home is its
/// made-up world's, so a demo's tray runs from that world's Applications.
pub fn tray_apps(home: &str) -> Vec<String> {
    let mut out = vec!["/Applications".to_string()];
    if home.is_empty() {
        return out;
    }
    let mut homes = vec![std::path::PathBuf::from(home)];
    if let Ok(real) = std::fs::canonicalize(home) {
        homes.push(real);
    }
    for h in homes {
        let apps = h.join("Applications").to_string_lossy().into_owned();
        if !out.contains(&apps) {
            out.push(apps);
        }
    }
    out
}

/// The checks, as the routes call them.
pub struct Checks {
    pub verifier: Arc<Verifier>,
    pub sys: Arc<System>,
}

impl Checks {
    pub fn new(store: Shared, sys: Arc<System>) -> Checks {
        // The service is its own process, so no app is its parent: the
        // Desktop that connects says which process is home.
        Checks { verifier: Arc::new(Verifier::new(store, Arg::Null)), sys }
    }

    async fn run(&self, peer: Option<Peer>, f: impl FnOnce(&Verifier, &mut VerifyCalls<'_>) -> Result<Arg> + Send + 'static) -> Value {
        let (v, sys) = (self.verifier.clone(), self.sys.clone());
        let out = tokio::task::spawn_blocking(move || {
            let mut c = VerifyCalls { sys: &sys, peer };
            f(&v, &mut c)
        })
        .await;
        match out {
            Ok(Ok(a)) => json(&a),
            // A check that couldn't run refuses, never lets through.
            Ok(Err(t)) => json!({ "ok": false, "checked": true, "reason": t.message }),
            Err(e) => json!({ "ok": false, "checked": true, "reason": e.to_string() }),
        }
    }

    /// `verify.lineage(pid)`: a process and the ones above it, as pids.
    pub fn lineage(&self, pid: &Arg) -> Arg {
        let mut c = VerifyCalls { sys: &self.sys, peer: None };
        self.verifier.lineage(&mut c, pid).unwrap_or(Arg::Null)
    }
}

#[async_trait]
impl Verify for Checks {
    async fn check(&self, peer: Peer, id: &str, hook: bool) -> Value {
        let id = Arg::text(id);
        self.run(Some(peer), move |v, c| v.check(c, &id, hook)).await
    }
    async fn person(&self, peer: Peer) -> Value {
        self.run(Some(peer), |v, c| v.person(c)).await
    }
    async fn window(&self, peer: Peer) -> Value {
        self.run(Some(peer), |v, c| v.window(c)).await
    }
    async fn desktop(&self, peer: Option<Peer>) -> Value {
        let connected = peer.is_some();
        self.run(peer, move |v, c| v.desktop(c, connected)).await
    }
    async fn tray(&self, peer: Peer, home: &str) -> Value {
        let apps = tray_apps(home);
        self.run(Some(peer), move |v, c| v.tray(c, &apps)).await
    }
    async fn actor(&self, peer: Peer, slug: &str, product: Value, pid: Value) -> Value {
        let who = crate::js::obj(vec![("slug", Arg::text(slug)), ("product", crate::js::arg(&product))]);
        let pid = crate::js::arg(&pid);
        self.run(Some(peer), move |v, c| v.actor(c, &who, &pid)).await
    }
    async fn register(&self, peer: Peer, product: Value, pid: Value) -> Value {
        let (product, pid) = (crate::js::arg(&product), crate::js::arg(&pid));
        self.run(Some(peer), move |v, c| v.register(c, &product, &pid)).await
    }
    async fn asker(&self, peer: Peer) -> Value {
        self.run(Some(peer), |v, c| v.asker(c)).await
    }
    async fn peer(&self, peer: Peer) -> String {
        let pid = self.run(Some(peer), |v, c| v.peer(c)).await;
        match pid {
            Value::Number(_) => text(&crate::js::arg(&pid)),
            _ => "?".into(),
        }
    }
    fn state(&self) -> Value {
        let c = self.verifier.counts();
        json!({
            "checked": number(c.checked),
            "refused": number(c.refused),
            "unchecked": number(c.unchecked),
            "unseen": number(c.unseen),
            "lastRefusal": c.last_refusal.as_ref().map_or(Value::Null, json),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checks() -> Checks {
        Checks::new(Shared::new(xenonet_engine::store::EngineStore::open(":memory:", 0.0).unwrap()), System::new())
    }

    #[tokio::test]
    async fn a_request_with_no_process_to_see_passes_unchecked_or_is_refused_as_unseen() {
        let c = checks();
        // A loopback address with no connection behind it: nothing owns it.
        let peer = Peer { remote: "127.0.0.1:9".parse().unwrap(), local: "127.0.0.1:1".parse().unwrap() };
        let v = c.check(peer, "claude:0190d3e0-0000-7000-8000-000000000001", false).await;
        // No process on the other end: let through unchecked, as the JS did.
        assert_eq!(v["ok"], true, "{v}");
        let you = c.person(peer).await;
        if c.sys.procs.supported {
            assert_eq!((you["ok"].clone(), you["unseen"].clone()), (json!(false), json!(true)), "{you}");
        }
        assert_eq!(c.peer(peer).await, "?");
        assert_eq!(c.state()["unchecked"], json!(if c.sys.procs.supported { 1 } else { 2 }));
        // A session of an app XenoNet doesn't check is refused, never let
        // through unchecked.
        let other = c.check(peer, "gemini-cli:x", false).await;
        assert_eq!(other["ok"], false, "{other}");
        assert_eq!(
            other["reason"],
            "requests in gemini-cli:x's name are refused: XenoNet can't tell Gemini CLI's processes apart, so it can't check that the request comes from that session"
        );
        assert_eq!(c.state()["unchecked"], json!(if c.sys.procs.supported { 1 } else { 2 }));
    }

    // A made-up Mac, answering the commands procs asks for: the Claude app
    // with two Claude Code sessions (200 and 300, both signed), a shell
    // under the first, and a plain terminal; each connection's process by
    // its remote port, as netstat names it.
    fn machine() -> Arc<System> {
        let rows = [
            (1, 0, "/sbin/launchd"),
            (100, 1, "/Applications/Claude.app/Contents/MacOS/Claude"),
            (200, 100, "/Users/me/.local/share/claude/versions/2.1.288"),
            (220, 200, "/bin/zsh"),
            (300, 100, "/Users/me/.local/share/claude/versions/2.1.288"),
            (400, 1, "/bin/zsh"),
        ];
        let ps = rows.iter().map(|(pid, ppid, p)| format!("{pid:>5} {ppid:>5} Fri Oct  3 16:34:29 2026 {p}\n")).collect::<Vec<_>>().concat();
        let by_port = [(51000, 220), (51001, 400), (51002, 300), (51003, 200)];
        let netstat = by_port.iter().map(|(port, pid)| format!("tcp4       0      0  127.0.0.1.{port}        127.0.0.1.47842        ESTABLISHED    111   2970  406912  146988 proc:{pid}            00102 00000104 0000000003bca9cb\n")).collect::<Vec<_>>().concat();
        System::with(
            "darwin",
            Box::new(move |c| match c.file.as_str() {
                "ps" if c.args.first().map(String::as_str) == Some("-axo") => Ok(ps.clone()),
                "netstat" => Ok(netstat.clone()),
                // Only the sessions' own processes are signed.
                "codesign" if c.args.last().is_some_and(|p| p == "200" || p == "300") => Ok(String::new()),
                _ => Err(xenonet_engine::procs::Failure { message: format!("Command failed: {}", c.file), stdout: None }),
            }),
        )
    }

    fn from(port: u16) -> Peer {
        Peer { remote: format!("127.0.0.1:{port}").parse().unwrap(), local: "127.0.0.1:47842".parse().unwrap() }
    }

    // The tray app in the home folder's Applications, started by launchd at
    // login (500), and the same executable started from a shell under
    // Claude Code (510); lsof names each executable by its real path.
    #[tokio::test]
    async fn the_tray_passes_from_its_bundle_in_an_applications_folder_and_never_under_an_agent() {
        const TRAY: &str = "/Users/me/Applications/XenoNet Tray.app/Contents/MacOS/XenoNet Tray";
        let rows = [
            (1, 0, "/sbin/launchd"),
            (100, 1, "/Applications/Claude.app/Contents/MacOS/Claude"),
            (200, 100, "/Users/me/.local/share/claude/versions/2.1.288"),
            (220, 200, "/bin/zsh"),
            (500, 1, TRAY),
            (510, 220, TRAY),
        ];
        let ps = rows.iter().map(|(pid, ppid, p)| format!("{pid:>5} {ppid:>5} Fri Oct  3 16:34:29 2026 {p}\n")).collect::<Vec<_>>().concat();
        let netstat = [(51010, 500), (51011, 510)]
            .iter()
            .map(|(port, pid)| format!("tcp4       0      0  127.0.0.1.{port}        127.0.0.1.47842        ESTABLISHED    111   2970  406912  146988 proc:{pid}            00102 00000104 0000000003bca9cb\n"))
            .collect::<Vec<_>>()
            .concat();
        let exes: Vec<(String, String)> = rows.iter().map(|(pid, _, p)| (pid.to_string(), (*p).to_string())).collect();
        let sys = System::with(
            "darwin",
            Box::new(move |c| match c.file.as_str() {
                "ps" if c.args.first().map(String::as_str) == Some("-axo") => Ok(ps.clone()),
                "netstat" => Ok(netstat.clone()),
                "lsof" if c.args.iter().any(|a| a == "txt") => {
                    let asked: Vec<&str> = c.args.get(2).map(|l| l.split(',').collect()).unwrap_or_default();
                    let lines: Vec<String> = exes.iter().filter(|(pid, _)| asked.contains(&pid.as_str())).map(|(pid, exe)| format!("p{pid}\nftxt\nn{exe}\n")).collect();
                    Ok(lines.concat())
                }
                _ => Err(xenonet_engine::procs::Failure { message: format!("Command failed: {}", c.file), stdout: None }),
            }),
        );
        let c = Checks::new(Shared::new(xenonet_engine::store::EngineStore::open(":memory:", 0.0).unwrap()), sys);
        assert_eq!(tray_apps("/Users/me"), ["/Applications", "/Users/me/Applications"]);
        assert_eq!(c.tray(from(51010), "/Users/me").await, json!({ "ok": true, "checked": true }));
        let under = c.tray(from(51011), "/Users/me").await;
        assert_eq!(under["reason"], "XenoNet's tray can't run under a Claude Code process", "{under}");
        let other = c.tray(from(51010), "/Users/you").await;
        assert_eq!(other["ok"], false, "another home's Applications: {other}");
    }

    #[tokio::test]
    async fn a_request_in_a_sessions_name_is_held_to_the_process_it_comes_from() {
        let c = Checks::new(Shared::new(xenonet_engine::store::EngineStore::open(":memory:", 0.0).unwrap()), machine());
        let s = "claude:0190d3e0-0000-7000-8000-00000000000a";
        // From the shell under Claude Code: it's that session's, now pinned
        // to its process.
        let ok = c.check(from(51000), s, false).await;
        assert_eq!((ok["ok"].clone(), ok["root"]["pid"].clone()), (json!(true), json!(200)), "{ok}");
        // From a plain terminal: no Claude Code above it.
        let no = c.check(from(51001), s, false).await;
        assert_eq!(no["ok"], false);
        assert!(no["reason"].as_str().unwrap().contains("have to come from a Claude Code process"), "{no}");
        // From the other session's process, while the first still runs.
        let other = c.check(from(51002), s, false).await;
        assert!(other["reason"].as_str().unwrap().contains("belongs to another Claude Code process"), "{other}");
        // Claude Code's own hook, from its signed process, is sure of it.
        let hook = c.check(from(51003), s, true).await;
        assert_eq!((hook["ok"].clone(), hook["sure"].clone()), (json!(true), json!(true)), "{hook}");
        // What's yours: refused from under an agent, and the process seen.
        let you = c.person(from(51000)).await;
        assert!(you["reason"].as_str().unwrap().starts_with("this is yours to do, and it came from under a Claude Code process"), "{you}");
        assert_eq!(c.person(from(51001)).await["ok"], true);
        assert_eq!(c.peer(from(51001)).await, "400");
        assert_eq!(c.state()["refused"], json!(3));
        assert_eq!(c.lineage(&Arg::Num(220.0)), Arg::List(vec![Arg::Num(220.0), Arg::Num(200.0), Arg::Num(100.0)]));
    }
}
