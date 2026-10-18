//! OpenCode: every TUI runs its own HTTP server, on a random port unless
//! started with --port: core/delivery/opencode.js. XenoNet finds the
//! running opencode processes and the ports they listen on, asks each which
//! sessions it has, and posts the message to the session with
//! prompt_async, which starts a turn in the live TUI.
//!
//! Commands, files and HTTP are the twin's, called as the JS called them:
//! "ps"/"exec" (a command's output, or "" when it failed), "readFile",
//! "readdir" and "readlink" (node:fs's), and for a server "get" (a GET's
//! { ok, status, json }) and "post" (prompt_async's { ok, status }), with
//! "processes", "ports" and "now".

use crate::peer::{list, Calls, Result, Thrown};
use crate::sql::Arg;
use crate::store::js_text;
use std::sync::Mutex;

/// XenoNet's ids take OpenCode's ses_… with "-" for "_".
pub fn open_code_id(id: &Arg) -> String {
    let s = js_text(id);
    let s = s.strip_prefix("opencode:").unwrap_or(&s);
    match s.strip_prefix("ses-") {
        Some(rest) => format!("ses_{rest}"),
        None => s.to_string(),
    }
}

/// And back: XenoNet's id for a session named the way OpenCode shows it.
/// Any other id comes back as it was.
pub fn from_open_code_id(id: &Arg) -> String {
    let s = if id.is_nullish() { String::new() } else { js_text(id) };
    let s = xenonet_base::text::js_trim(&s);
    match crate::re!(r"^(?:opencode:)?ses_([0-9A-Za-z]+)$").captures(s) {
        Some(m) => format!("opencode:ses-{}", &m[1]),
        None => s.to_string(),
    }
}

/// A process: its pid and command line.
#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: f64,
    pub args: String,
}

impl Proc {
    pub fn to_arg(&self) -> Arg {
        Arg::Obj(vec![("pid".into(), Arg::num(self.pid)), ("args".into(), Arg::text(self.args.clone()))])
    }
    pub fn of(a: &Arg) -> Proc {
        Proc {
            pid: match a.field("pid") {
                Arg::Num(n) => n,
                _ => f64::NAN,
            },
            args: js_text(&a.field("args")),
        }
    }
}

pub fn parse_ps(out: &str) -> Vec<Proc> {
    out.split('\n')
        .filter_map(|l| crate::re!(r"^\s*(\d+)\s+(.*)$").captures(l))
        .map(|m| Proc { pid: xenonet_base::text::js_number(Some(&serde_json::Value::String(m[1].to_string()))), args: m[2].to_string() })
        .collect()
}

/// opencode processes: the command itself, not a shell or tmux that names
/// it in its arguments.
pub fn open_code_processes(c: &mut dyn Calls, windows: bool) -> Result<Vec<Proc>> {
    if windows {
        let out = c.call(
            "",
            "ps",
            vec![
                Arg::text("powershell.exe"),
                Arg::List(vec![
                    Arg::text("-NoProfile"),
                    Arg::text("-Command"),
                    Arg::text("Get-CimInstance Win32_Process -Filter \"Name like 'opencode%'\" | ForEach-Object { \"$($_.ProcessId) $($_.CommandLine)\" }"),
                ]),
            ],
        )?;
        return Ok(parse_ps(&js_text(&out)));
    }
    let out = c.call("", "ps", vec![Arg::text("ps"), Arg::List(vec![Arg::text("-axo"), Arg::text("pid=,args=")])])?;
    Ok(parse_ps(&js_text(&out))
        .into_iter()
        .filter(|p| crate::re!(r"^(\S*[/\\])?opencode(\.exe)?(\s|$)").is_match(&p.args) || crate::re!(r"^\S*node\S*\s+\S*opencode-ai").is_match(&p.args))
        .collect())
}

fn unique(v: Vec<f64>) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for x in v {
        if !out.iter().any(|y| *y == x || (x.is_nan() && y.is_nan())) {
            out.push(x);
        }
    }
    out
}

fn words(s: &str) -> Vec<&str> {
    crate::re!(r"\s+").split(xenonet_base::text::js_trim(s)).collect()
}

fn number(s: &str) -> f64 {
    xenonet_base::text::js_number(Some(&serde_json::Value::String(s.to_string())))
}

