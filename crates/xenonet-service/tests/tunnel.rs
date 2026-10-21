// The actor gateway's tunnels (server/gateway-tunnel.js), with the tools
// stood in for, as test/gateway-tunnel.test.js has them: Tailscale Funnel
// set up with the command that matches the running Tailscale, what it
// already serves left alone, a child tunnel's address learned from its
// output and started again when it exits.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use xenonet_service::gateway::tunnel::*;

const PORT: u16 = 48850;
const HOST: &str = "mac.example-tailnet.ts.net";
const APP: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";
const BREW: &str = "/opt/homebrew/bin/tailscale";

fn status(state: &str, version: &str, funnel: bool) -> String {
    let caps = if funnel { json!({ "funnel": null, "https://tailscale.com/cap/funnel-ports?ports=443,8443,10000": null }) } else { json!({}) };
    json!({ "BackendState": state, "Version": version, "Self": { "DNSName": format!("{HOST}."), "CapMap": caps } }).to_string()
}

fn served(proxy: &str, allow: bool, more: bool) -> String {
    let mut handlers = json!({ "/": { "Proxy": proxy } });
    if more {
        handlers["/grafana"] = json!({ "Proxy": "http://127.0.0.1:3000" });
    }
    let mut conf = json!({ "TCP": { "443": { "HTTPS": true } }, "Web": { format!("{HOST}:443"): { "Handlers": handlers } } });
    if allow {
        conf["AllowFunnel"] = json!({ format!("{HOST}:443"): true });
    }
    conf.to_string()
}

struct Child {
    file: String,
    args: Vec<String>,
    lines: mpsc::UnboundedSender<String>,
    exit: Option<oneshot::Sender<String>>,
    killed: Arc<Mutex<bool>>,
}

type OnFunnel = Box<dyn Fn(&[String]) -> (RunOut, Option<String>) + Send + Sync>;

struct Fake {
    tools: HashMap<String, String>,
    status: String,
    conf: Mutex<String>,
    on_funnel: Option<OnFunnel>,
    ran: Mutex<Vec<Vec<String>>>,
    now: Mutex<f64>,
    children: Mutex<Vec<Child>>,
    /// What ps says each pid runs, and the pids XenoNet stopped.
    comms: Mutex<HashMap<String, String>>,
    stopped: Mutex<Vec<u32>>,
}

impl Fake {
    fn new(tools: &[(&str, &str)], status: String, conf: &str, on_funnel: Option<OnFunnel>) -> Arc<Fake> {
        Arc::new(Fake {
            tools: tools.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            status,
            conf: Mutex::new(conf.into()),
            on_funnel,
            ran: Mutex::new(vec![]),
            now: Mutex::new(1_000_000.0),
            children: Mutex::new(vec![]),
            comms: Mutex::new(HashMap::new()),
            stopped: Mutex::new(vec![]),
        })
    }
    fn tailscale(on_funnel: Option<OnFunnel>, conf: &str, status_text: String) -> Arc<Fake> {
        Fake::new(&[(BREW, "1.98.8"), (APP, "1.104.1")], status_text, conf, on_funnel)
    }
    fn ran(&self) -> Vec<Vec<String>> {
        self.ran.lock().unwrap().clone()
    }
}

