//! A client for a running XenoNet's HTTP API (core/client.js), for the
//! xenonet command and the MCP servers. XenoNet listens on 127.0.0.1: 47840
//! for XenoNet, 47842 for XenoNet Dev and 47841 for its demo. A port, --dev
//! or --demo picks one; otherwise XENONET_PORT, or else the first of
//! XenoNet and XenoNet Dev that answers.

use crate::http;
use crate::json::{self, number_string, Value};
use crate::text::{js_number, js_string};
use std::time::Duration;

pub const XENONET_PORT: u16 = 47840;
pub const DEMO_PORT: u16 = 47841;
pub const DEV_PORT: u16 = 47842;

/// How long a XenoNet has to say it's there. A port nobody listens on
/// answers at once either way, so these are for a busy machine, where
/// 600 ms made the xenonet command (and its tests) say XenoNet wasn't
/// running when it was. Looking for whichever XenoNet runs, each port gets
/// a quick probe. A port that's named (--port, --dev, --demo, or
/// XENONET_PORT, as the tests and the MCP servers name it) is the XenoNet
/// meant, so it gets longer.
pub const HEALTH_MS: u64 = 2000;
pub const NAMED_HEALTH_MS: u64 = 10000;

/// Which XenoNet: a port named (as JS's Number() read it), --demo, --dev,
/// and XENONET_PORT when it's set and not empty.
#[derive(Default, Clone, Debug)]
pub struct Pick {
    pub port: Option<f64>,
    pub demo: bool,
    pub dev: bool,
    pub env_port: Option<String>,
}

/// The ports to try, in order. A port that isn't one is kept, as JS keeps
/// it, and never answers.
pub fn candidates(p: &Pick) -> Vec<f64> {
    if let Some(port) = p.port {
        return vec![port];
    }
    if p.demo {
        return vec![DEMO_PORT.into()];
    }
    if p.dev {
        return vec![DEV_PORT.into()];
    }
    if let Some(e) = &p.env_port {
        return vec![js_number(Some(&Value::String(e.clone())))];
    }
    vec![XENONET_PORT.into(), DEV_PORT.into()]
}

pub fn health_ms(p: &Pick) -> u64 {
    if p.port.is_some() || p.demo || p.dev || p.env_port.is_some() {
        NAMED_HEALTH_MS
    } else {
        HEALTH_MS
    }
}

/// A port fetch would make a URL of.
fn real_port(n: f64) -> Option<u16> {
    (n.fract() == 0.0 && (0.0..=65535.0).contains(&n)).then_some(n as u16)
}

/// A XenoNet that answered, and what it said of itself.
#[derive(Clone, Debug)]
pub struct Server {
    pub port: u16,
    pub health: Value,
}

impl Server {
    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

// An answer's body as res.json() reads it: UTF-8, a byte order mark left
// out, and JSON.
fn json_of(body: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(body);
    json::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)).ok()
}

/// The first of these ports that answers (in their order, not in the order
/// they answer), each given `answer` to.
pub fn find(ports: &[f64], answer: Duration) -> Option<Server> {
    let probes: Vec<_> = ports
        .iter()
        .map(|&n| {
            std::thread::spawn(move || {
                let port = real_port(n)?;
                let a = http::request(port, "GET", "/api/health", &[], None, Some(answer)).ok()?;
                let health = json_of(a.body.as_ref().ok()?)?;
                a.ok().then_some(Server { port, health })
            })
        })
        .collect();
    probes.into_iter().map(|p| p.join().ok().flatten()).collect::<Vec<_>>().into_iter().flatten().next()
}

/// What `need` says when no XenoNet answers.
pub fn not_running(ports: &[f64]) -> String {
    let at: Vec<String> = ports.iter().map(|p| format!("127.0.0.1:{}", number_string(*p))).collect();
    format!("XenoNet isn't running (nothing answers on {}). Open XenoNet, or start XenoNet Dev from its checkout with npm run app.", at.join(" or "))
}

/// The XenoNet picked, or why there's none.
pub fn need(p: &Pick) -> Result<Server, String> {
    let ports = candidates(p);
    find(&ports, Duration::from_millis(health_ms(p))).ok_or_else(|| not_running(&ports))
}

/// What a XenoNet on this port is called when it doesn't say.
pub fn default_name(port: f64) -> &'static str {
    if port == f64::from(DEV_PORT) {
        "XenoNet Dev"
    } else if port == f64::from(DEMO_PORT) {
        "XenoNet (demo)"
    } else {
        "XenoNet"
    }
}

fn health_name(health: &Value) -> Option<String> {
    health.get("name").filter(|n| !n.is_null()).map(|n| n.as_str().map_or_else(|| js_string(n), str::to_string))
}

/// What a XenoNet calls itself.
pub fn name_of(s: &Server) -> String {
    health_name(&s.health).unwrap_or_else(|| default_name(s.port.into()).to_string())
}

/// An error answer: XenoNet's message, and the status when there was one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub message: String,
    pub status: Option<u16>,
}