/// parseInt(s, 16).
fn parse_hex(s: &str) -> f64 {
    let t = xenonet_base::text::js_trim(s);
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    let digits: String = t.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    if digits.is_empty() {
        return f64::NAN;
    }
    digits.chars().fold(0.0, |n, d| n * 16.0 + d.to_digit(16).unwrap() as f64)
}

/// The TCP ports a process listens on, on loopback or any address.
pub fn listening_ports(c: &mut dyn Calls, pid: f64, platform: &str) -> Result<Vec<f64>> {
    let pid_text = js_text(&Arg::num(pid));
    if platform == "win32" {
        let out = js_text(&c.call("", "exec", vec![Arg::text("netstat"), Arg::List(vec![Arg::text("-ano"), Arg::text("-p"), Arg::text("tcp")])])?);
        let mut ports = Vec::new();
        for l in out.split('\n') {
            let cols = words(l);
            if cols.get(3) == Some(&"LISTENING") && cols.get(4).map(|p| number(p)) == Some(pid) {
                ports.push(number(cols[1].rsplit(':').next().unwrap_or("")));
            }
        }
        return Ok(unique(ports));
    }
    if platform == "linux" {
        if let Ok(fds) = c.call("", "readdir", vec![Arg::text(format!("/proc/{pid_text}/fd"))]) {
            let mut inodes: Vec<String> = Vec::new();
            for fd in list(fds) {
                if let Ok(link) = c.call("", "readlink", vec![Arg::text(format!("/proc/{pid_text}/fd/{}", js_text(&fd)))]) {
                    if let Some(m) = crate::re!(r"^socket:\[(\d+)\]$").captures(&js_text(&link)) {
                        inodes.push(m[1].to_string());
                    }
                }
            }
            let mut ports = Vec::new();
            for f in ["/proc/net/tcp", "/proc/net/tcp6"] {
                let text = c.call("", "readFile", vec![Arg::text(f), Arg::text("utf8")]).map(|t| js_text(&t)).unwrap_or_default();
                for line in text.split('\n').skip(1) {
                    let cols = words(line);
                    if cols.get(3) == Some(&"0A") && cols.get(9).is_some_and(|i| inodes.iter().any(|n| n == i)) {
                        ports.push(parse_hex(cols[1].split(':').nth(1).unwrap_or("")));
                    }
                }
            }
            if !ports.is_empty() {
                return Ok(unique(ports));
            }
        }
    }
    let out =
        js_text(&c.call("", "exec", vec![Arg::text("lsof"), Arg::List(["-nP", "-a", "-p", &pid_text, "-iTCP", "-sTCP:LISTEN", "-Fn"].iter().map(|a| Arg::text(*a)).collect())])?);
    let ports = regex_lsof().captures_iter(&out).map(|m| number(&m[1])).collect();
    Ok(unique(ports))
}

/// /^n.*:(\d+)$/gm, with \r a line's end as JS's m flag has it.
fn regex_lsof() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?mR)^n[^\n\r\x{2028}\x{2029}]*:([0-9]+)$").unwrap())
}

/// One folder, as OpenCode spells it in two places: the same once a
/// trailing slash goes, and on Windows whatever the case or the slashes.
pub fn same_path(a: &Arg, b: &Arg, windows: bool) -> bool {
    let norm = |p: &Arg| {
        let s = js_text(p);
        let s = s.trim_end_matches(['\\', '/']).to_string();
        if windows {
            s.replace('\\', "/").to_lowercase()
        } else {
            s
        }
    };
    norm(a) == norm(b)
}

/// encodeURIComponent.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || "-_.!~*'()".contains(ch) {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// A server: its process, where it answers, and the folder it was started
/// in, as it says.
#[derive(Clone, Debug, PartialEq)]
pub struct Server {
    pub pid: Arg,
    pub base: String,
    pub dir: Arg,
}

impl Server {
    pub fn to_arg(&self) -> Arg {
        Arg::Obj(vec![("pid".into(), self.pid.clone()), ("base".into(), Arg::text(self.base.clone())), ("dir".into(), self.dir.clone())])
    }
}

#[derive(Default)]
struct Inner {
    cache_at: f64,
    servers: Vec<Server>,
    /// ses_… → (when, what OpenCode said), the oldest first.
    infos: Vec<(String, f64, Arg)>,
}

pub struct OpenCode {
    cache_ms: f64,
    inner: Mutex<Inner>,
}

impl OpenCode {
    pub fn new(cache_ms: f64) -> Self {
        OpenCode { cache_ms, inner: Mutex::new(Inner { cache_at: f64::NEG_INFINITY, ..Inner::default() }) }
    }

