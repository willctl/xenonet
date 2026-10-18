//! Spotify as a remote for the app you already use (core/spotify.js,
//! crates/xenonet-app/src/spotify.rs). The sign-in, the tokens and each
//! call's decisions are the Rust's, as jobs that ask for a secret, a fetch
//! or a log line; they're answered here. The refresh token goes only to
//! your credential store, the access token only into a request's header.
//! The demo's player is made up, and nothing reaches Spotify from it.

use crate::fx::now_ms;
use crate::fx::Locked;
use crate::secrets::Secrets;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_app::spotify::{self as core, Ask, Failed, Got, SpotifyError};
use xenonet_service::cores::{CoreError, Kind, Spotify, R};
use xenonet_service::wire::Emitter;

fn spotify_error(e: SpotifyError) -> CoreError {
    CoreError::of(Kind::Spotify, e.status, e.code, e.message)
}

pub struct SpotifyRemote {
    core: Mutex<core::Spotify>,
    secrets: Arc<Secrets>,
    emitter: Emitter,
}

impl SpotifyRemote {
    pub fn new(file: &str, port: u16, secrets: Arc<Secrets>, emitter: Emitter) -> Self {
        SpotifyRemote { core: Mutex::new(core::Spotify::new(file, json!(port))), secrets, emitter }
    }

    async fn secret(&self, ask: &Ask) -> Result<Value, String> {
        match ask {
            Ask::SecretGet(t) => self.secrets.get(t).await.map(|s| s.map_or(Value::Null, Value::String)),
            Ask::SecretHas(t) => self.secrets.has(t).await.map(Value::Bool),
            Ask::SecretRemove(t) => self.secrets.remove(t).await.map(Value::Bool),
            Ask::SecretSet { target, value, comment } => self.secrets.set(target, value, comment).await.map(Value::Bool),
            _ => Ok(Value::Null),
        }
    }

    // A job: what the Rust asks for, done here, until it's over.
    async fn run(&self, kind: &str, args: Value) -> R<Value> {
        let (mut job, mut ask) = self.core.locked().start(kind, &args, now_ms());
        let mut thrown = String::new();
        loop {
            let got = match &ask {
                Ask::Done(v) => return Ok(v.clone()),
                Ask::Fail(e) => return Err(spotify_error(e.clone())),
                Ask::Rethrow => return Err(CoreError::new(thrown)),
                Ask::Log { level, code, message } => {
                    self.emitter.log(level, code, message);
                    Got::Value(Value::Null)
                }
                Ask::Fetch { url, method, headers, body, timeout_ms, fallback, skip } => {
                    let headers: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
                    match xenonet_service::net::fetch(method, url, &headers, body.clone().map(String::into_bytes), Duration::from_millis(*timeout_ms as u64)).await {
                        Ok(res) => {
                            let body = if skip.contains(&res.status) { Value::Null } else { xenonet_base::json::parse(&res.text()).unwrap_or_else(|_| fallback.clone()) };
                            Got::Response { ok: res.ok(), status: res.status, body, retry_after: res.header("retry-after").map(str::to_string) }
                        }
                        Err(m) => {
                            thrown = m.clone();
                            Got::Threw(m)
                        }
                    }
                }
                secret => match self.secret(secret).await {
                    Ok(v) => Got::Value(v),
                    Err(m) => {
                        thrown = m.clone();
                        Got::Threw(m)
                    }
                },
            };
            (job, ask) = self.core.locked().resume(job, got, now_ms());
        }
    }
}

#[expect(clippy::result_large_err, reason = "a core's error, as the routes take it")]
fn settle<T>(r: Result<T, Failed>) -> R<T> {
    r.map_err(|f| match f {
        Failed::Spotify(e) => spotify_error(e),
        Failed::Fs(e) => CoreError::new(e.message),
    })
}

#[async_trait]
impl Spotify for SpotifyRemote {
    async fn status(&self) -> R<Value> {
        self.run("status", json!({})).await
    }

    async fn player(&self) -> R<Value> {
        self.run("player", json!({})).await
    }

    async fn command(&self, op: &str, value: &Value) -> R<()> {
        self.run("command", json!({ "op": op, "value": value })).await.map(|_| ())
    }

    async fn callback(&self, code: Option<&str>, state: Option<&str>, error: Option<&str>) -> R<()> {
        self.run("callback", json!({ "code": code, "state": state, "error": error })).await.map(|_| ())
    }

    async fn disconnect(&self) -> R<Value> {
        self.run("disconnect", json!({})).await
    }

    fn set_app(&self, client_id: &Value) -> R<Value> {
        let id = if client_id.is_null() { String::new() } else { xenonet_base::text::js_string(client_id) };
        settle(self.core.locked().set_app(&id))
    }

