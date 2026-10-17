//! core/spotify.js: Spotify as a remote for the Spotify app you already
//! use: what's playing, play and pause, the next track, and your volume
//! dipped while something waits on you. XenoNet plays no audio of
//! Spotify's itself.
//!
//! You connect with an app of your own on Spotify's developer dashboard
//! (its client id isn't a secret), through Authorization Code with PKCE:
//! Spotify's sign-in comes back to this XenoNet once, within ten minutes,
//! with the state it was sent with. The refresh token goes to the
//! credential store; the access token stays in this service's memory, is
//! sent only in a header, and never reaches the page. What reaches Spotify:
//! sign-in and token refreshes, reads of what's playing, and the commands
//! you give or turn on. Nothing of your sessions.
//!
//! Each call waits on the credential store and on Spotify, so it's a job:
//! `Spotify::start` and `Spotify::resume` say what to ask next (a secret,
//! a request), and whoever runs it does the asking.

use crate::fsx;
use crate::jstext::{encode_uri_component, search_params, slice16};
use crate::jsv::{get, raw, strict_eq, truthy};
use crate::nodepath::NATIVE;
use regex::Regex;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use xenonet_base::text::{js_number, js_string, js_trim};
use xenonet_base::units::js_round;

pub const SCOPES: [&str; 3] = ["user-read-playback-state", "user-modify-playback-state", "user-read-currently-playing"];
pub const REFRESH_TARGET: &str = "XenoNet/Spotify/refresh";
pub const REFRESH_COMMENT: &str = "Spotify refresh token, kept by XenoNet";
const ACCOUNTS: &str = "https://accounts.spotify.com";
const API: &str = "https://api.spotify.com/v1";
const PENDING_MS: f64 = 10.0 * 60e3;
const EARLY_MS: f64 = 90e3;
pub const CLIENT_ID: &str = "(?i)^[0-9a-f]{32}$";
pub const REDIRECT_SHOWN: &str = "http://127.0.0.1/api/spotify/callback";

fn client_id_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(CLIENT_ID).unwrap())
}

fn spotify_link() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^https://open\.spotify\.com/").unwrap())
}