    fn now(&self, c: &mut dyn Calls) -> Result<f64> {
        let t = c.call("", "now", vec![])?;
        Ok(match t {
            Arg::Num(n) => n,
            other => xenonet_base::text::js_number(Some(&other.to_json())),
        })
    }

    /// A GET's JSON, or what it threw (an answer that isn't ok: route and
    /// status).
    fn get(&self, c: &mut dyn Calls, base: &str, route: &str, ms: f64) -> Result<Arg> {
        let r = c.call("", "get", vec![Arg::text(format!("{base}{route}")), Arg::num(ms)])?;
        if !r.field("ok").truthy() {
            return Err(Thrown { message: format!("{route}: {}", js_text(&r.field("status"))), code: None, handle: None, stdout: None });
        }
        Ok(r.field("json"))
    }

    /// The OpenCode servers running now.
    pub fn servers(&self, c: &mut dyn Calls) -> Result<Vec<Server>> {
        let t = self.now(c)?;
        {
            let i = self.inner.lock().unwrap();
            if t - i.cache_at < self.cache_ms {
                return Ok(i.servers.clone());
            }
        }
        let mut found: Vec<Server> = Vec::new();
        for p in list(c.call("", "processes", vec![])?) {
            let args = js_text(&p.field("args"));
            let ports = match crate::re!(r"--port[= ](\d+)").captures(&args) {
                Some(m) => vec![Arg::num(number(&m[1]))],
                None => list(c.call("", "ports", vec![p.field("pid")])?),
            };
            for port in ports {
                let base = format!("http://127.0.0.1:{}", js_text(&port));
                if found.iter().any(|f| f.base == base) {
                    continue;
                }
                let Ok(h) = self.get(c, &base, "/global/health", 800.0) else { continue };
                if !h.field("healthy").truthy() {
                    continue;
                }
                let dir = match self.get(c, &base, "/path", 800.0) {
                    Ok(x) => x.field("directory").or(Arg::Null),
                    Err(_) => Arg::Null,
                };
                found.push(Server { pid: p.field("pid"), base, dir });
            }
        }
        let at = self.now(c)?;
        let mut i = self.inner.lock().unwrap();
        i.cache_at = at;
        i.servers = found.clone();
        Ok(found)
    }

    /// The server whose TUI has the session in hand: the one that reports
    /// a status for it, else one started in the session's folder, else any
    /// that knows it.
    pub fn server_for(&self, c: &mut dyn Calls, id: &Arg, windows: bool) -> Result<Option<Server>> {
        let ses = open_code_id(id);
        let (mut here, mut known) = (None, None);
        for s in self.servers(c)? {
            let Ok(status) = self.get(c, &s.base, "/session/status", 1500.0) else { continue };
            let has = match &status {
                Arg::Obj(f) => f.iter().any(|(k, _)| *k == ses),
                Arg::List(l) => ses.parse::<usize>().is_ok_and(|i| i < l.len() && i.to_string() == ses),
                _ => false,
            };
            if status.truthy() && has {
                return Ok(Some(s));
            }
            if here.is_none() {
                let Ok(info) = self.get(c, &s.base, &format!("/session/{}", encode_uri_component(&ses)), 1500.0) else { continue };
                if known.is_none() {
                    known = Some(s.clone());
                }
                let dir = info.field("directory");
                if dir.truthy() && s.dir.truthy() && same_path(&dir, &s.dir, windows) {
                    here = Some(s);
                }
            }
        }
        Ok(here.or(known))
    }

