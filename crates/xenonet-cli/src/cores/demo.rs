//! core/demo.js's createWorld for the command: the demo's made-up world,
//! in a temporary folder (xenonet-app's demo), with what it asks of the
//! rest of XenoNet answered here: whether git runs, the accounts' ids, the
//! local time, and randomness and the clock.

use crate::clock;
use crate::config::Config;
use crate::out::Fail;
use crate::sys;
use xenonet_app::demo::{Around, Demo};

struct Here;

impl Around for Here {
    fn git_available(&mut self) -> bool {
        // As core/git.js's gitAvailable: git on PATH, and on a Mac, the
        // Command Line Tools behind it.
        let git = super::native::on_path("git");
        if xenonet_sources::git::needs_dev_tools(sys::platform(), git.as_deref()) {
            return git.is_some() && super::native::run("/usr/bin/xcode-select", &["-p".into()], 3000.0, None).ok();
        }
        git.is_some()
    }
    fn claude_id(&mut self, account: &str, org: &str) -> String {
        xenonet_state::identities::claude_id(account, org)
    }
    fn github_id(&mut self, host: &str, login: &str) -> String {
        xenonet_state::identities::github_id(host, login)
    }
    fn git_id(&mut self, email: &str) -> String {
        xenonet_state::identities::git_id(email)
    }
    fn local(&mut self, ms: f64) -> [i64; 5] {
        clock::local(ms).map_or([0; 5], |l| [l.year, i64::from(l.month), i64::from(l.day), i64::from(l.hour), i64::from(l.minute)])
    }
    fn random(&mut self) -> f64 {
        let mut b = [0u8; 8];
        let _ = getrandom::fill(&mut b);
        (u64::from_le_bytes(b) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn uuid(&mut self) -> String {
        let mut b = [0u8; 16];
        let _ = getrandom::fill(&mut b);
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        let h = b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().concat();
        format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
    }
    fn now(&mut self) -> f64 {
        super::native::now()
    }
}

/// os.tmpdir(), its links followed (fs.realpathSync.native).
fn tmpdir() -> String {
    let t = std::env::temp_dir();
    std::fs::canonicalize(&t).unwrap_or(t).to_string_lossy().trim_end_matches(['/', '\\']).to_string()
}

/// Builds the demo workspace from scratch, one per port, as config.js did
/// for --demo, and the config that points at it.
pub fn create_world() -> Result<Config, Fail> {
    let now = super::native::now();
    let today = clock::local(now).map_or_else(String::new, |l| format!("{:04}-{:02}-{:02}", l.year, l.month, l.day));
    let port = std::env::var("XENONET_PORT").unwrap_or_else(|_| "47841".into());
    let mut demo = Demo::new(&tmpdir(), &port, &today);
    demo.create_world(now, &mut Here, sys::platform()).map_err(|e| Fail::error(e.message))?;
    let w = &demo.world;
    Ok(Config {
        state_dir: w.state_dir.clone(),
        cache_dir: w.cache_dir.clone(),
        home_dir: w.home.clone(),
        claude_projects_dir: w.claude_root.clone(),
        codex_home: w.codex_home.clone(),
    })
}