/// base64url without padding.
pub fn b64url(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        let k = c.len() + 1;
        for i in 0..k {
            out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// PKCE's S256 challenge for a verifier.
pub fn challenge_of(verifier: &str) -> String {
    b64url(&Sha256::digest(verifier.as_bytes()))
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpotifyError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

fn err(status: u16, code: &'static str, message: impl Into<String>) -> SpotifyError {
    SpotifyError { status, code, message: message.into() }
}

fn device_of(d: Option<&Value>) -> Value {
    if !truthy(d) {
        return Value::Null;
    }
    let mut m = Map::new();
    if let Some(id) = raw(d, "id") {
        m.insert("id".into(), id.clone());
    }
    if let Some(name) = raw(d, "name") {
        m.insert("name".into(), name.clone());
    }
    m.insert("volume".into(), get(d, "volume_percent").cloned().unwrap_or(Value::Null));
    Value::Object(m)
}

/// What's playing, as the page shows it: never more than this.
pub fn now_playing(p: Option<&Value>) -> Value {
    let item = raw(p, "item");
    if !truthy(p) || !truthy(item) {
        return json!({ "playing": false, "title": null, "artists": null, "url": null, "device": device_of(raw(p, "device")) });
    }
    // A track's artists, or a podcast episode's show.
    let names: Vec<Value> = match raw(item, "artists") {
        Some(Value::Array(a)) if !a.is_empty() => a.iter().map(|x| raw(Some(x), "name").cloned().unwrap_or(Value::Null)).collect(),
        _ => vec![raw(raw(item, "show"), "name").cloned().unwrap_or(Value::Null)],
    };
    let artists = names.iter().filter(|n| truthy(Some(n))).map(js_string).collect::<Vec<_>>().join(", ");
    let artists = slice16(&artists, 200);
    let link = raw(raw(item, "external_urls"), "spotify").and_then(Value::as_str).filter(|u| spotify_link().is_match(u));
    json!({
        "playing": truthy(raw(p, "is_playing")),
        "title": slice16(&get(item, "name").map(js_string).unwrap_or_default(), 200),
        "artists": if artists.is_empty() { Value::Null } else { Value::String(artists) },
        "url": link,
        "device": device_of(raw(p, "device")),
    })
}

/// What the dip while something waits on you does next.
#[derive(Clone, Debug, PartialEq)]
pub struct Duck {
    pub volume: Option<f64>,
    /// The dip under way after this: { from, to }, or null, or (when it
    /// was given and kept) what was given.
    pub held: Option<Value>,
}

/// While something waits on you, the volume dips, on this computer's
/// device only, and comes back after, unless you changed it meanwhile.
/// From what's waiting and the player, what to do; `held` is the dip under
/// way ({ from, to }), or null.
pub fn duck_plan(waiting: bool, player: Option<&Value>, device: Option<&Value>, held: Option<&Value>, dip: f64) -> Duck {
    let pd = raw(player, "device");
    let here = truthy(pd) && truthy(device) && strict_eq(raw(pd, "id"), device);
    let keep = |h: Option<&Value>| Duck { volume: None, held: h.cloned() };
    let volume = raw(pd, "volume");
    // Not playing here: nothing to dip, and a dip under way is let go when
    // nothing waits any more.
    if !here || !truthy(raw(player, "playing")) || volume.is_none_or(Value::is_null) {
        return if waiting { keep(held) } else { keep(Some(&Value::Null)) };
    }
    let v = js_number(volume);
    let held_on = truthy(held);
    if waiting && !held_on {
        let to = js_round(v * dip).max(5.0);
        return if to < v { Duck { volume: Some(to), held: Some(json!({ "from": volume, "to": xenonet_base::json::number(to) })) } } else { keep(held) };
    }
    let to = raw(held, "to");
    if !waiting && held_on {
        return if strict_eq(volume, to) { Duck { volume: Some(js_number(raw(held, "from"))), held: Some(Value::Null) } } else { keep(Some(&Value::Null)) };
    }
    // You changed it while it was dipped: yours stands.
    if waiting && held_on && !strict_eq(volume, to) {
        return keep(Some(&Value::Null));
    }
    keep(held)
}

/// spotify.json: your app's client id, which device is this computer, and
/// whether the dip is on.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub client_id: Option<String>,
    pub device: Option<String>,
    pub duck: bool,
}

impl Settings {
    pub fn to_value(&self) -> Value {
        json!({ "clientId": self.client_id, "device": self.device, "duck": self.duck })
    }
}

fn read_settings(file: &str) -> Settings {
    let none = Settings { client_id: None, device: None, duck: false };
    let Ok(text) = std::fs::read_to_string(file) else { return none };
    match xenonet_base::json::parse(&text) {
        Ok(s @ Value::Object(_)) => {
            let s = Some(&s);
            let id = get(s, "clientId").map_or_else(String::new, js_string);
            Settings { client_id: client_id_re().is_match(&id).then(|| raw(s, "clientId").map(js_string).unwrap_or_default()), device: raw(s, "device").and_then(Value::as_str).map(String::from), duck: raw(s, "duck") == Some(&Value::Bool(true)) }
        }
        Ok(Value::Null) | Err(_) => none,
        Ok(_) => Settings { client_id: None, device: None, duck: false },
    }
}

struct Pending {
    state: String,
    verifier: String,
    at: f64,
}

struct Access {
    token: Value,
    until: f64,
}

/// One Spotify connection: its settings, the sign-in under way, the access
/// token, and when Spotify said to come back.
pub struct Spotify {
    file: String,
    port: Value,
    settings: Settings,
    pending: Option<Pending>,
    access: Option<Access>,
    retry_at: f64,
}

/// What a job asks for next.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    SecretGet(&'static str),
    SecretHas(&'static str),
    SecretSet { target: &'static str, value: String, comment: &'static str },
    SecretRemove(&'static str),
    /// A request; its answer's body is read as JSON (or `fallback` when it
    /// isn't), except for a status in `skip`.
    Fetch { url: String, method: &'static str, headers: Vec<(&'static str, String)>, body: Option<String>, timeout_ms: f64, fallback: Value, skip: Vec<u16> },
    /// A line for XenoNet's log, then on.
    Log { level: &'static str, code: &'static str, message: &'static str },
    Done(Value),
    Fail(SpotifyError),
    /// Throws what the last thing asked threw.
    Rethrow,
}

/// Why a call that doesn't wait failed.
pub enum Failed {
    Spotify(SpotifyError),
    Fs(fsx::FsError),
}

/// What came back.
pub enum Got {
    Value(Value),
    Threw(String),
    Response { ok: bool, status: u16, body: Value, retry_after: Option<String> },
}

/// A job under way, kept as JSON by whoever runs it.
pub type Job = Value;

impl Spotify {
    pub fn new(file: &str, port: Value) -> Self {
        Spotify { file: file.into(), settings: read_settings(file), port, pending: None, access: None, retry_at: 0.0 }
    }

    fn redirect(&self) -> String {
        format!("http://127.0.0.1:{}/api/spotify/callback", js_string(&self.port))
    }

    pub fn settings(&self) -> Value {
        self.settings.to_value()
    }

    fn save(&self) -> Result<(), fsx::FsError> {
        fsx::mkdir_p(&NATIVE.dirname(&self.file), None)?;
        fsx::write_file(&self.file, format!("{}\n", xenonet_base::json::stringify(&self.settings.to_value(), 0)).as_bytes(), Some(0o600))
    }

    /// Your app's client id, from its page on Spotify's dashboard.
    pub fn set_app(&mut self, client_id: &str) -> Result<Value, Failed> {
        let id = js_trim(client_id);
        if !client_id_re().is_match(id) {
            return Err(Failed::Spotify(err(400, "bad-request", "a Spotify app's client id is 32 letters and digits, from its page on Spotify's developer dashboard")));
        }
        self.settings.client_id = Some(id.into());
        self.save().map_err(Failed::Fs)?;
        Ok(json!({ "app": true }))
    }

    /// Which device is this computer, and whether the dip is on; each only
    /// when given.
    pub fn set(&mut self, device: Option<&Value>, duck: Option<&Value>) -> Result<Value, fsx::FsError> {
        if let Some(d) = device {
            self.settings.device = d.as_str().filter(|s| !s.is_empty()).map(|s| slice16(s, 120));
        }
        if let Some(d) = duck {
            self.settings.duck = d == &Value::Bool(true);
        }
        self.save()?;
        Ok(json!({ "device": self.settings.device, "duck": self.settings.duck }))
    }

    /// Where to send you to sign in: Spotify's page, with a challenge and a
    /// state the callback must bring back.
    pub fn begin(&mut self, now: f64) -> Result<Value, SpotifyError> {
        let Some(client_id) = self.settings.client_id.clone() else { return Err(err(409, "no-app", "give XenoNet your Spotify app's client id first")) };
        let random = |n: usize| {
            let mut b = vec![0u8; n];
            getrandom::fill(&mut b).expect("the system's random numbers");
            b64url(&b)
        };
        let verifier = random(48);
        let state = random(16);
        let challenge = challenge_of(&verifier);
        let redirect = self.redirect();
        let scope = SCOPES.join(" ");
        let q = search_params(&[("client_id", &client_id), ("response_type", "code"), ("redirect_uri", &redirect), ("code_challenge_method", "S256"), ("code_challenge", &challenge), ("state", &state), ("scope", &scope)]);
        self.pending = Some(Pending { state, verifier, at: now });
        Ok(json!({ "url": format!("{ACCOUNTS}/authorize?{q}") }))
    }

    fn token_fetch(form: &[(&str, &str)]) -> Ask {
        Ask::Fetch {
            url: format!("{ACCOUNTS}/api/token"),
            method: "POST",
            headers: vec![("content-type", "application/x-www-form-urlencoded".into())],
            body: Some(search_params(form)),
            timeout_ms: 15000.0,
            fallback: json!({}),
            skip: Vec::new(),
        }
    }

    // A token request's answer: its body, or what Spotify refused.
    fn token_answer(got: Got) -> Result<Value, SpotifyError> {
        match got {
            Got::Threw(m) => Err(err(502, "unreachable", format!("couldn't reach Spotify: {m}"))),
            Got::Value(v) => Ok(v),
            Got::Response { ok, status, body, .. } => {
                if ok {
                    return Ok(body);
                }
                let b = Some(&body);
                let why = get(b, "error_description").or_else(|| get(b, "error")).map_or_else(|| status.to_string(), js_string);
                let code = if raw(b, "error").and_then(Value::as_str) == Some("invalid_grant") { "reconnect" } else { "refused" };
                Err(err(if status == 400 || status == 401 { 409 } else { 502 }, code, format!("Spotify said: {}", slice16(&why, 160))))
            }
        }
    }

    // Keeps a token answer: the access token here, and a refresh token, if
    // one came, in the credential store.
    fn keep(&mut self, body: &Value, now: f64) -> Option<Ask> {
        let b = Some(body);
        let expires = get(b, "expires_in").map_or(3600.0, |e| js_number(Some(e)));
        self.access = Some(Access { token: raw(b, "access_token").cloned().unwrap_or(Value::Null), until: now + expires * 1000.0 });
        // A refresh may come back without a new refresh token: the one
        // there is stays.
        raw(b, "refresh_token").filter(|r| truthy(Some(r))).map(|r| Ask::SecretSet { target: REFRESH_TARGET, value: js_string(r), comment: REFRESH_COMMENT })
    }

    /// Starts a job: "status", "callback" (with { code, state, error }),
    /// "disconnect", "player", or "command" (with { op, value }).
    pub fn start(&mut self, kind: &str, args: &Value, now: f64) -> (Job, Ask) {
        let mut job = Map::new();
        job.insert("kind".into(), json!(kind));
        match kind {
            "status" => {
                if self.settings.client_id.is_some() {
                    job.insert("stage".into(), json!("has"));
                    (Value::Object(job), Ask::SecretHas(REFRESH_TARGET))
                } else {
                    let s = self.status(json!(false));
                    (Value::Object(job), Ask::Done(s))
                }
            }
            "callback" => {
                let p = self.pending.take();
                let a = Some(args);
                let state = raw(a, "state");
                let Some(p) = p.filter(|p| truthy(state) && state.and_then(Value::as_str) == Some(p.state.as_str())) else {
                    return (Value::Object(job), Ask::Fail(err(400, "bad-state", "this sign-in isn't the one XenoNet started; start again from XenoNet")));
                };
                if now - p.at > PENDING_MS {
                    return (Value::Object(job), Ask::Fail(err(400, "expired", "this sign-in took too long; start again from XenoNet")));
                }
                if let Some(e) = raw(a, "error").filter(|e| truthy(Some(e))) {
                    return (Value::Object(job), Ask::Fail(err(400, "declined", format!("Spotify said: {}", slice16(&js_string(e), 80)))));
                }
                let Some(code) = raw(a, "code").filter(|c| truthy(Some(c))).map(js_string) else {
                    return (Value::Object(job), Ask::Fail(err(400, "bad-request", "Spotify sent no code")));
                };
                job.insert("stage".into(), json!("exchanged"));
                let redirect = self.redirect();
                let client = self.settings.client_id.clone().map_or_else(|| "null".into(), |c| c);
                (Value::Object(job), Self::token_fetch(&[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &redirect), ("client_id", &client), ("code_verifier", &p.verifier)]))
            }
            "disconnect" => {
                job.insert("stage".into(), json!("removed"));
                (Value::Object(job), Ask::SecretRemove(REFRESH_TARGET))
            }
            "player" | "command" => {
                let route = if kind == "player" {
                    Ok(("GET", "/me/player".to_string()))
                } else {
                    self.command_route(raw(Some(args), "op"), raw(Some(args), "value"))
                };
                match route {
                    Err(e) => (Value::Object(job), Ask::Fail(e)),
                    Ok((method, route)) => {
                        job.insert("method".into(), json!(method));
                        job.insert("route".into(), json!(route));
                        job.insert("retry".into(), json!(true));
                        self.call_start(job, now)
                    }
                }
            }
            _ => (Value::Object(job), Ask::Fail(err(400, "bad-request", format!("no Spotify job {kind}")))),
        }
    }

    fn connected(mut job: Map<String, Value>) -> (Job, Ask) {
        job.insert("stage".into(), json!("logged"));
        (Value::Object(job), Ask::Log { level: "info", code: "spotify.connected", message: "Spotify is connected" })
    }

    fn status(&self, connected: Value) -> Value {
        json!({ "app": self.settings.client_id.is_some(), "connected": connected, "device": self.settings.device, "duck": self.settings.duck, "redirect": REDIRECT_SHOWN })
    }

    // play, pause, next, previous, or volume (0 to 100), to this
    // computer's device once it's chosen.
    fn command_route(&self, op: Option<&Value>, value: Option<&Value>) -> Result<(&'static str, String), SpotifyError> {
        let device = self.settings.device.as_ref().map_or(String::new(), |d| format!("?device_id={}", encode_uri_component(d)));
        match op.and_then(Value::as_str) {
            Some("play") => Ok(("PUT", format!("/me/player/play{device}"))),
            Some("pause") => Ok(("PUT", format!("/me/player/pause{device}"))),
            Some("next") => Ok(("POST", format!("/me/player/next{device}"))),
            Some("previous") => Ok(("POST", format!("/me/player/previous{device}"))),
            Some("volume") => {
                let v = js_round(js_number(Some(value.unwrap_or(&Value::Null))));
                if !(v >= 0.0 && v <= 100.0) {
                    return Err(err(400, "bad-request", "volume is 0 to 100"));
                }
                let and = if device.is_empty() { String::new() } else { format!("&{}", &device[1..]) };
                Ok(("PUT", format!("/me/player/volume?volume_percent={}{and}", xenonet_base::json::number_string(v + 0.0))))
            }
            _ => Err(err(400, "bad-request", "play, pause, next, previous or volume")),
        }
    }

    // A call to Spotify's Web API: the token in its header, a 429's wait
    // kept to before the next call, one 401 refreshed and tried again.
    fn call_start(&mut self, mut job: Map<String, Value>, now: f64) -> (Job, Ask) {
        if now < self.retry_at {
            return (Value::Object(job), Ask::Fail(err(429, "busy", "Spotify asked XenoNet to wait a moment")));
        }
        // An access token, refreshed a little before it runs out.
        if let Some(a) = &self.access {
            if now < a.until - EARLY_MS {
                let token = a.token.clone();
                return self.api_fetch(job, &token);
            }
        }
        if self.settings.client_id.is_none() {
            return (Value::Object(job), Ask::Fail(err(409, "no-app", "XenoNet has no Spotify app to connect with yet")));
        }
        job.insert("stage".into(), json!("refresh"));
        (Value::Object(job), Ask::SecretGet(REFRESH_TARGET))
    }

    fn api_fetch(&mut self, mut job: Map<String, Value>, token: &Value) -> (Job, Ask) {
        let retry = job.get("retry") == Some(&json!(true));
        let method = match job.get("method").and_then(Value::as_str) {
            Some("PUT") => "PUT",
            Some("POST") => "POST",
            _ => "GET",
        };
        let route = job.get("route").and_then(Value::as_str).unwrap_or("").to_string();
        job.insert("stage".into(), json!("api"));
        let mut skip = vec![204, 429];
        if retry {
            skip.push(401);
        }
        (Value::Object(job), Ask::Fetch { url: format!("{API}{route}"), method, headers: vec![("authorization", format!("Bearer {}", js_string(token)))], body: None, timeout_ms: 10000.0, fallback: Value::Null, skip })
    }

    /// Takes what was asked for and says what's next.
    pub fn resume(&mut self, job: Job, got: Got, now: f64) -> (Job, Ask) {
        let mut job = match job {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        let stage = job.get("stage").and_then(Value::as_str).unwrap_or("").to_string();
        let kind = job.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
        let done = |job: Map<String, Value>, v: Value| (Value::Object(job), Ask::Done(v));
        let fail = |job: Map<String, Value>, e: SpotifyError| (Value::Object(job), Ask::Fail(e));
        match stage.as_str() {
            "has" => {
                // secrets.has(…).catch(() => false)
                let connected = match got {
                    Got::Value(v) => v,
                    _ => json!(false),
                };
                let s = self.status(connected);
                done(job, s)
            }
            "exchanged" => match Self::token_answer(got) {
                Err(e) => fail(job, e),
                Ok(body) => match self.keep(&body, now) {
                    Some(ask) => {
                        job.insert("stage".into(), json!("kept-callback"));
                        (Value::Object(job), ask)
                    }
                    None => Self::connected(job),
                },
            },
            "kept-callback" => match got {
                Got::Threw(_) => (Value::Object(job), Ask::Rethrow),
                _ => Self::connected(job),
            },
            "logged" => done(job, json!({ "connected": true })),
            "removed" => {
                if kind == "disconnect" {
                    self.access = None;
                    return done(job, json!({ "connected": false }));
                }
                // A refresh token Spotify won't take any more is forgotten.
                self.access = None;
                let status = job.get("status").and_then(Value::as_u64).unwrap_or(409) as u16;
                let message = job.get("error").and_then(Value::as_str).unwrap_or("").to_string();
                fail(job, err(status, "reconnect", message))
            }
            "refresh" => {
                // secrets.get(…).catch(() => null)
                let refresh = match got {
                    Got::Value(v) if truthy(Some(&v)) => js_string(&v),
                    _ => return fail(job, err(409, "not-connected", "Spotify isn't connected")),
                };
                job.insert("stage".into(), json!("refreshed"));
                let client = self.settings.client_id.clone().unwrap_or_default();
                (Value::Object(job), Self::token_fetch(&[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &client)]))
            }
            "refreshed" => match Self::token_answer(got) {
                Err(e) if e.code == "reconnect" => {
                    job.insert("stage".into(), json!("removed"));
                    job.insert("status".into(), json!(e.status));
                    job.insert("error".into(), json!(e.message));
                    (Value::Object(job), Ask::SecretRemove(REFRESH_TARGET))
                }
                Err(e) => fail(job, e),
                Ok(body) => match self.keep(&body, now) {
                    Some(ask) => {
                        job.insert("stage".into(), json!("kept"));
                        (Value::Object(job), ask)
                    }
                    None => {
                        let token = self.access.as_ref().map_or(Value::Null, |a| a.token.clone());
                        self.api_fetch(job, &token)
                    }
                },
            },
            "kept" => match got {
                Got::Threw(_) => (Value::Object(job), Ask::Rethrow),
                _ => {
                    let token = self.access.as_ref().map_or(Value::Null, |a| a.token.clone());
                    self.api_fetch(job, &token)
                }
            },
            "api" => {
                let (ok, status, body, retry_after) = match got {
                    Got::Threw(m) => return fail(job, err(502, "unreachable", format!("couldn't reach Spotify: {m}"))),
                    Got::Value(v) => (true, 200, v, None),
                    Got::Response { ok, status, body, retry_after } => (ok, status, body, retry_after),
                };
                let retry = job.get("retry") == Some(&json!(true));
                if status == 401 && retry {
                    self.access = None;
                    job.insert("retry".into(), json!(false));
                    return self.call_start(job, now);
                }
                if status == 429 {
                    let after = js_number(Some(&retry_after.map_or(Value::Null, Value::String)));
                    let after = if after == 0.0 || after.is_nan() { 5.0 } else { after };
                    self.retry_at = now + after.min(30.0) * 1000.0;
                    return fail(job, err(429, "busy", "Spotify asked XenoNet to wait a moment"));
                }
                let player = kind == "player";
                if status == 204 {
                    return done(job, if player { now_playing(None) } else { Value::Null });
                }
                if !ok {
                    if status == 403 {
                        return fail(job, err(409, "premium", "Spotify's player needs Premium"));
                    }
                    let why = get(get(Some(&body), "error"), "message").map_or_else(|| status.to_string(), js_string);
                    return fail(job, err(502, "refused", format!("Spotify said: {}", slice16(&why, 160))));
                }
                if player {
                    return done(job, now_playing(Some(&body)));
                }
                done(job, body)
            }
            _ => fail(job, err(400, "bad-request", "that Spotify job is over")),
        }
    }
}

/// The demo's made-up player: it plays and pauses so the page can be seen
/// and tested, and nothing reaches Spotify.
pub fn demo_player() -> Value {
    json!({ "playing": false, "title": "Night Drive", "artists": "The Demo Ensemble", "url": null, "device": { "id": "demo-device", "name": "This computer", "volume": 60 } })
}

pub fn demo_settings() -> Value {
    json!({ "clientId": "0".repeat(32), "device": "demo-device", "duck": true })
}

/// A command to the demo's player.
pub fn demo_command(p: &mut Value, op: Option<&str>, value: Option<&Value>) {
    match op {
        Some("play") => p["playing"] = json!(true),
        Some("pause") => p["playing"] = json!(false),
        Some("next") => {
            let next = if p["title"] == "Night Drive" { "Low Orbit" } else { "Night Drive" };
            p["title"] = json!(next);
        }
        Some("volume") => p["device"]["volume"] = xenonet_base::json::number(js_round(js_number(value))),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_as_spotify_checks_it() {
        // As Node's crypto makes it.
        assert_eq!(challenge_of("dBjftJeZ4CVP-mJ92K9qXkMSKOcmXzo0PPYfyHaOv3Y"), "AfspCjDGPb4YlKconZDqNIcar_ul8HSQx_KSqjKiMk0");
        assert_eq!(b64url(b"\xfb\xff"), "-_8");
    }

    #[test]
    fn the_dip_is_given_back_unless_you_changed_it() {
        let player = |v: i64| json!({ "playing": true, "device": { "id": "here", "name": "PC", "volume": v } });
        let d = duck_plan(true, Some(&player(70)), Some(&json!("here")), None, 0.35);
        assert_eq!(d, Duck { volume: Some(25.0), held: Some(json!({ "from": 70, "to": 25 })) });
        let back = duck_plan(false, Some(&player(25)), Some(&json!("here")), d.held.as_ref(), 0.35);
        assert_eq!(back, Duck { volume: Some(70.0), held: Some(Value::Null) });
        let changed = duck_plan(false, Some(&player(40)), Some(&json!("here")), Some(&json!({ "from": 70, "to": 25 })), 0.35);
        assert_eq!(changed.volume, None);
        assert_eq!(duck_plan(true, Some(&player(70)), Some(&json!("elsewhere")), None, 0.35).volume, None);
    }

    #[test]
    fn whats_playing_and_nothing_more() {
        assert_eq!(now_playing(Some(&json!({ "is_playing": true, "item": { "name": "Ep 1", "show": { "name": "The Show" } } })))["artists"], "The Show");
        assert_eq!(now_playing(Some(&json!({ "item": { "name": "x", "artists": [], "external_urls": { "spotify": "javascript:alert(1)" } } })))["url"], Value::Null);
        assert_eq!(now_playing(None)["playing"], false);
    }
}