#[async_trait]
impl TunnelHost for Fake {
    fn exists(&self, file: &str) -> bool {
        self.tools.contains_key(file)
    }
    async fn run(&self, file: &str, args: &[String], _timeout: Duration) -> RunOut {
        self.ran.lock().unwrap().push(std::iter::once(file.to_string()).chain(args.iter().cloned()).collect());
        let ok = |stdout: String| RunOut { ok: true, stdout, stderr: String::new(), timed_out: false };
        if file == "ps" {
            // ps -o comm= -p <pid>: what it runs, or a failure for a pid that's gone.
            return match self.comms.lock().unwrap().get(&args[3]) {
                Some(comm) => ok(format!("{comm}\n")),
                None => RunOut { ok: false, stdout: String::new(), stderr: String::new(), timed_out: false },
            };
        }
        match args.first().map(String::as_str) {
            Some("version" | "--version") => ok(format!("{}\n  tailscale commit: abc\n", self.tools.get(file).cloned().unwrap_or_default())),
            Some("status") => ok(self.status.clone()),
            Some("funnel") if args.get(1).map(String::as_str) == Some("status") => ok(self.conf.lock().unwrap().clone()),
            Some("funnel") => {
                let (out, conf) = match &self.on_funnel {
                    Some(f) => f(args),
                    None => (ok("Available on the internet".into()), None),
                };
                if let Some(c) = conf {
                    *self.conf.lock().unwrap() = c;
                }
                out
            }
            _ => RunOut { ok: false, stdout: String::new(), stderr: "unknown".into(), timed_out: false },
        }
    }
    fn spawn(&self, file: &str, args: &[String]) -> Result<Spawned, String> {
        let (ltx, lines) = mpsc::unbounded_channel();
        let (etx, exit) = oneshot::channel();
        let killed = Arc::new(Mutex::new(false));
        let mut children = self.children.lock().unwrap();
        let pid = 4000 + children.len() as u32;
        children.push(Child { file: file.into(), args: args.to_vec(), lines: ltx, exit: Some(etx), killed: killed.clone() });
        Ok(Spawned { pid: Some(pid), lines, exit, kill: Box::new(move || *killed.lock().unwrap() = true) })
    }
    fn now(&self) -> f64 {
        *self.now.lock().unwrap()
    }
    fn path_var(&self) -> String {
        String::new()
    }
    fn platform(&self) -> &'static str {
        "darwin"
    }
    fn kill_pid(&self, pid: u32) {
        self.stopped.lock().unwrap().push(pid);
    }
}

fn tunnel(host: Arc<Fake>, hooks: TunnelHooks) -> Tunnel {
    Tunnel::new(PORT, None, "/Users/nova", host, hooks)
}

#[test]
fn tailscales_status_and_funnels_settings_as_the_gateway_needs_them() {
    assert_eq!(tailscale_self(&status("Running", "1.104.1-t7f4", true)), json!({ "up": true, "version": "1.104.1", "host": HOST, "funnel": true }));
    assert_eq!(tailscale_self(&status("Running", "1.104.1-t7f4", false))["funnel"], json!(false));
    assert_eq!(tailscale_self(&status("NeedsLogin", "1.104.1-t7f4", true)), json!({ "up": false, "version": "1.104.1", "why": "Tailscale isn't signed in on this computer" }));
    assert_eq!(tailscale_self(&status("Stopped", "1.104.1", true))["why"], json!("Tailscale is turned off on this computer"));
    assert_eq!(tailscale_self("not json")["up"], json!(false));
    let p = |t: String| serde_json::from_str::<Value>(&t).unwrap();
    assert_eq!(funnel_of(&json!({}), HOST, PORT), json!({ "state": "none" }));
    assert_eq!(funnel_of(&p(served(&format!("http://127.0.0.1:{PORT}"), true, false)), HOST, PORT), json!({ "state": "ours" }));
    assert_eq!(funnel_of(&p(served(&format!("http://localhost:{PORT}"), false, false)), HOST, PORT), json!({ "state": "tailnet" }));
    assert_eq!(funnel_of(&p(served("http://127.0.0.1:48852", true, false)), HOST, PORT), json!({ "state": "elsewhere", "target": "http://127.0.0.1:48852" }));
    assert_eq!(funnel_of(&p(served(&format!("http://127.0.0.1:{PORT}"), true, true)), HOST, PORT), json!({ "state": "shared", "target": "/, /grafana" }));
}

#[test]
fn each_tool_is_found_on_path_and_where_its_usually_installed() {
    let sep = if cfg!(windows) { ";" } else { ":" };
    // As path.join writes it on this machine, for the test to run anywhere.
    let onpath = xenonet_base::paths::join("/home/me/bin", "ngrok");
    let files = ["/opt/homebrew/bin/ngrok".to_string(), "/usr/local/bin/ngrok".to_string(), onpath.clone()];
    let exists = |f: &str| files.iter().any(|x| x == f);
    assert_eq!(find_tools("ngrok", &format!("/home/me/bin{sep}/usr/bin"), None, "darwin", &exists), vec![onpath, "/opt/homebrew/bin/ngrok".into(), "/usr/local/bin/ngrok".into()]);
    assert_eq!(find_tools("ngrok", "", None, "darwin", &|_| false), Vec::<String>::new());
    assert_eq!(kind_ids(), vec!["tailscale", "cloudflare", "cloudflare-quick", "ngrok", "own"]);
}