/// A call to the API, to XenoNet `name` on `port`: its answer, or an error
/// with XenoNet's message. Headers given replace the ones it sends by
/// itself. A call has no deadline, as in JS.
pub fn request(port: f64, name: &str, method: &str, route: &str, body: Option<&Value>, headers: &[(String, String)]) -> Result<Value, Refused> {
    let mut all: Vec<(String, String)> = vec![("x-xenonet-api-version".into(), "1".into())];
    if body.is_some() {
        all.push(("content-type".into(), "application/json".into()));
    }
    for (k, v) in headers {
        all.retain(|(a, _)| !a.eq_ignore_ascii_case(k));
        all.push((k.clone(), v.clone()));
    }
    let text = body.map(|b| json::stringify(b, 0));
    let stopped = || Refused { message: format!("{name} stopped answering on 127.0.0.1:{} (restarting, or quit); try again in a moment", number_string(port)), status: None };
    let answer = real_port(port).ok_or_else(stopped).and_then(|p| http::request(p, method, route, &all, text.as_deref().map(str::as_bytes), None).map_err(|_| stopped()))?;
    let data = answer.body.as_ref().ok().and_then(|b| json_of(b)).unwrap_or_else(|| Value::Object(serde_json::Map::default()));
    if answer.ok() {
        return Ok(data);
    }
    if data.is_null() {
        return Err(Refused { message: "Cannot read properties of null (reading 'error')".into(), status: None });
    }
    let message = data
        .get("error")
        .filter(|e| !e.is_null())
        .map_or_else(|| format!("{route}: {} {}", answer.status, answer.reason), |e| e.as_str().map_or_else(|| js_string(e), str::to_string));
    Err(Refused { message, status: Some(answer.status) })
}

impl Server {
    /// `request` to this XenoNet, under the name it gave.
    pub fn request(&self, method: &str, route: &str, body: Option<&Value>, headers: &[(String, String)]) -> Result<Value, Refused> {
        let name = health_name(&self.health).unwrap_or_else(|| "XenoNet".into());
        request(self.port.into(), &name, method, route, body, headers)
    }

    pub fn get(&self, route: &str) -> Result<Value, Refused> {
        self.request("GET", route, None, &[])
    }

    pub fn post(&self, route: &str, body: &Value, headers: &[(String, String)]) -> Result<Value, Refused> {
        self.request("POST", route, Some(body), headers)
    }

    pub fn del(&self, route: &str) -> Result<Value, Refused> {
        self.request("DELETE", route, None, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    #[test]
    fn which_ports() {
        assert_eq!(candidates(&Pick { port: Some(5000.0), ..Default::default() }), [5000.0]);
        assert_eq!(candidates(&Pick { demo: true, ..Default::default() }), [47841.0]);
        assert_eq!(candidates(&Pick { env_port: Some("6000".into()), ..Default::default() }), [6000.0]);
        assert_eq!(candidates(&Pick::default()), [47840.0, 47842.0]);
        assert_eq!((health_ms(&Pick::default()), health_ms(&Pick { dev: true, ..Default::default() })), (HEALTH_MS, NAMED_HEALTH_MS));
        assert_eq!(
            not_running(&[47840.0, 47842.0]),
            "XenoNet isn't running (nothing answers on 127.0.0.1:47840 or 127.0.0.1:47842). Open XenoNet, or start XenoNet Dev from its checkout with npm run app."
        );
    }

    // A stand-in XenoNet that answers each request with what `answer` says
    // of its request line and body, and records them.
    fn stand_in(answer: fn(&str, &str) -> (u16, &'static str, String)) -> (u16, std::sync::mpsc::Receiver<(String, String)>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = s.unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let (mut first, mut line, mut length) = (String::new(), String::new(), 0);
                r.read_line(&mut first).unwrap();
                loop {
                    line.clear();
                    r.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                r.read_exact(&mut body).unwrap();
                let (first, body) = (first.trim().to_string(), String::from_utf8(body).unwrap());
                let (status, reason, text) = answer(&first, &body);
                tx.send((first, body)).unwrap();
                write!(s, "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n\r\n{text}", text.len()).unwrap();
            }
        });
        (port, rx)
    }

    #[test]
    fn calls_and_their_errors() {
        let (port, seen) = stand_in(|first, _| match first.split(' ').nth(1).unwrap() {
            "/api/health" => (200, "OK", r#"{"ok":true,"name":"Stand-in"}"#.into()),
            "/api/broken" => (500, "Internal Server Error", "{}".into()),
            "/api/refused" => (403, "Forbidden", r#"{"error":"an agent can't do that"}"#.into()),
            "/api/not-json" => (200, "OK", "<html>".into()),
            _ => (200, "OK", r#"{"ok":true}"#.into()),
        });
        let s = need(&Pick { port: Some(port.into()), ..Default::default() }).unwrap();
        assert_eq!((s.port, s.base(), name_of(&s)), (port, format!("http://127.0.0.1:{port}"), "Stand-in".to_string()));
        seen.recv().unwrap();
        assert_eq!(s.post("/api/x", &json::parse(r#"{"a":1}"#).unwrap(), &[]).unwrap(), json::parse(r#"{"ok":true}"#).unwrap());
        assert_eq!(seen.recv().unwrap(), ("POST /api/x HTTP/1.1".to_string(), r#"{"a":1}"#.to_string()));
        assert_eq!(s.get("/api/not-json").unwrap(), json::parse("{}").unwrap());
        assert_eq!(s.get("/api/refused"), Err(Refused { message: "an agent can't do that".into(), status: Some(403) }));
        assert_eq!(s.get("/api/broken").unwrap_err().message, "/api/broken: 500 Internal Server Error");
        let gone = Server { port: TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port(), health: Value::Null };
        assert_eq!(gone.del("/api/x").unwrap_err().message, format!("XenoNet stopped answering on 127.0.0.1:{} (restarting, or quit); try again in a moment", gone.port));
        assert!(find(&[gone.port.into(), f64::NAN], Duration::from_millis(300)).is_none());
    }
}