    fn begin(&self) -> R<Value> {
        self.core.locked().begin(now_ms()).map_err(spotify_error)
    }

    fn set(&self, device: &Value, duck: &Value) -> R<Value> {
        // undefined (left out) leaves each as it is; the routes pass null for it.
        let opt = |v: &Value| (!v.is_null()).then_some(v.clone());
        let (device, duck) = (opt(device), opt(duck));
        self.core.locked().set(device.as_ref(), duck.as_ref()).map_err(|e| CoreError::new(e.message))
    }

    fn duck_plan(&self, waiting: bool, player: &Value, device: &Value, held: &Value) -> R<Value> {
        Ok(plan(waiting, player, device, held))
    }
}

fn plan(waiting: bool, player: &Value, device: &Value, held: &Value) -> Value {
    fn some(v: &Value) -> Option<&Value> {
        (!v.is_null()).then_some(v)
    }
    let d = core::duck_plan(waiting, some(player), some(device), some(held), 0.35);
    let mut out = match d.volume {
        Some(v) => json!({ "op": "volume", "volume": xenonet_base::json::number(v) }),
        None => json!({ "op": null }),
    };
    out["held"] = d.held.unwrap_or(Value::Null);
    out
}

/// The demo's Spotify: a made-up player that plays and pauses, starting
/// paused.
pub struct DemoSpotify {
    player: Mutex<Value>,
    settings: Mutex<Value>,
}

impl DemoSpotify {
    pub fn new() -> Self {
        DemoSpotify { player: Mutex::new(core::demo_player()), settings: Mutex::new(core::demo_settings()) }
    }
}

impl Default for DemoSpotify {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Spotify for DemoSpotify {
    async fn status(&self) -> R<Value> {
        let s = self.settings.locked().clone();
        Ok(json!({ "app": true, "connected": true, "device": s["device"], "duck": s["duck"], "redirect": core::REDIRECT_SHOWN, "demo": true }))
    }

    async fn player(&self) -> R<Value> {
        Ok(self.player.locked().clone())
    }

    async fn command(&self, op: &str, value: &Value) -> R<()> {
        let mut p = self.player.locked();
        core::demo_command(&mut p, Some(op), (!value.is_null()).then_some(value));
        Ok(())
    }

    async fn callback(&self, _code: Option<&str>, _state: Option<&str>, _error: Option<&str>) -> R<()> {
        Ok(())
    }

    async fn disconnect(&self) -> R<Value> {
        Ok(json!({ "connected": false }))
    }

    fn set_app(&self, _client_id: &Value) -> R<Value> {
        Ok(json!({ "app": true }))
    }

    fn begin(&self) -> R<Value> {
        Ok(json!({ "url": null, "demo": true }))
    }

    fn set(&self, device: &Value, duck: &Value) -> R<Value> {
        let mut s = self.settings.locked();
        if !device.is_null() {
            s["device"] = device.clone();
        }
        if !duck.is_null() {
            s["duck"] = json!(xenonet_base::text::truthy(Some(duck)));
        }
        Ok(json!({ "device": s["device"], "duck": s["duck"] }))
    }

    fn duck_plan(&self, waiting: bool, player: &Value, device: &Value, held: &Value) -> R<Value> {
        Ok(plan(waiting, player, device, held))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_demo_plays_and_pauses_and_never_reaches_spotify() {
        let s = DemoSpotify::new();
        let st = s.status().await.unwrap();
        assert_eq!((st["connected"].clone(), st["demo"].clone()), (json!(true), json!(true)));
        let before = s.player().await.unwrap();
        s.command("play", &Value::Null).await.unwrap();
        let after = s.player().await.unwrap();
        assert_ne!(before["playing"], after["playing"]);
        assert_eq!(s.set(&json!("phone"), &json!(true)).unwrap(), json!({ "device": "phone", "duck": true }));
        assert_eq!(s.begin().unwrap(), json!({ "url": null, "demo": true }));
    }

    #[tokio::test]
    async fn without_an_app_nothing_is_asked_of_spotify() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-spotify-{}", std::process::id()));
        let s = SpotifyRemote::new(&dir.join("spotify.json").to_string_lossy(), 47999, Arc::new(Secrets::memory()), Emitter::new());
        let st = s.status().await.unwrap();
        assert_eq!(st["connected"], false);
        assert!(s.set_app(&json!("not an id")).unwrap_err().is(Kind::Spotify));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_dip_while_something_waits() {
        let player = json!({ "playing": true, "device": { "id": "here", "volume": 80 } });
        let p = plan(true, &player, &json!("here"), &Value::Null);
        assert_eq!(p["op"], "volume");
        assert!(p["held"].is_object());
    }
}