#[test]
fn a_tunnel_tools_lines_the_address_it_got_that_its_up_and_what_went_wrong() {
    let quick = read_line("cloudflare-quick", "2026-10-08T10:00:00Z INF |  https://brave-otter-sails.trycloudflare.com  |").unwrap();
    assert_eq!(
        (quick.msg.as_str(), quick.host.as_deref()),
        ("2026-10-08T10:00:00Z INF |  https://brave-otter-sails.trycloudflare.com  |", Some("brave-otter-sails.trycloudflare.com"))
    );
    assert!(read_line("cloudflare", "2026-10-08T10:00:01Z INF Registered tunnel connection connIndex=0 location=ord01").unwrap().ready);
    assert!(read_line("cloudflare", "2026-10-08T10:00:01Z ERR Couldn't start tunnel error=\"no such tunnel\"").unwrap().error);
    let started = read_line("ngrok", &json!({ "lvl": "info", "msg": "started tunnel", "url": "https://xenonet-demo.ngrok-free.app" }).to_string()).unwrap();
    assert_eq!(started, Line { msg: "started https://xenonet-demo.ngrok-free.app".into(), host: Some("xenonet-demo.ngrok-free.app".into()), ready: true, error: false });
    let bad = read_line("ngrok", &json!({ "lvl": "eror", "msg": "session closing", "err": "authentication failed: ERR_NGROK_4018" }).to_string()).unwrap();
    assert_eq!((bad.msg.as_str(), bad.error), ("authentication failed: ERR_NGROK_4018", true));
    assert_eq!(read_line("ngrok", ""), None);
    let origin = format!("http://127.0.0.1:{PORT}");
    assert_eq!(child_args("cloudflare", PORT, Some("xenonet"), None).unwrap(), vec!["tunnel", "--no-autoupdate", "run", "--url", &origin, "xenonet"]);
    assert_eq!(child_args("cloudflare-quick", PORT, None, None).unwrap(), vec!["tunnel", "--no-autoupdate", "--url", &origin]);
    let local = format!("127.0.0.1:{PORT}");
    assert_eq!(
        child_args("ngrok", PORT, None, Some("xenonet-demo.ngrok-free.app")).unwrap(),
        vec!["http", &local, "--log", "stdout", "--log-format", "json", "--url", "https://xenonet-demo.ngrok-free.app"]
    );
}

#[tokio::test]
async fn tailscale_funnel_set_up_with_the_command_that_matches_and_nothing_done_when_its_there() {
    let conf = served(&format!("http://127.0.0.1:{PORT}"), true, false);
    let host = Fake::tailscale(Some(Box::new(move |_| (RunOut { ok: true, ..Default::default() }, Some(conf.clone())))), "{}", status("Running", "1.104.1-t7f4", true));
    let hosts = Arc::new(Mutex::new(Vec::new()));
    let h = hosts.clone();
    let t = tunnel(host.clone(), TunnelHooks { on_host: Arc::new(move |x| h.lock().unwrap().push(x.to_string())), ..Default::default() });
    let v = t.up("tailscale", None, None, false).await.unwrap();
    assert_eq!((v["kind"].clone(), v["state"].clone(), v["why"].clone()), (json!("tailscale"), json!("up"), Value::Null));
    assert_eq!(*hosts.lock().unwrap(), vec![HOST.to_string()]);
    // Homebrew's older command is passed over for the app's, which matches.
    assert_eq!(host.ran().into_iter().find(|r| r[1] == "funnel" && r[2] == "--bg").unwrap(), vec![APP, "funnel", "--bg", "--yes", "48850"]);
    let before = host.ran().len();
    t.ensure().await;
    assert_eq!(host.ran()[before..].iter().filter(|r| r[1] == "funnel" && r[2] != "status").count(), 0, "already set up");
    // Off turns Funnel off, as it forwards to this gateway alone.
    t.down().await;
    assert_eq!(host.ran().last().unwrap(), &vec![APP, "funnel", "--yes", "--https=443", "off"]);
    assert_eq!(t.view()["state"], json!("off"));
}

