//! core/demo.js's runtime: the made-up world xenonet-app's demo.rs writes
//! (created before anything reads the folders), its sessions as the
//! adapters would list them, and the driver that keeps its live sessions
//! writing, one step every 2.2 seconds. What the demo asks of the rest of
//! XenoNet (git, the identities' ids, the clock) is answered here.

use crate::util::{local, local_day, lock, now_ms};
use serde_json::Value;
use std::sync::Mutex;
use tokio::sync::OnceCell;
use xenonet_app::demo::{repo_steps, Around, Demo};
use xenonet_service::cores::{CoreError, R};
use xenonet_state::identities;

/// What the demo asks of the rest of XenoNet.
pub struct Here;

impl Around for Here {
    fn git_available(&mut self) -> bool {
        crate::git::git_available()
    }
    fn claude_id(&mut self, account: &str, org: &str) -> String {
        identities::claude_id(account, org)
    }
    fn github_id(&mut self, host: &str, login: &str) -> String {
        identities::github_id(host, login)
    }
    fn git_id(&mut self, email: &str) -> String {
        identities::git_id(email)
    }
    fn local(&mut self, ms: f64) -> [i64; 5] {
        local(ms)
    }
    fn random(&mut self) -> f64 {
        let mut b = [0u8; 8];
        let _ = getrandom_fill(&mut b);
        (u64::from_le_bytes(b) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn uuid(&mut self) -> String {
        let mut b = [0u8; 16];
        let _ = getrandom_fill(&mut b);
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        let h = b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().concat();
        format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
    }
    fn now(&mut self) -> f64 {
        now_ms()
    }
}

// The system's randomness, which crypto.randomUUID and Math.random draw on.
fn getrandom_fill(b: &mut [u8]) -> Result<(), getrandom::Error> {
    getrandom::fill(b)
}

pub struct DemoWorld {
    demo: Mutex<Demo>,
    world: Mutex<Option<Value>>,
    clients: OnceCell<()>,
}

impl DemoWorld {
    /// The demo for the instance on `port`: one world per port, so demo
    /// servers running side by side don't wipe each other's.
    pub fn new(port: u16) -> DemoWorld {
        let tmp = std::env::temp_dir();
        let tmp = std::fs::canonicalize(&tmp).unwrap_or(tmp);
        DemoWorld { demo: Mutex::new(Demo::new(&tmp.to_string_lossy(), &port.to_string(), &local_day(now_ms()))), world: Mutex::new(None), clients: OnceCell::new() }
    }

    /// Builds the demo workspace from scratch (once).
    pub fn create(&self) -> R<Value> {
        let mut made = lock(&self.world);
        if let Some(w) = made.as_ref() {
            return Ok(w.clone());
        }
        let w = lock(&self.demo).create_world(now_ms(), &mut Here, crate::config::platform()).map_err(|e| CoreError::new(e.message))?;
        *made = Some(w.clone());
        Ok(w)
    }

    /// The sessions of an app as its adapter would list them.
    pub fn sessions(&self, app: &str) -> Result<Vec<Value>, String> {
        lock(&self.demo).sessions_of(app).map_err(|e| e.message)
    }

    pub fn native_projects(&self) -> Value {
        lock(&self.demo).native_projects()
    }

    pub fn transcript(&self, raw_id: &str) -> Result<Value, String> {
        lock(&self.demo).transcript(raw_id)
    }

    /// Each session's id, app, title and project (the demo's actors').
    pub fn session_list(&self) -> Vec<Value> {
        lock(&self.demo).session_list()
    }

    /// The k-th step of the live sessions' round robin, now.
    pub fn step(&self, k: usize) {
        lock(&self.demo).step_live(k, now_ms(), &mut Here);
    }

    /// The second folder's repos, made the first time something asks for
    /// that folder, side by side.
    pub async fn clients_ready(&self) {
        self.clients
            .get_or_init(|| async {
                let repos = lock(&self.demo).client_repos.clone();
                let steps = repo_steps();
                let runs: Vec<_> = repos
                    .into_iter()
                    .map(|r| {
                        let steps = steps.clone();
                        tokio::spawn(async move {
                            for args in &steps {
                                let ok = tokio::process::Command::new("git")
                                    .args(args)
                                    .current_dir(&r.dir)
                                    .stdin(std::process::Stdio::null())
                                    .stdout(std::process::Stdio::null())
                                    .stderr(std::process::Stdio::null())
                                    .status()
                                    .await
                                    .is_ok_and(|s| s.success());
                                // No git: the projects still show, without
                                // repo status.
                                if !ok {
                                    return;
                                }
                            }
                            for (rel, text) in &r.dirty {
                                let _ = std::fs::write(std::path::Path::new(&r.dir).join(rel), text);
                            }
                        })
                    })
                    .collect();
                for run in runs {
                    let _ = run.await;
                }
            })
            .await;
    }
}

/// The Demo core: createWorld, for the service.
pub struct DemoCore(pub std::sync::Arc<DemoWorld>);

impl xenonet_service::cores::Demo for DemoCore {
    fn create_world(&self) -> R<Value> {
        self.0.create()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_times_as_the_js_makes_them() {
        let u = Here.uuid();
        assert_eq!((u.len(), &u[14..15]), (36, "4"));
        let r = Here.random();
        assert!((0.0..1.0).contains(&r));
        let [y, m, d, h, min] = local(now_ms());
        assert!(y > 2000 && (1..=12).contains(&m) && (1..=31).contains(&d) && (0..24).contains(&h) && (0..60).contains(&min));
    }
}