    /// What OpenCode says of a session: its title, its folder, when it last
    /// changed, and whether a turn is under way; null when no running
    /// OpenCode knows it. Each answer is kept for the cache's while.
    pub fn info(&self, c: &mut dyn Calls, id: &Arg, windows: bool) -> Result<Arg> {
        let ses = open_code_id(id);
        let hit = self.inner.lock().unwrap().infos.iter().find(|(k, _, _)| *k == ses).map(|(_, at, v)| (*at, v.clone()));
        if let Some((at, v)) = hit {
            if self.now(c)? - at < self.cache_ms {
                return Ok(v);
            }
        }
        let mut value = Arg::Null;
        if let Ok(Some(s)) = self.server_for(c, id, windows) {
            let path = format!("/session/{}", encode_uri_component(&ses));
            let both = c.call_all(vec![
                ("", "get", vec![Arg::text(format!("{}{path}", s.base)), Arg::num(1500.0)]),
                ("", "get", vec![Arg::text(format!("{}/session/status", s.base)), Arg::num(1500.0)]),
            ]);
            let mut got = Vec::new();
            for (r, route) in both.into_iter().zip([path.as_str(), "/session/status"]) {
                match r {
                    Ok(r) if r.field("ok").truthy() => got.push(Some(r.field("json"))),
                    Ok(r) => {
                        let _ = route;
                        let _ = r;
                        got.push(None);
                    }
                    Err(_) => got.push(None),
                }
            }
            if let [Some(session), Some(status)] = &got[..] {
                let st = status.field(&ses).field("type");
                let title = session.field("title");
                let updated = xenonet_base::text::js_number(Some(&session.field("time").field("updated").to_json()));
                let updated = match session.field("time").field("updated") {
                    Arg::Undef => Arg::Null,
                    _ if updated == 0.0 || updated.is_nan() => Arg::Null,
                    _ => Arg::num(updated),
                };
                value = Arg::Obj(vec![
                    ("title".into(), if matches!(title, Arg::Text(_)) { title } else { Arg::Null }),
                    ("directory".into(), session.field("directory").or(Arg::Null)),
                    ("updated".into(), updated),
                    ("busy".into(), Arg::Bool(st.truthy() && st.as_str() != Some("idle"))),
                ]);
            }
        }
        let at = self.now(c)?;
        let mut i = self.inner.lock().unwrap();
        i.infos.retain(|(k, _, _)| *k != ses);
        i.infos.push((ses, at, value.clone()));
        if i.infos.len() > 256 {
            i.infos.remove(0);
        }
        Ok(value)
    }

    pub fn forget(&self) {
        let mut i = self.inner.lock().unwrap();
        i.cache_at = f64::NEG_INFINITY;
        i.servers.clear();
        i.infos.clear();
    }

    pub fn deliver(&self, c: &mut dyn Calls, id: &Arg, text: &Arg, windows: bool) -> Result<Arg> {
        let Some(s) = self.server_for(c, id, windows)? else {
            return Ok(Arg::Obj(vec![("ok".into(), Arg::Bool(false)), ("why".into(), Arg::text("no running OpenCode has this session"))]));
        };
        let url = format!("{}/session/{}/prompt_async", s.base, encode_uri_component(&open_code_id(id)));
        let mut part = vec![("type".to_string(), Arg::text("text"))];
        if !matches!(text, Arg::Undef) {
            part.push(("text".into(), text.clone()));
        }
        let body = Arg::Obj(vec![("parts".into(), Arg::List(vec![Arg::Obj(part)]))]);
        let body = xenonet_base::json::stringify(&body.to_json(), 0);
        let r = c.call("", "post", vec![Arg::text(url), Arg::text(body)])?;
        let status = r.field("status");
        if status == Arg::num(204.0) || r.field("ok").truthy() {
            return Ok(Arg::Obj(vec![("ok".into(), Arg::Bool(true)), ("where".into(), Arg::text(s.base))]));
        }
        Ok(Arg::Obj(vec![("ok".into(), Arg::Bool(false)), ("why".into(), Arg::text(format!("OpenCode answered {}", js_text(&status))))]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_go_both_ways() {
        assert_eq!(open_code_id(&Arg::text("opencode:ses-abc123")), "ses_abc123");
        assert_eq!(from_open_code_id(&Arg::text(" ses_abc123 ")), "opencode:ses-abc123");
        assert_eq!(from_open_code_id(&Arg::text("claude:x")), "claude:x");
    }

    #[test]
    fn processes_and_ports_read_as_the_js_read_them() {
        let ps = parse_ps("  10 opencode --port 4096\n 20 /bin/zsh -c opencode\nx\n");
        assert_eq!(ps, vec![Proc { pid: 10.0, args: "opencode --port 4096".into() }, Proc { pid: 20.0, args: "/bin/zsh -c opencode".into() }]);
        let ports: Vec<f64> = regex_lsof().captures_iter("p10\nn127.0.0.1:4096\r\nn*:5000\n").map(|m| number(&m[1])).collect();
        assert_eq!(ports, vec![4096.0, 5000.0]);
        assert_eq!(parse_hex("1F90"), 8080.0);
        assert!(same_path(&Arg::text("C:\\X\\"), &Arg::text("c:/x"), true));
        assert_eq!(encode_uri_component("ses_a b/é"), "ses_a%20b%2F%C3%A9");
    }
}