#[tokio::test]
async fn tailscale_funnel_leaves_what_it_forwards_elsewhere_until_repair_and_passes_on_what_tailscale_asks() {
    let ours = served(&format!("http://127.0.0.1:{PORT}"), true, false);
    let elsewhere = Fake::tailscale(
        Some(Box::new(move |_| (RunOut { ok: true, ..Default::default() }, Some(ours.clone())))),
        &served("http://127.0.0.1:48852", true, false),
        status("Running", "1.104.1", true),
    );
    let t1 = tunnel(elsewhere.clone(), TunnelHooks::default());
    let why = t1.up("tailscale", None, None, false).await.unwrap()["why"].as_str().unwrap().to_string();
    assert!(why.contains("already forwards to http://127.0.0.1:48852, not to this XenoNet's gateway: Repair points it here"), "{why}");
    assert!(!elsewhere.ran().iter().any(|r| r.get(2).map(String::as_str) == Some("--bg")));
    assert_eq!(t1.up("tailscale", None, None, true).await.unwrap()["state"], json!("up"));

    let shared = Fake::tailscale(None, &served(&format!("http://127.0.0.1:{PORT}"), true, true), status("Running", "1.104.1", true));
    let t2 = tunnel(shared.clone(), TunnelHooks::default());
    assert!(t2.up("tailscale", None, None, true).await.unwrap()["why"].as_str().unwrap().contains("already serves other paths on this computer's port 443 (/, /grafana)"));
    t2.down().await;
    assert!(!shared.ran().iter().any(|r| r.iter().any(|a| a == "off")), "turning off leaves what isn't the gateway's alone");

    let blocked = Fake::tailscale(None, "{}", status("Running", "1.104.1", false));
    assert!(tunnel(blocked, TunnelHooks::default()).up("tailscale", None, None, false).await.unwrap()["why"]
        .as_str()
        .unwrap()
        .contains("your tailnet doesn't allow Funnel for this computer yet"));

    let asks = Fake::tailscale(
        Some(Box::new(|_| {
            (
                RunOut {
                    ok: false,
                    stdout: "Funnel is not enabled on your tailnet.\nTo enable, visit:\n\n         https://login.tailscale.com/f/funnel?node=abc\n".into(),
                    stderr: String::new(),
                    timed_out: true,
                },
                None,
            )
        })),
        "{}",
        status("Running", "1.104.1", true),
    );
    assert_eq!(
        tunnel(asks, TunnelHooks::default()).up("tailscale", None, None, false).await.unwrap()["why"],
        json!("Tailscale asks you to allow Funnel for this computer at https://login.tailscale.com/f/funnel?node=abc; then Repair")
    );
    let none = Fake::new(&[], String::new(), "{}", None);
    assert_eq!(tunnel(none, TunnelHooks::default()).up("tailscale", None, None, false).await.unwrap()["why"], json!("XenoNet can't find Tailscale on this computer"));
    let out = Fake::tailscale(None, "{}", status("NeedsLogin", "1.104.1", true));
    assert_eq!(tunnel(out, TunnelHooks::default()).up("tailscale", None, None, false).await.unwrap()["why"], json!("Tailscale isn't signed in on this computer"));
}

async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_tunnel_xenonet_runs_learned_restarted_failed_and_stopped() {
    let host = Fake::new(&[("/opt/homebrew/bin/ngrok", "3.24.0")], String::new(), "{}", None);
    let hosts = Arc::new(Mutex::new(Vec::new()));
    let ready = Arc::new(Mutex::new(0));
    let (h, r) = (hosts.clone(), ready.clone());
    let t = tunnel(
        host.clone(),
        TunnelHooks { on_host: Arc::new(move |x| h.lock().unwrap().push(x.to_string())), on_ready: Arc::new(move || *r.lock().unwrap() += 1), ..Default::default() },
    );
    assert_eq!(t.up("ngrok", None, None, false).await.unwrap()["state"], json!("starting"));
    {
        let c = host.children.lock().unwrap();
        assert_eq!((c[0].file.as_str(), &c[0].args[..2]), ("/opt/homebrew/bin/ngrok", &["http".to_string(), format!("127.0.0.1:{PORT}")][..]));
        c[0].lines.send(json!({ "lvl": "info", "msg": "started tunnel", "url": "https://xenonet-demo.ngrok-free.app" }).to_string()).unwrap();
    }
    settle().await;
    assert_eq!((t.view()["state"].clone(), hosts.lock().unwrap().clone(), *ready.lock().unwrap()), (json!("up"), vec!["xenonet-demo.ngrok-free.app".to_string()], 1));
    // It exits: XenoNet waits, then starts it again, longer each time.
    for (i, wait) in [5u64, 20, 60].into_iter().enumerate() {
        {
            let mut c = host.children.lock().unwrap();
            let last = c.last_mut().unwrap();
            last.lines.send(json!({ "lvl": "eror", "msg": "x", "err": "failed to reconnect session" }).to_string()).unwrap();
            settle_sync();
            let _ = last.exit.take().unwrap().send("it exited with code 1".into());
        }
        settle().await;
        let why = t.view()["why"].as_str().unwrap().to_string();
        assert_eq!(why, format!("it exited with code 1: failed to reconnect session; trying again in {wait} s"));
        *host.now.lock().unwrap() += 1000.0;
        tokio::time::advance(Duration::from_secs(wait)).await;
        settle().await;
        assert_eq!(host.children.lock().unwrap().len(), i + 2);
    }
    // A fourth exit in ten minutes: it stays failed until a check ten minutes on.
    {
        let mut c = host.children.lock().unwrap();
        let _ = c.last_mut().unwrap().exit.take().unwrap().send("it exited with code 1".into());
    }
    settle().await;
    assert_eq!(t.view()["state"], json!("failed"));
    t.ensure().await;
    assert_eq!(host.children.lock().unwrap().len(), 4, "not yet");
    *host.now.lock().unwrap() += 10.0 * 60e3;
    t.ensure().await;
    assert_eq!(host.children.lock().unwrap().len(), 5);
    // Another kind stops it first; XenoNet going stops it too.
    t.up("own", None, None, false).await.unwrap();
    assert_eq!((*host.children.lock().unwrap()[4].killed.lock().unwrap(), t.view()["state"].clone()), (true, json!("yours")));
    t.up("cloudflare-quick", None, None, false).await.unwrap();
    t.stop();
    let c = host.children.lock().unwrap();
    assert!(c.len() == 5 || *c.last().unwrap().killed.lock().unwrap(), "no cloudflared here, or it was stopped");
}

fn settle_sync() {}

#[tokio::test(start_paused = true)]
async fn a_tunnel_left_running_is_stopped_only_while_its_pid_is_still_that_tool() {
    let dir = std::env::temp_dir().join(format!("xenonet-tunnel-pid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pid_file = dir.join("gateway-tunnel.pid");
    // What the file says, what ps says that pid runs now, and whether
    // XenoNet stops it.
    for (left, comm, stopped) in [
        ("4242", Some("/opt/homebrew/bin/ngrok"), true),
        ("4242\n", Some("NGROK"), true),
        ("4242", Some("/bin/zsh"), false),
        ("4242", Some("/Applications/Safari.app/Contents/MacOS/Safari"), false),
        ("4242", None, false),
        ("1", Some("ngrok"), false),
        ("0", Some("ngrok"), false),
        ("-4242", Some("ngrok"), false),
        ("4242; kill -9 1", Some("ngrok"), false),
        ("", Some("ngrok"), false),
    ] {
        let host = Fake::new(&[("/opt/homebrew/bin/ngrok", "3.24.0")], String::new(), "{}", None);
        let pid = left.trim().to_string();
        if let Some(c) = comm {
            host.comms.lock().unwrap().insert(pid.clone(), c.to_string());
        }
        std::fs::write(&pid_file, left).unwrap();
        let t = Tunnel::new(PORT, Some(&dir.to_string_lossy()), "/Users/nova", host.clone(), TunnelHooks::default());
        t.up("ngrok", None, None, false).await.unwrap();
        settle().await;
        let want: Vec<u32> = if stopped { vec![pid.parse().unwrap()] } else { vec![] };
        assert_eq!(*host.stopped.lock().unwrap(), want, "{left:?} {comm:?}");
        let asked_ps = host.ran().iter().any(|r| r[0] == "ps");
        assert_eq!(asked_ps, pid.parse::<u32>().is_ok_and(|p| p > 1), "ps only for a pid past init: {left:?}");
        // The file now names the tunnel this XenoNet started.
        assert_eq!(std::fs::read_to_string(&pid_file).unwrap(), "4000");
        t.stop();
        settle().await;
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
